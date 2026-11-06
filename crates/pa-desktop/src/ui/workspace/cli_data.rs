//! Switch the desktop to a different CLI profile without copying its credentials.
use super::{ActiveModal, DaemonTarget, LaunchOptions, Path, PathBuf, PrimeDesktopView};

const UNSAVED_EDITS: &str =
    "Finish editing drafts and credentials before changing the CLI data folder.";

impl PrimeDesktopView {
    fn has_unsaved_edits(&self) -> bool {
        self.auth_write_pending
            || !self.composer_input.text.is_empty()
            || !self.staged_attachments.is_empty()
            || !self.pending_attachments.is_empty()
            || self.drafts.values().any(|drafts| !drafts.is_empty())
            || self.harness_editor_open
    }

    pub(crate) fn use_cli_folder(&mut self, selected: Option<PathBuf>) {
        if selected.as_ref().unwrap_or(&self.preferences_dir) == &self.agent_dir {
            return;
        }
        if self.has_unsaved_edits() {
            self.toast = Some((UNSAVED_EDITS.into(), true));
            return;
        }
        self.background.run(&self.bridge, move || {
            let valid = selected.as_deref().is_none_or(is_cli_folder);
            move |view: &mut Self| {
                if valid {
                    view.switch_cli_folder(selected);
                } else {
                    view.toast = Some((
                        "Choose the CLI data folder that contains sessions, settings.json or auth.json.".into(),
                        true,
                    ));
                }
            }
        });
    }

    fn switch_cli_folder(&mut self, selected: Option<PathBuf>) {
        // The user could have edited a draft while the directory was being checked.
        if self.has_unsaved_edits() {
            self.toast = Some((UNSAVED_EDITS.into(), true));
            return;
        }
        let mut prefs = self.prefs.clone();
        prefs.cli_data_dir = selected.filter(|dir| dir != &self.preferences_dir);
        prefs.daemon_target = DaemonTarget::DesktopHarness;
        if let Err(err) = prefs.save(&self.preferences_dir) {
            self.toast = Some((format!("Could not save the CLI data folder: {err}"), true));
            return;
        }
        let options = LaunchOptions {
            initial_cwd: Some(PathBuf::from(&self.working_cwd)),
            initial_modal: Some(ActiveModal::SettingsAuth),
            ..LaunchOptions::default()
        };
        let mut next = Self::new(options, self.preferences_dir.clone());
        next.waker.clone_from(&self.waker);
        next.provider_browser = self.provider_browser;
        *self = next;
        self.waker.wake_by_ref();
    }
}

fn is_cli_folder(path: &Path) -> bool {
    path.join("sessions").is_dir()
        || path.join("harness").is_dir()
        || ["settings.json", "auth.json", "models.json"]
            .iter()
            .any(|name| path.join(name).is_file())
}
