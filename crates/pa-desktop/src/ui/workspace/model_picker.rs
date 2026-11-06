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
            || !self
                .catalog_models
                .iter()
                .any(|m| m.provider == provider && m.id == model_id)
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
