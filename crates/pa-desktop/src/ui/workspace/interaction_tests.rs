//! Exercise the same Rune input routing and asynchronous store updates as the window.
use std::time::{Duration, Instant};

use rune::{Env, Key, KeyEvent, Mods, Runtime};
use tempfile::TempDir;

use super::{ActiveModal, LaunchOptions, PrimeDesktopView};
use crate::state::models::parse_model_catalog;
use crate::theme::DesktopPreferences;

fn workspace(profile: &TempDir) -> Runtime<PrimeDesktopView> {
    let state = PrimeDesktopView::new(
        LaunchOptions {
            initial_cwd: Some(profile.path().to_owned()),
            daemon_socket: Some(profile.path().join("isolated.sock")),
            ..Default::default()
        },
        profile.path().to_owned(),
    );
    let mut app = Runtime::new(
        state,
        PrimeDesktopView::view,
        (1360., 860.),
        Env {
            system_dark: true,
            scale: 1.,
        },
    );
    app.poll = Some(PrimeDesktopView::poll);
    app.on_key = Some(crate::shortcuts::handle_key);
    wait_until(&mut app, |s| s.toast.is_some());
    app
}

fn wait_until(app: &mut Runtime<PrimeDesktopView>, ready: impl Fn(&PrimeDesktopView) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.frame(1. / 60.);
        if ready(&app.state) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for workspace state"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn key(app: &mut Runtime<PrimeDesktopView>, key: Key, cmd: bool) {
    let ev = KeyEvent {
        key,
        mods: Mods {
            cmd,
            ..Mods::default()
        },
    };
    app.key(ev.clone());
    app.key_up(ev);
    app.frame(1. / 60.);
}

#[test]
fn reduce_motion_toggles_from_settings_and_survives_relaunch() {
    let profile = TempDir::new().unwrap();
    std::fs::write(
        profile.path().join("desktop-ui.json"),
        r#"{"sidebarOpen":false}"#,
    )
    .unwrap();
    let mut app = workspace(&profile);
    key(&mut app, Key::Command(','), true);
    assert_eq!(app.state.active_modal, Some(ActiveModal::SettingsAuth));
    assert!(!app.state.prefs.reduce_motion && !app.state.prefs.sidebar_open);
    let (x, y) = app.find("reduce-motion").expect("motion control");
    app.pointer_down(x, y);
    app.pointer_up(x, y);
    app.frame(0.1);
    let saved = DesktopPreferences::load(profile.path());
    assert!(saved.reduce_motion && !saved.sidebar_open);
    drop(app);
    assert!(workspace(&profile).state.prefs.reduce_motion);
}

#[test]
fn search_and_completion_keys_act_on_the_highlighted_row() {
    let profile = TempDir::new().unwrap();
    let mut app = workspace(&profile);
    app.state.populate_draft("/");
    app.invalidate();
    app.frame(0.);
    for k in [Key::Down, Key::Down, Key::Up, Key::Tab] {
        key(&mut app, k, false);
    }
    assert_eq!(app.state.composer_input.text, "/refine");

    app.state.populate_draft("/");
    app.invalidate();
    app.frame(0.);
    for k in [Key::Down, Key::Text("c".into()), Key::Enter] {
        key(&mut app, k, false);
    }
    assert_eq!(app.state.composer_input.text, "/compact");

    app.state.populate_draft("/mo");
    app.invalidate();
    app.frame(0.);
    key(&mut app, Key::Enter, false);
    assert_eq!(app.state.composer_input.text, "/model");

    app.state.catalog_models = parse_model_catalog(&serde_json::json!({"models":[
        {"provider":"example","id":"one"}, {"provider":"example","id":"two"}]}))
    .0;
    key(&mut app, Key::Command('k'), true);
    key(&mut app, Key::Down, false);
    key(&mut app, Key::Enter, false);
    assert_eq!(app.state.active_modal, Some(ActiveModal::ModelPicker));
    key(&mut app, Key::Down, false);
    assert_eq!(app.state.modal_selected_idx, 1);
    key(&mut app, Key::Text("t".into()), false);
    assert_eq!(app.state.modal_selected_idx, 0);
    key(&mut app, Key::Enter, false);
    assert!(app.state.active_modal.is_none() && app.state.composer_input.focused);
}

