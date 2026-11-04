//! Project navigation and unsent drafts. Only the daemon owns conversation history.
use super::{
    Context, Draft, Path, PathPromptOptions, PrimeDesktopView, SessionSelection, TextInput, Window,
};
use crate::state::projects::ProjectGroup;
use gpui::Focusable as _;

use crate::state::DraftKey;

impl PrimeDesktopView {
    pub(crate) fn current_project(&self) -> &str {
        self.prefs.project_for(&self.working_cwd)
    }

    pub(crate) fn project_groups(&self, query: &str) -> Vec<ProjectGroup> {
        self.prefs.groups(
            self.current_project(),
            &self.active_sessions,
            &self.saved_sessions,
            query,
        )
    }

    pub(crate) fn toggle_project(&mut self, cwd: &str, cx: &mut Context<Self>) {
        if !self.sidebar_search_input.read(cx).text().trim().is_empty() {
            return;
        }
        let default_collapsed =
            self.current_project() != cwd && !self.prefs.project_folders.iter().any(|p| p == cwd);
        let collapsed = self
            .prefs
            .project_collapsed
            .entry(cwd.to_owned())
            .or_insert(default_collapsed);
        *collapsed = !*collapsed;
        self.save_preferences();
        cx.notify();
    }

    pub(crate) fn unpin_project(&mut self, cwd: &str, cx: &mut Context<Self>) {
        if !self.prefs.project_folders.iter().any(|p| p == cwd) {
            return;
        }
        self.prefs.project_folders.retain(|p| p != cwd);
        self.bridge.track_project_action(
            &self.agent_dir,
            Path::new(cwd),
            pa_telemetry::DesktopProjectAction::Removed,
        );
        self.prefs.project_collapsed.remove(cwd);
        self.save_preferences();
        cx.notify();
    }

    pub(crate) fn new_project_chat(
        &mut self,
        cwd: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.selection, SessionSelection::Idle) || cwd.is_empty() {
            return;
        }
        self.auto_attach = false;
        self.stash_draft(cx);
        self.reset_session(cx);
        cwd.clone_into(&mut self.working_cwd);
        self.remember_project();
        self.restore_draft(cx);
        self.sidebar_search_input.update(cx, TextInput::clear);
        window.focus(&self.composer_input.focus_handle(cx));
        cx.notify();
    }

    pub fn start_fresh_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.current_project().to_owned();
        self.new_project_chat(&root, window, cx);
    }

    pub fn prompt_choose_project_dir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add project".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(folder) = paths.first()
            {
                let _ = this.update_in(cx, |view, window, cx| view.add_project(folder, window, cx));
            }
        })
        .detach();
    }

    fn add_project(&mut self, folder: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.selection, SessionSelection::Idle) {
            return;
        }
        let cwd = folder.to_string_lossy().into_owned();
        if !self
            .prefs
            .project_folders
            .iter()
            .any(|p| Path::new(p) == folder)
        {
            self.prefs.project_folders.push(cwd.clone());
            self.bridge.track_project_action(
                &self.agent_dir,
                folder,
                pa_telemetry::DesktopProjectAction::Added,
            );
        }
        self.prefs.last_project = None;
        self.new_project_chat(&cwd, window, cx);
    }

    pub(super) fn remember_project(&mut self) {
        let root = self.current_project().to_owned();
        let expand = self.prefs.project_collapsed.get(&root) == Some(&true);
        if expand {
            self.prefs.project_collapsed.insert(root.clone(), false);
        }
        if expand || self.prefs.last_project.as_ref() != Some(&root) {
            self.prefs.last_project = Some(root);
            self.save_preferences();
        }
    }

    pub(super) fn draft_key(&self) -> DraftKey {
        if self.session.active_session_id.is_empty() {
            DraftKey::Project(self.working_cwd.clone())
        } else {
            DraftKey::Session(self.session.active_session_id.clone())
        }
    }

    pub(super) fn stash_draft(&mut self, cx: &mut Context<Self>) {
        let text = self.composer_input.read(cx).text();
        if text.is_empty() && self.staged_images.is_empty() {
            return;
        }
        let draft: Draft = (text.to_owned(), std::mem::take(&mut self.staged_images));
        self.drafts
            .entry(self.draft_key())
            .or_default()
            .push_front(draft);
        self.composer_input.update(cx, TextInput::clear);
    }

    pub(super) fn restore_draft(&mut self, cx: &mut Context<Self>) {
        let key = self.draft_key();
        if self.composer_input.read(cx).text().is_empty()
            && self.staged_images.is_empty()
            && let Some(drafts) = self.drafts.get_mut(&key)
            && let Some((text, images)) = drafts.pop_front()
        {
            if drafts.is_empty() {
                self.drafts.remove(&key);
            }
            self.composer_input
                .update(cx, |input, cx| input.set_text(text, cx));
            self.staged_images = images;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::DesktopPreferences;
    use crate::ui::workspace::LaunchOptions;

    #[test]
    fn adding_the_same_folder_twice_keeps_one_project_and_restores_it_at_launch() {
        let root = tempfile::tempdir().unwrap();
        let agent = root.path().join("agent");
        let folder = root.path().join("repo");
        std::fs::create_dir(&folder).unwrap();
        let options = LaunchOptions {
            daemon_socket: Some(root.path().join("no-listener")),
            ..LaunchOptions::default()
        };
        let mut app = gpui::TestAppContext::single();
        let (view, cx) = app.add_window_view(|window, cx| {
            PrimeDesktopView::new(options.clone(), agent.clone(), window, cx)
        });
        view.update_in(cx, |view, window, cx| {
            view.add_project(&folder, window, cx);
            view.add_project(&folder, window, cx);
            assert!(view.session.active_session_id.is_empty());
        });
        let prefs = DesktopPreferences::load(&agent);
        assert_eq!(prefs.project_folders, [folder.to_str().unwrap()]);
        let (reopened, cx) =
            app.add_window_view(|window, cx| PrimeDesktopView::new(options, agent, window, cx));
        reopened.read_with(cx, |view, _| {
            assert_eq!(Path::new(view.current_project()), folder);
        });
        app.quit();
    }
}
