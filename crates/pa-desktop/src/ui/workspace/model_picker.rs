//! Model and reasoning controls operate on the session that supplied their catalog.
use super::{ActiveModal, BridgeRequest, CatalogModel, PrimeDesktopView, SessionSelection};

impl PrimeDesktopView {
    #[must_use]
    pub fn current_catalog_model(&self) -> Option<&CatalogModel> {
        self.catalog_models.iter().find(|model| {
            Some(model.provider.as_str()) == self.session.model_provider.as_deref()
                && Some(model.id.as_str()) == self.session.model_id.as_deref()
        })
    }

    #[must_use]
    pub fn model_setting_pending(&self) -> bool {
        self.pending_model_setting.is_some()
    }

    #[must_use]
    pub fn model_catalog_loaded(&self) -> bool {
        self.catalog_loaded || !self.catalog_models.is_empty()
    }

    #[must_use]
    pub fn model_picker_ready(&self) -> bool {
        self.daemon_connected
            && !self.session.active_session_id.is_empty()
            && matches!(self.selection, SessionSelection::Idle)
            && !self.model_setting_pending()
    }

    pub(super) fn prepare_model_catalog(&mut self) {
        if !self.daemon_connected || !matches!(self.selection, SessionSelection::Idle) {
            return;
        }
        if self.session.active_session_id.is_empty() {
            self.open_session(BridgeRequest::CreateSession {
                cwd: self.working_cwd.clone(),
                name: None,
                initial_prompt: None,
                images: Vec::new(),
            });
        } else if self.catalog_models.is_empty() {
            self.catalog_loaded = false;
            self.bridge.send(BridgeRequest::RefreshInspector {
                active_session_id: self.session.active_session_id.clone(),
            });
        }
    }

    #[must_use]
    pub fn can_open_reasoning_picker(&self) -> bool {
        if self.reasoning_picker_open {
            return true;
        }
        self.daemon_connected
            && !self.model_setting_pending()
            && matches!(self.selection, SessionSelection::Idle)
            && (self.session.active_session_id.is_empty()
                || !self.model_catalog_loaded()
                || self
                    .current_catalog_model()
                    .is_some_and(|model| model.supported_levels.len() > 1))
    }

    pub(super) fn validate_reasoning_picker(&mut self) {
        if self.reasoning_picker_open
            && self.model_catalog_loaded()
            && !self
                .current_catalog_model()
                .is_some_and(|model| model.supported_levels.len() > 1)
        {
            self.close_reasoning_picker();
            let message = if self.current_catalog_model().is_some() {
                "This model has a fixed reasoning effort."
            } else {
                "Reasoning options are unavailable for the current model."
            };
            self.toast = Some((message.into(), false));
        }
    }

    pub fn open_reasoning_picker(&mut self) {
        if !self.can_open_reasoning_picker() {
            return;
        }
        if self.active_modal.is_some() {
            self.close_modal();
        }
        self.prepare_model_catalog();
        self.reasoning_picker_open = true;
    }

    pub fn close_reasoning_picker(&mut self) {
        self.reasoning_picker_open = false;
    }

    /// Apply the canonical desktop/daemon level, even when the provider maps it
    /// to another string in the wire catalog (for example, "max" to "maximum").
    pub fn select_reasoning_level(
        &mut self,
        expected_session: &str,
        provider: &str,
        model_id: &str,
        level: &str,
    ) -> bool {
        if !self.reasoning_picker_open
            || !self.model_picker_ready()
            || expected_session != self.session.active_session_id
            || !self.current_catalog_model().is_some_and(|model| {
                model.provider == provider
                    && model.id == model_id
                    && model
                        .supported_levels
                        .iter()
                        .any(|supported| supported == level)
            })
        {
            return false;
        }
        self.close_reasoning_picker();
        if self.session.thinking_level != level {
            let request_id = uuid::Uuid::new_v4();
            self.pending_model_setting = Some(request_id);
            self.bridge.send(BridgeRequest::SetThinkingLevel {
                request_id,
                active_session_id: expected_session.into(),
                provider: provider.into(),
                model_id: model_id.into(),
                level: level.into(),
            });
        }
        true
    }

