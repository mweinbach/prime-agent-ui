//! Non-blocking async daemon bridge (`pa-desktop` <-> `pa-daemon`).
//!
//! Runs all socket and process work on a dedicated 2-thread Tokio runtime so
//! the GPUI Metal render thread never blocks on network, IPC, or process spawns.

mod catalog;
mod data;
pub use data::CliData;
mod cli_socket;
mod inspector;
use inspector::load_inspector_snapshot;
mod telemetry;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use futures::channel::mpsc as futures_mpsc;
use pa_types::daemon::{
    DAEMON_PROTOCOL_NAME, DAEMON_PROTOCOL_VERSION, DAEMON_SCHEMA_ID, DaemonCommand,
    DaemonCommandEnvelope, DaemonCommandFrameType, DaemonProtocolInfo, DaemonResponse,
    DaemonSessionLifecycle, PromptInput, StreamingBehavior,
};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::state::{
    ActiveSessionRow, McpRow, RlmChildRow, SavedSessionRow, ScheduleRow, ShellProcessRow,
    StagedImage,
};
use crate::theme::DaemonTarget;

/// Default dedicated socket filename for the Desktop Harness supervisor inside `socket_dir()`.
pub const DESKTOP_DAEMON_SOCKET_NAME: &str = "desktop-daemon.sock";

/// Commands dispatched from the GPUI foreground view to the background I/O worker.
#[derive(Debug, Clone)]
pub enum BridgeRequest {
    Connect {
        target: DaemonTarget,
        explicit_socket: Option<PathBuf>,
    },
    RefreshWorkspace {
        cwd: Option<String>,
    },
    LoadOlderSessions {
        cursor: String,
    },
    CreateSession {
        cwd: String,
        name: Option<String>,
        initial_prompt: Option<String>,
        images: Vec<StagedImage>,
    },
    OpenSavedSession {
        session_path: String,
        cwd: Option<String>,
    },
    AttachSession {
        active_session_id: String,
    },
    KillSession {
        active_session_id: String,
    },
    RenameActiveSession {
        active_session_id: String,
        name: String,
    },
    RenameSavedSession {
        session_path: String,
        name: String,
    },
    DeleteSavedSession {
        session_path: String,
    },
    SendPrompt {
        active_session_id: String,
        message: String,
        images: Vec<StagedImage>,
    },
    SteerSession {
        active_session_id: String,
        message: String,
    },
    FollowUpSession {
        active_session_id: String,
        message: String,
    },
    AbortSession {
        active_session_id: String,
    },
    AbortAndSendQueued {
        active_session_id: String,
    },
    ClearQueue {
        active_session_id: String,
    },
    SetModel {
        active_session_id: String,
        provider: String,
        model_id: String,
    },
    SetThinkingLevel {
        active_session_id: String,
        level: String,
    },
    SetAutoCompaction {
        active_session_id: String,
        enabled: bool,
    },
    SetAutoRetry {
        active_session_id: String,
        enabled: bool,
    },
    Compact {
        active_session_id: String,
        instructions: Option<String>,
    },
    Refine {
        active_session_id: String,
        instructions: Option<String>,
        rollback_id: Option<String>,
        global: bool,
    },
    ReloadSession {
        active_session_id: String,
    },
    ExecuteBash {
        active_session_id: String,
        command: String,
    },
    AbortBash {
        active_session_id: String,
    },
    RefreshInspector {
        active_session_id: String,
    },
    TailShell {
        active_session_id: String,
        id: String,
        persistent: bool,
    },
    KillShell {
        active_session_id: String,
        id: String,
        persistent: bool,
    },
    SetRlmMaxDepth {
        active_session_id: String,
        max_depth: u64,
    },
    CancelRlmChild {
        active_session_id: String,
        child_id: String,
    },
    DeleteRlmSubagent {
        active_session_id: String,
        child_id: String,
    },
    SendAgentMessage {
        from_active_session_id: Option<String>,
        target_active_session_id: String,
        message: String,
    },
    StartSideQuestion {
        active_session_id: String,
        question: String,
    },
    AbortSideQuestion {
        active_session_id: String,
        side_question_id: String,
    },
    CreateHeartbeat {
        active_session_id: String,
        schedule: String,
        prompt: String,
    },
    ManageHeartbeat {
        active_session_id: String,
        job_id: String,
        action: String,
    },
    CreateCron {
        active_session_id: String,
        schedule: String,
        prompt: String,
    },
    CancelCron {
        active_session_id: Option<String>,
        job_id: String,
    },
    SetMcpToken {
        active_session_id: String,
        server: String,
        token: String,
    },
    RemoveMcp {
        active_session_id: String,
        server: String,
    },
    ExportSessionHtml {
        active_session_id: String,
        output_path: PathBuf,
    },
}

/// Asynchronous events pushed from the background worker to the GPUI foreground task.
#[derive(Debug, Clone)]
pub enum BridgeEvent {
    ConnectionStatus {
        connected: bool,
        target: DaemonTarget,
        socket_path: PathBuf,
        daemon_version: Option<String>,
        error: Option<String>,
    },
    ActiveSessions(Vec<ActiveSessionRow>),
    RosterDelta {
        changed: Vec<ActiveSessionRow>,
        removed: Vec<String>,
        resync: bool,
    },
    SavedSessionsPage {
        cursor: Option<String>,
        next_cursor: Option<String>,
        rows: Vec<SavedSessionRow>,
    },
    SessionTransitionFailed {
        message: String,
    },
    PromptRejected {
        draft: crate::state::DraftKey,
        message: String,
        images: Vec<StagedImage>,
        reason: String,
    },
    SessionAttached {
        active_session_id: String,
        snapshot: Value,
    },
    SnapshotChunkBegin {
        active_session_id: String,
        header: Value,
    },
    SnapshotChunk {
        active_session_id: String,
        messages: Vec<Value>,
    },
    SnapshotChunkEnd {
        active_session_id: String,
    },
    SessionEvent {
        active_session_id: String,
        event: Value,
    },
    SideQuestionStarted {
        active_session_id: String,
        side_question_id: String,
    },
    SideQuestionEvent {
        active_session_id: String,
        event: Value,
    },
    SessionClosed {
        active_session_id: String,
        reason: String,
    },
    SessionOpening {
        active_session_id: String,
    },
    AttachRedirect {
        requested: String,
        actual: String,
    },
    ModelCatalogLoaded {
        active_session_id: String,
        data: Value,
    },
    SubagentUpdated {
        active_session_id: String,
        sequence: u64,
        row: RlmChildRow,
    },
    SubagentsFetched {
        active_session_id: String,
        result: std::result::Result<(u64, Vec<RlmChildRow>), String>,
    },
    InspectorSnapshot {
        active_session_id: String,
        rlm_max_depth: Option<u64>,
        shells: Vec<ShellProcessRow>,
        schedules: Vec<ScheduleRow>,
        mcp: Vec<McpRow>,
        stats: Option<Value>,
        system_prompt: Option<String>,
    },
    ShellTailLoaded {
        active_session_id: String,
        id: String,
        output: String,
    },
    BackgroundRefreshNeeded,
    Toast {
        message: String,
        is_error: bool,
    },
}

