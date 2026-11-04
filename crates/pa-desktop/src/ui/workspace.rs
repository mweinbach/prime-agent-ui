//! Root GPUI workspace view (`PrimeDesktopView`): coordinates the three-pane
//! layout, keyboard shortcuts, native macOS sheets, and live daemon event stream.

mod cli_data;
mod projects;
use crate::state::{DraftKey, subagents::Subagents};

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use futures::StreamExt as _;
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, FontWeight, IntoElement, KeyBinding, Menu,
    MenuItem, PathPromptOptions, Render, ScrollHandle, SystemMenuType, Window, actions, div, point,
    prelude::*, px,
};
use pa_core::refinement::{HarnessScope, RefinementKind};

use crate::bridge::{BridgeEvent, BridgeHandle, BridgeRequest, socket_path_for_target};
use crate::state::harness_store::{
    HarnessEntryDraft, HarnessSnapshot, ProviderAuthItem, delete_harness_entry,
    load_provider_auth_items, set_provider_api_key, upsert_harness_entry,
};
use crate::state::models::{CatalogModel, THINKING_LEVELS, parse_model_catalog};
use crate::state::transcript::AttachedSessionState;
use crate::state::{
    ActiveModal, ActiveSessionRow, McpRow, SavedSessionRow, ScheduleRow, SessionTelemetry,
    ShellProcessRow, StagedImage,
};
use crate::theme::{DaemonTarget, DesktopPreferences, InspectorTab, Palette, ThemeMode, font_sans};
use crate::ui::inspector::render_inspector;
use crate::ui::modals::render_active_modal;
use crate::ui::sidebar::render_sidebar;
use crate::ui::stage::render_stage;
use crate::ui::text_input::{InputKind, TextChanged, TextInput, TextSubmitted};

actions!(
    pa_desktop_actions,
    [
        NewSession,
        OpenProjectFolder,
        QuickSwitcher,
        OpenModelPicker,
        CycleThinkingEffort,
        CycleDetailView,
        ToggleSidebar,
        ToggleInspector,
        OpenHarnessStudio,
        ToggleSideQuestion,
        TriggerCompact,
        TriggerRefine,
        ToggleTheme,
        OpenSettings,
        DismissOverlay,
        QuitApp,
    ]
);

/// Register global application shortcuts and native macOS menus.
pub fn register_workspace_bindings_and_menus(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewSession, None),
        KeyBinding::new("secondary-shift-o", OpenProjectFolder, None),
        KeyBinding::new("secondary-k", QuickSwitcher, None),
        KeyBinding::new("secondary-m", OpenModelPicker, None),
        KeyBinding::new("secondary-shift-e", CycleThinkingEffort, None),
        KeyBinding::new("secondary-o", CycleDetailView, None),
        KeyBinding::new("secondary-b", ToggleSidebar, None),
        KeyBinding::new("secondary-i", ToggleInspector, None),
        KeyBinding::new("secondary-shift-h", OpenHarnessStudio, None),
        KeyBinding::new("secondary-/", ToggleSideQuestion, None),
        KeyBinding::new("secondary-shift-k", TriggerCompact, None),
        KeyBinding::new("secondary-shift-r", TriggerRefine, None),
        KeyBinding::new("secondary-shift-l", ToggleTheme, None),
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("escape", DismissOverlay, None),
        KeyBinding::new("secondary-q", QuitApp, None),
    ]);

    cx.on_action(|_: &QuitApp, cx| cx.quit());

    cx.set_menus(vec![
        Menu {
            name: "Prime Agent".into(),
            items: vec![
                MenuItem::action("Settings & Providers…", OpenSettings),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Quit Prime Agent", QuitApp),
            ],
        },
        Menu {
            name: "Session".into(),
            items: vec![
                MenuItem::action("New Chat in Project", NewSession),
                MenuItem::action("Add Project…", OpenProjectFolder),
                MenuItem::action("Quick Switcher…", QuickSwitcher),
                MenuItem::separator(),
                MenuItem::action("Select Model…", OpenModelPicker),
                MenuItem::action("Cycle Thinking Effort", CycleThinkingEffort),
                MenuItem::action("Side Question (/btw)", ToggleSideQuestion),
                MenuItem::separator(),
                MenuItem::action("Compact Context", TriggerCompact),
                MenuItem::action("Refine Continual Harness", TriggerRefine),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Toggle Sidebar", ToggleSidebar),
                MenuItem::action("Toggle Harness Inspector", ToggleInspector),
                MenuItem::action("Open Continual Harness Studio", OpenHarnessStudio),
                MenuItem::action("Cycle Transcript Detail", CycleDetailView),
                MenuItem::separator(),
                MenuItem::action("Toggle Light / Dark Theme", ToggleTheme),
            ],
        },
    ]);
}

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

