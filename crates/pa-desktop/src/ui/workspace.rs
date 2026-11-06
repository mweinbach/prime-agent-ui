//! Shared desktop controller. Rune events and daemon updates meet on the UI thread.

mod background;
mod cli_data;
mod dialogs;
#[cfg(test)]
mod interaction_tests;
mod interactions;
mod layout;
mod projects;

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use pa_core::refinement::{HarnessScope, RefinementKind};
use rune::TextEdit;

use crate::bridge::{BridgeEvent, BridgeHandle, BridgeRequest, socket_path_for_target};
use crate::state::harness_store::{
    HarnessEntryDraft, HarnessSnapshot, ProviderAuthItem, delete_harness_entry,
    load_provider_auth_items, set_provider_api_key, upsert_harness_entry,
};
use crate::state::models::{CatalogModel, parse_model_catalog};
use crate::state::transcript::AttachedSessionState;
use crate::state::{
    ActiveModal, ActiveSessionRow, DraftKey, McpRow, SavedSessionRow, ScheduleRow,
    SessionTelemetry, ShellProcessRow, StagedImage, subagents::Subagents,
};
use crate::theme::{DaemonTarget, DesktopPreferences, InspectorTab, ThemeMode};

/// Launch configuration for the desktop window.
#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    pub daemon_socket: Option<PathBuf>,
    pub shared_daemon: bool,
    pub initial_cwd: Option<PathBuf>,
    pub theme_override: Option<ThemeMode>,
    pub initial_tab: Option<InspectorTab>,
    pub initial_modal: Option<ActiveModal>,
}

// Bound in-memory drafts and base64 encoding on the desktop; providers can impose lower limits.
const MAX_PROMPT_BYTES: usize = 1024 * 1024;
const MAX_ATTACHMENT_BYTES: usize = 15 * 1024 * 1024;
const MAX_ATTACHMENTS: usize = 16;

type Draft = (String, Vec<StagedImage>);

#[derive(Default)]
enum SessionSelection {
    #[default]
    Idle,
    Opening,
    WaitingFor(String),
}

impl SessionSelection {
    fn accepts_snapshot(&self, attached: &str, incoming: &str) -> bool {
        match self {
            Self::Idle => !attached.is_empty() && incoming == attached,
            Self::Opening => false,
            Self::WaitingFor(expected) => expected == incoming,
        }
    }
}

/// Application state shared by the native window and headless Rune runtime.
pub struct PrimeDesktopView {
    events: futures::channel::mpsc::UnboundedReceiver<BridgeEvent>,
    background: background::Background,
    waker: std::task::Waker,
    pending_dialog: Option<dialogs::Dialog>,
    viewport_width: f32,
    pub(super) confirm: Option<crate::ui::modals::Confirmation>,
    history: Vec<String>,
    history_cursor: Option<usize>,
    history_draft: String,
    pub bridge: BridgeHandle,
    pub prefs: DesktopPreferences,
    pub explicit_socket: Option<PathBuf>,
    pub agent_dir: PathBuf,
    pub(super) preferences_dir: PathBuf,
    pub(super) auth_write_pending: bool,
    pub working_cwd: String,

    pub daemon_connected: bool,
    pub daemon_socket_display: String,
    pub daemon_version: Option<String>,
    pub toast: Option<(String, bool)>,

    pub active_sessions: Vec<ActiveSessionRow>,
    pub saved_sessions: Vec<SavedSessionRow>,
    pub(crate) saved_next_cursor: Option<String>,
    /// A sidebar filter, independent of the conversation open in the stage.
    pub(super) sidebar_project: Option<String>,
    pub(super) sidebar_history_open: bool,

    pub session: AttachedSessionState,
    selection: SessionSelection,
    auto_attach: bool,
    store_request: u64,
    drafts: HashMap<DraftKey, VecDeque<Draft>>,
    pub telemetry: SessionTelemetry,
    pub catalog_models: Vec<CatalogModel>,
    pub configured_providers: HashSet<String>,
    pub model_picker_configured_only: bool,

    pub rlm_children: Subagents,
    pub rlm_max_depth: Option<u64>,
    pub shells: Vec<ShellProcessRow>,
    pub shell_tails: HashMap<String, String>,
    pub schedules: Vec<ScheduleRow>,
    pub mcp_rows: Vec<McpRow>,
    pub system_prompt: Option<String>,

    pub harness: HarnessSnapshot,
    pub harness_scope: HarnessScope,
    pub harness_kind_filter: Option<RefinementKind>,
    pub harness_editor_open: bool,
    pub harness_editor_kind: RefinementKind,
    pub harness_editing_id: Option<String>,

    pub provider_auth: Vec<ProviderAuthItem>,
    pub selected_auth_provider: String,

    pub active_modal: Option<ActiveModal>,
    pub(super) modal_selected_idx: usize,
    pub(super) slash_selected_idx: usize,
    pub renaming_session: bool,
    pub agent_mail_target: Option<String>,
    pub staged_images: Vec<StagedImage>,