/// Non-blocking handle held by the GPUI window entity.
pub struct BridgeHandle {
    tx: mpsc::UnboundedSender<BridgeRequest>,
    runtime: Arc<tokio::runtime::Runtime>,
}

impl BridgeHandle {
    /// Spawn the 2-thread background Tokio worker and return `(handle, ui_event_rx)`.
    ///
    /// # Errors
    ///
    /// Returns an error only if the OS refuses to create the Tokio runtime threads.
    pub fn spawn(data: CliData) -> Result<(Self, futures_mpsc::UnboundedReceiver<BridgeEvent>)> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("pa-desktop-io")
            .enable_all()
            .build()
            .context("build pa-desktop tokio runtime")?;

        let (req_tx, req_rx) = mpsc::unbounded_channel::<BridgeRequest>();
        let (evt_tx, evt_rx) = futures_mpsc::unbounded::<BridgeEvent>();

        runtime.spawn(run_worker_loop(req_rx, evt_tx, data));

        Ok((
            Self {
                tx: req_tx,
                runtime: Arc::new(runtime),
            },
            evt_rx,
        ))
    }

    /// Enqueue a request without awaiting any network or socket operation.
    pub fn send(&self, request: BridgeRequest) {
        let _ = self.tx.send(request);
    }
}

/// Resolve the socket path for the selected [`DaemonTarget`].
#[must_use]
pub(crate) fn socket_path_for_target(
    target: DaemonTarget,
    explicit: Option<&Path>,
    data: &CliData,
) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_path_buf();
    }
    match target {
        DaemonTarget::DesktopHarness => data.desktop_socket(),
        DaemonTarget::SharedCli => cli_socket::preferred_cli_socket(),
    }
}

/// Locate the companion `prime-agent` executable built from this repository or app bundle.
#[must_use]
pub fn locate_prime_agent_binary() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join("prime-agent");
        if sibling.is_file() {
            return Some(sibling);
        }
    }
    // Integration-test executables live one directory below their companion binary.
    let exe = std::env::current_exe().ok()?;
    let deps = exe.parent()?;
    if deps.file_name()? != "deps" {
        return None;
    }
    let candidate = deps.parent()?.join("prime-agent");
    candidate.is_file().then_some(candidate)
}

#[derive(PartialEq, Eq)]
enum LaunchPolicy {
    AttachOnly,
    StartCompanion,
}

async fn ensure_supervisor_listening(
    socket_path: &Path,
    policy: LaunchPolicy,
    data: &CliData,
) -> Result<RawClient> {
    match RawClient::connect(socket_path).await {
        Ok(client) => return Ok(client),
        Err(error) => {
            let missing_listener = error.downcast_ref::<std::io::Error>().is_some_and(|io| {
                matches!(
                    io.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                )
            });
            if policy == LaunchPolicy::AttachOnly || !missing_listener {
                return Err(error)
                    .with_context(|| format!("Cannot connect to {}", socket_path.display()));
            }
        }
    }

    let binary = locate_prime_agent_binary().ok_or_else(|| {
        anyhow!(
            "Could not find companion `prime-agent` binary to start supervisor at {}. Build with `cargo build -p pa-cli`.",
            socket_path.display()
        )
    })?;

    if let Some(parent) = socket_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut cmd = Command::new(&binary);
    data.configure(&mut cmd);
    cmd.arg("--mode")
        .arg("daemon")
        .arg("--daemon-socket")
        .arg(socket_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // Inside `Prime Agent.app/Contents/MacOS/prime-agent`, bundled `skills/` and
    // `prime-agent-runtime/` live in `../Resources/` so bundle signing stays strict-clean.
    if let Some(macos_dir) = binary.parent()
        && let Some(contents_dir) = macos_dir.parent()
    {
        let resources_dir = contents_dir.join("Resources");
        if resources_dir.join("prime-agent-runtime").is_dir()
            && std::env::var_os("PI_PACKAGE_DIR").is_none()
        {
            cmd.env("PI_PACKAGE_DIR", &resources_dir);
        }
    }

    cmd.spawn()
        .with_context(|| format!("spawn `{}` --mode daemon", binary.display()))?;

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(client) = RawClient::connect(socket_path).await {
            return Ok(client);
        }
        if Instant::now() >= deadline {
            return Err(anyhow!(
                "Timed out waiting for Prime Agent supervisor at {}",
                socket_path.display()
            ));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

struct PendingMap {
    map: Mutex<HashMap<String, oneshot::Sender<Result<DaemonResponse>>>>,
}

impl PendingMap {
    fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
        }
    }

    fn insert(&self, id: String, tx: oneshot::Sender<Result<DaemonResponse>>) {
        self.map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, tx);
    }

    fn complete(&self, id: &str, resp: DaemonResponse) {
        if let Some(tx) = self
            .map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id)
        {
            let _ = tx.send(Ok(resp));
        }
    }

    fn fail_all(&self, reason: &str) {
        let mut guard = self
            .map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (_, tx) in guard.drain() {
            let _ = tx.send(Err(anyhow!(reason.to_string())));
        }
    }
}

#[derive(Clone)]
struct RawClient {
    client_id: String,
    hello: Value,
    seq: Arc<AtomicU64>,
    pending: Arc<PendingMap>,
    writer: mpsc::UnboundedSender<String>,
    inbound_frames: Arc<Mutex<Option<mpsc::UnboundedReceiver<Value>>>>,
}

