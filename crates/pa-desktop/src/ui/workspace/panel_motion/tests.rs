use std::{path::Path, time::Duration};

use gpui::{Bounds, Entity, Modifiers, Pixels, TestAppContext, VisualTestContext, point, px, size};
use tempfile::TempDir;

use crate::ui::workspace::{
    LaunchOptions, PrimeDesktopView, register_workspace_bindings_and_menus,
};

fn frame(cx: &mut VisualTestContext, elapsed_ms: u64) {
    cx.executor()
        .advance_clock(Duration::from_millis(elapsed_ms));
    cx.update(|window, cx| window.draw(cx).clear());
}

fn bounds(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"))
}

fn near(actual: Pixels, expected: Pixels) {
    assert!(
        (actual - expected).abs() <= px(1.),
        "expected {expected:?}, got {actual:?}"
    );
}

fn closed_workspace<'a>(
    app: &'a mut TestAppContext,
    profile: &Path,
    width: Pixels,
) -> (Entity<PrimeDesktopView>, &'a mut VisualTestContext) {
    app.dispatcher.allow_parking();
    app.update(register_workspace_bindings_and_menus);
    let (view, cx) = app.add_window_view(|window, cx| {
        PrimeDesktopView::new(
            LaunchOptions {
                initial_cwd: Some(profile.to_owned()),
                daemon_socket: Some(profile.join("offline.sock")),
                ..LaunchOptions::default()
            },
            profile.to_owned(),
            window,
            cx,
        )
    });
    cx.executor()
        .block_test(cx.condition(&view, |view, _| view.toast.is_some()));
    cx.simulate_resize(size(width, px(700.)));
    view.update_in(cx, |view, _, cx| {
        view.toast = None;
        view.prefs.sidebar_open = false;
        view.prefs.inspector_open = false;
        view.prefs.reduce_motion = true;
        cx.notify();
    });
    frame(cx, 0);
    view.update_in(cx, |view, _, cx| {
        view.prefs.reduce_motion = false;
        cx.notify();
    });
    frame(cx, 0);
    (view, cx)
}

#[test]
fn docked_panels_reflow_the_stage_and_reverse_from_the_visible_width() {
    let profile = TempDir::new().unwrap();
    let mut app = TestAppContext::single();
    let (view, cx) = closed_workspace(&mut app, profile.path(), px(1280.));
    let full_stage = bounds(cx, "workspace-stage").size.width;

    cx.simulate_keystrokes("secondary-b secondary-i");
    frame(cx, 0);
    frame(cx, 45);
    let partial_sidebar = bounds(cx, "sidebar-dock-clip").size.width;
    let partial_inspector = bounds(cx, "inspector-dock-clip").size.width;
    assert!(partial_sidebar > px(0.) && partial_inspector > px(0.));
    near(
        bounds(cx, "workspace-stage").size.width,
        full_stage - partial_sidebar - partial_inspector,
    );

    frame(cx, 250);
    let full_sidebar = bounds(cx, "sidebar-dock-clip").size.width;
    let full_inspector = bounds(cx, "inspector-dock-clip").size.width;
    assert!(partial_sidebar < full_sidebar && partial_inspector < full_inspector);

    cx.simulate_keystrokes("secondary-b");
    frame(cx, 0);
    frame(cx, 50);
    let reversing_at = bounds(cx, "sidebar-dock-clip").size.width;
    assert!(reversing_at > px(0.) && reversing_at < full_sidebar);
    cx.simulate_keystrokes("secondary-b");
    frame(cx, 0);
    near(bounds(cx, "sidebar-dock-clip").size.width, reversing_at);
    frame(cx, 30);
    assert!(bounds(cx, "sidebar-dock-clip").size.width > reversing_at);
    frame(cx, 250);
    near(bounds(cx, "sidebar-dock-clip").size.width, full_sidebar);

    cx.simulate_keystrokes("secondary-b secondary-i");
    frame(cx, 0);
    frame(cx, 25);
    assert!(bounds(cx, "workspace-stage").size.width < full_stage);
    view.update_in(cx, |view, _, cx| {
        view.prefs.reduce_motion = true;
        cx.notify();
    });
    frame(cx, 0);
    near(bounds(cx, "workspace-stage").size.width, full_stage);
    app.quit();
}

#[test]
fn narrow_drawers_slide_over_the_stage_and_release_the_scrim_after_dismissal() {
    let profile = TempDir::new().unwrap();
    let mut app = TestAppContext::single();
    let (view, cx) = closed_workspace(&mut app, profile.path(), px(680.));
    let stage = bounds(cx, "workspace-stage");
    let project = bounds(cx, "stage-project-folder").center();
    cx.simulate_click(project, Modifiers::none());
    frame(cx, 0);
    frame(cx, 45);
    let entering = bounds(cx, "sidebar-drawer");
    assert!(entering.left() < stage.left() && entering.right() > stage.left());
    near(bounds(cx, "workspace-stage").size.width, stage.size.width);
    frame(cx, 250);
    near(bounds(cx, "sidebar-drawer").left(), stage.left());

    let scrim = bounds(cx, "drawer-scrim");
    cx.simulate_click(
        point(scrim.right() - px(4.), scrim.center().y),
        Modifiers::none(),
    );
    assert!(!view.read_with(cx, |view, _| view.prefs.sidebar_open));
    frame(cx, 0);
    frame(cx, 35);
    let leaving = bounds(cx, "sidebar-drawer");
    assert!(leaving.left() < stage.left() && leaving.right() > stage.left());
    frame(cx, 250);

    cx.simulate_keystrokes("secondary-i");
    frame(cx, 0);
    frame(cx, 45);
    let entering = bounds(cx, "inspector-drawer");
    assert!(entering.left() < stage.right() && entering.right() > stage.right());
    near(bounds(cx, "workspace-stage").size.width, stage.size.width);
    frame(cx, 250);
    near(bounds(cx, "inspector-drawer").right(), stage.right());
    cx.simulate_click(
        point(scrim.left() + px(4.), scrim.center().y),
        Modifiers::none(),
    );
    assert!(!view.read_with(cx, |view, _| view.prefs.inspector_open));
    frame(cx, 0);
    frame(cx, 250);
    // A stale debug rectangle cannot tell whether a transparent scrim still captures input.
    cx.simulate_click(project, Modifiers::none());
    assert!(view.read_with(cx, |view, _| view.prefs.sidebar_open));
    app.quit();
}
