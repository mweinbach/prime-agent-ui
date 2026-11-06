//! Drive the model chooser through the same focus, text editing and selection path as the window.
use super::*;
use crate::state::models::parse_model_catalog;
use crate::ui::workspace::LaunchOptions;
use rune::{Env, KeyEvent, Runtime};
use serde_json::json;

fn key(app: &mut Runtime<PrimeDesktopView>, key: Key) {
    let event = KeyEvent {
        key,
        mods: rune::Mods::default(),
    };
    app.key(event.clone());
    app.key_up(event);
    app.frame(1.0 / 60.0);
}

fn click(app: &mut Runtime<PrimeDesktopView>, key: &str) {
    let (x, y) = app.find(key).expect("picker control");
    app.pointer_down(x, y);
    app.pointer_up(x, y);
    app.frame(1.0 / 60.0);
}

#[test]
fn picker_tracks_the_current_model_across_providers_and_filters_the_full_catalog() {
    let profile = tempfile::tempdir().unwrap();
    let mut state = PrimeDesktopView::new(
        LaunchOptions {
            initial_cwd: Some(profile.path().to_owned()),
            daemon_socket: Some(profile.path().join("offline.sock")),
            ..Default::default()
        },
        profile.path().to_owned(),
    );
    state.daemon_connected = true;
    state.session.active_session_id = "picker-session".into();
    state.session.model_provider = Some("local".into());
    state.session.model_id = Some("shared".into());
    let mut models = (0..110)
        .map(|index| json!({"provider":"local","id":format!("engine-{index:03}")}))
        .collect::<Vec<_>>();
    models.extend([
        json!({"provider":"other","id":"shared","name":"Shared model"}),
        json!({"provider":"local","id":"shared","name":"Shared model"}),
    ]);
    state.catalog_models =
        parse_model_catalog(&json!({"configuredProviders":["local"],"models":models})).0;
    state.open_modal(ActiveModal::ModelPicker);
    let mut app = Runtime::new(
        state,
        PrimeDesktopView::view,
        (1360.0, 860.0),
        Env::default(),
    );
    app.frame(0.0);
    let choices = app.state.matching_models();
    assert_eq!(choices.len(), 112);
    assert_eq!(
        app.state.catalog_models[choices[0]].selector(),
        "local/shared"
    );
    let checks = app
        .elements()
        .into_iter()
        .filter(|e| e.key.as_deref() == Some("current-model-check"))
        .collect::<Vec<_>>();
    assert_eq!(checks.len(), 1);
    assert!(checks[0].path.contains("model-option-local/shared"));

    key(&mut app, Key::Down);
    assert_eq!(app.state.modal_selected_idx, 1);
    app.paste("shared");
    app.frame(1.0 / 60.0);
    assert_eq!(app.state.modal_selected_idx, 0);
    assert_eq!(app.state.matching_models().len(), 2);
    click(&mut app, "toggle-configured-models");
    assert!(app.state.modal_search_input.focused);
    assert_eq!(app.state.matching_models().len(), 1);
    click(&mut app, "show-all-models");
    key(&mut app, Key::Down);
    let index = app.state.matching_models()[app.state.modal_selected_idx];
    assert_eq!(app.state.catalog_models[index].selector(), "other/shared");
    let mut chosen = app.state.catalog_models[index].clone();
    app.update(|view, _| {
        view.catalog_models
            .retain(|model| model.provider != "other");
    });
    app.frame(0.0);
    key(&mut app, Key::Enter);
    assert_eq!(app.state.active_modal, Some(ActiveModal::ModelPicker));
    assert!(
        !app.state.model_setting_pending(),
        "removing a highlighted model must never submit the row that replaced it"
    );
    chosen.configured = true; // The refreshed provider now permits a model change.
    app.update(|view, _| view.catalog_models.insert(0, chosen));
    app.frame(0.0);
    key(&mut app, Key::Enter);
    assert!(app.state.active_modal.is_none());
    assert!(app.state.model_setting_pending());
}