/// Root window view entity.
pub struct PrimeDesktopView {
    pub focus_handle: FocusHandle,
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
    pub renaming_session: bool,
    pub agent_mail_target: Option<String>,
    pub staged_images: Vec<StagedImage>,

    pub transcript_scroll: ScrollHandle,
    pub composer_input: Entity<TextInput>,
    pub sidebar_search_input: Entity<TextInput>,
    pub modal_search_input: Entity<TextInput>,
    pub rename_input: Entity<TextInput>,
    pub side_question_input: Entity<TextInput>,
    pub agent_mail_input: Entity<TextInput>,
    pub refine_input: Entity<TextInput>,
    pub harness_title_input: Entity<TextInput>,
    pub harness_body_input: Entity<TextInput>,
    pub quick_bash_input: Entity<TextInput>,
    pub schedule_interval_input: Entity<TextInput>,
    pub schedule_prompt_input: Entity<TextInput>,
    pub mcp_server_input: Entity<TextInput>,
    pub mcp_token_input: Entity<TextInput>,
    pub api_key_input: Entity<TextInput>,
}

impl PrimeDesktopView {
    /// Construct the root desktop workspace view and spawn the background daemon bridge.
    ///
    /// # Panics
    ///
    /// Panics only if the operating system refuses to create the 2-thread Tokio I/O runtime.
    pub fn new(
        options: LaunchOptions,
        preferences_dir: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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
        let (bridge, mut evt_rx) =
            BridgeHandle::spawn(data.clone()).expect("spawn pa-desktop I/O bridge");
        let sock_path =
            socket_path_for_target(prefs.daemon_target, options.daemon_socket.as_deref(), &data);
        bridge.send(BridgeRequest::Connect {
            target: prefs.daemon_target,
            explicit_socket: options.daemon_socket.clone(),
        });

        let composer_input = TextInput::new(cx, InputKind::Composer, "Message Prime Agent…");
        let sidebar_search_input =
            TextInput::new(cx, InputKind::SingleLine, "Search projects and chats…");
        let modal_search_input = TextInput::new(
            cx,
            InputKind::SingleLine,
            "Filter by model, provider, or command…",
        );
        let rename_input = TextInput::new(cx, InputKind::SingleLine, "Session title…");
        let side_question_input = TextInput::new(
            cx,
            InputKind::SingleLine,
            "Ask a side question without interrupting…",
        );
        let agent_mail_input =
            TextInput::new(cx, InputKind::SingleLine, "Message subagent or peer…");
        let refine_input = TextInput::new(cx, InputKind::SingleLine, "Optional refinement focus…");
        let harness_title_input = TextInput::new(cx, InputKind::SingleLine, "Entry title…");
        let harness_body_input = TextInput::new(
            cx,
            InputKind::MultilineEditor,
            "Entry content / policy / instructions…",
        );
        let quick_bash_input = TextInput::new(cx, InputKind::SingleLine, "cargo test --workspace");
        let schedule_interval_input = TextInput::new(cx, InputKind::SingleLine, "every 10m");
        let schedule_prompt_input =
            TextInput::new(cx, InputKind::SingleLine, "Check status & report…");
        let mcp_server_input = TextInput::new(cx, InputKind::SingleLine, "Server name");
        let mcp_token_input = TextInput::new(cx, InputKind::Secret, "Bearer token");
        let api_key_input = TextInput::new(cx, InputKind::Secret, "Paste a new API key…");

        sidebar_search_input.update(cx, |inp, _| inp.set_font_size(12.5));
        refine_input.update(cx, |inp, _| inp.set_font_size(12.0));
        quick_bash_input.update(cx, |inp, _| inp.set_font_size(12.0));
        schedule_interval_input.update(cx, |inp, _| inp.set_font_size(11.5));
        schedule_prompt_input.update(cx, |inp, _| inp.set_font_size(11.5));
        mcp_server_input.update(cx, |inp, _| inp.set_font_size(11.5));
        mcp_token_input.update(cx, |inp, _| inp.set_font_size(11.5));

        cx.subscribe_in(
            &composer_input,
            window,
            |view, _, _: &TextSubmitted, window, cx| {
                view.submit_composer(window, cx);
            },
        )
        .detach();
        cx.subscribe_in(
            &rename_input,
            window,
            |view, input, _: &TextSubmitted, window, cx| {
                let name = input.read(cx).text().to_owned();
                view.commit_rename_session(&name, window, cx);
            },
        )
        .detach();
        cx.subscribe_in(
            &side_question_input,
            window,
            |view, _, _: &TextSubmitted, window, cx| {
                view.submit_side_question(window, cx);
            },
        )
        .detach();
        for input in [
            &composer_input,
            &sidebar_search_input,
            &modal_search_input,
            &api_key_input,
        ] {
            cx.subscribe(input, |_, _, _: &TextChanged, cx| cx.notify())
                .detach();
        }

        // Event-driven foreground stream: wakes ONLY when a daemon frame arrives (0% idle CPU/GPU).
        cx.spawn(async move |this, cx| {
            while let Some(event) = evt_rx.next().await {
                let ok = this.update(cx, |view, cx| {
                    view.apply_bridge_event(event, cx);
                });
                if ok.is_err() {
                    break;
                }
            }
        })
        .detach();

        let auto_attach = prefs.daemon_target == crate::theme::DaemonTarget::DesktopHarness
            && options.daemon_socket.is_none();
        let mut instance = Self {
            focus_handle: cx.focus_handle(),
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
            renaming_session: false,
            agent_mail_target: None,
            staged_images: Vec::new(),
            transcript_scroll: ScrollHandle::new(),
            composer_input,
            sidebar_search_input,
            modal_search_input,
            rename_input,
            side_question_input,
            agent_mail_input,
            refine_input,
            harness_title_input,
            harness_body_input,
            quick_bash_input,
            schedule_interval_input,
            schedule_prompt_input,
            mcp_server_input,
            mcp_token_input,
            api_key_input,
        };
        instance.sync_input_palettes(cx);
        instance.reload_local_stores(cx);
        let focused = match instance.active_modal {
            Some(ActiveModal::ModelPicker | ActiveModal::QuickSwitcher) => {
                &instance.modal_search_input
            }
            Some(ActiveModal::SettingsAuth) => &instance.api_key_input,
            None => &instance.composer_input,
        };
        window.focus(&focused.focus_handle(cx));
        instance
    }

