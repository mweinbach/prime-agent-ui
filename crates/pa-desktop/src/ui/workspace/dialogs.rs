//! Native file dialogs. Read and encode attachments off the render thread.
use rune::{FileDialogOptions, FileDialogResult, Tx};

use super::{BridgeRequest, DraftKey, PathBuf, PrimeDesktopView};

pub(super) enum Dialog {
    AddProject,
    CliFolder,
    AttachFiles(DraftKey),
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

    pub fn prompt_attach_files(&mut self) {
        self.request_dialog(Dialog::AttachFiles(self.draft_key()));
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
            Dialog::AttachFiles(target) => {
                options.title = "Attach files".into();
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
}