#[test]
fn narrow_offline_picker_displays_one_dismissible_error_below_search() {
    let profile = tempfile::tempdir().unwrap();
    let mut state = PrimeDesktopView::new(
        LaunchOptions {
            daemon_socket: Some(profile.path().join("absent.sock")),
            ..LaunchOptions::default()
        },
        profile.path().to_owned(),
    );
    state.toast = Some((
        format!(
            "Connection failed at /{}",
            "a-long-unbroken-path/".repeat(8)
        ),
        true,
    ));
    state.open_modal(ActiveModal::ModelPicker);
    let mut app = Runtime::new(
        state,
        PrimeDesktopView::view,
        (660.0, 540.0),
        Env::default(),
    );
    app.frame(0.0);
    let bounds = |key: &str| {
        app.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap()
            .visible
    };
    let search = bounds("modal-search");
    let notice = bounds("toast-banner");
    assert!(notice[1] >= search[1] + search[3]);
    assert!(notice[3] > 20.0 && notice[2] > 100.0);
    let plain_hint = "Connect to the daemon to load models.";
    assert!(
        !app.elements()
            .iter()
            .any(|el| el.text.as_deref() == Some(plain_hint))
    );
    click(&mut app, "toast-banner");
    assert!(app.state.toast.is_none());
    assert_eq!(app.state.active_modal, Some(ActiveModal::ModelPicker));
    assert!(
        app.elements()
            .iter()
            .any(|el| el.text.as_deref() == Some(plain_hint))
    );
}

#[test]
fn model_chooser_offers_provider_connection_and_keeps_the_search_on_cancel() {
    let profile = tempfile::tempdir().unwrap();
    let mut view = PrimeDesktopView::new(
        LaunchOptions {
            daemon_socket: Some(profile.path().join("offline.sock")),
            initial_cwd: Some(profile.path().to_owned()),
            ..LaunchOptions::default()
        },
        profile.path().to_owned(),
    );
    view.daemon_connected = true;
    view.session.active_session_id = "offline-selection".into();
    view.catalog_models = parse_model_catalog(&json!({"models":[
        {"provider":"prime-inference","id":"atlas","name":"Atlas"}
    ]}))
    .0;
    view.provider_auth = vec![ProviderAuthItem {
        display_name: "Prime Inference".into(),
        provider_id: "prime-inference".into(),
        configured: false,
        auth_kind: "Not configured".into(),
        hint: String::new(),
        can_connect: true,
        can_edit_api_key: true,
        has_stored_api_key: false,
    }];
    view.open_modal(ActiveModal::ModelPicker);
    let mut app = Runtime::new(view, PrimeDesktopView::view, (780.0, 650.0), Env::default());
    app.frame(0.0);
    click(&mut app, "toggle-configured-models");
    let bounds = |key: &str| {
        app.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap()
    };
    let action = bounds("model-connect-provider");
    let sheet = bounds("model-picker");
    assert!((action.visible[3] - action.rect[3]).abs() < 0.5);
    assert!(sheet.visible[1] + sheet.visible[3] >= action.visible[1] + action.visible[3] + 5.0);
    click(&mut app, "model-connect-provider");
    assert_eq!(app.state.active_modal, Some(ActiveModal::ProviderConnect));
    click(&mut app, "connect-provider-search");
    app.paste("prime inference");
    app.frame(0.0);
    assert!(app.find("provider-option-prime-inference").is_some());
    key(&mut app, Key::Enter);
    assert!(app.find("connect-use-browser").is_some());
    assert!(app.find("connect-use-key").is_some());
    click(&mut app, "provider-connect-cancel");
    assert_eq!(app.state.active_modal, Some(ActiveModal::ModelPicker));
    assert!(app.state.model_picker_configured_only);
    click(&mut app, "show-all-models");
    app.paste("atlas");
    app.frame(0.0);
    key(&mut app, Key::Enter);
    assert_eq!(app.state.active_modal, Some(ActiveModal::ProviderConnect));
    assert!(!app.state.model_setting_pending());
    click(&mut app, "provider-connect-cancel");
    assert_eq!(app.state.active_modal, Some(ActiveModal::ModelPicker));
    assert_eq!(app.state.modal_search_input.text, "atlas");
}