    fn sync_input_palettes(&self, cx: &mut Context<Self>) {
        let p = Palette::for_mode(self.prefs.theme);
        let inputs = [
            &self.composer_input,
            &self.sidebar_search_input,
            &self.modal_search_input,
            &self.rename_input,
            &self.side_question_input,
            &self.agent_mail_input,
            &self.refine_input,
            &self.harness_title_input,
            &self.harness_body_input,
            &self.quick_bash_input,
            &self.schedule_interval_input,
            &self.schedule_prompt_input,
            &self.mcp_server_input,
            &self.mcp_token_input,
            &self.api_key_input,
        ];
        for entity in inputs {
            entity.update(cx, |inp, _| {
                inp.set_colors(p.text_primary, p.text_muted, p.accent, p.accent_soft);
            });
        }
    }

    #[allow(clippy::too_many_lines)]
    fn apply_bridge_event(&mut self, event: BridgeEvent, cx: &mut Context<Self>) {
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
                    self.stash_draft(cx);
                    self.reset_session(cx);
                    self.restore_draft(cx);
                    self.selection = SessionSelection::Idle;
                    self.active_sessions.clear();
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
                    && self.composer_input.read(cx).text().is_empty()
                    && self.staged_images.is_empty())
                .then(|| {
                    rows.iter()
                        .find(|r| r.rlm_depth == 0 && r.cwd == self.working_cwd)
                })
                .flatten()
                .map(|r| r.active_session_id.clone());
                self.active_sessions = rows;
                if let Some(active_session_id) = candidate {
                    self.open_session(BridgeRequest::AttachSession { active_session_id }, cx);
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
                self.restore_draft(cx);
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
                self.restore_draft(cx);
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
                    self.reset_session(cx);
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
                    self.reload_local_stores(cx);
                    self.restore_draft(cx);
                }
                self.scroll_transcript_to_bottom();
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
                    self.reset_session(cx);
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
                    self.restore_draft(cx);
                    self.reload_local_stores(cx);
                    self.scroll_transcript_to_bottom();
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
                        let was_near_bottom = self.is_transcript_near_bottom();
                        let etype = event
                            .get("type")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        self.session.apply_live_event(&event);
                        if was_near_bottom {
                            self.scroll_transcript_to_bottom();
                        }
                        if matches!(etype.as_str(), "turn_end" | "agent_end" | "compaction_end") {
                            self.reload_local_stores(cx);
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
                    self.stash_draft(cx);
                    self.reset_session(cx);
                    self.restore_draft(cx);
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
        cx.notify();
    }

