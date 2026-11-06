//! File loads and previews belong to the draft that requested them.
use std::path::PathBuf;

use uuid::Uuid;

use super::{ActiveModal, DraftKey, PrimeDesktopView, SessionSelection, StagedAttachment};
use crate::state::attachments::{
    MAX_ATTACHMENT_BYTES, MAX_ATTACHMENTS, MAX_PROMPT_BYTES, prompt_size,
};

impl PrimeDesktopView {
    #[must_use]
    pub fn attachments_loading(&self) -> bool {
        self.pending_attachments.contains(&self.draft_key())
    }

    #[must_use]
    pub fn selected_attachment_preview(&self) -> Option<&StagedAttachment> {
        let id = self.attachment_preview_id?;
        self.staged_attachments.iter().find(|a| a.id == id)
    }

    pub fn open_attachment_preview(&mut self, id: Uuid) {
        if self.attachment_preview_id != Some(id)
            && self.staged_attachments.iter().any(|a| a.id == id)
        {
            self.close_attachment_preview();
            self.reasoning_picker_open = false;
            self.attachment_preview_id = Some(id);
            self.load_expanded_preview();
        }
    }

    pub fn close_attachment_preview(&mut self) {
        self.attachment_preview_id = None;
        self.expanded_attachment = None;
        self.attachment_preview_error = None;
    }

    #[must_use]
    pub fn expanded_attachment_pixels(&self, id: Uuid) -> Option<&rune::Pixels> {
        self.expanded_attachment
            .as_ref()
            .filter(|(selected, _)| *selected == id)
            .map(|(_, pixels)| pixels)
    }

    #[must_use]
    pub fn selected_attachment_preview_error(&self) -> Option<&str> {
        self.attachment_preview_error.as_deref()
    }

    fn load_expanded_preview(&mut self) {
        if self.image_decode_in_progress {
            return;
        }
        let Some(attachment) = self.selected_attachment_preview() else {
            return;
        };
        let Some((mime, bytes)) = attachment.image_source() else {
            return;
        };
        let id = attachment.id;
        self.image_decode_in_progress = true;
        self.background.run(&self.bridge, move || {
            let expanded = crate::state::image_preview::decode_expanded(&bytes, mime);
            move |view: &mut Self| {
                view.image_decode_in_progress = false;
                if view.attachment_preview_id == Some(id) {
                    match expanded {
                        Ok(pixels) => view.expanded_attachment = Some((id, pixels)),
                        Err(err) => {
                            view.attachment_preview_error =
                                Some(format!("Could not load the larger preview: {err}"));
                        }
                    }
                } else {
                    // Switches coalesce: decode only the latest selection when the
                    // current job releases its buffers.
                    view.load_expanded_preview();
                }
            }
        });
    }

    pub fn remove_attachment(&mut self, id: Uuid) {
        self.staged_attachments.retain(|a| a.id != id);
        if self.attachment_preview_id == Some(id) {
            self.close_attachment_preview();
        }
    }

    pub fn attach_files(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        if !matches!(self.selection, SessionSelection::Idle) {
            self.toast = Some((
                "Wait for the chat to open before attaching files.".into(),
                false,
            ));
            return;
        }
        if self.attachments_loading() || self.pending_attachments.len() >= 2 {
            self.toast = Some((
                "Files are still loading. Add more when they finish.".into(),
                false,
            ));
            return;
        }
        if paths.len() > MAX_ATTACHMENTS.saturating_sub(self.staged_attachments.len()) {
            self.toast = Some((
                format!("Attach up to {MAX_ATTACHMENTS} files per prompt."),
                true,
            ));
            return;
        }
        let target = self.draft_key();
        let used = self.staged_attachments.iter().map(|a| a.byte_len).sum();
        let mut remaining = MAX_ATTACHMENT_BYTES.saturating_sub(used);
        let mut text_bytes = prompt_size("", &self.staged_attachments);
        self.pending_attachments.insert(target.clone());
        self.background.run(&self.bridge, move || {
            let loaded = paths
                .iter()
                .map(|path| {
                    let file = StagedAttachment::read(path, remaining)?;
                    remaining -= file.byte_len;
                    text_bytes += prompt_size("", std::slice::from_ref(&file));
                    anyhow::ensure!(
                        text_bytes <= MAX_PROMPT_BYTES,
                        "Attached text exceeds the 1 MiB prompt limit."
                    );
                    Ok(file)
                })
                .collect::<anyhow::Result<Vec<_>>>();
            move |view: &mut Self| view.finish_attachment_load(target, loaded)
        });
    }

