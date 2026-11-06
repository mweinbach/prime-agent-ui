//! Route a provider connection back to its invoking surface, and accept worker
//! updates only for the attempt that still owns the selected CLI profile.
use super::background::Poster;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::oneshot;
use uuid::Uuid;

use super::{ActiveModal, BridgeRequest, PrimeDesktopView};
use crate::provider_connect::{
    self, ConnectMethod, ConnectStatus, ConnectionPane, ConnectionUi, LoginEvent, LoginGate,
};
use crate::state::provider_credentials::CredentialSlot;

#[derive(Clone, Copy, PartialEq, Eq)]
enum CredentialAction {
    Connect(ConnectMethod),
    RemoveKey,
}

pub(super) struct LoginAttempt {
    id: Uuid,
    opening_browser: bool,
    action: CredentialAction,
    gate: Arc<LoginGate>,
    stop: Option<oneshot::Sender<()>>,
    input: Option<oneshot::Sender<String>>,
    choice: Option<oneshot::Sender<usize>>,
}

impl Drop for LoginAttempt {
    fn drop(&mut self) {
        self.gate.cancel();
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

pub(super) struct ConnectionOrigin {
    modal: Option<ActiveModal>,
    query: String,
    index: usize,
    model_key: Option<(String, String)>,
    retry: Option<ModelIntent>,
}

pub(super) struct ModelIntent {
    session: String,
    provider: String,
    model: String,
    previous_model: Option<(String, String)>,
}

pub(super) struct ConnectedCatalog {
    pub(super) request: Uuid,
    intent: ModelIntent,
}

impl PrimeDesktopView {
    pub(in crate::ui) fn open_provider_connect(&mut self, provider: &str) {
        self.show_connection(provider, None);
    }

    pub(super) fn connect_for_model(&mut self, session: &str, provider: &str, model: &str) {
        if session.is_empty() || session != self.session.active_session_id {
            return;
        }
        if self
            .active_modal
            .is_some_and(|modal| modal != ActiveModal::ModelPicker)
        {
            self.toast = Some((
                "Sign in from the model picker to use that provider.".into(),
                true,
            ));
            return;
        }
        let retry = ModelIntent {
            session: session.into(),
            provider: provider.into(),
            model: model.into(),
            previous_model: self.current_model_identity(),
        };
        self.show_connection(provider, Some(retry));
    }

    fn current_model_identity(&self) -> Option<(String, String)> {
        self.session
            .model_provider
            .clone()
            .zip(self.session.model_id.clone())
    }

    fn show_connection(&mut self, provider: &str, retry: Option<ModelIntent>) {
        if self.auth_write_pending && self.login_attempt.is_none() {
            return;
        }
        let pane = if provider.is_empty() {
            None
        } else {
            let Some(row) = self
                .provider_auth
                .iter()
                .find(|item| item.provider_id == provider)
            else {
                self.toast = Some((
                    "Could not find this provider in the selected CLI profile.".into(),
                    true,
                ));
                return;
            };
            if !row.can_connect && !row.can_edit_api_key {
                self.toast = Some((
                    "This credential is managed outside the desktop.".into(),
                    true,
                ));
                return;
            }
            Some(ConnectionPane {
                provider_id: provider.into(),
                display_name: row.display_name.clone(),
                can_browser: row.can_connect,
                can_api_key: row.can_edit_api_key,
                status: ConnectStatus::Choose,
                progress: String::new(),
                error: None,
                browser: None,
                prompt: None,
                choice: None,
            })
        };
        if !self.stop_login_attempt() {
            return;
        }
        if self.active_modal != Some(ActiveModal::ProviderConnect) {
            self.connection_origin = Some(ConnectionOrigin {
                modal: if retry.is_some() {
                    Some(ActiveModal::ModelPicker)
                } else {
                    self.active_modal
                },
                query: self.modal_search_input.text.clone(),
                index: self.modal_selected_idx,
                model_key: self.modal_model_key.clone(),
                retry,
            });
        }
        // Browsing to a different provider turns this into a generic connection.
        // Its credentials must not trigger the earlier provider's model change.
        if let Some(origin) = self.connection_origin.as_mut()
            && origin
                .retry
                .as_ref()
                .is_some_and(|intent| intent.provider != provider)
        {
            origin.retry = None;
        }
        self.connected_catalog = None;
        self.reasoning_picker_open = false;
        self.connection_input.replace_text("");
        let method = pane
            .as_ref()
            .and_then(|p| match (p.can_browser, p.can_api_key) {
                (true, false) => Some(ConnectMethod::Browser),
                (false, true) => Some(ConnectMethod::ApiKey),
                _ => None,
            });
        if pane.is_none() {
            self.connection_input.set_secure(false);
            self.reload_local_stores();
        }
        self.provider_connection = pane;
        self.active_modal = Some(ActiveModal::ProviderConnect);
        self.focus_active_surface();
        if let Some(method) = method {
            self.start_provider_connection(method);
        }
    }

    pub(in crate::ui) fn start_provider_connection(&mut self, method: ConnectMethod) {
        let Some(pane) = self.provider_connection.as_ref() else {
            return;
        };
        if self.active_modal != Some(ActiveModal::ProviderConnect)
            || self.login_attempt.is_some()
            || !(if method == ConnectMethod::Browser {
                pane.can_browser
            } else {
                pane.can_api_key
            })
        {
            return;
        }
        let provider = pane.provider_id.clone();
        self.connection_input.replace_text("");
        let pane = self.provider_connection.as_mut().expect("active provider");
        pane.clear_step();
        pane.error = None;
        pane.status = ConnectStatus::Working;
        pane.progress = "Starting connection…".into();
        let dir = self.agent_dir.clone();
        let (id, gate, stopped) = self.begin_credential_write(CredentialAction::Connect(method));
        let poster = self.background.poster();
        self.bridge.spawn_io(async move {
            let result = tokio::select! {
                biased;
                _ = stopped => return,
                outcome = run_connection(id, method, dir, provider, gate, poster.clone()) => outcome,
            };
            poster.post(move |s| s.login_finished(id, result));
        });
    }

    fn begin_credential_write(
        &mut self,
        action: CredentialAction,
    ) -> (Uuid, Arc<LoginGate>, oneshot::Receiver<()>) {
        let id = Uuid::new_v4();
        let gate = Arc::new(LoginGate::default());
        let (stop, stopped) = oneshot::channel();
        self.login_attempt = Some(LoginAttempt {
            id,
            opening_browser: false,
            action,
            gate: Arc::clone(&gate),
            stop: Some(stop),
            input: None,
            choice: None,
        });
        self.auth_write_pending = true;
        (id, gate, stopped)
    }

    fn login_event(&mut self, id: Uuid, event: LoginEvent) {
        if self.active_modal != Some(ActiveModal::ProviderConnect) {
            return;
        }
        let (Some(attempt), Some(pane)) = (&mut self.login_attempt, &mut self.provider_connection)
        else {
            return;
        };
        if attempt.id != id || pane.status != ConnectStatus::Working {
            return;
        }
        match event {
            LoginEvent::Progress(message) => pane.progress = message,
            LoginEvent::Browser(step) => pane.browser = Some(step),
            LoginEvent::Prompt(prompt, reply) => {
                pane.choice = None;
                attempt.choice = None;
                self.connection_input.replace_text("");
                self.connection_input.set_secure(prompt.secret);
                pane.prompt = Some(prompt);
                attempt.input = Some(reply);
                self.focus_input(|s| &mut s.connection_input);
            }
            LoginEvent::Choice(choice, reply) => {
                pane.clear_step();
                pane.choice = Some(choice);
                attempt.input = None;
                attempt.choice = Some(reply);
                self.connection_input.replace_text("");
            }
            LoginEvent::InvalidBrowser => {
                self.login_finished(id, Err("The provider returned an unexpected sign-in link."));
            }
        }
    }

    fn login_saving(&mut self, id: Uuid) {
        if self.login_attempt.as_ref().is_some_and(|a| a.id == id)
            && let Some(pane) = self.provider_connection.as_mut()
        {
            pane.clear_step();
            pane.status = ConnectStatus::Saving;
            pane.progress = "Saving credential…".into();
            self.connection_input.replace_text("");
        }
    }

    fn login_finished(&mut self, id: Uuid, result: Result<(), &'static str>) {
        if self.login_attempt.as_ref().is_none_or(|a| a.id != id) {
            return;
        }
        let action = self
            .login_attempt
            .take()
            .expect("matching login attempt")
            .action;
        self.auth_write_pending = false;
        self.connection_input.replace_text("");
        let Some(pane) = self.provider_connection.as_mut() else {
            return;
        };
        pane.clear_step();
        if let Err(message) = result {
            pane.status = ConnectStatus::Failed;
            pane.error = Some(message.into());
            return;
        }
        let message = if action == CredentialAction::Connect(ConnectMethod::ApiKey)
            && pane.provider_id != "prime-inference"
        {
            format!("Saved API key for {}.", pane.display_name)
        } else {
            format!("Connected to {}.", pane.display_name)
        };
        let retry = self.restore_connection_origin();
        self.reload_local_stores();
        self.toast = Some((message, false));
        if let Some(intent) = retry
            && self.daemon_connected
            && intent.session == self.session.active_session_id
            && intent.previous_model == self.current_model_identity()
            && self.active_modal == Some(ActiveModal::ModelPicker)
        {
            let request = Uuid::new_v4();
            self.bridge.send(BridgeRequest::RefreshConnectedCatalog {
                request,
                active_session_id: intent.session.clone(),
            });
            self.connected_catalog = Some(ConnectedCatalog { request, intent });
        } else if self.daemon_connected && !self.session.active_session_id.is_empty() {
            self.bridge.send(BridgeRequest::RefreshInspector {
                active_session_id: self.session.active_session_id.clone(),
            });
        }
    }

    pub(super) fn finish_connected_catalog(
        &mut self,
        request: Uuid,
        session: &str,
        revision: u64,
        data: Option<serde_json::Value>,
    ) {
        if self
            .connected_catalog
            .as_ref()
            .is_none_or(|pending| pending.request != request)
        {
            return;
        }
        let pending = self
            .connected_catalog
            .take()
            .expect("matching catalog request")
            .intent;
        if pending.session != session
            || self.session.active_session_id != session
            || self.current_model_identity() != pending.previous_model
            || self.active_modal != Some(ActiveModal::ModelPicker)
        {
            return;
        }
        if revision >= self.catalog_revision {
            let Some(data) = data else {
                self.toast = Some((
                    "Connected, but the model list could not be refreshed.".into(),
                    true,
                ));
                return;
            };
            self.accept_model_catalog(revision, &data);
        }
        if self
            .catalog_models
            .iter()
            .any(|m| m.configured && m.provider == pending.provider && m.id == pending.model)
        {
            self.select_model_after_connect(session, &pending.provider, &pending.model);
        } else {
            self.toast = Some((
                "Connected, but the selected model is unavailable in this session.".into(),
                true,
            ));
        }
    }

    pub(in crate::ui) fn open_connection_browser(&mut self, tx: &mut rune::Tx) {
        const FAILURE: &str =
            "The browser could not be opened. Check your default browser and try again.";
        let Some(pane) = self.provider_connection.as_mut() else {
            return;
        };
        let Some(step) = pane.browser.as_ref() else {
            return;
        };
        pane.error = None;
        let Some(open) = self.provider_browser else {
            if tx.open_url(&step.url).is_err() {
                pane.error = Some(FAILURE.into());
            }
            return;
        };
        let attempt_id = if let Some(attempt) = self.login_attempt.as_mut() {
            if std::mem::replace(&mut attempt.opening_browser, true) {
                return;
            }
            Some(attempt.id)
        } else {
            None
        };
        let url = step.url.clone();
        self.background.run(&self.bridge, move || {
            let opened = open(&url);
            move |view: &mut Self| {
                if view.login_attempt.as_ref().map(|current| current.id) != attempt_id {
                    return;
                }
                if let Some(attempt) = view.login_attempt.as_mut() {
                    attempt.opening_browser = false;
                }
                if !opened
                    && let Some(pane) = view.provider_connection.as_mut()
                    && pane.status == ConnectStatus::Working
                    && pane
                        .browser
                        .as_ref()
                        .is_some_and(|current| current.url == url)
                {
                    pane.error = Some(FAILURE.into());
                }
            }
        });
    }

    pub(in crate::ui) fn submit_connection_input(&mut self) {
        let (Some(attempt), Some(pane)) = (&mut self.login_attempt, &mut self.provider_connection)
        else {
            return;
        };
        let Some(prompt) = &pane.prompt else {
            return;
        };
        if self.connection_input.text.trim().is_empty() && !prompt.allow_empty {
            return;
        }
        if let Some(reply) = attempt.input.take() {
            let value = self.connection_input.text.clone();
            self.connection_input.replace_text("");
            pane.prompt = None;
            let _ = reply.send(value);
        }
    }

    pub(in crate::ui) fn select_connection_choice(&mut self, index: usize) {
        let (Some(attempt), Some(pane)) = (&mut self.login_attempt, &mut self.provider_connection)
        else {
            return;
        };
        if pane
            .choice
            .as_ref()
            .is_none_or(|p| index >= p.options.len())
        {
            return;
        }
        if let Some(reply) = attempt.choice.take() {
            pane.choice = None;
            let _ = reply.send(index);
        }
    }

    pub(in crate::ui) fn confirm_remove_provider_key(&mut self, provider: &str) {
        if self.auth_write_pending || self.active_modal != Some(ActiveModal::SettingsAuth) {
            return;
        }
        let Some(row) = self.provider_auth.iter().find(|row| {
            row.provider_id == provider && row.can_edit_api_key && row.has_stored_api_key
        }) else {
            return;
        };
        let name = row.display_name.clone();
        let provider_id = provider.to_owned();
        let profile = self.agent_dir.clone();
        let (id, gate, stopped) = self.begin_credential_write(CredentialAction::RemoveKey);
        let poster = self.background.poster();
        self.bridge.spawn_io(async move {
            let removal =
                remove_key_after_confirmation(id, &profile, provider_id, &name, gate, &poster);
            let result = tokio::select! {
                biased;
                _ = stopped => return,
                outcome = removal => outcome,
            };
            poster.post(move |view| view.finish_key_removal(id, &profile, &name, result));
        });
    }

    fn finish_key_removal(
        &mut self,
        id: Uuid,
        profile: &std::path::Path,
        name: &str,
        result: Result<bool, &'static str>,
    ) {
        if self
            .login_attempt
            .as_ref()
            .is_none_or(|attempt| attempt.id != id)
        {
            return;
        }
        self.login_attempt = None;
        self.auth_write_pending = false;
        if self.agent_dir != profile {
            return;
        }
        match result {
            Ok(true) => {
                self.toast = Some((format!("Removed the saved API key for {name}."), false));
                self.reload_local_stores();
                if self.daemon_connected && !self.session.active_session_id.is_empty() {
                    self.bridge.send(BridgeRequest::RefreshInspector {
                        active_session_id: self.session.active_session_id.clone(),
                    });
                }
            }
            Ok(false) => {}
            Err(message) => {
                self.toast = Some((message.into(), true));
                self.reload_local_stores();
            }
        }
    }

    fn stop_login_attempt(&mut self) -> bool {
        if let Some(attempt) = &self.login_attempt {
            // Test and take the gate atomically against the storage lock's commit.
            if !attempt.gate.cancel() && attempt.gate.is_committing() {
                return false;
            }
            self.login_attempt = None;
            self.auth_write_pending = false;
        }
        true
    }

    fn restore_connection_origin(&mut self) -> Option<ModelIntent> {
        self.provider_connection = None;
        self.connection_input.replace_text("");
        let origin = self.connection_origin.take()?;
        self.active_modal = origin.modal;
        self.modal_search_input.replace_text(origin.query);
        self.modal_selected_idx = origin.index;
        self.modal_model_key = origin.model_key;
        self.focus_active_surface();
        origin.retry
    }

    pub(in crate::ui) fn cancel_provider_connection(&mut self) {
        if self.stop_login_attempt() {
            self.restore_connection_origin();
        }
    }

    pub(super) fn leave_provider_connection(&mut self) -> bool {
        if self
            .login_attempt
            .as_ref()
            .is_some_and(|job| job.action == CredentialAction::RemoveKey)
        {
            if !self.stop_login_attempt() {
                return false;
            }
            self.confirm = None;
        }
        if self.active_modal == Some(ActiveModal::ProviderConnect) {
            if !self.stop_login_attempt() {
                return false;
            }
            self.restore_connection_origin();
        }
        self.connected_catalog = None;
        true
    }
}

/// Capture the credential before asking for consent. The CLI can rotate it
/// while the dialog is open; the worker removes only the captured record.
async fn remove_key_after_confirmation(
    id: Uuid,
    profile: &std::path::Path,
    provider: String,
    name: &str,
    gate: Arc<LoginGate>,
    poster: &Poster,
) -> Result<bool, &'static str> {
    let directory = profile.to_owned();
    let slot = tokio::task::spawn_blocking(move || CredentialSlot::read(&directory, &provider))
        .await
        .ok()
        .and_then(Result::ok)
        .filter(CredentialSlot::has_stored_api_key)
        .ok_or("No editable saved key was found. Refresh Settings and try again.")?;
    let (answer, confirmed) = oneshot::channel();
    let question = format!("Remove the saved API key for {name}?");
    let expected_dir = profile.to_owned();
    poster.post(move |s| {
        if s.agent_dir == expected_dir
            && s.active_modal == Some(ActiveModal::SettingsAuth)
            && s.login_attempt
                .as_ref()
                .is_some_and(|attempt| attempt.id == id)
        {
            s.confirm(question, "Remove key", move |_| {
                let _ = answer.send(());
            });
        }
    });
    if confirmed.await.is_err() || gate.is_cancelled() {
        return Ok(false);
    }
    tokio::task::spawn_blocking(move || slot.remove_confirmed_key(|| gate.begin_commit()))
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or("Could not remove the saved key. Check the CLI data folder or try again if the credential changed.")?;
    Ok(true)
}

/// All file and provider I/O stays off the Rune thread. The snapshot belongs
/// to this provider slot; writes unrelated CLIs make to other slots still merge.
async fn run_connection(
    id: Uuid,
    method: ConnectMethod,
    dir: PathBuf,
    provider: String,
    gate: Arc<LoginGate>,
    poster: Poster,
) -> Result<(), &'static str> {
    let profile = dir.clone();
    let name = provider.clone();
    let slot = tokio::task::spawn_blocking(move || CredentialSlot::read(&profile, &name))
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or("Could not read this provider’s credential. Check the CLI data folder.")?;
    let events = poster.clone();
    let slot = Arc::new(slot);
    let ui = ConnectionUi::new(
        provider,
        dir,
        Some(Arc::clone(&slot)),
        Arc::clone(&gate),
        move |event| events.post(move |s| s.login_event(id, event)),
    );
    let credential = provider_connect::login(method, &ui).await?;
    if gate.is_cancelled() {
        return Err("Connection cancelled.");
    }
    tokio::task::spawn_blocking(move || {
        slot.save_checked(credential, || {
            if !gate.begin_commit() {
                return false;
            }
            poster.post(move |s| s.login_saving(id));
            true
        })
    })
    .await
    .ok()
    .and_then(Result::ok)
    .ok_or("Could not save the credential. Check the CLI data folder or reconnect if the credential changed.")
}

#[cfg(test)]
mod tests;