    fn is_transcript_near_bottom(&self) -> bool {
        let max_y = f32::from(self.transcript_scroll.max_offset().height);
        let cur_y = -f32::from(self.transcript_scroll.offset().y);
        max_y <= 4.0 || (max_y - cur_y) < 120.0
    }

    pub fn scroll_transcript_to_bottom(&self) {
        let max_h = self.transcript_scroll.max_offset().height;
        if max_h > px(0.0) {
            self.transcript_scroll.set_offset(point(px(0.0), -max_h));
        }
    }

    pub fn reload_local_stores(&mut self, cx: &mut Context<Self>) {
        self.store_request += 1;
        let request = self.store_request;
        let dir = self.agent_dir.clone();
        let file = self.session.session_file.clone();
        let id = self.session.session_id.clone();
        let load = cx.background_spawn(async move {
            (
                HarnessSnapshot::load(&dir, file.as_deref(), Some(&id)),
                load_provider_auth_items(&dir),
            )
        });
        cx.spawn(async move |this, cx| {
            let (harness, auth) = load.await;
            let _ = this.update(cx, |view, cx| {
                if view.store_request == request {
                    view.harness = harness;
                    view.provider_auth = auth;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn reset_session(&mut self, cx: &mut Context<Self>) {
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
            &self.side_question_input,
            &self.agent_mail_input,
            &self.quick_bash_input,
            &self.mcp_token_input,
        ] {
            input.update(cx, TextInput::clear);
        }
    }

    pub(crate) fn open_session(&mut self, request: BridgeRequest, cx: &mut Context<Self>) {
        if !self.ready_for_request(cx) {
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
            self.stash_draft(cx);
        }
        // Keep the current transcript visible until the attach succeeds. A refused attach
        // leaves the user on the parent and restores its composer draft.
        self.bridge.send(request);
        cx.notify();
    }

    fn ready_for_request(&mut self, cx: &mut Context<Self>) -> bool {
        let error = if !self.daemon_connected {
            "Not connected. Use Refresh to reconnect."
        } else if !matches!(self.selection, SessionSelection::Idle) {
            "Opening the session. Wait before sending another request."
        } else {
            return true;
        };
        self.toast = Some((error.to_owned(), true));
        cx.notify();
        false
    }

    pub fn populate_draft(&mut self, prompt: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.composer_input.update(cx, |inp, cx| {
            inp.set_text(prompt.to_string(), cx);
        });
        window.focus(&self.composer_input.focus_handle(cx));
    }

    pub fn submit_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let trimmed = self.composer_input.read(cx).text().trim().to_owned();
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
        if !local && trimmed != "/new" && !self.ready_for_request(cx) {
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
            cx.notify();
            return;
        }

        self.composer_input.update(cx, |inp, cx| {
            inp.record_history(&trimmed);
            inp.clear(cx);
        });
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
            cx.notify();
            return;
        }
        match trimmed.as_str() {
            "/new" => {
                self.start_fresh_session(window, cx);
                return;
            }
            "/model" => {
                self.open_modal(ActiveModal::ModelPicker, window, cx);
                return;
            }
            "/harness" => {
                self.open_harness_tab(cx);
                return;
            }
            "/compact" => {
                self.trigger_compact(cx);
                return;
            }
            "/refine" => {
                self.trigger_refine(cx);
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
                self.prompt_export_html(cx);
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
            cx.notify();
            return;
        }

        if self.session.active_session_id.is_empty() {
            self.open_session(
                BridgeRequest::CreateSession {
                    cwd: self.working_cwd.clone(),
                    name: None,
                    initial_prompt: Some(trimmed),
                    images,
                },
                cx,
            );
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
        cx.notify();
    }

    pub fn submit_steer(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.submit_queued(
            |active_session_id, message| BridgeRequest::SteerSession {
                active_session_id,
                message,
            },
            cx,
        );
    }

    pub fn submit_follow_up(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.submit_queued(
            |active_session_id, message| BridgeRequest::FollowUpSession {
                active_session_id,
                message,
            },
            cx,
        );
    }

    fn submit_queued(
        &mut self,
        request: impl FnOnce(String, String) -> BridgeRequest,
        cx: &mut Context<Self>,
    ) {
        if !self.ready_for_request(cx) || self.session.active_session_id.is_empty() {
            return;
        }
        if !self.staged_images.is_empty() {
            self.toast = Some((
                "Remove attachments before steering or queuing a follow-up.".to_owned(),
                true,
            ));
            cx.notify();
            return;
        }
        let msg = self.composer_input.read(cx).text().trim().to_owned();
        if msg.is_empty() {
            return;
        }
        if msg.len() > MAX_PROMPT_BYTES {
            self.toast = Some((
                "Prompt exceeds 1 MiB. Shorten it before sending.".to_owned(),
                true,
            ));
            cx.notify();
            return;
        }
        self.composer_input.update(cx, |inp, cx| {
            inp.record_history(&msg);
            inp.clear(cx);
        });
        self.bridge
            .send(request(self.session.active_session_id.clone(), msg));
        self.restore_draft(cx);
        cx.notify();
    }

    pub fn toggle_bash_prefix(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cur = self.composer_input.read(cx).text().to_string();
        let next = if let Some(rest) = cur.strip_prefix('!') {
            rest.trim_start().to_string()
        } else {
            format!("! {cur}")
        };
        self.composer_input
            .update(cx, |inp, cx| inp.set_text(next, cx));
        window.focus(&self.composer_input.focus_handle(cx));
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.prefs.sidebar_open = !self.prefs.sidebar_open;
        self.save_preferences();
        cx.notify();
    }

    pub fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        self.prefs.inspector_open = !self.prefs.inspector_open;
        self.save_preferences();
        cx.notify();
    }

    pub fn set_inspector_tab(&mut self, tab: InspectorTab, cx: &mut Context<Self>) {
        self.prefs.inspector_open = true;
        self.prefs.inspector_tab = tab;
        self.save_preferences();
        if tab == InspectorTab::Harness || tab == InspectorTab::System {
            self.reload_local_stores(cx);
        }
        cx.notify();
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

    pub fn open_harness_tab(&mut self, cx: &mut Context<Self>) {
        self.set_inspector_tab(InspectorTab::Harness, cx);
    }

    pub fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        self.prefs.theme = self.prefs.theme.toggled();
        self.save_preferences();
        self.sync_input_palettes(cx);
        cx.notify();
    }

    pub fn cycle_detail_level(&mut self, cx: &mut Context<Self>) {
        self.prefs.detail_level = self.prefs.detail_level.next();
        self.save_preferences();
        cx.notify();
    }

    pub fn cycle_thinking_level(&mut self, cx: &mut Context<Self>) {
        let cur_idx = THINKING_LEVELS
            .iter()
            .position(|&l| l == self.session.thinking_level)
            .unwrap_or(4);
        let next = THINKING_LEVELS[(cur_idx + 1) % THINKING_LEVELS.len()];
        if !self.session.active_session_id.is_empty() {
            self.bridge.send(BridgeRequest::SetThinkingLevel {
                active_session_id: self.session.active_session_id.clone(),
                level: next.to_string(),
            });
        }
        cx.notify();
    }

    fn save_preferences(&mut self) {
        if let Err(err) = self.prefs.save(&self.preferences_dir) {
            self.toast = Some((format!("Cannot save desktop preferences: {err}"), true));
        }
    }

    pub fn switch_daemon_target(&mut self, cx: &mut Context<Self>) {
        if self.prefs.cli_data_dir.is_some() {
            self.toast = Some((
                "Use the default data folder before attaching to the CLI daemon.".into(),
                true,
            ));
            cx.notify();
            return;
        }
        self.prefs.daemon_target = self.prefs.daemon_target.toggled();
        self.save_preferences();
        self.daemon_connected = false;
        self.explicit_socket = None;
        self.selection = SessionSelection::Idle;
        self.auto_attach = false;
        self.stash_draft(cx);
        self.reset_session(cx);
        self.restore_draft(cx);
        self.active_sessions.clear();
        self.saved_sessions.clear();
        self.saved_next_cursor = None;
        self.bridge.send(BridgeRequest::Connect {
            target: self.prefs.daemon_target,
            explicit_socket: None,
        });
        cx.notify();
    }

    pub fn refresh_all(&mut self, cx: &mut Context<Self>) {
        self.reload_local_stores(cx);
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
        cx.notify();
    }

    pub fn open_modal(&mut self, modal: ActiveModal, window: &mut Window, cx: &mut Context<Self>) {
        if modal == ActiveModal::ModelPicker
            && self.session.active_session_id.is_empty()
            && self.daemon_connected
            && matches!(self.selection, SessionSelection::Idle)
        {
            self.open_session(
                BridgeRequest::CreateSession {
                    cwd: self.working_cwd.clone(),
                    name: None,
                    initial_prompt: None,
                    images: Vec::new(),
                },
                cx,
            );
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
            self.reload_local_stores(cx);
        }
        self.active_modal = Some(modal);
        self.modal_search_input.update(cx, TextInput::clear);
        match modal {
            ActiveModal::ModelPicker | ActiveModal::QuickSwitcher => {
                window.focus(&self.modal_search_input.focus_handle(cx));
            }
            ActiveModal::SettingsAuth => {
                window.focus(&self.api_key_input.focus_handle(cx));
            }
        }
        cx.notify();
    }

    pub fn open_settings_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_modal(ActiveModal::SettingsAuth, window, cx);
    }

    pub fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active_modal = None;
        self.api_key_input.update(cx, TextInput::clear);
        self.renaming_session = false;
        window.focus(&self.composer_input.focus_handle(cx));
        cx.notify();
    }

    pub fn select_model(
        &mut self,
        provider: &str,
        model_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.session.active_session_id.is_empty() {
            self.bridge.send(BridgeRequest::SetModel {
                active_session_id: self.session.active_session_id.clone(),
                provider: provider.to_string(),
                model_id: model_id.to_string(),
            });
        }
        self.close_modal(window, cx);
    }

    pub fn execute_palette_action(
        &mut self,
        action_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_modal = None;
        match action_id {
            "act-new" => self.start_fresh_session(window, cx),
            "act-model" => self.open_modal(ActiveModal::ModelPicker, window, cx),
            "act-folder" => self.prompt_choose_project_dir(window, cx),
            "act-harness" => self.open_harness_tab(cx),
            "act-compact" => self.trigger_compact(cx),
            "act-theme" => self.toggle_theme(cx),
            _ => {}
        }
        cx.notify();
    }

    pub fn begin_rename_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.active_session_id.is_empty() {
            return;
        }
        let current = self.session.display_title();
        self.rename_input
            .update(cx, |inp, cx| inp.set_text(current, cx));
        self.renaming_session = true;
        window.focus(&self.rename_input.focus_handle(cx));
        cx.notify();
    }

    pub fn commit_rename_session(
        &mut self,
        new_name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.renaming_session = false;
        let trimmed = new_name.trim().to_string();
        if !trimmed.is_empty() && !self.session.active_session_id.is_empty() {
            self.session.session_name = Some(trimmed.clone());
            self.bridge.send(BridgeRequest::RenameActiveSession {
                active_session_id: self.session.active_session_id.clone(),
                name: trimmed,
            });
        }
        window.focus(&self.composer_input.focus_handle(cx));
        cx.notify();
    }

    pub fn toggle_side_question(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.session.side_question.open = !self.session.side_question.open;
        if self.session.side_question.open {
            window.focus(&self.side_question_input.focus_handle(cx));
        } else {
            window.focus(&self.composer_input.focus_handle(cx));
        }
        cx.notify();
    }

    pub fn submit_side_question(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let q = self.side_question_input.read(cx).text().trim().to_string();
        if q.is_empty() || self.session.active_session_id.is_empty() {
            return;
        }
        self.session.side_question.question.clone_from(&q);
        self.session.side_question.answer.clear();
        self.bridge.send(BridgeRequest::StartSideQuestion {
            active_session_id: self.session.active_session_id.clone(),
            question: q,
        });
        self.side_question_input.update(cx, TextInput::clear);
        cx.notify();
    }

    pub fn trigger_compact(&mut self, cx: &mut Context<Self>) {
        if !self.session.active_session_id.is_empty() {
            self.bridge.send(BridgeRequest::Compact {
                active_session_id: self.session.active_session_id.clone(),
                instructions: None,
            });
            self.toast = Some(("Compacting session context…".to_string(), false));
            cx.notify();
        }
    }

    pub fn trigger_refine(&mut self, cx: &mut Context<Self>) {
        if !self.session.active_session_id.is_empty() {
            self.bridge.send(BridgeRequest::Refine {
                active_session_id: self.session.active_session_id.clone(),
                instructions: None,
                rollback_id: None,
                global: false,
            });
            self.toast = Some(("Running Continual Harness refinement…".to_string(), false));
            cx.notify();
        }
    }

    pub fn open_new_harness_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.harness_editor_open = true;
        self.harness_editing_id = None;
        self.harness_title_input.update(cx, TextInput::clear);
        self.harness_body_input.update(cx, TextInput::clear);
        window.focus(&self.harness_title_input.focus_handle(cx));
        cx.notify();
    }

