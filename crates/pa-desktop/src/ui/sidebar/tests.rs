use std::path::Path;

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use serde_json::json;

use crate::state::SavedSessionRow;
use crate::theme::DesktopPreferences;
use crate::ui::workspace::{LaunchOptions, PrimeDesktopView};

const ALPHA: &str = "/work/alpha";
const BETA: &str = "/work/beta";
const ARCHIVE: &str = "/work/archive";

fn saved(cwd: &str, name: &str) -> SavedSessionRow {
    SavedSessionRow::from_wire(&json!({
        "id": name, "name": name, "cwd": cwd, "path": format!("/chats/{name}.jsonl"),
    }))
    .unwrap()
}

fn open_sidebar<'a>(
    app: &'a mut TestAppContext,
    profile: &Path,
    rows: Vec<SavedSessionRow>,
) -> (Entity<PrimeDesktopView>, &'a mut VisualTestContext) {
    app.add_window_view(|window, cx| {
        let mut view = PrimeDesktopView::new(
            LaunchOptions {
                daemon_socket: Some(profile.join("no-listener")),
                initial_cwd: Some(ALPHA.into()),
                ..LaunchOptions::default()
            },
            profile.to_owned(),
            window,
            cx,
        );
        view.prefs.project_folders = vec![ALPHA.into(), BETA.into()];
        view.prefs.project_collapsed.insert(BETA.into(), true);
        view.saved_sessions = rows;
        view
    })
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear());
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    draw(cx);
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    cx.simulate_click(bounds.center(), Modifiers::default());
    draw(cx);
}

fn search(cx: &mut VisualTestContext, view: &Entity<PrimeDesktopView>, query: &str) {
    view.update_in(cx, |view, _, cx| {
        view.sidebar_search_input
            .update(cx, |input, cx| input.set_text(query, cx));
    });
    draw(cx);
}

#[test]
fn browsing_a_project_leaves_the_current_draft_until_compose_is_clicked() {
    let profile = tempfile::tempdir().unwrap();
    let mut app = TestAppContext::single();
    let (view, cx) = open_sidebar(
        &mut app,
        profile.path(),
        vec![saved(ALPHA, "original"), saved(BETA, "destination")],
    );
    // This command would submit locally even without a connected daemon.
    view.update_in(cx, |view, window, cx| {
        view.populate_draft("/model", window, cx);
    });
    click(cx, "project-focus-/work/beta");
    view.read_with(cx, |view, cx| {
        assert_eq!(view.working_cwd, ALPHA);
        assert_eq!(view.composer_input.read(cx).text(), "/model");
        assert!(view.active_modal.is_none());
        assert_eq!(view.sidebar_project.as_deref(), Some(BETA));
    });
    assert!(
        cx.debug_bounds("sidebar-saved-/chats/destination.jsonl")
            .is_some()
    );
    search(cx, &view, "original");
    assert!(cx.debug_bounds("sidebar-empty").is_some());
    click(cx, "sidebar-search-clear");
    view.read_with(cx, |view, cx| {
        assert!(view.sidebar_search_input.read(cx).text().is_empty());
    });
    click(cx, "sidebar-compose");
    view.read_with(cx, |view, cx| {
        assert_eq!(view.working_cwd, BETA);
        assert!(view.composer_input.read(cx).text().is_empty());
        assert!(view.session.active_session_id.is_empty());
    });
    click(cx, "sidebar-all-projects");
    view.read_with(cx, |view, _| assert!(view.sidebar_project.is_none()));
    click(cx, "sidebar-draft-/work/alpha");
    view.read_with(cx, |view, cx| {
        assert_eq!(view.working_cwd, ALPHA);
        assert_eq!(view.composer_input.read(cx).text(), "/model");
    });
    app.quit();
}

#[test]
fn search_reaches_beyond_the_project_preview_and_history_projects_can_be_pinned() {
    let profile = tempfile::tempdir().unwrap();
    let mut app = TestAppContext::single();
    let mut rows: Vec<_> = (1..=5)
        .map(|i| saved(ALPHA, &format!("Chat {i}")))
        .collect();
    rows.push(saved(ALPHA, "buried needle"));
    rows.push(saved(ALPHA, "seventh"));
    rows.push(saved(ARCHIVE, "older work"));
    let (view, cx) = open_sidebar(&mut app, profile.path(), rows);
    draw(cx);
    assert!(cx.debug_bounds("project-more-/work/alpha").is_some());
    // These rows have never been drawn. GPUI retains selector bounds after a row disappears.
    assert!(
        cx.debug_bounds("sidebar-saved-/chats/buried needle.jsonl")
            .is_none()
    );
    assert!(cx.debug_bounds("project-focus-/work/archive").is_none());
    search(cx, &view, "needle");
    assert!(
        cx.debug_bounds("sidebar-saved-/chats/buried needle.jsonl")
            .is_some()
    );
    click(cx, "sidebar-search-clear");
    click(cx, "project-more-/work/alpha");
    view.read_with(cx, |view, _| {
        assert_eq!(view.sidebar_project.as_deref(), Some(ALPHA));
    });
    assert!(
        cx.debug_bounds("sidebar-saved-/chats/seventh.jsonl")
            .is_some()
    );
    click(cx, "sidebar-all-projects");
    click(cx, "project-toggle-/work/alpha");
    view.read_with(cx, |view, _| {
        assert_eq!(view.prefs.project_collapsed.get(ALPHA), Some(&true));
    });
    click(cx, "sidebar-history");
    let folder = cx.debug_bounds("project-focus-/work/archive").unwrap();
    cx.simulate_mouse_move(folder.center(), None, Modifiers::default());
    click(cx, "project-pin-/work/archive");
    let prefs = DesktopPreferences::load(profile.path());
    assert!(prefs.project_folders.iter().any(|cwd| cwd == ARCHIVE));
    view.read_with(cx, |view, _| assert_eq!(view.working_cwd, ALPHA));
    app.quit();
}