    pub(super) follow_tail: bool,
    scroll_to_tail: bool,
    pub composer_input: TextEdit,
    pub sidebar_search_input: TextEdit,
    pub modal_search_input: TextEdit,
    pub rename_input: TextEdit,
    pub side_question_input: TextEdit,
    pub agent_mail_input: TextEdit,
    pub refine_input: TextEdit,
    pub harness_title_input: TextEdit,
    pub harness_body_input: TextEdit,
    pub quick_bash_input: TextEdit,
    pub schedule_interval_input: TextEdit,
    pub schedule_prompt_input: TextEdit,
    pub mcp_server_input: TextEdit,
    pub mcp_token_input: TextEdit,
    pub api_key_input: TextEdit,
}

impl PrimeDesktopView {
    /// Construct the root desktop workspace view and spawn the background daemon bridge.
    ///
    /// # Panics
    ///
    /// Panics only if the operating system refuses to create the 2-thread Tokio I/O runtime.
    pub fn new(options: LaunchOptions, preferences_dir: PathBuf) -> Self {
        let mut prefs = DesktopPreferences::load(&preferences_dir);
        let agent_dir = prefs
            .cli_data_dir
            .clone()
            .unwrap_or_else(|| preferences_dir.clone());
        if prefs.cli_data_dir.is_some() && options.daemon_socket.is_none() {
            prefs.daemon_target = DaemonTarget::DesktopHarness;
        } else if options.shared_daemon {
            prefs.daemon_target = crate::theme::DaemonTarget::SharedCli;
        }
        if let Some(theme) = options.theme_override {
            prefs.theme = theme;
        }
        if let Some(tab) = options.initial_tab {
            prefs.inspector_open = true;
            prefs.inspector_tab = tab;
        }

        let working_cwd = options
            .initial_cwd
            .or_else(|| prefs.last_project.as_deref().map(PathBuf::from))
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .filter(|path| path.parent().is_some())
            })
            .or_else(pa_types::platform::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .to_string_lossy()
            .to_string();

        let data = crate::bridge::CliData::new(agent_dir.clone());
        let (bridge, events) =
            BridgeHandle::spawn(data.clone()).expect("spawn pa-desktop I/O bridge");
        let sock_path =
            socket_path_for_target(prefs.daemon_target, options.daemon_socket.as_deref(), &data);
        bridge.send(BridgeRequest::Connect {
            target: prefs.daemon_target,
            explicit_socket: options.daemon_socket.clone(),
        });

        let auto_attach = prefs.daemon_target == crate::theme::DaemonTarget::DesktopHarness
            && options.daemon_socket.is_none();
        let mut instance = Self {
            events,
            background: background::Background::new(),
            waker: futures::task::noop_waker(),
            pending_dialog: None,
            viewport_width: 1360.0,
            confirm: None,
            history: Vec::new(),
            history_cursor: None,
            history_draft: String::new(),
            bridge,
            prefs,
            explicit_socket: options.daemon_socket,
            agent_dir,
            preferences_dir,
            auth_write_pending: false,
            working_cwd,
            daemon_connected: false,
            daemon_socket_display: sock_path.to_string_lossy().to_string(),
            daemon_version: None,
            toast: None,
            active_sessions: Vec::new(),
            saved_sessions: Vec::new(),
            saved_next_cursor: None,
            sidebar_project: None,
            sidebar_history_open: false,
            session: AttachedSessionState::default(),
            selection: SessionSelection::Idle,
            auto_attach,
            store_request: 0,
            drafts: HashMap::new(),
            telemetry: SessionTelemetry::default(),
            catalog_models: Vec::new(),
            configured_providers: HashSet::new(),
            model_picker_configured_only: false,
            rlm_children: Subagents::default(),
            rlm_max_depth: None,
            shells: Vec::new(),
            shell_tails: HashMap::new(),
            schedules: Vec::new(),
            mcp_rows: Vec::new(),
            system_prompt: None,
            harness: HarnessSnapshot::default(),
            harness_scope: HarnessScope::Global,
            harness_kind_filter: None,
            harness_editor_open: false,
            harness_editor_kind: RefinementKind::Memory,
            harness_editing_id: None,
            provider_auth: Vec::new(),
            selected_auth_provider: "prime-inference".to_string(),
            active_modal: options.initial_modal,
            modal_selected_idx: 0,
            slash_selected_idx: 0,
            renaming_session: false,
            agent_mail_target: None,
            staged_images: Vec::new(),
            follow_tail: true,
            scroll_to_tail: false,
            composer_input: TextEdit::new(""),
            sidebar_search_input: TextEdit::new(""),
            modal_search_input: TextEdit::new(""),
            rename_input: TextEdit::new(""),
            side_question_input: TextEdit::new(""),
            agent_mail_input: TextEdit::new(""),
            refine_input: TextEdit::new(""),
            harness_title_input: TextEdit::new(""),
            harness_body_input: TextEdit::new(""),
            quick_bash_input: TextEdit::new(""),
            schedule_interval_input: TextEdit::new(""),
            schedule_prompt_input: TextEdit::new(""),
            mcp_server_input: TextEdit::new(""),
            mcp_token_input: TextEdit::new("").secure(),
            api_key_input: TextEdit::new("").secure(),
        };
        instance.reload_local_stores();
        instance.focus_active_surface();
        instance
    }

    fn apply_bridge_event(&mut self, event: BridgeEvent) {
        match event {
            BridgeEvent::ConnectionStatus {
                connected,
                target,
                socket_path,
                daemon_version,
                error,
            } => {
                if self.prefs.daemon_target != target {
                    return;
                }
                self.daemon_connected = connected;
                if !connected {
                    self.stash_draft();
                    self.reset_session();
                    self.restore_draft();
                    self.selection = SessionSelection::Idle;
                    self.active_sessions.clear();
                    // Reset invalidates the pending store request, including the
                    // initial load if the offline socket answered first.
                    self.reload_local_stores();
                }
                self.daemon_socket_display = socket_path.to_string_lossy().to_string();
                self.daemon_version = daemon_version;
                if let Some(err) = error {
                    self.toast = Some((err, true));
                }
            }
            BridgeEvent::ActiveSessions(rows) => {
                let candidate = (std::mem::take(&mut self.auto_attach)
                    && self.session.active_session_id.is_empty()
                    && self.composer_input.text.is_empty()
                    && self.staged_images.is_empty())
                .then(|| {
                    rows.iter()
                        .find(|r| r.rlm_depth == 0 && r.cwd == self.working_cwd)
                })
                .flatten()
                .map(|r| r.active_session_id.clone());
                self.active_sessions = rows;
                if let Some(active_session_id) = candidate {
                    self.open_session(BridgeRequest::AttachSession { active_session_id });
                }
            }
            BridgeEvent::RosterDelta {
                changed,
                removed,
                resync,
            } => {
                if resync {
                    self.active_sessions.clear();
                }
                self.active_sessions
                    .retain(|s| !removed.contains(&s.agent_id));
                for row in changed {
                    if let Some(index) = self
                        .active_sessions
                        .iter()
                        .position(|s| s.agent_id == row.agent_id)
                    {
                        self.active_sessions[index] = row;
                    } else {
                        self.active_sessions.push(row);
                    }
                }
            }
            BridgeEvent::SavedSessionsPage {
                cursor,
                next_cursor,
                rows,
            } => {
                if cursor.is_none() {
                    self.saved_sessions = rows;
                } else if cursor == self.saved_next_cursor {
                    let mut known: HashSet<_> = self
                        .saved_sessions
                        .iter()
                        .map(|row| row.path.clone())
                        .collect();
                    self.saved_sessions.extend(
                        rows.into_iter()
                            .filter(|row| known.insert(row.path.clone())),
                    );
                } else {
                    return;
                }
                self.saved_next_cursor = next_cursor;
            }
            BridgeEvent::SessionOpening { active_session_id } => {
                if matches!(self.selection, SessionSelection::Opening) {
                    self.selection = SessionSelection::WaitingFor(active_session_id);
                }
            }
            BridgeEvent::AttachRedirect { requested, actual } => {
                if matches!(&self.selection, SessionSelection::WaitingFor(id) if id == &requested) {
                    self.selection = SessionSelection::WaitingFor(actual);
                }
            }
            BridgeEvent::SessionTransitionFailed { message } => {
                self.selection = SessionSelection::Idle;
                self.restore_draft();
                self.toast = Some((message, true));
            }
            BridgeEvent::PromptRejected {
                draft,
                message,
                images,
                reason,
            } => {
                self.drafts
                    .entry(draft)
                    .or_default()
                    .push_back((message, images));
                self.restore_draft();
                self.toast = Some((reason, true));
            }
            BridgeEvent::SessionAttached {
                active_session_id,
                snapshot,
            } => {
                if !self
                    .selection
                    .accepts_snapshot(&self.session.active_session_id, &active_session_id)
                {
                    return;
                }
                let newly_attached = self.session.active_session_id != active_session_id;
                if newly_attached {
                    self.reset_session();
                }
                self.selection = SessionSelection::Idle;
                if let Some(state_val) = snapshot.get("state") {
                    self.telemetry.update_from_state(state_val);
                }
                self.session.load_snapshot(&active_session_id, &snapshot);
                if !self.session.cwd.is_empty() {
                    self.working_cwd.clone_from(&self.session.cwd);
                    self.remember_project();
                }
                if newly_attached {
                    self.bridge
                        .send(BridgeRequest::RefreshInspector { active_session_id });
                    self.reload_local_stores();
                    self.restore_draft();
                }
                self.follow_transcript();
            }
            BridgeEvent::SnapshotChunkBegin {
                active_session_id,
                header,
            } => {
                if !self
                    .selection
                    .accepts_snapshot(&self.session.active_session_id, &active_session_id)
                {
                    return;
                }
                if self.session.active_session_id != active_session_id {
                    self.reset_session();
                }
                if let Some(state) = header.get("state") {
                    self.telemetry.update_from_state(state);
                }
                self.session
                    .begin_chunked_snapshot(&active_session_id, &header);
            }
            BridgeEvent::SnapshotChunk {
                active_session_id,
                messages,
            } => {
                if self.session.active_session_id == active_session_id {
                    self.session.append_snapshot_chunk(&messages);
                }
            }
            BridgeEvent::SnapshotChunkEnd { active_session_id } => {
                if self.session.active_session_id == active_session_id {
                    self.session.finish_chunked_snapshot();
                    self.selection = SessionSelection::Idle;
                    if !self.session.cwd.is_empty() {
                        self.working_cwd.clone_from(&self.session.cwd);
                        self.remember_project();
                    }
                    self.bridge
                        .send(BridgeRequest::RefreshInspector { active_session_id });
                    self.restore_draft();
                    self.reload_local_stores();
                    self.follow_transcript();
                }
            }
            BridgeEvent::SessionEvent {
                active_session_id,
                event,
            } => {
                if self.session.active_session_id == active_session_id {
                    if event.get("type").and_then(serde_json::Value::as_str)
                        == Some("desktop_state_sync")
                    {
                        if let Some(st) = event.get("state") {
                            self.session.apply_state_object(st);
                            self.telemetry.update_from_state(st);
                        }
                    } else {
                        let etype = event
                            .get("type")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        self.session.apply_live_event(&event);
                        if matches!(etype.as_str(), "turn_end" | "agent_end" | "compaction_end") {
                            self.reload_local_stores();
                            self.bridge
                                .send(BridgeRequest::RefreshInspector { active_session_id });
                        }
                    }
                }
            }
            BridgeEvent::SideQuestionStarted {
                active_session_id,
                side_question_id,
            } => {
                if self.session.active_session_id == active_session_id {
                    self.session.side_question.running_id = Some(side_question_id);
                }
            }
            BridgeEvent::SideQuestionEvent {
                active_session_id,
                event,
            } => {
                if self.session.active_session_id == active_session_id {
                    self.session.apply_side_question_event(&event);
                }
            }
            BridgeEvent::SessionClosed {
                active_session_id,
                reason,
            } => {
                self.active_sessions
                    .retain(|s| s.active_session_id != active_session_id);
                if self.session.active_session_id == active_session_id {
                    self.stash_draft();
                    self.reset_session();
                    self.restore_draft();
                    self.selection = SessionSelection::Idle;
                    self.toast = Some((format!("Session closed ({reason})"), false));
                }
            }
            BridgeEvent::ModelCatalogLoaded {
                active_session_id,
                data,
            } => {
                if self.session.active_session_id != active_session_id {
                    return;
                }
                let (models, providers) = parse_model_catalog(&data);
                self.catalog_models = models;
                self.configured_providers = providers;
            }
            BridgeEvent::SubagentUpdated {
                active_session_id,
                row,
                sequence,
            } => {
                if self.session.active_session_id == active_session_id {
                    self.rlm_children.update(row, sequence);
                }
            }
            BridgeEvent::SubagentsFetched {
                active_session_id,
                result,
            } => {
                if self.session.active_session_id == active_session_id {
                    match result {
                        Ok((sequence, rows)) => self.rlm_children.apply_snapshot(rows, sequence),
                        Err(error) => self.rlm_children.load_error = Some(error),
                    }
                }
            }
            BridgeEvent::InspectorSnapshot {
                active_session_id,
                rlm_max_depth,
                shells,
                schedules,
                mcp,
                stats,
                system_prompt,
            } => {
                if self.session.active_session_id == active_session_id {
                    if rlm_max_depth.is_some() {
                        self.rlm_max_depth = rlm_max_depth;
                    }
                    self.shells = shells;
                    self.schedules = schedules;
                    self.mcp_rows = mcp;
                    if let Some(ref st) = stats {
                        self.telemetry.update_from_stats(st);
                    }
                    if system_prompt.is_some() {
                        self.system_prompt = system_prompt;
                    }
                }
            }
            BridgeEvent::ShellTailLoaded {
                active_session_id,
                id,
                output,
            } => {
                if self.session.active_session_id == active_session_id {
                    self.shell_tails.insert(id, output);
                }
            }
            BridgeEvent::BackgroundRefreshNeeded => {
                if !self.session.active_session_id.is_empty() {
                    self.bridge.send(BridgeRequest::RefreshInspector {
                        active_session_id: self.session.active_session_id.clone(),
                    });
                }
            }
            BridgeEvent::Toast { message, is_error } => {
                self.toast = Some((message, is_error));
            }
        }
    }

    pub fn reload_local_stores(&mut self) {
        self.store_request += 1;
        let request = self.store_request;
        let dir = self.agent_dir.clone();
        let file = self.session.session_file.clone();
        let id = self.session.session_id.clone();
        self.background.run(&self.bridge, move || {
            let harness = HarnessSnapshot::load(&dir, file.as_deref(), Some(&id));
            let auth = load_provider_auth_items(&dir);
            move |view: &mut Self| {
                if view.store_request == request {
                    view.harness = harness;
                    view.provider_auth = auth;
                }
            }
        });
    }

    pub(in crate::ui) fn transcript_viewport_key(&self) -> String {
        format!("transcript-scroll-{}", self.session.active_session_id)
    }

    fn follow_transcript(&mut self) {
        self.follow_tail = true;
        if !self.session.active_session_id.is_empty() {
            self.scroll_to_tail = true;
            self.waker.wake_by_ref();
        }
    }

    fn reset_session(&mut self) {
        self.follow_tail = true;
        self.scroll_to_tail = false;
        self.session = AttachedSessionState::default();
        self.telemetry = SessionTelemetry::default();
        self.catalog_models.clear();
        self.configured_providers.clear();
        self.rlm_children = Subagents::default();
        self.rlm_max_depth = None;
        self.shells.clear();
        self.shell_tails.clear();
        self.schedules.clear();
        self.mcp_rows.clear();
        self.system_prompt = None;
        self.renaming_session = false;
        self.agent_mail_target = None;
        self.harness_editor_open = false;
        self.harness_editing_id = None;
        self.store_request += 1;
        self.harness.local_dir = None;
        self.harness.local_entries.clear();
        self.harness.history.clear();
        for input in [
            &mut self.side_question_input,
            &mut self.agent_mail_input,
            &mut self.quick_bash_input,
            &mut self.mcp_token_input,
        ] {
            input.replace_text("");
        }
    }

    pub(crate) fn open_session(&mut self, request: BridgeRequest) {
        if !self.ready_for_request() {
            return;
        }
        self.auto_attach = false;
        self.selection = match &request {
            BridgeRequest::AttachSession { active_session_id } => {
                SessionSelection::WaitingFor(active_session_id.clone())
            }
            BridgeRequest::CreateSession { .. } | BridgeRequest::OpenSavedSession { .. } => {
                SessionSelection::Opening
            }
            _ => return,
        };
        if !matches!(&request, BridgeRequest::CreateSession { .. }) {
            self.stash_draft();
        }
        // Keep the current transcript visible until the attach succeeds. A refused attach
        // leaves the user on the parent and restores its composer draft.
        self.bridge.send(request);
    }

    fn ready_for_request(&mut self) -> bool {
        let error = if !self.daemon_connected {
            "Not connected. Use Refresh to reconnect."
        } else if !matches!(self.selection, SessionSelection::Idle) {
            "Opening the session. Wait before sending another request."
        } else {
            return true;
        };
        self.toast = Some((error.to_owned(), true));

        false
    }

    pub fn populate_draft(&mut self, prompt: &str) {
        self.composer_input.replace_text(prompt);
        self.slash_selected_idx = 0;
        self.focus_input(|s| &mut s.composer_input);
    }

    pub fn submit_composer(&mut self) {
        let trimmed = self.composer_input.text.trim().to_owned();
        if trimmed.is_empty() && self.staged_images.is_empty() {
            return;
        }
        let btw = trimmed == "/btw"
            || trimmed
                .strip_prefix("/btw")
                .is_some_and(|rest| rest.starts_with(char::is_whitespace));
        let local = matches!(trimmed.as_str(), "/harness" | "/model");
        let session_command = btw
            || trimmed.starts_with('!')
            || matches!(
                trimmed.as_str(),
                "/compact" | "/refine" | "/reload" | "/export"
            );
        if !local && trimmed != "/new" && !self.ready_for_request() {
            return;
        }
        let error = if trimmed.len() > MAX_PROMPT_BYTES {
            Some("Prompt exceeds 1 MiB. Shorten it before sending.")
        } else if !self.staged_images.is_empty()
            && (local || session_command || trimmed == "/new" || self.session.is_streaming)
        {
            Some("Remove attachments to send a command or steer a running turn.")
        } else if session_command && self.session.active_session_id.is_empty() {
            Some("Open a session before using this command.")
        } else if trimmed == "!" {
            Some("Enter a shell command after !.")
        } else {
            None
        };
        if let Some(message) = error {
            self.toast = Some((message.to_owned(), true));

            return;
        }

        self.follow_transcript();
        self.record_history(&trimmed);
        self.composer_input.replace_text("");
        let images = std::mem::take(&mut self.staged_images);

        // Builtin client slash shortcuts
        if btw {
            let q = trimmed.trim_start_matches("/btw").trim().to_owned();
            self.session.side_question.open = true;
            if !q.is_empty() && !self.session.active_session_id.is_empty() {
                self.session.side_question.question.clone_from(&q);
                self.bridge.send(BridgeRequest::StartSideQuestion {
                    active_session_id: self.session.active_session_id.clone(),
                    question: q,
                });
            }

            return;
        }
        match trimmed.as_str() {
            "/new" => {
                self.start_fresh_session();
                return;
            }
            "/model" => {
                self.open_modal(ActiveModal::ModelPicker);
                return;
            }
            "/harness" => {
                self.open_harness_tab();
                return;
            }
            "/compact" => {
                self.trigger_compact();
                return;
            }
            "/refine" => {
                self.trigger_refine();
                return;
            }
            "/reload" => {
                if !self.session.active_session_id.is_empty() {
                    self.bridge.send(BridgeRequest::ReloadSession {
                        active_session_id: self.session.active_session_id.clone(),
                    });
                }
                return;
            }
            "/export" => {
                self.prompt_export_html();
                return;
            }
            _ => {}
        }

        if let Some(cmd) = trimmed.strip_prefix('!').map(str::trim)
            && !cmd.is_empty()
            && !self.session.active_session_id.is_empty()
        {
            self.bridge.send(BridgeRequest::ExecuteBash {
                active_session_id: self.session.active_session_id.clone(),
                command: cmd.to_string(),
            });

            return;
        }

        if self.session.active_session_id.is_empty() {
            self.open_session(BridgeRequest::CreateSession {
                cwd: self.working_cwd.clone(),
                name: None,
                initial_prompt: Some(trimmed),
                images,
            });
        } else if self.session.is_streaming {
            self.bridge.send(BridgeRequest::SteerSession {
                active_session_id: self.session.active_session_id.clone(),
                message: trimmed,
            });
        } else {
            self.bridge.send(BridgeRequest::SendPrompt {
                active_session_id: self.session.active_session_id.clone(),
                message: trimmed,
                images,
            });
        }
    }

    pub fn submit_steer(&mut self) {
        self.submit_queued(|active_session_id, message| BridgeRequest::SteerSession {
            active_session_id,
            message,
        });
    }

    pub fn submit_follow_up(&mut self) {
        self.submit_queued(
            |active_session_id, message| BridgeRequest::FollowUpSession {
                active_session_id,
                message,
            },
        );
    }

    fn submit_queued(&mut self, request: impl FnOnce(String, String) -> BridgeRequest) {
        if !self.ready_for_request() || self.session.active_session_id.is_empty() {
            return;
        }
        if !self.staged_images.is_empty() {
            self.toast = Some((
                "Remove attachments before steering or queuing a follow-up.".to_owned(),
                true,
            ));

            return;
        }
        let msg = self.composer_input.text.trim().to_owned();
        if msg.is_empty() {
            return;
        }
        if msg.len() > MAX_PROMPT_BYTES {
            self.toast = Some((
                "Prompt exceeds 1 MiB. Shorten it before sending.".to_owned(),
                true,
            ));

            return;
        }
        self.follow_transcript();
        self.record_history(&msg);
        self.composer_input.replace_text("");
        self.bridge
            .send(request(self.session.active_session_id.clone(), msg));
        self.restore_draft();
    }

    pub fn toggle_bash_prefix(&mut self) {
        let cur = self.composer_input.text.clone();
        let next = if let Some(rest) = cur.strip_prefix('!') {
            rest.trim_start().to_string()
        } else {
            format!("! {cur}")
        };
        self.composer_input.replace_text(next);
        self.focus_input(|s| &mut s.composer_input);
    }

    pub(crate) fn parent_session_target(&self) -> Option<String> {
        let child = self
            .active_sessions
            .iter()
            .find(|row| row.active_session_id == self.session.active_session_id)?;
        self.active_sessions
            .iter()
            .find(|parent| {
                child.parent_active_session_id.as_ref() == Some(&parent.active_session_id)
                    || child.parent_session_id.is_some()
                        && child.parent_session_id == parent.session_id
                    || child.parent_session_path.is_some()
                        && child.parent_session_path == parent.session_file
            })
            .map(|row| row.active_session_id.clone())
    }

    pub fn open_harness_tab(&mut self) {
        self.set_inspector_tab(InspectorTab::Harness);
    }

    pub fn toggle_theme(&mut self) {
        self.prefs.theme = self.prefs.theme.toggled();
        self.save_preferences();
    }

    pub fn cycle_detail_level(&mut self) {
        self.prefs.detail_level = self.prefs.detail_level.next();
        self.save_preferences();
    }

    fn save_preferences(&mut self) {
        if let Err(err) = self.prefs.save(&self.preferences_dir) {
            self.toast = Some((format!("Cannot save desktop preferences: {err}"), true));
        }
    }

    pub fn switch_daemon_target(&mut self) {
        if self.prefs.cli_data_dir.is_some() {
            self.toast = Some((
                "Use the default data folder before attaching to the CLI daemon.".into(),
                true,
            ));

            return;
        }
        self.prefs.daemon_target = self.prefs.daemon_target.toggled();
        self.save_preferences();
        self.daemon_connected = false;
        self.explicit_socket = None;
        self.selection = SessionSelection::Idle;
        self.auto_attach = false;
        self.stash_draft();
        self.reset_session();
        self.restore_draft();
        self.active_sessions.clear();
        self.saved_sessions.clear();
        self.saved_next_cursor = None;
        self.bridge.send(BridgeRequest::Connect {
            target: self.prefs.daemon_target,
            explicit_socket: None,
        });
    }

    pub fn refresh_all(&mut self) {
        self.reload_local_stores();
        if !self.daemon_connected {
            self.bridge.send(BridgeRequest::Connect {
                target: self.prefs.daemon_target,
                explicit_socket: self.explicit_socket.clone(),
            });
            return;
        }
        self.bridge
            .send(BridgeRequest::RefreshWorkspace { cwd: None });
        if !self.session.active_session_id.is_empty() {
            self.bridge.send(BridgeRequest::RefreshInspector {
                active_session_id: self.session.active_session_id.clone(),
            });
        }
    }

    pub fn open_modal(&mut self, modal: ActiveModal) {
        if modal == ActiveModal::ModelPicker
            && self.session.active_session_id.is_empty()
            && self.daemon_connected
            && matches!(self.selection, SessionSelection::Idle)
        {
            self.open_session(BridgeRequest::CreateSession {
                cwd: self.working_cwd.clone(),
                name: None,
                initial_prompt: None,
                images: Vec::new(),
            });
        }
        if modal == ActiveModal::ModelPicker
            && self.catalog_models.is_empty()
            && !self.session.active_session_id.is_empty()
        {
            self.bridge.send(BridgeRequest::RefreshInspector {
                active_session_id: self.session.active_session_id.clone(),
            });
        }
        if modal == ActiveModal::SettingsAuth {
            self.reload_local_stores();
        }
        self.modal_selected_idx = 0;
        self.active_modal = Some(modal);
        self.modal_search_input.replace_text("");
        self.focus_active_surface();
    }

    pub fn open_settings_modal(&mut self) {
        self.open_modal(ActiveModal::SettingsAuth);
    }

    pub fn close_modal(&mut self) {
        self.active_modal = None;
        self.api_key_input.replace_text("");
        self.focus_active_surface();
    }

    pub fn select_model(&mut self, provider: &str, model_id: &str) {
        if !self.session.active_session_id.is_empty() {
            self.bridge.send(BridgeRequest::SetModel {
                active_session_id: self.session.active_session_id.clone(),
                provider: provider.to_string(),
                model_id: model_id.to_string(),
            });
        }
        self.close_modal();
    }

    pub fn execute_palette_action(&mut self, action_id: &str) {
        self.close_modal();
        match action_id {
            "act-new" => self.start_fresh_session(),
            "act-model" => self.open_modal(ActiveModal::ModelPicker),
            "act-folder" => self.prompt_choose_project_dir(),
            "act-harness" => self.open_harness_tab(),
            "act-compact" => self.trigger_compact(),
            "act-theme" => self.toggle_theme(),
            _ => {}
        }
    }

    pub fn begin_rename_session(&mut self) {
        if self.session.active_session_id.is_empty() {
            return;
        }
        let current = self.session.display_title();
        self.rename_input.replace_text(current);
        self.renaming_session = true;
        self.focus_input(|s| &mut s.rename_input);
    }

    pub fn commit_rename_session(&mut self, new_name: &str) {
        self.renaming_session = false;
        let trimmed = new_name.trim().to_string();
        if !trimmed.is_empty() && !self.session.active_session_id.is_empty() {
            self.session.session_name = Some(trimmed.clone());
            self.bridge.send(BridgeRequest::RenameActiveSession {
                active_session_id: self.session.active_session_id.clone(),
                name: trimmed,
            });
        }
        self.focus_input(|s| &mut s.composer_input);
    }

    pub fn toggle_side_question(&mut self) {
        self.session.side_question.open = !self.session.side_question.open;
        if self.session.side_question.open {
            self.focus_input(|s| &mut s.side_question_input);
        } else {
            self.focus_input(|s| &mut s.composer_input);
        }
    }

    pub fn submit_side_question(&mut self) {
        let q = self.side_question_input.text.trim().to_string();
        if q.is_empty() || self.session.active_session_id.is_empty() {
            return;
        }
        self.session.side_question.question.clone_from(&q);
        self.session.side_question.answer.clear();
        self.bridge.send(BridgeRequest::StartSideQuestion {
            active_session_id: self.session.active_session_id.clone(),
            question: q,
        });
        self.side_question_input.replace_text("");
    }

    pub fn trigger_compact(&mut self) {
        if !self.session.active_session_id.is_empty() {
            self.bridge.send(BridgeRequest::Compact {
                active_session_id: self.session.active_session_id.clone(),
                instructions: None,
            });
            self.toast = Some(("Compacting session context…".to_string(), false));
        }
    }

    pub fn trigger_refine(&mut self) {
        self.request_refinement(None, false);
    }

    pub fn open_new_harness_editor(&mut self) {
        self.harness_editor_open = true;
        self.harness_editing_id = None;
        self.harness_title_input.replace_text("");
        self.harness_body_input.replace_text("");
        self.focus_input(|s| &mut s.harness_title_input);
    }

    pub fn edit_existing_harness_entry(
        &mut self,
        kind: RefinementKind,
        id: &str,
        title: &str,
        content: &str,
    ) {
        self.harness_editor_open = true;
        self.harness_editor_kind = kind;
        self.harness_editing_id = Some(id.to_string());
        self.harness_title_input.replace_text(title.to_string());
        self.harness_body_input.replace_text(content.to_string());
        self.focus_input(|s| &mut s.harness_body_input);
    }

    pub fn save_harness_editor(&mut self) {
        let title = self.harness_title_input.text.trim().to_string();
        let body = self.harness_body_input.text.trim().to_string();
        if title.is_empty() || body.is_empty() {
            self.toast = Some(("Entry title and content are required".to_string(), true));

            return;
        }
        match upsert_harness_entry(
            &self.agent_dir,
            self.session.session_file.as_deref(),
            Some(self.session.session_id.as_str()),
            &HarnessEntryDraft {
                scope: self.harness_scope,
                kind: self.harness_editor_kind,
                id_override: self.harness_editing_id.as_deref(),
                title: &title,
                content: &body,
                category_path: None,
            },
        ) {
            Ok(_) => {
                self.close_harness_editor();
                self.reload_local_stores();
                self.toast = Some(("Saved continual harness entry".to_string(), false));
            }
            Err(err) => {
                self.toast = Some((format!("{err:#}"), true));
            }
        }
    }

    pub fn remove_harness_entry(&mut self, kind: RefinementKind, id: &str) {
        match delete_harness_entry(
            &self.agent_dir,
            self.session.session_file.as_deref(),
            Some(self.session.session_id.as_str()),
            self.harness_scope,
            kind,
            id,
        ) {
            Ok(_) => {
                self.reload_local_stores();
                self.toast = Some((format!("Deleted `{id}`"), false));
            }
            Err(err) => {
                self.toast = Some((format!("{err:#}"), true));
            }
        }
    }

    pub fn save_selected_provider_key(&mut self) {
        if self.auth_write_pending {
            return;
        }
        let provider = self.selected_auth_provider.clone();
        let key = self.api_key_input.text.trim().to_owned();
        let removing = key.is_empty();
        if !self.provider_auth.iter().any(|item| {
            item.provider_id == provider
                && item.can_edit_api_key
                && (!removing || item.has_stored_api_key)
        }) {
            return;
        }
        self.auth_write_pending = true;
        self.api_key_input.replace_text("");
        let dir = self.agent_dir.clone();
        self.background.run(&self.bridge, move || {
            let ok = set_provider_api_key(&dir, &provider, &key).is_ok();
            move |view: &mut Self| {
                view.auth_write_pending = false;
                view.reload_local_stores();
                if ok && !view.session.active_session_id.is_empty() {
                    view.bridge.send(BridgeRequest::RefreshInspector {
                        active_session_id: view.session.active_session_id.clone(),
                    });
                }
                let message = if !ok {
                    format!("Could not update the CLI credential for {provider}. Its file may be unreadable or the record may have changed.")
                } else if removing {
                    format!("Removed the stored key for {provider}.")
                } else {
                    format!("Saved the key for {provider}.")
                };
                view.toast = Some((message, !ok));
            }
        });
    }
}

