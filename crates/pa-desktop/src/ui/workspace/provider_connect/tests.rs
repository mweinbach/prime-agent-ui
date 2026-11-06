use super::*;
use crate::ui::workspace::LaunchOptions;
use std::time::{Duration, Instant};
use tempfile::TempDir;

#[track_caller]
fn wait_for(view: &mut PrimeDesktopView, predicate: impl Fn(&PrimeDesktopView) -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !predicate(view) {
        view.drain_events();
        assert!(
            Instant::now() < end,
            "timed out waiting for connection state"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn workspace(profile: &TempDir) -> PrimeDesktopView {
    let mut view = PrimeDesktopView::new(
        LaunchOptions {
            daemon_socket: Some(profile.path().join("offline.sock")),
            initial_cwd: Some(profile.path().into()),
            ..LaunchOptions::default()
        },
        profile.path().into(),
    );
    wait_for(&mut view, |s| {
        !s.provider_auth.is_empty() && s.toast.is_some()
    });
    view
}

#[test]
fn cancelling_a_key_prompt_preserves_the_draft_and_ignores_late_events() {
    let profile = tempfile::tempdir().unwrap();
    let mut view = workspace(&profile);
    view.populate_draft("unsent chat content");
    view.open_modal(ActiveModal::ModelPicker);
    view.modal_search_input.replace_text("coding model");
    view.open_provider_connect("openai");
    wait_for(&mut view, |s| {
        s.provider_connection
            .as_ref()
            .is_some_and(|p| p.prompt.is_some())
    });
    let attempt = view.login_attempt.as_ref().unwrap();
    let old = attempt.id;
    let gate = Arc::clone(&attempt.gate);
    view.connection_input.replace_text("synthetic-unsubmitted");
    view.cancel_provider_connection();
    assert!(gate.is_cancelled() && !gate.begin_commit());
    assert!(view.connection_input.text.is_empty());
    assert_eq!(view.active_modal, Some(ActiveModal::ModelPicker));
    assert_eq!(view.modal_search_input.text, "coding model");
    view.open_provider_connect("openai");
    let new = view.login_attempt.as_ref().unwrap().id;
    assert_ne!(old, new);
    view.login_event(old, LoginEvent::InvalidBrowser);
    view.login_finished(old, Ok(()));
    assert_eq!(view.login_attempt.as_ref().unwrap().id, new);
    assert_eq!(view.composer_input.text, "unsent chat content");
    view.cancel_provider_connection();
    assert!(!profile.path().join("auth.json").exists());
}

#[test]
fn generic_key_save_returns_to_settings_without_changing_the_selected_model() {
    let profile = tempfile::tempdir().unwrap();
    let mut view = workspace(&profile);
    view.session.active_session_id = "session".into();
    view.session.model_provider = Some("existing".into());
    view.session.model_id = Some("existing-model".into());
    view.open_settings_modal();
    view.open_provider_connect("openai");
    wait_for(&mut view, |s| {
        s.provider_connection
            .as_ref()
            .is_some_and(|p| p.prompt.is_some())
    });
    view.connection_input.replace_text("test-only-value");
    view.submit_connection_input();
    wait_for(&mut view, |s| !s.auth_write_pending);
    assert_eq!(view.active_modal, Some(ActiveModal::SettingsAuth));
    assert_eq!(view.session.model_id.as_deref(), Some("existing-model"));
    assert!(view.connected_catalog.is_none() && view.connection_input.text.is_empty());
    let stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(profile.path().join("auth.json")).unwrap()).unwrap();
    assert_eq!(stored["openai"]["key"], "test-only-value");
    assert_eq!(view.toast.as_ref().unwrap().0, "Saved API key for OpenAI.");
}
#[test]
fn a_pre_login_refresh_cannot_overwrite_newer_provider_availability() {
    let profile = tempfile::tempdir().unwrap();
    let mut view = workspace(&profile);
    view.session.active_session_id = "session".into();
    for (revision, configured) in [(9, true), (8, false)] {
        let providers = if configured {
            vec!["example"]
        } else {
            Vec::new()
        };
        view.apply_bridge_event(crate::bridge::BridgeEvent::ModelCatalogLoaded {
            revision,
            active_session_id: "session".into(),
            data: serde_json::json!({"configuredProviders":providers, "models":[{"provider":"example","id":"model"}]}),
        });
    }
    assert!(view.catalog_models[0].configured);
    assert!(view.configured_providers.contains("example"));
}
#[test]
fn catalog_confirmation_does_not_select_a_model_after_the_user_changed_sessions() {
    let profile = tempfile::tempdir().unwrap();
    let mut view = workspace(&profile);
    view.daemon_connected = true;
    view.active_modal = Some(ActiveModal::ModelPicker);
    view.session.active_session_id = "new-session".into();
    let request = Uuid::new_v4();
    view.connected_catalog = Some(ConnectedCatalog {
        request,
        intent: ModelIntent {
            session: "old-session".into(),
            provider: "example".into(),
            model: "requested".into(),
            previous_model: None,
        },
    });
    view.finish_connected_catalog(
        request,
        "old-session",
        1,
        Some(serde_json::json!({
            "configuredProviders":["example"], "models":[{"provider":"example","id":"requested"}]
        })),
    );
    assert!(view.connected_catalog.is_none());
    assert!(view.pending_model_setting.is_none());
    assert!(view.catalog_models.is_empty());
}
fn settings_app(profile: &TempDir) -> rune::Runtime<PrimeDesktopView> {
    crate::state::harness_store::set_provider_api_key(
        profile.path(),
        "prime-inference",
        "initial-synthetic-key",
    )
    .unwrap();
    let mut state = workspace(profile);
    state.prefs.reduce_motion = true;
    state.toast = None;
    state.open_settings_modal();
    let mut app = rune::Runtime::new(
        state,
        PrimeDesktopView::view,
        (1280., 1000.),
        rune::Env::default(),
    );
    app.frame(0.);
    app
}

fn click(app: &mut rune::Runtime<PrimeDesktopView>, key: &str) {
    app.invalidate();
    app.frame(1. / 60.);
    app.settle_all();
    app.frame(1. / 60.);
    let (x, y) = app
        .find(key)
        .unwrap_or_else(|| panic!("missing control {key}"));
    app.pointer_down(x, y);
    app.pointer_up(x, y);
    app.frame(1. / 60.);
}

fn saved_prime_key(profile: &TempDir) -> Option<String> {
    let doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(profile.path().join("auth.json")).unwrap()).unwrap();
    doc["prime-inference"]["key"].as_str().map(str::to_owned)
}

#[test]
fn editing_a_stored_prime_key_uses_the_same_connection_flow() {
    let profile = tempfile::tempdir().unwrap();
    let mut app = settings_app(&profile);
    assert!(app.find("api-key").is_none());
    click(&mut app, "auth-prov-prime-inference");
    let pane = app.state.provider_connection.as_ref().unwrap();
    assert_eq!(pane.provider_id, "prime-inference");
    assert_eq!(pane.status, ConnectStatus::Choose);
    click(&mut app, "connect-use-key");
    wait_for(&mut app.state, |s| {
        s.provider_connection
            .as_ref()
            .is_some_and(|p| p.prompt.is_some())
    });
    app.invalidate();
    app.frame(1. / 60.);
    assert!(app.find("provider-connect-input").is_some());
    assert!(app.state.connection_input.is_secure());
    // Cancel before providing a key; no access check contacts a real provider.
    click(&mut app, "provider-connect-cancel");
    assert_eq!(app.state.active_modal, Some(ActiveModal::SettingsAuth));
    assert_eq!(
        saved_prime_key(&profile).as_deref(),
        Some("initial-synthetic-key")
    );
}

#[test]
fn removing_a_key_requires_confirmation_for_the_unchanged_provider_slot() {
    let profile = tempfile::tempdir().unwrap();
    let mut app = settings_app(&profile);
    click(&mut app, "remove-provider-key-prime-inference");
    wait_for(&mut app.state, |s| s.confirm.is_some());
    click(&mut app, "cancel-confirm");
    wait_for(&mut app.state, |s| !s.auth_write_pending);
    assert_eq!(
        saved_prime_key(&profile).as_deref(),
        Some("initial-synthetic-key")
    );
    click(&mut app, "remove-provider-key-prime-inference");
    wait_for(&mut app.state, |s| s.confirm.is_some());
    crate::state::harness_store::set_provider_api_key(
        profile.path(),
        "prime-inference",
        "rotated-synthetic-key",
    )
    .unwrap();
    click(&mut app, "accept-confirm");
    wait_for(&mut app.state, |s| !s.auth_write_pending);
    assert_eq!(
        saved_prime_key(&profile).as_deref(),
        Some("rotated-synthetic-key")
    );
    assert!(app.state.toast.as_ref().is_some_and(|(_, error)| *error));
    click(&mut app, "toast-banner");
    click(&mut app, "remove-provider-key-prime-inference");
    wait_for(&mut app.state, |s| s.confirm.is_some());
    click(&mut app, "accept-confirm");
    wait_for(&mut app.state, |s| !s.auth_write_pending);
    assert!(saved_prime_key(&profile).is_none());
}