#[test]
fn device_sign_in_displays_only_its_public_instructions_and_masks_manual_response() {
    use crate::provider_connect::{BrowserStep, ConnectStatus, ConnectionPane, InputPrompt};

    let profile = tempfile::tempdir().unwrap();
    let mut view = PrimeDesktopView::new(
        LaunchOptions {
            daemon_socket: Some(profile.path().join("offline.sock")),
            ..LaunchOptions::default()
        },
        profile.path().to_owned(),
    );
    view.active_modal = Some(ActiveModal::ProviderConnect);
    view.provider_connection = Some(ConnectionPane {
        provider_id: "fixture".into(),
        display_name: "Fixture".into(),
        can_browser: true,
        can_api_key: false,
        status: ConnectStatus::Working,
        progress: "Waiting for sign-in".into(),
        error: None,
        browser: Some(BrowserStep {
            url: "https://example.invalid/authorize?state=private-fixture-state".into(),
            instructions: Some("Enter the code in your browser.".into()),
            code: Some("ABCD-1234".into()),
        }),
        prompt: Some(InputPrompt {
            label: "Paste the callback URL".into(),
            placeholder: None,
            secret: true,
            allow_empty: false,
        }),
        choice: None,
    });
    view.connection_input.set_secure(true);
    view.connection_input
        .replace_text("private-fixture-response");
    let mut app = Runtime::new(view, PrimeDesktopView::view, (660.0, 540.0), Env::default());
    app.frame(0.0);
    let elements = app.elements();
    let input = elements
        .iter()
        .find(|node| node.key.as_deref() == Some("provider-connect-input"))
        .expect("manual response field");
    assert!(app.element_is_secure(&input.path));
    assert!(input.visible[2] > 0.0 && input.visible[3] > 0.0);
    for node in elements {
        assert!(
            !node
                .text
                .as_deref()
                .unwrap_or("")
                .contains("private-fixture")
        );
        assert!(
            !node
                .label
                .as_deref()
                .unwrap_or("")
                .contains("private-fixture")
        );
    }
    assert!(
        app.find("provider-copy-code").is_some() && app.find("provider-open-browser").is_some()
    );
    // Headless clients record the request and never open an OS browser.
    click(&mut app, "provider-open-browser");
    assert_eq!(
        app.take_url_requests(),
        ["https://example.invalid/authorize?state=private-fixture-state"]
    );
    // Use an OS opener that fails, then succeeds on retry. The live login
    // challenge is kept inside the modal and its URL is never shown as an error.
    app.state.provider_browser = Some(|_| false);
    app.poll = Some(PrimeDesktopView::poll);
    click(&mut app, "provider-open-browser");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while app.find("provider-connect-error").is_none() {
        app.frame(1.0 / 60.0);
        assert!(
            std::time::Instant::now() < deadline,
            "browser error did not appear"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let error = app
        .elements()
        .into_iter()
        .find(|node| node.key.as_deref() == Some("provider-connect-error"))
        .unwrap();
    assert!(error.visible[2] > 0.0 && error.visible[3] > 0.0);
    assert!(
        !error
            .text
            .as_deref()
            .unwrap_or("")
            .contains("private-fixture")
    );
    app.state.provider_browser = Some(|_| true);
    click(&mut app, "provider-open-browser");
    assert!(app.find("provider-connect-error").is_none());
    assert_eq!(app.state.active_modal, Some(ActiveModal::ProviderConnect));
}