fn read_image_file(path: &Path, remaining: usize) -> anyhow::Result<StagedImage> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => anyhow::bail!("Choose a PNG, JPEG, WebP or GIF image."),
    };
    let file = std::fs::File::open(path)?;
    let meta = file.metadata()?;
    anyhow::ensure!(meta.is_file(), "Choose a regular image file.");
    anyhow::ensure!(
        meta.len() <= remaining as u64,
        "Images exceed the 15 MiB attachment limit."
    );
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(remaining as u64 + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= remaining,
        "Images exceed the 15 MiB attachment limit."
    );
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("image")
        .to_string();
    let base64_data = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(StagedImage {
        filename,
        mime_type: mime.to_string(),
        base64_data,
        byte_len: bytes.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_stale_snapshots_while_opening_another_session() {
        assert!(!SessionSelection::Opening.accepts_snapshot("", "old"));
        let pending = SessionSelection::WaitingFor("next".into());
        assert!(!pending.accepts_snapshot("old", "old"));
        assert!(pending.accepts_snapshot("", "next"));
        assert!(!SessionSelection::Idle.accepts_snapshot("next", "old"));
    }

    #[test]
    fn image_picker_respects_remaining_bytes_before_encoding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.png");
        std::fs::write(&path, b"test").unwrap();
        assert!(read_image_file(&path, 3).is_err());
        assert_eq!(read_image_file(&path, 4).unwrap().base64_data, "dGVzdA==");
    }
}
