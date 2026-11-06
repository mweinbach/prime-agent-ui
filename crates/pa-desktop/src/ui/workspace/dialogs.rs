//! Native file dialogs. Read and encode attachments off the render thread.
use rune::{FileDialogOptions, FileDialogResult, Tx};

use super::{
    BridgeRequest, DraftKey, MAX_ATTACHMENT_BYTES, MAX_ATTACHMENTS, PathBuf, PrimeDesktopView,
    read_image_file,
};

pub(super) enum Dialog {
    AddProject,
    CliFolder,
    AttachImage(DraftKey),
    Export(String),
}

impl PrimeDesktopView {
    fn request_dialog(&mut self, dialog: Dialog) {
        self.pending_dialog = Some(dialog);
        self.waker.wake_by_ref();
    }

    pub fn prompt_choose_project_dir(&mut self) {
        self.request_dialog(Dialog::AddProject);
    }

    pub(crate) fn prompt_choose_cli_dir(&mut self) {
        self.request_dialog(Dialog::CliFolder);
    }

    pub fn prompt_attach_images(&mut self) {
        self.request_dialog(Dialog::AttachImage(self.draft_key()));
    }

    pub fn prompt_export_html(&mut self) {
        if !self.session.active_session_id.is_empty() {
            self.request_dialog(Dialog::Export(self.session.active_session_id.clone()));
        }
    }

    pub(super) fn show_dialog(&self, dialog: Dialog, tx: &mut Tx) {
        let mut options = FileDialogOptions {
            directory: Some(PathBuf::from(&self.working_cwd)),
            ..FileDialogOptions::default()
        };
        match dialog {
            Dialog::AddProject => {
                options.title = "Add project".into();
                tx.open_folder(options, |s: &mut Self, result, _| {
                    if let Some(path) = s.dialog_path(result) {
                        s.add_project(&path);
                    }
                });
            }
            Dialog::CliFolder => {
                options.title = "Use CLI data folder".into();
                options.directory = Some(self.agent_dir.clone());
                tx.open_folder(options, |s: &mut Self, result, _| {
                    if let Some(path) = s.dialog_path(result) {
                        s.use_cli_folder(Some(path));
                    }
                });
            }
            Dialog::AttachImage(target) => {
                options.title = "Attach image".into();
                let options = options.filter("Images", &["png", "jpg", "jpeg", "webp", "gif"]);
                tx.open_file(options, move |s: &mut Self, result, _| {
                    if let Some(path) = s.dialog_path(result)
                        && target == s.draft_key()
                    {
                        s.attach_files(vec![path]);
                    }
                });
            }
            Dialog::Export(active_session_id) => {
                options.title = "Export session".into();
                options.file_name = Some("prime-agent-session.html".into());
                tx.save_file(options, move |s: &mut Self, result, _| {
                    if let Some(output_path) = s.dialog_path(result) {
                        s.bridge.send(BridgeRequest::ExportSessionHtml {
                            active_session_id,
                            output_path,
                        });
                    }
                });
            }
        }
    }

    fn dialog_path(&mut self, result: FileDialogResult) -> Option<PathBuf> {
        match result {
            Ok(path) => path,
            Err(err) => {
                self.toast = Some((format!("File picker: {err}"), true));
                None
            }
        }
    }

    pub(in crate::ui) fn attach_files(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        if paths.len() > MAX_ATTACHMENTS.saturating_sub(self.staged_images.len()) {
            self.toast = Some((
                format!("Attach up to {MAX_ATTACHMENTS} images per prompt."),
                true,
            ));
            return;
        }
        let target = self.draft_key();
        let bytes = self.staged_images.iter().map(|image| image.byte_len).sum();
        let mut remaining = MAX_ATTACHMENT_BYTES.saturating_sub(bytes);
        self.background.run(&self.bridge, move || {
            let loaded = paths
                .iter()
                .map(|path| {
                    let image = read_image_file(path, remaining)?;
                    remaining -= image.byte_len;
                    Ok(image)
                })
                .collect::<anyhow::Result<Vec<_>>>();
            move |view: &mut Self| {
                if target != view.draft_key() {
                    return;
                }
                let bytes: usize = view.staged_images.iter().map(|i| i.byte_len).sum();
                match loaded {
                    Ok(images)
                        if view.staged_images.len() + images.len() <= MAX_ATTACHMENTS
                            && images.iter().map(|i| i.byte_len).sum::<usize>()
                                <= MAX_ATTACHMENT_BYTES.saturating_sub(bytes) =>
                    {
                        view.staged_images.extend(images);
                    }
                    Ok(_) => {
                        view.toast =
                            Some(("Attachments exceed the limit for one prompt.".into(), true));
                    }
                    Err(err) => view.toast = Some((err.to_string(), true)),
                }
            }
        });
    }
}