#[test]
fn editor_return_advances_fields_and_modified_return_saves() {
    let profile = TempDir::new().unwrap();
    let mut app = workspace(&profile);
    app.state.open_harness_tab();
    app.state.open_new_harness_editor();
    app.invalidate();
    app.frame(0.);
    key(&mut app, Key::Text("note".into()), false);
    key(&mut app, Key::Enter, false);
    assert!(app.state.harness_body_input.focused);
    for k in [Key::Text("a".into()), Key::Enter, Key::Text("b".into())] {
        key(&mut app, k, false);
    }
    assert_eq!(app.state.harness_body_input.text, "a\nb");
    key(&mut app, Key::Enter, true);
    assert!(!app.state.harness_editor_open && app.state.composer_input.focused);
}

#[test]
fn escape_dismisses_editors_in_order_during_a_running_turn() {
    let profile = TempDir::new().unwrap();
    let mut app = workspace(&profile);
    app.state.prefs.inspector_open = false;
    app.state.prefs.sidebar_open = false;
    app.state.session.active_session_id = "running".into();
    app.state.session.is_streaming = true;
    app.state.session.side_question.open = true;
    app.state.harness_editor_open = true;
    app.state.agent_mail_target = Some("child".into());
    app.invalidate();
    app.frame(0.);
    key(&mut app, Key::Escape, false);
    assert!(app.state.agent_mail_target.is_none() && app.state.harness_editor_open);
    key(&mut app, Key::Escape, false);
    assert!(!app.state.harness_editor_open && app.state.session.side_question.open);
    key(&mut app, Key::Escape, false);
    assert!(!app.state.session.side_question.open && app.state.session.is_streaming);
    assert!(app.state.composer_input.focused);
}

#[cfg(unix)]
#[test]
fn choosing_a_cli_folder_reloads_its_shared_keys_and_remembers_only_the_path() {
    use crate::bridge::{CliData, socket_path_for_target};
    use crate::theme::DaemonTarget;
    let profile = TempDir::new().unwrap();
    let selected = profile.path().join("cli");
    std::fs::create_dir(&selected).unwrap();
    let credential = br#"{"import-fixture":{"type":"api_key","key":"never-copy-this-fixture"}}"#;
    std::fs::write(selected.join("auth.json"), credential).unwrap();
    let data = CliData::new(selected.clone());
    let socket = socket_path_for_target(DaemonTarget::DesktopHarness, None, &data);
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let mut app = workspace(&profile);
    app.state.use_cli_folder(Some(selected.clone()));
    wait_until(&mut app, |s| {
        s.agent_dir == selected
            && s.provider_auth
                .iter()
                .any(|p| p.provider_id == "import-fixture" && p.has_stored_api_key)
    });
    let prefs = DesktopPreferences::load(profile.path());
    assert_eq!(prefs.cli_data_dir.as_deref(), Some(selected.as_path()));
    assert!(
        !std::fs::read_to_string(profile.path().join("desktop-ui.json"))
            .unwrap()
            .contains("never-copy-this-fixture")
    );
    assert!(!profile.path().join("auth.json").exists());
    assert_eq!(
        std::fs::read(selected.join("auth.json")).unwrap(),
        credential
    );
    drop(listener);
    std::fs::remove_file(socket).unwrap();
}

#[test]
fn resizing_a_docked_sidebar_to_a_drawer_keeps_the_stage_usable() {
    let profile = TempDir::new().unwrap();
    let mut app = workspace(&profile);
    app.state.prefs.sidebar_open = true;
    app.state.prefs.inspector_open = false;
    app.invalidate();
    app.frame(0.);
    let (wide, _) = app.presented("center-pane").unwrap();
    assert!(wide[0] > 200.);
    assert!(app.find("drawer-scrim").is_none());

    app.set_viewport((660., 540.), 1.);
    for _ in 0..3 {
        app.frame(1. / 60.);
    }
    assert!(app.find("sidebar-drawer").is_some());
    assert!(app.find("drawer-scrim").is_some());
    let (narrow, _) = app.presented("center-pane").unwrap();
    assert!(
        narrow[0].abs() < 1. && (narrow[2] - 660.).abs() < 1.,
        "stage after resize: {narrow:?}"
    );
    key(&mut app, Key::Escape, false);
    assert!(app.find("sidebar-drawer").is_none());
    assert!(app.find("drawer-scrim").is_none());
    assert!(app.find("composer-input").is_some());
}

