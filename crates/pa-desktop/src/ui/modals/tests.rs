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
    let chosen = app.state.catalog_models[index].clone();
    app.update(|view, _| {
        view.catalog_models
            .retain(|model| model.provider != "other")
    });
    app.frame(0.0);
    key(&mut app, Key::Enter);
    assert_eq!(app.state.active_modal, Some(ActiveModal::ModelPicker));
    assert!(
        !app.state.model_setting_pending(),
        "removing a highlighted model must never submit the row that replaced it"
    );
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