    fn finish_attachment_load(
        &mut self,
        target: DraftKey,
        loaded: anyhow::Result<Vec<StagedAttachment>>,
    ) {
        self.pending_attachments.remove(&target);
        match loaded {
            Ok(files) => {
                let attachments = if target == self.draft_key()
                    && matches!(self.selection, SessionSelection::Idle)
                {
                    &mut self.staged_attachments
                } else {
                    // stash_draft also saves an empty placeholder for an in-flight
                    // batch. Rejected prompts wait behind that draft in the deque.
                    let drafts = self.drafts.entry(target).or_default();
                    if drafts.is_empty() {
                        drafts.push_front(Default::default());
                    }
                    &mut drafts.front_mut().unwrap().1
                };
                let byte_len: usize = attachments.iter().chain(&files).map(|a| a.byte_len).sum();
                if attachments.len() + files.len() <= MAX_ATTACHMENTS
                    && byte_len <= MAX_ATTACHMENT_BYTES
                    && prompt_size("", attachments) + prompt_size("", &files) <= MAX_PROMPT_BYTES
                {
                    attachments.extend(files);
                } else {
                    self.toast = Some((
                        "Files exceed the attachment limit for one prompt.".into(),
                        true,
                    ));
                }
            }
            Err(error) => {
                // The picker may have stashed an empty draft while this batch was
                // running. A failed load must not leave a phantom sidebar chat.
                if let Some(drafts) = self.drafts.get_mut(&target) {
                    drafts.retain(|(text, files)| !text.is_empty() || !files.is_empty());
                    if drafts.is_empty() {
                        self.drafts.remove(&target);
                    }
                }
                self.toast = Some((format!("{error:#}"), true));
            }
        }
        if self.reasoning_picker_open {
            self.open_reasoning_picker();
        } else if self.active_modal == Some(ActiveModal::ModelPicker) {
            self.prepare_model_catalog();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{BridgeEvent, BridgeRequest};
    use crate::ui::workspace::LaunchOptions;

    #[test]
    fn in_flight_files_cannot_be_sent_or_moved_to_the_new_chat() {
        let profile = tempfile::tempdir().unwrap();
        let cwd = profile.path().to_string_lossy().into_owned();
        let mut view = PrimeDesktopView::new(
            LaunchOptions {
                daemon_socket: Some(profile.path().join("offline.sock")),
                initial_cwd: Some(profile.path().into()),
                ..Default::default()
            },
            profile.path().into(),
        );
        let path = profile.path().join("draft.txt");
        std::fs::write(&path, b"snapshot").unwrap();
        let attachment = StagedAttachment::read(&path, MAX_ATTACHMENT_BYTES).unwrap();
        let id = attachment.id;
        let original = DraftKey::Project(cwd.clone());
        view.daemon_connected = true;
        view.pending_attachments.insert(original.clone());
        view.populate_draft("keep the message");
        view.submit_composer();
        assert_eq!(view.composer_input.text, "keep the message");
        assert!(matches!(view.selection, SessionSelection::Idle));
        view.open_session(BridgeRequest::AttachSession {
            active_session_id: "other".into(),
        });
        // Until the daemon attaches, draft_key() still identifies the old chat:
        // an arriving batch belongs in its stash, not the visible composer.
        view.finish_attachment_load(original, Ok(vec![attachment]));
        assert!(view.staged_attachments.is_empty());
        view.apply_bridge_event(BridgeEvent::SessionAttached {
            active_session_id: "other".into(),
            snapshot: serde_json::json!({}),
        });
        assert!(view.staged_attachments.is_empty());
        view.new_project_chat(&cwd);
        assert_eq!(view.composer_input.text, "keep the message");
        assert_eq!(view.staged_attachments[0].id, id);
        view.open_attachment_preview(id);
        view.remove_attachment(id);
        assert!(view.selected_attachment_preview().is_none());
        assert!(view.staged_attachments.is_empty());
    }

    #[test]
    fn a_failed_load_in_a_different_project_leaves_no_empty_draft() {
        let profile = tempfile::tempdir().unwrap();
        let mut view = PrimeDesktopView::new(
            LaunchOptions {
                daemon_socket: Some(profile.path().join("offline.sock")),
                initial_cwd: Some(profile.path().into()),
                ..Default::default()
            },
            profile.path().into(),
        );
        let original = view.draft_key();
        view.pending_attachments.insert(original.clone());
        view.new_project_chat("/another");
        assert!(view.drafts.contains_key(&original));
        view.finish_attachment_load(original.clone(), Err(anyhow::anyhow!("unsupported")));
        assert!(!view.drafts.contains_key(&original));
    }

    #[test]
    fn rejected_prompt_restores_typed_message_and_clickable_attachment() {
        let profile = tempfile::tempdir().unwrap();
        let mut view = PrimeDesktopView::new(
            LaunchOptions {
                daemon_socket: Some(profile.path().join("offline.sock")),
                ..Default::default()
            },
            profile.path().into(),
        );
        let path = profile.path().join("test.txt");
        std::fs::write(&path, b"unchanged").unwrap();
        let file = StagedAttachment::read(&path, MAX_ATTACHMENT_BYTES).unwrap();
        let id = file.id;
        view.apply_bridge_event(BridgeEvent::PromptRejected {
            draft: view.draft_key(),
            message: "Try later".into(),
            attachments: vec![file],
            reason: "refused".into(),
        });
        assert_eq!(view.composer_input.text, "Try later");
        view.open_attachment_preview(id);
        assert_eq!(view.selected_attachment_preview().unwrap().id, id);
    }
}