#[test]
fn attached_history_follows_until_scrolled_and_jump_resumes_following() {
    use crate::bridge::BridgeEvent;
    use crate::state::transcript::TranscriptEntry;
    let profile = TempDir::new().unwrap();
    let mut app = workspace(&profile);
    app.state.toast = None;
    let messages = (0..60)
        .map(|i| serde_json::json!({"role":"user","content":format!("Question {i}")}))
        .collect::<Vec<_>>();
    app.state.selection = super::SessionSelection::WaitingFor("history".into());
    app.state.apply_bridge_event(BridgeEvent::SessionAttached {
        active_session_id: "history".into(),
        snapshot: serde_json::json!({"messages": messages}),
    });
    app.invalidate();
    let viewport = "transcript-scroll-history";
    for _ in 0..8 {
        app.frame(1. / 60.);
    }
    assert_eq!(app.scroll_at_bottom(viewport), Some(true));
    let (x, y) = app.find(viewport).unwrap();
    app.wheel(x, y, 720.);
    for _ in 0..30 {
        app.frame(1. / 60.);
    }
    assert_eq!(app.scroll_at_bottom(viewport), Some(false));
    app.state.session.entries.push(TranscriptEntry::User {
        key: "later".into(),
        text: "A new message while reading history".into(),
        skill_name: None,
        image_count: 0,
    });
    app.invalidate();
    app.frame(0.);
    assert_eq!(app.scroll_at_bottom(viewport), Some(false));
    let (x, y) = app
        .find("jump-to-bottom")
        .expect("reader can resume following");
    app.pointer_down(x, y);
    app.pointer_up(x, y);
    for _ in 0..90 {
        app.frame(1. / 60.);
    }
    assert_eq!(app.scroll_at_bottom(viewport), Some(true));
    let (x, y) = app.find(viewport).unwrap();
    app.wheel(x, y, 720.);
    for _ in 0..30 {
        app.frame(1. / 60.);
    }
    assert_eq!(app.scroll_at_bottom(viewport), Some(false));
    app.state.daemon_connected = true;
    app.state.populate_draft("resume");
    app.state.submit_composer();
    app.invalidate();
    for _ in 0..90 {
        app.frame(1. / 60.);
    }
    assert_eq!(app.scroll_at_bottom(viewport), Some(true));
}

#[test]
fn long_error_paths_wrap_inside_a_dismissible_modal_notification() {
    let profile = TempDir::new().unwrap();
    let mut app = workspace(&profile);
    app.set_viewport((660., 540.), 1.);
    app.state.prefs.reduce_motion = true;
    app.state.open_settings_modal();
    app.state.toast = Some((
        format!("Cannot connect to /tmp/{}/daemon.sock", "long".repeat(95)),
        true,
    ));
    app.invalidate();
    for _ in 0..30 {
        app.frame(1. / 60.);
    }
    let (banner, _) = app.presented("toast-banner").unwrap();
    let (message, _) = app.presented("toast-message").unwrap();
    assert!(message[3] > 30., "message should wrap: {message:?}");
    assert!(banner[0] >= 0. && banner[0] + banner[2] <= 660.);
    assert!(message[0] >= banner[0] && message[0] + message[2] <= banner[0] + banner[2] + 1.);
    let (x, y) = app.find("toast-banner").unwrap();
    app.pointer_down(x, y);
    app.pointer_up(x, y);
    app.frame(1. / 60.);
    assert!(app.state.toast.is_none());
    assert_eq!(app.state.active_modal, Some(ActiveModal::SettingsAuth));
}
