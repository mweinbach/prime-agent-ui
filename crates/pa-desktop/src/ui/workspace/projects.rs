//! Project navigation and per-session drafts.
use super::{Draft, DraftKey, Path, PrimeDesktopView, SessionSelection};
use crate::state::projects::ProjectGroup;

impl PrimeDesktopView {
    pub(crate) fn current_project(&self) -> &str {
        self.prefs.project_for(&self.working_cwd)
    }

    pub(crate) fn project_groups(&self, query: &str) -> Vec<ProjectGroup> {
        self.prefs.groups(
            self.current_project(),
            &self.active_sessions,
            &self.saved_sessions,
            self.drafts
                .keys()
                .filter_map(|key| match key {
                    DraftKey::Project(cwd) => Some(cwd.as_str()),
                    DraftKey::Session(_) => None,
                })
                .chain(
                    (self.session.active_session_id.is_empty()
                        && (!self.composer_input.text.is_empty()
                            || !self.staged_images.is_empty()))
                    .then_some(self.working_cwd.as_str()),
                ),
            query,
        )
    }

    pub(crate) fn toggle_project(&mut self, cwd: &str) {
        if !self.sidebar_search_input.text.trim().is_empty() {
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
    }

    pub(crate) fn pin_project(&mut self, cwd: &str) {
        let folder = Path::new(cwd);
        if cwd.is_empty()
            || self
                .prefs
                .project_folders
                .iter()
                .any(|p| Path::new(p) == folder)
        {
            return;
        }
        self.prefs.project_folders.push(cwd.to_owned());
        self.bridge.track_project_action(
            &self.agent_dir,
            folder,
            pa_telemetry::DesktopProjectAction::Added,
        );
        self.save_preferences();
    }

    pub(crate) fn unpin_project(&mut self, cwd: &str) {
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
        if self.sidebar_project.as_deref() == Some(cwd) {
            self.show_all_projects();
        }
        self.save_preferences();
    }

    pub(crate) fn new_project_chat(&mut self, cwd: &str) {
        if !matches!(self.selection, SessionSelection::Idle) || cwd.is_empty() {
            return;
        }
        self.close_sidebar_overlay();
        self.reasoning_picker_open = false;
        self.auto_attach = false;
        self.stash_draft();
        self.reset_session();
        cwd.clone_into(&mut self.working_cwd);
        self.remember_project();
        if self
            .sidebar_project
            .as_deref()
            .is_some_and(|scoped| scoped != self.current_project())
        {
            let root = self.current_project().to_owned();
            self.focus_sidebar_project(&root);
        }
        self.restore_draft();
        self.sidebar_search_input.replace_text("");
        self.focus_input(|s| &mut s.composer_input);
    }

    pub fn start_fresh_session(&mut self) {
        let project = if self.prefs.sidebar_open {
            self.sidebar_project
                .as_deref()
                .unwrap_or_else(|| self.current_project())
        } else {
            self.current_project()
        }
        .to_owned();
        self.new_project_chat(&project);
    }

    pub(super) fn add_project(&mut self, folder: &Path) {
        if !matches!(self.selection, SessionSelection::Idle) {
            return;
        }
        let cwd = folder.to_string_lossy().into_owned();
        self.pin_project(&cwd);
        self.new_project_chat(&cwd);
        self.focus_sidebar_project(&cwd);
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

    pub(super) fn stash_draft(&mut self) {
        let text = self.composer_input.text.as_str();
        if text.is_empty() && self.staged_images.is_empty() {
            return;
        }
        let draft: Draft = (text.to_owned(), std::mem::take(&mut self.staged_images));
        self.drafts
            .entry(self.draft_key())
            .or_default()
            .push_front(draft);
        self.composer_input.replace_text("");
    }

    pub(super) fn restore_draft(&mut self) {
        let key = self.draft_key();
        if self.composer_input.text.is_empty()
            && self.staged_images.is_empty()
            && let Some(drafts) = self.drafts.get_mut(&key)
            && let Some((text, images)) = drafts.pop_front()
        {
            if drafts.is_empty() {
                self.drafts.remove(&key);
            }
            self.composer_input.replace_text(text);
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
    fn pinning_history_keeps_the_open_session_and_unsent_prompt() {
        let root = tempfile::tempdir().unwrap();
        let agent = root.path().join("agent");
        let here = "/work/current";
        let history = "/work/older";
        let options = LaunchOptions {
            initial_cwd: Some(here.into()),
            daemon_socket: Some(root.path().join("no-listener")),
            ..LaunchOptions::default()
        };
        let mut view = PrimeDesktopView::new(options, agent.clone());
        {
            view.saved_sessions.push(
                crate::state::SavedSessionRow::from_wire(&serde_json::json!({
                    "id": "old", "path": "/history/old.jsonl", "cwd": history,
                }))
                .unwrap(),
            );
            view.remember_project();
            view.session.active_session_id = "resident".into();
            view.populate_draft("keep my changes");
            view.sidebar_search_input.replace_text("older");
            let found = view.project_groups("older");
            assert!(!found[0].is_project(view.current_project()));
            view.pin_project(history);
            view.pin_project(history);
            let pinned = view.project_groups("older");
            assert!(pinned[0].is_project(view.current_project()) && pinned[0].added);
            assert_eq!(pinned[0].chats, found[0].chats);
            assert_eq!(view.session.active_session_id, "resident");
            assert_eq!(view.working_cwd, here);
            assert_eq!(view.composer_input.text.as_str(), "keep my changes");
            assert_eq!(view.sidebar_search_input.text.as_str(), "older");
        }
        let prefs = DesktopPreferences::load(&agent);
        assert_eq!(prefs.project_folders, [history]);
        assert_eq!(prefs.last_project.as_deref(), Some(here));
    }

    #[test]
    fn unsent_project_chat_survives_regrouping_and_reopens_at_its_exact_cwd() {
        let root = tempfile::tempdir().unwrap();
        let checkout = "/work/app/checkout";
        let elsewhere = "/elsewhere";
        let options = LaunchOptions {
            initial_cwd: Some(checkout.into()),
            daemon_socket: Some(root.path().join("no-listener")),
            ..LaunchOptions::default()
        };
        let mut view = PrimeDesktopView::new(options, root.path().join("agent"));
        {
            view.populate_draft("resume from checkout");
            view.new_project_chat(elsewhere);
            view.pin_project("/work/app");
            let groups = view.project_groups("");
            let current = groups
                .iter()
                .find(|g| g.folder() == Some(elsewhere))
                .unwrap();
            assert!(current.draft_cwds.is_empty());
            let project = groups.iter().find(|g| g.added).unwrap();
            assert_eq!(project.draft_cwds, [checkout]);
            view.unpin_project("/work/app");
            let history = view.project_groups("checkout");
            assert_eq!(history[0].folder(), Some(checkout));
            assert!(!history[0].is_project(elsewhere));
            assert_eq!(history[0].draft_cwds, [checkout]);
            view.new_project_chat(&history[0].draft_cwds[0]);
            assert_eq!(view.working_cwd, checkout);
            assert_eq!(view.composer_input.text.as_str(), "resume from checkout");
            assert!(view.session.active_session_id.is_empty());
        }
    }

    #[test]
    fn new_chat_uses_the_browsed_project_only_when_the_sidebar_is_open() {
        let root = tempfile::tempdir().unwrap();
        let here = "/work/current";
        let other = "/work/other";
        let options = LaunchOptions {
            initial_cwd: Some(here.into()),
            daemon_socket: Some(root.path().join("no-listener")),
            ..LaunchOptions::default()
        };
        let mut view = PrimeDesktopView::new(options, root.path().join("agent"));
        {
            view.pin_project(other);
            view.focus_sidebar_project(other);
            assert_eq!(view.working_cwd, here);
            view.prefs.sidebar_open = false;
            view.start_fresh_session();
            assert_eq!(view.working_cwd, here);
            view.prefs.sidebar_open = true;
            view.focus_sidebar_project(other);
            view.start_fresh_session();
            assert_eq!(view.working_cwd, other);
            view.unpin_project(other);
            assert!(view.sidebar_project.is_none());
            assert_eq!(view.working_cwd, other);
        }
    }

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
        let mut view = PrimeDesktopView::new(options.clone(), agent.clone());
        {
            view.add_project(&folder);
            view.add_project(&folder);
            assert!(view.session.active_session_id.is_empty());
            assert_eq!(view.sidebar_project.as_deref(), folder.to_str());
        }
        let prefs = DesktopPreferences::load(&agent);
        assert_eq!(prefs.project_folders, [folder.to_str().unwrap()]);
        let reopened = PrimeDesktopView::new(options, agent);
        assert_eq!(Path::new(reopened.current_project()), folder);
    }
}