impl RawClient {
    async fn connect(socket_path: &Path) -> Result<Self> {
        let stream = tokio::time::timeout(
            Duration::from_secs(3),
            pa_types::platform::transport::connect_transport(socket_path),
        )
        .await
        .map_err(|_| anyhow!("connect timeout: {}", socket_path.display()))??;

        let (read_half, mut write_half) = stream.split();
        let (line_tx, mut line_rx) = mpsc::unbounded_channel::<String>();
        let (frame_tx, frame_rx) = mpsc::unbounded_channel::<Value>();
        let (hello_tx, hello_rx) = oneshot::channel::<Value>();
        let pending = Arc::new(PendingMap::new());
        let reader_pending = Arc::clone(&pending);

        tokio::spawn(async move {
            while let Some(mut line) = line_rx.recv().await {
                line.push('\n');
                if write_half.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
            let _ = write_half.shutdown().await;
        });

        let reader_handle = tokio::spawn(async move {
            let mut reader = BufReader::new(read_half);
            let mut buf = String::new();
            let mut hello_slot = Some(hello_tx);
            loop {
                buf.clear();
                match reader.read_line(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let Ok(val) = serde_json::from_str::<Value>(buf.trim()) else {
                    continue;
                };
                match val.get("type").and_then(Value::as_str) {
                    Some("daemon_hello") => {
                        if let Some(tx) = hello_slot.take() {
                            let _ = tx.send(val);
                        }
                    }
                    Some("response") => {
                        if let Ok(resp) = serde_json::from_value::<DaemonResponse>(val) {
                            let id = resp.id.clone().unwrap_or_default();
                            reader_pending.complete(&id, resp);
                        }
                    }
                    _ => {
                        let _ = frame_tx.send(val);
                    }
                }
            }
            reader_pending.fail_all("daemon connection closed");
        });

        let hello = tokio::time::timeout(Duration::from_secs(10), hello_rx)
            .await
            .map_err(|_| {
                reader_handle.abort();
                anyhow!("daemon hello timeout")
            })?
            .map_err(|_| anyhow!("daemon closed before hello"))?;

        anyhow::ensure!(
            hello.get("schemaId").and_then(Value::as_str) == Some(DAEMON_SCHEMA_ID)
                && hello.pointer("/protocol/version").and_then(Value::as_u64)
                    == Some(DAEMON_PROTOCOL_VERSION),
            "Incompatible Prime Agent daemon at {}. It was left running.",
            socket_path.display()
        );
        let client_id = hello
            .get("clientId")
            .and_then(Value::as_str)
            .unwrap_or("pa-desktop")
            .to_string();

        Ok(Self {
            client_id,
            hello,
            seq: Arc::new(AtomicU64::new(1)),
            pending,
            writer: line_tx,
            inbound_frames: Arc::new(Mutex::new(Some(frame_rx))),
        })
    }

    fn take_frame_receiver(&self) -> Option<mpsc::UnboundedReceiver<Value>> {
        self.inbound_frames
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    async fn request(&self, command: DaemonCommand, timeout: Duration) -> Result<DaemonResponse> {
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        let id = format!("desktop_{n}");
        let envelope = DaemonCommandEnvelope {
            frame_type: DaemonCommandFrameType::Command,
            id: id.clone(),
            protocol: DaemonProtocolInfo {
                name: DAEMON_PROTOCOL_NAME.into(),
                version: DAEMON_PROTOCOL_VERSION,
            },
            client_id: Some(self.client_id.clone()),
            command,
        };
        let encoded = serde_json::to_string(&envelope)?;
        let (tx, rx) = oneshot::channel();
        self.pending.insert(id.clone(), tx);
        if self.writer.send(encoded).is_err() {
            self.pending.fail_all("daemon writer closed");
            return Err(anyhow!("daemon socket disconnected"));
        }
        tokio::time::timeout(timeout, rx)
            .await
            .map_err(|_| anyhow!("request `{id}` timed out"))?
            .map_err(|_| anyhow!("request `{id}` cancelled"))?
    }
}

async fn run_worker_loop(
    mut req_rx: mpsc::UnboundedReceiver<BridgeRequest>,
    evt_tx: futures_mpsc::UnboundedSender<BridgeEvent>,
    data: CliData,
) {
    let mut active_client: Option<RawClient> = None;
    let mut tasks = tokio::task::JoinSet::new();

    loop {
        let req = tokio::select! {
            Some(_) = tasks.join_next(), if !tasks.is_empty() => continue,
            req = req_rx.recv() => match req { Some(req) => req, None => break },
        };
        match req {
            BridgeRequest::Connect {
                target: current_target,
                explicit_socket,
            } => {
                tasks.shutdown().await;
                let sock =
                    socket_path_for_target(current_target, explicit_socket.as_deref(), &data);
                let connection = match current_target {
                    DaemonTarget::SharedCli => {
                        cli_socket::connect_cli_socket(explicit_socket.as_deref()).await
                    }
                    DaemonTarget::DesktopHarness => {
                        let policy = if explicit_socket.is_none() {
                            LaunchPolicy::StartCompanion
                        } else {
                            LaunchPolicy::AttachOnly
                        };
                        ensure_supervisor_listening(&sock, policy, &data)
                            .await
                            .map(|client| (sock.clone(), client))
                    }
                };
                match connection {
                    Ok((sock, client)) => {
                        let version = client
                            .hello
                            .get("appVersion")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        let _ = evt_tx.unbounded_send(BridgeEvent::ConnectionStatus {
                            connected: true,
                            target: current_target,
                            socket_path: sock.clone(),
                            daemon_version: version,
                            error: None,
                        });
                        if let Some(frame_rx) = client.take_frame_receiver() {
                            let tx = evt_tx.clone();
                            let roster_client = client.clone();
                            tasks.spawn(async move {
                                // Establish the roster baseline, then apply queued deltas in wire order.
                                refresh_roster(&roster_client, &tx).await;
                                forward_daemon_frames(frame_rx, tx.clone()).await;
                                let _ = tx.unbounded_send(BridgeEvent::ConnectionStatus {
                                    connected: false,
                                    target: current_target,
                                    socket_path: sock,
                                    daemon_version: None,
                                    error: Some(
                                        "Lost connection to the daemon. Refresh to reconnect."
                                            .into(),
                                    ),
                                });
                            });
                        }
                        let c = client.clone();
                        let tx = evt_tx.clone();
                        tasks.spawn(async move {
                            refresh_saved(&c, None, &tx).await;
                        });
                        active_client = Some(client);
                    }
                    Err(err) => {
                        active_client = None;
                        let _ = evt_tx.unbounded_send(BridgeEvent::ConnectionStatus {
                            connected: false,
                            target: current_target,
                            socket_path: sock,
                            daemon_version: None,
                            error: Some(format!("{err:#}")),
                        });
                    }
                }
            }
            other => {
                let transition = matches!(
                    other,
                    BridgeRequest::AttachSession { .. }
                        | BridgeRequest::CreateSession { .. }
                        | BridgeRequest::OpenSavedSession { .. }
                );
                let client = active_client.clone();
                let tx = evt_tx.clone();
                tasks.spawn(async move {
                    let outcome = match client {
                        Some(client) => dispatch_request(&client, other, &tx).await,
                        None => Err(anyhow!("Not connected. Refresh to reconnect.")),
                    };
                    if let Err(error) = outcome {
                        let message = format!("{error:#}");
                        let _ = tx.unbounded_send(if transition {
                            BridgeEvent::SessionTransitionFailed { message }
                        } else {
                            BridgeEvent::Toast {
                                message,
                                is_error: true,
                            }
                        });
                    }
                });
            }
        }
    }
}

async fn forward_daemon_frames(
    mut rx: mpsc::UnboundedReceiver<Value>,
    evt_tx: futures_mpsc::UnboundedSender<BridgeEvent>,
) {
    while let Some(frame) = rx.recv().await {
        let Some(kind) = frame.get("type").and_then(Value::as_str) else {
            continue;
        };
        match kind {
            "session_event" => {
                if let Some(sid) = frame.get("activeSessionId").and_then(Value::as_str) {
                    let event = frame.get("event").cloned().unwrap_or(Value::Null);
                    if event.get("type").and_then(Value::as_str) == Some("rlm_child_update") {
                        if let Some(sequence) =
                            frame.pointer("/meta/sequence").and_then(Value::as_u64)
                            && let Some(row) = event.get("child").and_then(RlmChildRow::from_wire)
                        {
                            let _ = evt_tx.unbounded_send(BridgeEvent::SubagentUpdated {
                                active_session_id: sid.to_owned(),
                                sequence,
                                row,
                            });
                        }
                        continue;
                    }
                    let _ = evt_tx.unbounded_send(BridgeEvent::SessionEvent {
                        active_session_id: sid.to_string(),
                        event,
                    });
                }
            }
            "side_question_event" => {
                if let Some(sid) = frame.get("activeSessionId").and_then(Value::as_str) {
                    let event = frame.get("event").cloned().unwrap_or(Value::Null);
                    let _ = evt_tx.unbounded_send(BridgeEvent::SideQuestionEvent {
                        active_session_id: sid.to_string(),
                        event,
                    });
                }
            }
            "roster_update" => {
                let mut changed = Vec::new();
                let mut removed = frame
                    .get("removed")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                for entry in frame
                    .get("changed")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if !resident_entry(entry) {
                        if let Some(id) = entry.get("agentId").and_then(Value::as_str) {
                            removed.push(id.to_owned());
                        }
                    } else if let Some(row) = ActiveSessionRow::from_wire(entry) {
                        changed.push(row);
                    }
                }
                let resync = frame
                    .get("resync")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let _ = evt_tx.unbounded_send(BridgeEvent::RosterDelta {
                    changed,
                    removed,
                    resync,
                });
            }
            "session_attached" => {
                if let Some(sid) = frame.get("activeSessionId").and_then(Value::as_str) {
                    let snapshot = frame
                        .get("snapshot")
                        .cloned()
                        .unwrap_or_else(|| frame.clone());
                    let _ = evt_tx.unbounded_send(BridgeEvent::SessionAttached {
                        active_session_id: sid.to_string(),
                        snapshot,
                    });
                }
            }
            "session_snapshot_begin" => {
                if let Some(sid) = frame.get("activeSessionId").and_then(Value::as_str) {
                    let _ = evt_tx.unbounded_send(BridgeEvent::SnapshotChunkBegin {
                        active_session_id: sid.to_string(),
                        header: frame,
                    });
                }
            }
            "session_snapshot_chunk" => {
                if let Some(sid) = frame.get("activeSessionId").and_then(Value::as_str) {
                    let msgs = frame
                        .get("messages")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let _ = evt_tx.unbounded_send(BridgeEvent::SnapshotChunk {
                        active_session_id: sid.to_string(),
                        messages: msgs,
                    });
                }
            }
            "session_snapshot_end" => {
                if let Some(sid) = frame.get("activeSessionId").and_then(Value::as_str) {
                    let _ = evt_tx.unbounded_send(BridgeEvent::SnapshotChunkEnd {
                        active_session_id: sid.to_string(),
                    });
                }
            }
            "session_closed" => {
                if let Some(sid) = frame.get("activeSessionId").and_then(Value::as_str) {
                    let reason = frame
                        .get("reason")
                        .and_then(Value::as_str)
                        .unwrap_or("closed")
                        .to_string();
                    let _ = evt_tx.unbounded_send(BridgeEvent::SessionClosed {
                        active_session_id: sid.to_string(),
                        reason,
                    });
                }
            }
            "model_catalog_changed" | "heartbeats_changed" => {
                let _ = evt_tx.unbounded_send(BridgeEvent::BackgroundRefreshNeeded);
            }
            _ => {}
        }
    }
}

fn resident_entry(value: &Value) -> bool {
    value.get("status").and_then(Value::as_str) != Some("inactive")
        && value.get("queuedChild").and_then(Value::as_bool) != Some(true)
}

async fn refresh_sessions(
    client: &RawClient,
    cwd: Option<String>,
    evt_tx: &futures_mpsc::UnboundedSender<BridgeEvent>,
) {
    refresh_roster(client, evt_tx).await;
    refresh_saved(client, cwd, evt_tx).await;
}

async fn refresh_roster(client: &RawClient, evt_tx: &futures_mpsc::UnboundedSender<BridgeEvent>) {
    match client
        .request(
            DaemonCommand::RosterSubscribe {
                id: None,
                rest: Map::new(),
            },
            Duration::from_secs(10),
        )
        .await
        .and_then(check_daemon_ok)
    {
        Ok(data) => {
            if let Some(items) = data.get("roster").and_then(Value::as_array) {
                let rows = items
                    .iter()
                    .filter(|v| resident_entry(v))
                    .filter_map(ActiveSessionRow::from_wire)
                    .collect();
                let _ = evt_tx.unbounded_send(BridgeEvent::ActiveSessions(rows));
            }
        }
        Err(error) => {
            let _ = evt_tx.unbounded_send(BridgeEvent::Toast {
                message: format!("Could not load active agents: {error:#}"),
                is_error: true,
            });
        }
    }
}

async fn refresh_saved(
    client: &RawClient,
    cwd: Option<String>,
    evt_tx: &futures_mpsc::UnboundedSender<BridgeEvent>,
) {
    if let Err(error) = catalog::load_saved_page(client, cwd, None, evt_tx).await {
        let _ = evt_tx.unbounded_send(BridgeEvent::Toast {
            message: format!("Could not load saved chats: {error:#}"),
            is_error: true,
        });
    }
}

fn check_daemon_ok(resp: DaemonResponse) -> Result<Value> {
    if resp.success {
        Ok(resp.data.unwrap_or(Value::Null))
    } else {
        Err(anyhow!(
            "{}",
            resp.error
                .unwrap_or_else(|| format!("{} failed", resp.command))
        ))
    }
}

fn wire_images(images: &[StagedImage]) -> Option<Value> {
    if images.is_empty() {
        return None;
    }
    Some(Value::Array(
        images
            .iter()
            .map(|img| {
                json!({
                    "type": "image",
                    "mimeType": img.mime_type,
                    "data": img.base64_data,
                })
            })
            .collect(),
    ))
}

async fn submit_prompt(
    client: &RawClient,
    session: String,
    message: String,
    images: Vec<StagedImage>,
    events: &futures_mpsc::UnboundedSender<BridgeEvent>,
) -> Result<()> {
    let response = client
        .request(
            DaemonCommand::Prompt {
                id: None,
                active_session_id: session.clone(),
                message: message.clone(),
                input: PromptInput {
                    content: None,
                    images: wire_images(&images),
                    streaming_behavior: None,
                    queue_if_busy: Some(true),
                    expand_prompt_templates: Some(true),
                    source: None,
                    agent_message_id: None,
                    custom_message: None,
                    queue_key: None,
                    prefix_messages: None,
                    admission_id: None,
                    rlm_notice_nonce: None,
                },
                rest: Map::new(),
            },
            Duration::from_secs(20),
        )
        .await?;
    if response.success {
        return Ok(());
    }
    if matches!(
        response.error_info,
        Some(pa_types::daemon::DaemonErrorInfo::CommandResultUncertain { .. })
    ) {
        return Err(anyhow!(
            "Prompt delivery is uncertain. Reopen the session before resending."
        ));
    }
    let reason = response
        .error
        .unwrap_or_else(|| "The harness refused the prompt".into());
    let _ = events.unbounded_send(BridgeEvent::PromptRejected {
        draft: crate::state::DraftKey::Session(session),
        message,
        images,
        reason,
    });
    Ok(())
}

#[allow(clippy::too_many_lines)]
async fn dispatch_request(
    client: &RawClient,
    req: BridgeRequest,
    evt_tx: &futures_mpsc::UnboundedSender<BridgeEvent>,
) -> Result<()> {
    let std_timeout = Duration::from_secs(20);
    match req {
        BridgeRequest::Connect { .. } => {}
        BridgeRequest::RefreshWorkspace { cwd } => {
            refresh_sessions(client, cwd, evt_tx).await;
        }
        BridgeRequest::LoadOlderSessions { cursor } => {
            catalog::load_saved_page(client, None, Some(cursor), evt_tx).await?;
        }
        BridgeRequest::CreateSession {
            cwd,
            name,
            initial_prompt,
            images,
        } => {
            let mut extra = Map::new();
            extra.insert("cwd".to_string(), json!(cwd));
            let ready: Result<String> = async {
                let resp = client
                    .request(
                        DaemonCommand::Create {
                            id: None,
                            session_path: None,
                            continue_recent: Some(false),
                            no_session: Some(false),
                            name,
                            config: Some(json!({ "cwd": cwd })),
                            telemetry_disabled: Some(true),
                            runtime_metadata: None,
                            lifecycle: Some(DaemonSessionLifecycle::Resident),
                            env: None,
                            launch_env: None,
                            rest: extra,
                        },
                        Duration::from_secs(30),
                    )
                    .await?;
                let data = check_daemon_ok(resp)?;
                let sid = data
                    .get("activeSessionId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("create returned no activeSessionId"))?
                    .to_string();
                attach_and_hydrate(client, &sid, evt_tx).await?;
                Ok(sid)
            }
            .await;
            let sid = match ready {
                Ok(id) => id,
                Err(error) => {
                    if let Some(message) = initial_prompt {
                        let _ = evt_tx.unbounded_send(BridgeEvent::PromptRejected {
                            draft: crate::state::DraftKey::Project(cwd),
                            message,
                            images,
                            reason: format!("{error:#}"),
                        });
                    }
                    return Err(error);
                }
            };
            if let Some(prompt) =
                initial_prompt.filter(|p| !p.trim().is_empty() || !images.is_empty())
            {
                submit_prompt(client, sid, prompt, images, evt_tx).await?;
            }
            refresh_sessions(client, None, evt_tx).await;
        }
        BridgeRequest::OpenSavedSession { session_path, cwd } => {
            let mut extra = Map::new();
            if let Some(ref c) = cwd {
                extra.insert("cwd".to_string(), json!(c));
            }
            let resp = client
                .request(
                    DaemonCommand::Create {
                        id: None,
                        session_path: Some(session_path),
                        continue_recent: Some(false),
                        no_session: Some(false),
                        name: None,
                        config: cwd.map(|c| json!({ "cwd": c })),
                        telemetry_disabled: Some(true),
                        runtime_metadata: None,
                        lifecycle: Some(DaemonSessionLifecycle::Resident),
                        env: None,
                        launch_env: None,
                        rest: extra,
                    },
                    Duration::from_secs(30),
                )
                .await?;

            // The supervisor reuses the current worker if it owns this file. A
            // lease refusal names a holder elsewhere, not a routable local id.
            if matches!(
                resp.error_info,
                Some(pa_types::daemon::DaemonErrorInfo::SessionAlreadyActive { .. })
            ) {
                return Err(anyhow!(
                    "This chat is open in another process. To join a running CLI session, choose CLI daemon in Settings. Otherwise, close the session there before opening it here."
                ));
            }
            let data = check_daemon_ok(resp)?;
            let sid = data
                .get("activeSessionId")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("missing activeSessionId"))?;

            attach_and_hydrate(client, sid, evt_tx).await?;
        }
        BridgeRequest::AttachSession { active_session_id } => {
            attach_and_hydrate(client, &active_session_id, evt_tx).await?;
        }
        BridgeRequest::KillSession { active_session_id } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::Kill {
                            id: None,
                            active_session_id,
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
            refresh_sessions(client, None, evt_tx).await;
        }
        BridgeRequest::RenameActiveSession {
            active_session_id,
            name,
        } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::SetSessionName {
                            id: None,
                            active_session_id,
                            name,
                            worker_token: None,
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
            refresh_sessions(client, None, evt_tx).await;
        }
        BridgeRequest::RenameSavedSession { session_path, name } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::RenameSavedSession {
                            id: None,
                            active_session_id: None,
                            session_path,
                            name,
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
            refresh_sessions(client, None, evt_tx).await;
        }
        BridgeRequest::DeleteSavedSession { session_path } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::DeleteSavedSession {
                            id: None,
                            active_session_id: None,
                            session_path,
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
            refresh_sessions(client, None, evt_tx).await;
        }
        BridgeRequest::SendPrompt {
            active_session_id,
            message,
            images,
        } => {
            submit_prompt(client, active_session_id, message, images, evt_tx).await?;
        }
        BridgeRequest::SteerSession {
            active_session_id,
            message,
        } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::Steer {
                            id: None,
                            active_session_id,
                            message,
                            input: PromptInput {
                                content: None,
                                images: None,
                                streaming_behavior: Some(StreamingBehavior::Steer),
                                queue_if_busy: Some(true),
                                expand_prompt_templates: Some(true),
                                source: None,
                                agent_message_id: None,
                                custom_message: None,
                                queue_key: None,
                                prefix_messages: None,
                                admission_id: None,
                                rlm_notice_nonce: None,
                            },
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
        }
        BridgeRequest::FollowUpSession {
            active_session_id,
            message,
        } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::FollowUp {
                            id: None,
                            active_session_id,
                            message,
                            input: PromptInput {
                                content: None,
                                images: None,
                                streaming_behavior: Some(StreamingBehavior::FollowUp),
                                queue_if_busy: Some(true),
                                expand_prompt_templates: Some(true),
                                source: None,
                                agent_message_id: None,
                                custom_message: None,
                                queue_key: None,
                                prefix_messages: None,
                                admission_id: None,
                                rlm_notice_nonce: None,
                            },
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
        }
        BridgeRequest::AbortSession { active_session_id } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::Abort {
                            id: None,
                            active_session_id,
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
        }
        BridgeRequest::AbortAndSendQueued { active_session_id } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::AbortAndSendQueued {
                            id: None,
                            active_session_id,
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
        }
        BridgeRequest::ClearQueue { active_session_id } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::ClearQueue {
                            id: None,
                            active_session_id,
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
        }
        BridgeRequest::SetModel {
            active_session_id,
            provider,
            model_id,
        } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::SetModel {
                            id: None,
                            active_session_id: active_session_id.clone(),
                            provider: provider.clone(),
                            model_id: model_id.clone(),
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
            hydrate_session_state(client, &active_session_id, evt_tx).await;
            let _ = evt_tx.unbounded_send(BridgeEvent::Toast {
                message: format!("Switched model to {provider}/{model_id}"),
                is_error: false,
            });
        }
        BridgeRequest::SetThinkingLevel {
            active_session_id,
            level,
        } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::SetThinkingLevel {
                            id: None,
                            active_session_id: active_session_id.clone(),
                            level: level.clone(),
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
            hydrate_session_state(client, &active_session_id, evt_tx).await;
        }
        BridgeRequest::SetAutoCompaction {
            active_session_id,
            enabled,
        } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::SetAutoCompaction {
                            id: None,
                            active_session_id: active_session_id.clone(),
                            enabled,
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
            load_inspector_snapshot(client, &active_session_id, evt_tx).await;
        }
        BridgeRequest::SetAutoRetry {
            active_session_id,
            enabled,
        } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::SetAutoRetry {
                            id: None,
                            active_session_id: active_session_id.clone(),
                            enabled,
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
            load_inspector_snapshot(client, &active_session_id, evt_tx).await;
        }
        BridgeRequest::Compact {
            active_session_id,
            instructions,
        } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::Compact {
                            id: None,
                            active_session_id,
                            custom_instructions: instructions.filter(|s| !s.trim().is_empty()),
                            rest: Map::new(),
                        },
                        Duration::from_secs(120),
                    )
                    .await?,
            )?;
        }
        BridgeRequest::Refine {
            active_session_id,
            instructions,
            rollback_id,
            global,
        } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::Refine {
                            id: None,
                            active_session_id: active_session_id.clone(),
                            instructions: instructions.filter(|s| !s.trim().is_empty()),
                            rollback_id,
                            global: Some(global),
                            rest: Map::new(),
                        },
                        Duration::from_secs(120),
                    )
                    .await?,
            )?;
            let _ = evt_tx.unbounded_send(BridgeEvent::Toast {
                message: "Continual Harness refinement completed".to_string(),
                is_error: false,
            });
            load_inspector_snapshot(client, &active_session_id, evt_tx).await;
        }
        BridgeRequest::ReloadSession { active_session_id } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::Reload {
                            id: None,
                            active_session_id: active_session_id.clone(),
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
            attach_and_hydrate(client, &active_session_id, evt_tx).await?;
        }
        BridgeRequest::ExecuteBash {
            active_session_id,
            command,
        } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::ExecuteBash {
                            id: None,
                            active_session_id,
                            command,
                            exclude_from_context: Some(false),
                            transient: Some(false),
                            run_id: None,
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
        }
        BridgeRequest::AbortBash { active_session_id } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::AbortBash {
                            id: None,
                            active_session_id,
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
        }
        BridgeRequest::RefreshInspector { active_session_id } => {
            if let Ok(resp) = client
                .request(
                    DaemonCommand::GetModelCatalog {
                        id: None,
                        active_session_id: active_session_id.clone(),
                        rest: Map::new(),
                    },
                    std_timeout,
                )
                .await
                && resp.success
                && let Some(data) = resp.data
            {
                let _ = evt_tx.unbounded_send(BridgeEvent::ModelCatalogLoaded {
                    active_session_id: active_session_id.clone(),
                    data,
                });
            }
            load_inspector_snapshot(client, &active_session_id, evt_tx).await;
        }
        BridgeRequest::TailShell {
            active_session_id,
            id,
            persistent,
        } => {
            let cmd = if persistent {
                DaemonCommand::TailPersistentShell {
                    id: None,
                    shell_id: id.clone(),
                    lines: 80,
                    rest: Map::new(),
                }
            } else {
                DaemonCommand::TailKernelBash {
                    id: None,
                    active_session_id: active_session_id.clone(),
                    activity_id: id.clone(),
                    lines: Some(80),
                    rest: Map::new(),
                }
            };
            let data = check_daemon_ok(client.request(cmd, std_timeout).await?)?;
            let out = data
                .get("output")
                .or_else(|| data.get("tail"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let _ = evt_tx.unbounded_send(BridgeEvent::ShellTailLoaded {
                active_session_id,
                id,
                output: out,
            });
        }
        BridgeRequest::KillShell {
            active_session_id,
            id,
            persistent,
        } => {
            let cmd = if persistent {
                DaemonCommand::KillPersistentShell {
                    id: None,
                    shell_id: id,
                    rest: Map::new(),
                }
            } else {
                DaemonCommand::KillKernelBash {
                    id: None,
                    active_session_id: active_session_id.clone(),
                    activity_id: id,
                    rest: Map::new(),
                }
            };
            check_daemon_ok(client.request(cmd, std_timeout).await?)?;
            load_inspector_snapshot(client, &active_session_id, evt_tx).await;
        }
        BridgeRequest::SetRlmMaxDepth {
            active_session_id,
            max_depth,
        } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::SetRlmMaxDepth {
                            id: None,
                            active_session_id: active_session_id.clone(),
                            max_depth,
                            global: Some(false),
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
            load_inspector_snapshot(client, &active_session_id, evt_tx).await;
        }
        BridgeRequest::CancelRlmChild {
            active_session_id,
            child_id,
        } => {
            let result = check_daemon_ok(
                client
                    .request(
                        DaemonCommand::CancelRlmChild {
                            id: None,
                            active_session_id: active_session_id.clone(),
                            child_id,
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
            if result.get("cancelled").and_then(Value::as_bool) != Some(true) {
                let _ = evt_tx.unbounded_send(BridgeEvent::Toast {
                    message: "This child is no longer running.".to_owned(),
                    is_error: true,
                });
            }
            load_inspector_snapshot(client, &active_session_id, evt_tx).await;
        }
        BridgeRequest::DeleteRlmSubagent {
            active_session_id,
            child_id,
        } => {
            let result = check_daemon_ok(
                client
                    .request(
                        DaemonCommand::DeleteRlmSubagent {
                            id: None,
                            active_session_id: active_session_id.clone(),
                            child_id,
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
            if result.get("deleted").and_then(Value::as_bool) != Some(true) {
                let _ = evt_tx.unbounded_send(BridgeEvent::Toast {
                    message: "The child was not removed.".to_owned(),
                    is_error: true,
                });
            }
            load_inspector_snapshot(client, &active_session_id, evt_tx).await;
        }
        BridgeRequest::SendAgentMessage {
            from_active_session_id,
            target_active_session_id,
            message,
        } => {
            let receipt = check_daemon_ok(
                client
                    .request(
                        DaemonCommand::SendMessage {
                            id: None,
                            target_active_session_id,
                            message,
                            from_active_session_id,
                            agent_origin: Some(false),
                            delivery_mode: None,
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
            let _ = evt_tx.unbounded_send(BridgeEvent::Toast {
                message: match receipt.get("deliveryStatus").and_then(Value::as_str) {
                    Some("queued") => "Message queued for agent",
                    Some("delivered") => "Delivered message to agent",
                    _ => "Message accepted",
                }
                .to_owned(),
                is_error: false,
            });
        }
        BridgeRequest::StartSideQuestion {
            active_session_id,
            question,
        } => {
            let sq_id = format!("sq-{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
            let _ = evt_tx.unbounded_send(BridgeEvent::SideQuestionStarted {
                active_session_id: active_session_id.clone(),
                side_question_id: sq_id.clone(),
            });
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::StartSideQuestion {
                            id: None,
                            active_session_id,
                            side_question_id: sq_id,
                            question,
                            previous_turns: None,
                            rest: Map::new(),
                        },
                        Duration::from_secs(90),
                    )
                    .await?,
            )?;
        }
        BridgeRequest::AbortSideQuestion {
            active_session_id,
            side_question_id,
        } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::AbortSideQuestion {
                            id: None,
                            active_session_id,
                            side_question_id,
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
        }
        BridgeRequest::CreateHeartbeat {
            active_session_id,
            schedule,
            prompt,
        } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::HeartbeatSet {
                            id: None,
                            active_session_id: active_session_id.clone(),
                            schedule,
                            prompt,
                            delivery_mode: Some(json!("follow_up")),
                            promote_owned_session: None,
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
            load_inspector_snapshot(client, &active_session_id, evt_tx).await;
        }
        BridgeRequest::ManageHeartbeat {
            active_session_id,
            job_id,
            action,
        } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::HeartbeatManage {
                            id: None,
                            active_session_id: active_session_id.clone(),
                            job_id,
                            action: json!(action),
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
            load_inspector_snapshot(client, &active_session_id, evt_tx).await;
        }
        BridgeRequest::CreateCron {
            active_session_id,
            schedule,
            prompt,
        } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::CronAdd {
                            id: None,
                            active_session_id: active_session_id.clone(),
                            schedule,
                            prompt,
                            promote_owned_session: None,
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
            load_inspector_snapshot(client, &active_session_id, evt_tx).await;
        }
        BridgeRequest::CancelCron {
            active_session_id,
            job_id,
        } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::CronCancel {
                            id: None,
                            active_session_id: active_session_id.clone(),
                            job_id,
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
            if let Some(sid) = active_session_id {
                load_inspector_snapshot(client, &sid, evt_tx).await;
            }
        }
        BridgeRequest::SetMcpToken {
            active_session_id,
            server,
            token,
        } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::SetMcpStaticToken {
                            id: None,
                            active_session_id: active_session_id.clone(),
                            server: server.clone(),
                            token,
                            rest: Map::new(),
                        },
                        Duration::from_secs(30),
                    )
                    .await?,
            )?;
            let _ = evt_tx.unbounded_send(BridgeEvent::Toast {
                message: format!("Connected MCP server `{server}`"),
                is_error: false,
            });
            load_inspector_snapshot(client, &active_session_id, evt_tx).await;
        }
        BridgeRequest::RemoveMcp {
            active_session_id,
            server,
        } => {
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::RemoveMcpConnection {
                            id: None,
                            active_session_id: active_session_id.clone(),
                            server,
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
            load_inspector_snapshot(client, &active_session_id, evt_tx).await;
        }
        BridgeRequest::ExportSessionHtml {
            active_session_id,
            output_path,
        } => {
            let out_str = output_path.to_string_lossy().to_string();
            check_daemon_ok(
                client
                    .request(
                        DaemonCommand::ExportHtml {
                            id: None,
                            active_session_id,
                            output_path: Some(out_str.clone()),
                            rest: Map::new(),
                        },
                        std_timeout,
                    )
                    .await?,
            )?;
            let _ = evt_tx.unbounded_send(BridgeEvent::Toast {
                message: format!("Exported HTML transcript to {out_str}"),
                is_error: false,
            });
        }
    }
    Ok(())
}

async fn attach_and_hydrate(
    client: &RawClient,
    active_session_id: &str,
    evt_tx: &futures_mpsc::UnboundedSender<BridgeEvent>,
) -> Result<()> {
    let _ = evt_tx.unbounded_send(BridgeEvent::SessionOpening {
        active_session_id: active_session_id.to_owned(),
    });
    let resp = client
        .request(
            DaemonCommand::Attach {
                id: None,
                active_session_id: active_session_id.to_string(),
                client_id: Some(client.client_id.clone()),
                capabilities: None,
                resume_cursor: None,
                telemetry_disabled: Some(true),
                recovery_config: None,
                env: None,
                launch_env: None,
                rest: Map::new(),
            },
            Duration::from_secs(30),
        )
        .await?;
    let data = check_daemon_ok(resp)?;
    let actual = data
        .get("activeSessionId")
        .and_then(Value::as_str)
        .unwrap_or(active_session_id)
        .to_owned();
    if actual != active_session_id {
        let _ = evt_tx.unbounded_send(BridgeEvent::AttachRedirect {
            requested: active_session_id.to_owned(),
            actual: actual.clone(),
        });
    }
    if let Some(snapshot) = data.get("snapshot") {
        let _ = evt_tx.unbounded_send(BridgeEvent::SessionAttached {
            active_session_id: actual,
            snapshot: snapshot.clone(),
        });
    } else if data.get("state").is_some() || data.get("messages").is_some() {
        let _ = evt_tx.unbounded_send(BridgeEvent::SessionAttached {
            active_session_id: actual,
            snapshot: data,
        });
    }

    Ok(())
}

async fn hydrate_session_state(
    client: &RawClient,
    active_session_id: &str,
    evt_tx: &futures_mpsc::UnboundedSender<BridgeEvent>,
) {
    if let Ok(resp) = client
        .request(
            DaemonCommand::GetState {
                id: None,
                active_session_id: active_session_id.to_string(),
                rest: Map::new(),
            },
            Duration::from_secs(10),
        )
        .await
        && resp.success
        && let Some(state_val) = resp.data
    {
        let _ = evt_tx.unbounded_send(BridgeEvent::SessionEvent {
            active_session_id: active_session_id.to_string(),
            event: json!({
                "type": "desktop_state_sync",
                "state": state_val,
            }),
        });
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_incompatible_supervisor_is_never_sent_a_shutdown() {
        use tokio::io::AsyncReadExt as _;
        let tmp = tempfile::tempdir_in("/tmp").unwrap();
        let path = tmp.path().join("daemon.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket
                .write_all(b"{\"type\":\"daemon_hello\",\"schemaId\":\"other\"}\n")
                .await
                .unwrap();
            let mut received = Vec::new();
            socket.read_to_end(&mut received).await.unwrap();
            received
        });
        let result = ensure_supervisor_listening(
            &path,
            LaunchPolicy::StartCompanion,
            &CliData::new(tmp.path().to_owned()),
        )
        .await;
        assert!(result.is_err());
        assert!(server.await.unwrap().is_empty());
    }

    /// Critical-path guard (AGENTS.md): dispatching a request from the UI
    /// thread must return immediately even when the transport endpoint hangs
    /// and never sends a handshake or reply.
    #[test]
    fn bridge_send_never_blocks_ui_when_socket_hangs() {
        let tmp = tempfile::tempdir().unwrap();
        let sock = tmp.path().join("hanging.sock");
        // Bind a UnixListener that accepts nothing / never greets with daemon_hello.
        let _hanging_listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();

        let (bridge, _rx) = BridgeHandle::spawn(CliData::new(tmp.path().to_owned())).unwrap();
        let start = Instant::now();
        bridge.send(BridgeRequest::Connect {
            target: DaemonTarget::DesktopHarness,
            explicit_socket: Some(sock),
        });
        bridge.send(BridgeRequest::RefreshWorkspace { cwd: None });
        assert!(
            start.elapsed() < Duration::from_millis(25),
            "BridgeHandle::send blocked the caller thread: {:?}",
            start.elapsed()
        );
    }
}
