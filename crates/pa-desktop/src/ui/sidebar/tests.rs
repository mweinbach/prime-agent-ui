use crate::state::SavedSessionRow;
use crate::theme::DesktopPreferences;
use crate::ui::workspace::{LaunchOptions, PrimeDesktopView};
use rune::{Env, Runtime};
use serde_json::json;
use std::path::Path;

const ALPHA: &str = "/work/alpha";
const BETA: &str = "/work/beta";
const ARCHIVE: &str = "/work/archive";

fn saved(cwd: &str, name: &str) -> SavedSessionRow {
    SavedSessionRow::from_wire(
        &json!({"id":name,"name":name,"cwd":cwd,"path":format!("/chats/{name}.jsonl")}),
    )
    .unwrap()
}

fn open_sidebar(profile: &Path, rows: Vec<SavedSessionRow>) -> Runtime<PrimeDesktopView> {
    let mut state = PrimeDesktopView::new(
        LaunchOptions {
            daemon_socket: Some(profile.join("no-listener")),
            initial_cwd: Some(ALPHA.into()),
            ..LaunchOptions::default()
        },
        profile.to_owned(),
    );
    state.prefs.project_folders = vec![ALPHA.into(), BETA.into()];
    state.prefs.project_collapsed.insert(BETA.into(), true);
    state.saved_sessions = rows;
    let mut app = Runtime::new(state, PrimeDesktopView::view, (1360., 860.), Env::default());
    draw(&mut app);
    app
}

fn draw(app: &mut Runtime<PrimeDesktopView>) {
    app.settle_all();
    app.frame(1.0 / 60.0);
}

fn visible(app: &Runtime<PrimeDesktopView>, key: &str) -> Option<[f32; 4]> {
    app.elements()
        .into_iter()
        .find(|node| {
            node.key.as_deref() == Some(key)
                && !node.exiting
                && node.visible[2] > 0.0
                && node.visible[3] > 0.0
        })
        .map(|node| node.visible)
}

fn click(app: &mut Runtime<PrimeDesktopView>, key: &str) {
    draw(app);
    let [x, y, w, h] = visible(app, key).unwrap_or_else(|| panic!("missing {key}"));
    app.pointer_down(x + w / 2.0, y + h / 2.0);
    app.pointer_up(x + w / 2.0, y + h / 2.0);
    draw(app);
}

fn search(app: &mut Runtime<PrimeDesktopView>, query: &str) {
    app.update(|s, _| s.sidebar_search_input.replace_text(query));
    draw(app);
}

#[test]
fn browsing_a_project_leaves_the_current_draft_until_compose_is_clicked() {
    let profile = tempfile::tempdir().unwrap();
    let mut app = open_sidebar(
        profile.path(),
        vec![saved(ALPHA, "original"), saved(BETA, "destination")],
    );
    app.update(|s, _| s.populate_draft("/model"));
    click(&mut app, "project-focus-/work/beta");
    assert_eq!(app.state.working_cwd, ALPHA);
    assert_eq!(app.state.composer_input.text, "/model");
    assert!(app.state.active_modal.is_none());
    assert_eq!(app.state.sidebar_project.as_deref(), Some(BETA));
    assert!(visible(&app, "saved-chat-/chats/destination.jsonl").is_some());
    search(&mut app, "original");
    assert!(visible(&app, "sidebar-empty").is_some());
    click(&mut app, "sidebar-search-clear");
    assert!(app.state.sidebar_search_input.text.is_empty());
    click(&mut app, "sidebar-compose");
    assert_eq!(app.state.working_cwd, BETA);
    assert!(app.state.composer_input.text.is_empty());
    assert!(app.state.session.active_session_id.is_empty());
    click(&mut app, "sidebar-all-projects");
    assert!(app.state.sidebar_project.is_none());
    click(&mut app, "sidebar-draft-/work/alpha");
    assert_eq!(app.state.working_cwd, ALPHA);
    assert_eq!(app.state.composer_input.text, "/model");
}

#[test]
fn search_reaches_beyond_the_project_preview_and_history_projects_can_be_pinned() {
    let profile = tempfile::tempdir().unwrap();
    let mut rows: Vec<_> = (1..=5)
        .map(|i| saved(ALPHA, &format!("Chat {i}")))
        .collect();
    rows.extend([
        saved(ALPHA, "buried needle"),
        saved(ALPHA, "seventh"),
        saved(ARCHIVE, "older work"),
    ]);
    let mut app = open_sidebar(profile.path(), rows);
    assert!(visible(&app, "project-more-/work/alpha").is_some());
    assert!(visible(&app, "saved-chat-/chats/buried needle.jsonl").is_none());
    assert!(visible(&app, "project-focus-/work/archive").is_none());
    search(&mut app, "needle");
    assert!(visible(&app, "saved-chat-/chats/buried needle.jsonl").is_some());
    click(&mut app, "sidebar-search-clear");
    click(&mut app, "project-more-/work/alpha");
    assert_eq!(app.state.sidebar_project.as_deref(), Some(ALPHA));
    assert!(visible(&app, "saved-chat-/chats/seventh.jsonl").is_some());
    click(&mut app, "sidebar-all-projects");
    click(&mut app, "project-toggle-/work/alpha");
    assert_eq!(app.state.prefs.project_collapsed.get(ALPHA), Some(&true));
    click(&mut app, "sidebar-history");
    click(&mut app, "project-pin-/work/archive");
    let prefs = DesktopPreferences::load(profile.path());
    assert!(prefs.project_folders.iter().any(|p| p == ARCHIVE));
    assert_eq!(app.state.working_cwd, ALPHA);
}

#[test]
fn narrow_drawers_are_exclusive_and_opening_a_chat_returns_focus_to_the_composer() {
    let profile = tempfile::tempdir().unwrap();
    let mut app = open_sidebar(profile.path(), vec![saved(ALPHA, "history")]);
    app.set_viewport((700.0, 640.0), 1.0);
    draw(&mut app);
    draw(&mut app);
    assert!(app.state.prefs.sidebar_open);
    app.update(|s, _| s.toggle_inspector());
    assert!(app.state.prefs.inspector_open);
    assert!(!app.state.prefs.sidebar_open);
    app.update(|s, _| s.toggle_sidebar());
    assert!(app.state.prefs.sidebar_open);
    assert!(!app.state.prefs.inspector_open);
    click(&mut app, "sidebar-search");
    assert!(app.state.sidebar_search_input.focused);
    click(&mut app, "saved-chat-/chats/history.jsonl");
    assert!(!app.state.prefs.sidebar_open);
    assert!(!app.state.prefs.inspector_open);
    assert!(app.state.composer_input.focused);
}