    pub fn select_model_for(
        &mut self,
        expected_session: &str,
        provider: &str,
        model_id: &str,
    ) -> bool {
        if self.active_modal != Some(ActiveModal::ModelPicker)
            || !self.model_picker_ready()
            || self.session.active_session_id != expected_session
            || !self.catalog_models.iter().any(|m| {
                m.provider == provider
                    && m.id == model_id
                    && m.matches_query(&self.modal_search_input.text)
                    && (!self.model_picker_configured_only || m.configured)
            })
        {
            return false;
        }
        if self.session.model_provider.as_deref() != Some(provider)
            || self.session.model_id.as_deref() != Some(model_id)
        {
            let request_id = uuid::Uuid::new_v4();
            self.pending_model_setting = Some(request_id);
            self.bridge.send(BridgeRequest::SetModel {
                request_id,
                active_session_id: expected_session.into(),
                provider: provider.into(),
                model_id: model_id.into(),
            });
        }
        self.close_modal();
        true
    }

    pub fn select_model(&mut self, provider: &str, model_id: &str) {
        let session_id = self.session.active_session_id.clone();
        self.select_model_for(&session_id, provider, model_id);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::bridge::BridgeEvent;
    use crate::ui::workspace::LaunchOptions;

    fn connected_view(profile: &TempDir) -> PrimeDesktopView {
        // An explicit socket prevents launching a daemon. Apply its replies manually,
        // including late frames from a previously attached session.
        let mut view = PrimeDesktopView::new(
            LaunchOptions {
                daemon_socket: Some(profile.path().join("model-picker.sock")),
                initial_cwd: Some(profile.path().into()),
                ..Default::default()
            },
            profile.path().into(),
        );
        view.daemon_connected = true;
        view.session.active_session_id = "attached".into();
        view.session.apply_state_object(&json!({
            "model":{"provider":"example","id":"alpha"}, "thinkingLevel":"low"
        }));
        view.apply_bridge_event(BridgeEvent::ModelCatalogLoaded {
            active_session_id: "attached".into(),
            data: json!({"models":[
                {"provider":"example","id":"alpha","reasoning":true,
                 "thinkingLevelMap":{"minimal":null,"max":"maximum"}},
                {"provider":"example","id":"fixed","reasoning":false}
            ]}),
        });
        view
    }

    #[test]
    fn effort_requires_the_same_session_model_and_an_advertised_canonical_level() {
        let profile = tempfile::tempdir().unwrap();
        let mut view = connected_view(&profile);
        view.open_reasoning_picker();
        for (sid, model, level) in [
            ("previous", "alpha", "max"),
            ("attached", "fixed", "off"),
            ("attached", "alpha", "minimal"),
            ("attached", "alpha", "maximum"),
        ] {
            assert!(!view.select_reasoning_level(sid, "example", model, level));
        }
        assert!(!view.model_setting_pending());
        assert!(view.select_reasoning_level("attached", "example", "alpha", "max"));
        let request_id = view.pending_model_setting.unwrap();
        assert!(!view.model_picker_ready());
        view.populate_draft("keep until the daemon applies the new effort");
        view.submit_composer();
        assert_eq!(
            view.composer_input.text,
            "keep until the daemon applies the new effort"
        );
        for (sid, id) in [("other", request_id), ("attached", uuid::Uuid::new_v4())] {
            view.apply_bridge_event(BridgeEvent::ModelSettingFinished {
                active_session_id: sid.into(),
                request_id: id,
                state: Ok(json!({"thinkingLevel": "high"})),
            });
            assert!(view.model_setting_pending());
            assert_eq!(view.session.thinking_level, "low");
        }
        view.apply_bridge_event(BridgeEvent::ModelSettingFinished {
            active_session_id: "attached".into(),
            request_id,
            state: Ok(json!({"thinkingLevel":"max"})),
        });
        assert!(view.model_picker_ready());
        assert_eq!(view.session.thinking_level, "max");
    }

    #[test]
    fn fixed_effort_models_have_no_picker_and_stale_model_rows_do_nothing() {
        let profile = tempfile::tempdir().unwrap();
        let mut view = connected_view(&profile);
        view.open_modal(ActiveModal::ModelPicker);
        assert!(!view.select_model_for("old-session", "example", "fixed"));
        assert!(!view.select_model_for("attached", "unknown", "alpha"));
        assert!(view.select_model_for("attached", "example", "fixed"));
        view.apply_bridge_event(BridgeEvent::ModelSettingFinished {
            active_session_id: "attached".into(),
            request_id: view.pending_model_setting.unwrap(),
            state: Ok(json!({"model":"example/fixed","thinkingLevel":"off"})),
        });
        assert!(!view.can_open_reasoning_picker());
        view.open_reasoning_picker();
        assert!(!view.reasoning_picker_open);
    }
}