    pub fn edit_existing_harness_entry(
        &mut self,
        kind: RefinementKind,
        id: &str,
        title: &str,
        content: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.harness_editor_open = true;
        self.harness_editor_kind = kind;
        self.harness_editing_id = Some(id.to_string());
        self.harness_title_input
            .update(cx, |i, cx| i.set_text(title.to_string(), cx));
        self.harness_body_input
            .update(cx, |i, cx| i.set_text(content.to_string(), cx));
        window.focus(&self.harness_body_input.focus_handle(cx));
        cx.notify();
    }

    pub fn save_harness_editor(&mut self, cx: &mut Context<Self>) {
        let title = self.harness_title_input.read(cx).text().trim().to_string();
        let body = self.harness_body_input.read(cx).text().trim().to_string();
        if title.is_empty() || body.is_empty() {
            self.toast = Some(("Entry title and content are required".to_string(), true));
            cx.notify();
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
                self.harness_editor_open = false;
                self.harness_editing_id = None;
                self.reload_local_stores(cx);
                self.toast = Some(("Saved continual harness entry".to_string(), false));
            }
            Err(err) => {
                self.toast = Some((format!("{err:#}"), true));
            }
        }
        cx.notify();
    }

    pub fn remove_harness_entry(&mut self, kind: RefinementKind, id: &str, cx: &mut Context<Self>) {
        match delete_harness_entry(
            &self.agent_dir,
            self.session.session_file.as_deref(),
            Some(self.session.session_id.as_str()),
            self.harness_scope,
            kind,
            id,
        ) {
            Ok(_) => {
                self.reload_local_stores(cx);
                self.toast = Some((format!("Deleted `{id}`"), false));
            }
            Err(err) => {
                self.toast = Some((format!("{err:#}"), true));
            }
        }
        cx.notify();
    }

    pub fn save_selected_provider_key(&mut self, cx: &mut Context<Self>) {
        if self.auth_write_pending {
            return;
        }
        let provider = self.selected_auth_provider.clone();
        let key = self.api_key_input.read(cx).text().trim().to_owned();
        let removing = key.is_empty();
        if !self.provider_auth.iter().any(|item| {
            item.provider_id == provider
                && item.can_edit_api_key
                && (!removing || item.has_stored_api_key)
        }) {
            return;
        }
        self.auth_write_pending = true;
        self.api_key_input.update(cx, TextInput::clear);
        let dir = self.agent_dir.clone();
        let store = cx.background_spawn(async move {
            (
                set_provider_api_key(&dir, &provider, &key).is_ok(),
                provider,
            )
        });
        cx.spawn(async move |this, cx| {
            let (ok, provider) = store.await;
            let _ = this.update(cx, |view, cx| {
                view.auth_write_pending = false;
                view.reload_local_stores(cx);
                if ok && !view.session.active_session_id.is_empty() {
                    view.bridge.send(BridgeRequest::RefreshInspector { active_session_id: view.session.active_session_id.clone() });
                }
                let message = if !ok { format!("Could not update the CLI credential for {provider}. Its file may be unreadable or the record may have changed.") }
                    else if removing { format!("Removed the stored key for {provider}.") } else { format!("Saved the key for {provider}.") };
                view.toast = Some((message, !ok));
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub fn prompt_attach_images(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach Images".into()),
        });
        let target = self.draft_key();
        let count = self.staged_images.len();
        let bytes: usize = self.staged_images.iter().map(|img| img.byte_len).sum();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let loaded = cx
                .background_spawn(async move {
                    anyhow::ensure!(
                        count + paths.len() <= MAX_ATTACHMENTS,
                        "Attach up to {MAX_ATTACHMENTS} images per prompt."
                    );
                    let mut remaining = MAX_ATTACHMENT_BYTES.saturating_sub(bytes);
                    let mut images = Vec::with_capacity(paths.len());
                    for path in paths {
                        let image = read_image_file(&path, remaining)?;
                        remaining -= image.byte_len;
                        images.push(image);
                    }
                    anyhow::Ok(images)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.draft_key() != target {
                    return;
                }
                match loaded {
                    Ok(images)
                        if view.staged_images.len() + images.len() <= MAX_ATTACHMENTS
                            && view
                                .staged_images
                                .iter()
                                .chain(&images)
                                .map(|img| img.byte_len)
                                .sum::<usize>()
                                <= MAX_ATTACHMENT_BYTES =>
                    {
                        view.staged_images.extend(images);
                    }
                    Ok(_) => {
                        view.toast = Some((
                            "Attachments exceed the limit for one prompt.".to_owned(),
                            true,
                        ));
                    }
                    Err(error) => view.toast = Some((error.to_string(), true)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn prompt_export_html(&mut self, cx: &mut Context<Self>) {
        if self.session.active_session_id.is_empty() {
            return;
        }
        let sid = self.session.active_session_id.clone();
        let base_dir = PathBuf::from(&self.working_cwd);
        let rx = cx.prompt_for_new_path(&base_dir, Some("prime-agent-session.html"));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(out_path))) = rx.await {
                let _ = this.update(cx, |view, _cx| {
                    view.bridge.send(BridgeRequest::ExportSessionHtml {
                        active_session_id: sid,
                        output_path: out_path,
                    });
                });
            }
        })
        .detach();
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

impl Focusable for PrimeDesktopView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PrimeDesktopView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_mode(self.prefs.theme);
        let width = window.viewport_size().width;
        let sidebar_docked = self.prefs.sidebar_open && width >= px(760.0);
        let inspector_docked = self.prefs.inspector_open && width >= px(1000.0);
        let sidebar_overlay = self.prefs.sidebar_open && !sidebar_docked;
        let inspector_overlay = self.prefs.inspector_open && !inspector_docked;

        div()
            .id("pa-desktop-root")
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .bg(palette.bg_canvas)
            .text_color(palette.text_primary)
            .font_family(font_sans())
            // Global Action Handlers
            .on_action(cx.listener(|this, _: &NewSession, window, cx| {
                this.start_fresh_session(window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenProjectFolder, window, cx| {
                this.prompt_choose_project_dir(window, cx);
            }))
            .on_action(cx.listener(|this, _: &QuickSwitcher, window, cx| {
                this.open_modal(ActiveModal::QuickSwitcher, window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenModelPicker, window, cx| {
                this.open_modal(ActiveModal::ModelPicker, window, cx);
            }))
            .on_action(cx.listener(|this, _: &CycleThinkingEffort, _, cx| {
                this.cycle_thinking_level(cx);
            }))
            .on_action(cx.listener(|this, _: &CycleDetailView, _, cx| {
                this.cycle_detail_level(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.toggle_sidebar(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleInspector, _, cx| {
                this.toggle_inspector(cx);
            }))
            .on_action(cx.listener(|this, _: &OpenHarnessStudio, _, cx| {
                this.open_harness_tab(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSideQuestion, window, cx| {
                this.toggle_side_question(window, cx);
            }))
            .on_action(cx.listener(|this, _: &TriggerCompact, _, cx| {
                this.trigger_compact(cx);
            }))
            .on_action(cx.listener(|this, _: &TriggerRefine, _, cx| {
                this.trigger_refine(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleTheme, _, cx| {
                this.toggle_theme(cx);
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                this.open_settings_modal(window, cx);
            }))
            .on_action(cx.listener(move |this, _: &DismissOverlay, window, cx| {
                if this.active_modal.is_some() || this.renaming_session {
                    this.close_modal(window, cx);
                } else if inspector_overlay {
                    this.toggle_inspector(cx);
                } else if sidebar_overlay {
                    this.toggle_sidebar(cx);
                } else if this.session.is_streaming && !this.session.active_session_id.is_empty() {
                    this.bridge.send(BridgeRequest::AbortSession {
                        active_session_id: this.session.active_session_id.clone(),
                    });
                }
            }))
            // Three-Zone Spatial Layout
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    .when(sidebar_docked, |d| {
                        d.child(render_sidebar(self, palette, cx))
                    })
                    .child(render_stage(self, palette, sidebar_docked, cx))
                    .when(inspector_docked, |d| {
                        d.child(render_inspector(self, palette, cx))
                    }),
            )
            .when(sidebar_overlay || inspector_overlay, |d| {
                let panel = if inspector_overlay {
                    render_inspector(self, palette, cx)
                } else {
                    render_sidebar(self, palette, cx)
                };
                d.child(
                    div()
                        .id("drawer-scrim")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .bg(palette.bg_scrim)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            if inspector_overlay {
                                view.toggle_inspector(cx);
                            } else {
                                view.toggle_sidebar(cx);
                            }
                        })),
                )
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .when(inspector_overlay, gpui::Styled::right_0)
                        .when(!inspector_overlay, gpui::Styled::left_0)
                        .occlude()
                        .shadow_lg()
                        .child(panel),
                )
            })
            // Active Modal Overlay
            .when_some(self.active_modal, |d, modal| {
                d.child(render_active_modal(modal, self, palette, cx))
            })
            // Toast Notification Pill (bottom-right)
            .when_some(self.toast.clone(), |d, (msg, is_err)| {
                d.child(
                    div()
                        .id("toast-banner")
                        .absolute()
                        .bottom(px(22.0))
                        .right(px(22.0))
                        .max_w(px(420.0))
                        .px(px(14.0))
                        .py(px(9.0))
                        .rounded_xl()
                        .border_1()
                        .border_color(if is_err {
                            palette.danger
                        } else {
                            palette.border_strong
                        })
                        .bg(palette.bg_elevated)
                        .shadow_lg()
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toast = None;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .text_size(px(12.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(if is_err {
                                    palette.danger
                                } else {
                                    palette.text_primary
                                })
                                .child(msg),
                        ),
                )
            })
    }
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
