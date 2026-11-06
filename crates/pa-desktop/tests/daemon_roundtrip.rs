#![allow(clippy::too_many_lines)]
//! End-to-end integration test between `pa-desktop`'s async `BridgeHandle`
//! and a real supervised `prime-agent --mode daemon` process in an isolated
//! temporary state directory.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use pa_core::refinement::{HarnessScope, RefinementKind};
use pa_desktop::bridge::{BridgeEvent, BridgeHandle, BridgeRequest, CliData};
use pa_desktop::state::harness_store::{
    HarnessEntryDraft, HarnessSnapshot, delete_harness_entry, load_provider_auth_items,
    set_provider_api_key, upsert_harness_entry,
};
use pa_desktop::state::models::parse_model_catalog;
use pa_desktop::state::transcript::{AssistantBlock, AttachedSessionState, TranscriptEntry};
use pa_desktop::theme::DaemonTarget;
use pa_desktop::ui::workspace::{LaunchOptions, PrimeDesktopView};

use rune::{Env, Key, KeyEvent, Runtime};

struct IsolatedDaemon {
    child: Child,
    root: tempfile::TempDir,
    socket: PathBuf,
    agent_dir: PathBuf,
    work_dir: PathBuf,
}

impl IsolatedDaemon {
    fn spawn() -> Self {
        Self::with_kernel(None)
    }

    fn with_kernel(kernel: Option<&Path>) -> Self {
        let binary = pa_desktop::bridge::locate_prime_agent_binary()
            .expect("build the companion `prime-agent` before running daemon_roundtrip");
        // Keep Unix sockets short, using the same path spelling before and after
        // the session's parents exist (/tmp is a symlink on macOS).
        #[cfg(unix)]
        let root = tempfile::tempdir_in(std::fs::canonicalize("/tmp").unwrap());
        #[cfg(windows)]
        let root = tempfile::tempdir();
        let root = root.expect("isolated daemon tempdir");
        let socket = root.path().join("d.sock");
        let agent_dir = root.path().join("agent-state");
        let work_dir = root.path().join("project");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::create_dir_all(&work_dir).unwrap();
        let mut command = Command::new(binary);
        command
            .args(["--mode", "daemon", "--daemon-socket"])
            .arg(&socket)
            .current_dir(&work_dir)
            .env_clear()
            .env("HOME", root.path())
            .env("TMPDIR", root.path())
            .env("PRIME_AGENT_CODING_AGENT_DIR", &agent_dir)
            // No REPL calls in these turns; do not bootstrap a Python environment.
            .env(
                "PRIME_AGENT_KERNEL_PYTHON",
                kernel.map_or_else(|| root.path().join("no-kernel"), Path::to_owned),
            )
            .env(
                "PI_PACKAGE_DIR",
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../harness"),
            )
            .env(pa_daemon::worker::WORKER_SUPERVISOR_LOST_EXIT_MS_ENV, "0")
            .env("DO_NOT_TRACK", "1")
            .env("PI_OFFLINE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        command.env("PATH", "/usr/bin:/bin");
        #[cfg(windows)]
        {
            command.env("TMP", root.path()).env("TEMP", root.path());
            // Only inherit OS and shell discovery paths, never provider credentials.
            command.envs(
                ["SystemRoot", "ProgramFiles", "ProgramFiles(x86)", "PATH"]
                    .into_iter()
                    .filter_map(|name| std::env::var_os(name).map(|value| (name, value))),
            );
            pa_core::platform::process::set_no_window(&mut command);
        }
        let child = command.spawn().expect("spawn isolated prime-agent daemon");
        let daemon = Self {
            child,
            root,
            socket,
            agent_dir,
            work_dir,
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let wait_sock = Instant::now() + Duration::from_secs(10);
        while !rt.block_on(pa_daemon::socket::can_connect(
            &daemon.socket,
            Duration::from_millis(250),
        )) {
            assert!(Instant::now() < wait_sock, "daemon did not start listening");
            std::thread::sleep(Duration::from_millis(20));
        }
        daemon
    }

    fn open_workspace(&self, preferences_dir: &Path) -> Runtime<PrimeDesktopView> {
        let state = PrimeDesktopView::new(
            LaunchOptions {
                daemon_socket: Some(self.socket.clone()),
                initial_cwd: Some(self.work_dir.clone()),
                ..LaunchOptions::default()
            },
            preferences_dir.to_path_buf(),
        );
        let mut desktop = Runtime::new(
            state,
            PrimeDesktopView::view,
            (1360.0, 860.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        desktop.poll = Some(PrimeDesktopView::poll);
        desktop.frame(0.0);
        desktop
    }

    fn open_scripted_chat(&self, reply: &str) -> Runtime<PrimeDesktopView> {
        let script = self.root.path().join("faux.json");
        std::fs::write(
            &script,
            serde_json::json!({"engine": "faux", "responses": [reply]}).to_string(),
        )
        .expect("write faux provider script");

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        // Only test setup uses the raw create command. The desktop bridge does not
        // expose the daemon's config.script verification seam to users.
        let link = pa_daemon::supervisor_link::SupervisorLink::new(self.socket.clone());
        let created = rt
            .block_on(link.request_success(
                serde_json::json!({
                    "type": "create",
                    "telemetryDisabled": true,
                    "lifecycle": "resident",
                    "config": { "cwd": self.work_dir, "script": script }
                }),
                Duration::from_secs(25),
            ))
            .expect("create faux session");
        let sid = created["activeSessionId"]
            .as_str()
            .expect("active session id");
        let mut desktop = self.open_workspace(&self.agent_dir);
        wait_for_workspace(&mut desktop, |workspace| {
            workspace
                .active_sessions
                .iter()
                .any(|row| row.active_session_id == sid)
        });
        click(&mut desktop, &format!("live-session-{sid}"));
        wait_for_workspace(&mut desktop, |workspace| {
            workspace.session.active_session_id == sid
        });
        desktop
    }
}

#[track_caller]
fn wait_for_workspace(
    desktop: &mut Runtime<PrimeDesktopView>,
    predicate: impl Fn(&PrimeDesktopView) -> bool,
) {
    let start = Instant::now();
    loop {
        desktop.frame(1.0 / 60.0);
        if predicate(&desktop.state) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "timed out waiting for workspace update"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn click(desktop: &mut Runtime<PrimeDesktopView>, key: &str) {
    desktop.frame(1.0 / 60.0);
    let element = desktop
        .elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some(key) && !element.exiting)
        .unwrap_or_else(|| panic!("missing {key}"));
    let [left, top, width, height] = element.visible;
    assert!(width > 0.0 && height > 0.0, "{key} is clipped");
    let (x, y) = (left + width / 2.0, top + height / 2.0);
    desktop.pointer_down(x, y);
    desktop.pointer_up(x, y);
    desktop.frame(1.0 / 60.0);
}

fn press(desktop: &mut Runtime<PrimeDesktopView>, key: Key) {
    desktop.key(KeyEvent {
        key,
        mods: rune::Mods::default(),
    });
    desktop.frame(1.0 / 60.0);
}

impl Drop for IsolatedDaemon {
    fn drop(&mut self) {
        let pid = i32::try_from(self.child.id());
        // SIGTERM lets the supervisor reap its workers before TempDir removes their journals.
        #[cfg(unix)]
        let stopped = pid.is_ok_and(|pid| {
            use pa_core::platform::process::{Signal, kill_pid};
            kill_pid(pid, Signal::Term)
        });
        #[cfg(windows)]
        let stopped = pid.is_ok_and(pa_core::platform::process::kill_process_group_or_pid);
        if !stopped {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

#[test]
fn desktop_bridge_supervises_session_bash_catalog_and_harness_store() {
    let daemon = IsolatedDaemon::spawn();
    let agent_dir = &daemon.agent_dir;
    let work_dir = &daemon.work_dir;

    // Test Continual Harness & Auth 0o600 persistence against the isolated agent_dir.
    set_provider_api_key(agent_dir, "google", "AIza-test-key").unwrap();
    let providers = load_provider_auth_items(agent_dir);
    assert!(
        providers
            .iter()
            .any(|p| p.provider_id == "google" && p.configured)
    );

    let entry = upsert_harness_entry(
        agent_dir,
        None,
        None,
        &HarnessEntryDraft {
            scope: HarnessScope::Global,
            kind: RefinementKind::Memory,
            id_override: Some("desktop_smoke_mem"),
            title: "Desktop Smoke Memory",
            content: "Keep frame render paths free of synchronous IPC.",
            category_path: None,
        },
    )
    .unwrap();
    assert_eq!(entry.version, 1);
    let snap = HarnessSnapshot::load(agent_dir, None, None);
    assert_eq!(snap.global_entries.len(), 1);
    assert!(
        delete_harness_entry(
            agent_dir,
            None,
            None,
            HarnessScope::Global,
            RefinementKind::Memory,
            "desktop_smoke_mem"
        )
        .unwrap()
    );

    let (bridge, mut rx) = BridgeHandle::spawn(CliData::new(daemon.agent_dir.clone())).unwrap();
    bridge.send(BridgeRequest::Connect {
        target: DaemonTarget::DesktopHarness,
        explicit_socket: Some(daemon.socket.clone()),
    });

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();

    rt.block_on(async {
        let mut session_state = AttachedSessionState::default();
        let mut connected = false;
        let mut attached_sid: Option<String> = None;
        let mut saw_catalog = false;
        let mut saw_bash_done = false;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
        while tokio::time::Instant::now() < deadline {
            let next_evt = tokio::time::timeout(Duration::from_secs(5), rx.next()).await;
            let Some(Some(evt)) = next_evt.ok() else {
                continue;
            };
            match evt {
                BridgeEvent::ConnectionStatus {
                    connected: is_conn,
                    error,
                    ..
                } => {
                    assert!(is_conn, "daemon connect failed: {error:?}");
                    connected = true;
                    bridge.send(BridgeRequest::CreateSession {
                        cwd: work_dir.to_string_lossy().to_string(),
                        name: Some("Desktop Integration Session".to_string()),
                        initial_prompt: None,
                        attachments: Vec::new(),
                    });
                }
                BridgeEvent::SessionAttached {
                    active_session_id,
                    snapshot,
                } => {
                    session_state.load_snapshot(&active_session_id, &snapshot);
                    if attached_sid.is_some() { continue; }
                    attached_sid = Some(active_session_id.clone());
                    bridge.send(BridgeRequest::RefreshInspector { active_session_id: active_session_id.clone() });
                    bridge.send(BridgeRequest::ExecuteBash {
                        active_session_id,
                        command: "printf 'desktop-bridge-ok'".to_string(),
                    });
                }
                BridgeEvent::ModelCatalogLoaded { data: val, .. } => {
                    let (models, _) = parse_model_catalog(&val);
                    assert!(!models.is_empty(), "catalog should carry compiled models");
                    saw_catalog = true;
                }
                BridgeEvent::SessionEvent {
                    active_session_id,
                    event,
                } if attached_sid.as_deref() == Some(active_session_id.as_str()) => {
                    session_state.apply_live_event(&event);
                    if session_state.entries.iter().any(|e| {
                        matches!(
                            e,
                            TranscriptEntry::UserBash {
                                running: false,
                                output,
                                ..
                            } if output.contains("desktop-bridge-ok")
                        )
                    }) {
                        saw_bash_done = true;
                    }
                }
                _ => {}
            }
            if connected && attached_sid.is_some() && saw_catalog && saw_bash_done {
                break;
            }
        }

        assert!(connected, "never connected to daemon");
        let first_sid = attached_sid.expect("never attached to created session");
        assert!(saw_catalog, "never received model catalog");
        assert!(saw_bash_done, "never observed completed !bash output");

        let saved_file = session_state
            .session_file
            .clone()
            .expect("created resident session must report sessionFile");

        // Kill the active worker and reopen the persisted JSONL session via OpenSavedSession.
        bridge.send(BridgeRequest::KillSession {
            active_session_id: first_sid.clone(),
        });

        let mut reopened_with_history = false;
        let reopen_deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        let mut saw_close = false;
        let mut sent_reopen = false;
        while tokio::time::Instant::now() < reopen_deadline {
            let next_evt = tokio::time::timeout(Duration::from_secs(3), rx.next()).await;
            let Some(Some(evt)) = next_evt.ok() else {
                continue;
            };
            match evt {
                BridgeEvent::SessionClosed {
                    active_session_id, ..
                } if active_session_id == first_sid => saw_close = true,
                // The worker publishes session_closed before the supervisor finishes the
                // kill. Reopen only after its post-kill list shows the resident is gone.
                BridgeEvent::ActiveSessions(rows) if saw_close && rows.is_empty() && !sent_reopen => {
                    sent_reopen = true;
                    bridge.send(BridgeRequest::OpenSavedSession {
                        session_path: saved_file.clone(),
                        cwd: Some(work_dir.to_string_lossy().to_string()),
                    });
                }
                BridgeEvent::SessionTransitionFailed { message } => panic!("open failed: {message}"),
                BridgeEvent::Toast { message, is_error: true } => panic!("session command failed: {message}"),
                BridgeEvent::SessionAttached {
                    active_session_id,
                    snapshot,
                } if sent_reopen => {
                    assert_ne!(active_session_id, first_sid, "reopened before the old worker retired");
                    let mut restored = AttachedSessionState::default();
                    restored.load_snapshot(&active_session_id, &snapshot);
                    assert_eq!(restored.display_title(), "Desktop Integration Session");
                    assert!(
                        restored.entries.iter().any(|e| matches!(
                            e,
                            TranscriptEntry::UserBash { output, .. } if output.contains("desktop-bridge-ok")
                        )),
                        "reopened session snapshot must preserve bash history"
                    );
                    reopened_with_history = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(
            reopened_with_history,
            "OpenSavedSession did not reopen persisted session history (received close: {saw_close}, sent reopen: {sent_reopen})"
        );
    });
}

#[test]
fn return_in_the_composer_sends_the_prompt_and_displays_the_reply() {
    const PROMPT: &str = "check notes";
    const REPLY: &str = "The desktop bridge sent it.";
    let daemon = IsolatedDaemon::spawn();
    let mut desktop = daemon.open_scripted_chat(REPLY);
    {
        let workspace = &desktop.state;
        assert_eq!(workspace.session.model_provider.as_deref(), Some("faux"));
    }
    click(&mut desktop, "composer-input");
    press(&mut desktop, Key::Text(PROMPT.into()));
    {
        let workspace = &desktop.state;
        assert_eq!(workspace.composer_input.text, PROMPT);
    }
    press(&mut desktop, Key::Enter);
    wait_for_workspace(&mut desktop, |workspace| {
        matches!(
            workspace.session.entries.last(),
            Some(TranscriptEntry::Assistant {
                streaming: false,
                ..
            })
        )
    });
    {
        let workspace = &desktop.state;
        assert!(workspace.composer_input.text.is_empty());
        let messages = &workspace.session.entries;
        assert!(
            matches!(messages.as_slice(), [
            TranscriptEntry::User { text, .. },
            TranscriptEntry::Assistant { blocks, streaming: false, error: None, .. }
        ] if text == PROMPT && matches!(blocks.as_slice(), [AssistantBlock::Text(reply)] if reply == REPLY)),
            "expected the typed prompt and settled reply, got {messages:?}"
        );
    }
}

#[test]
fn previewed_attachments_send_the_same_file_contents_and_original_image() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use pa_daemon::session_store::SessionFile;

    const ORIGINAL_TEXT: &str = "Water the seedlings at 8 a.m. 🌱";
    // A 3 × 2 RGB image. The daemon should receive the exact PNG, independently of its resized UI preview.
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAMAAAACCAIAAAASFvFNAAAAFUlEQVR4nGPYmOqQ21Z0eFkRA5wFAFooCS16Lx7fAAAAAElFTkSuQmCC";
    let daemon = IsolatedDaemon::spawn();
    let mut desktop = daemon.open_scripted_chat("Attachments received.");
    let document = daemon.work_dir.join("garden-notes.md");
    let photo = daemon.work_dir.join("garden.png");
    std::fs::write(&document, ORIGINAL_TEXT).unwrap();
    std::fs::write(&photo, STANDARD.decode(PNG).unwrap()).unwrap();
    desktop.drop_files(
        vec![document.clone(), photo.clone()],
        desktop.find("composer-input"),
    );
    wait_for_workspace(&mut desktop, |w| w.staged_attachments.len() == 2);

    // Previews and delivery use the staged copy, even if the originals change or disappear.
    std::fs::write(&document, "Changed on disk").unwrap();
    std::fs::remove_file(&photo).unwrap();
    let document_card = desktop
        .state
        .staged_attachments
        .iter()
        .find(|a| a.filename == "garden-notes.md")
        .unwrap();
    let document_key = format!("preview-attachment-{}", document_card.id);
    click(&mut desktop, &document_key);
    assert!(
        desktop.find("attachment-preview-text").is_some(),
        "text preview did not open"
    );
    assert!(
        desktop
            .elements()
            .iter()
            .any(|e| e.text.as_ref().is_some_and(|t| t.contains(ORIGINAL_TEXT)))
    );
    click(&mut desktop, "attachment-preview-close");

    let image_card = desktop
        .state
        .staged_attachments
        .iter()
        .find(|a| a.filename == "garden.png")
        .unwrap();
    let image_key = format!("preview-attachment-{}", image_card.id);
    click(&mut desktop, &image_key);
    assert!(desktop.find("attachment-preview-image").is_some());
    click(&mut desktop, "attachment-preview-close");
    click(&mut desktop, "composer-input");
    // The files themselves are a valid prompt.
    press(&mut desktop, Key::Enter);
    wait_for_workspace(&mut desktop, |w| {
        matches!(
            w.session.entries.last(),
            Some(TranscriptEntry::Assistant {
                streaming: false,
                error: None,
                ..
            })
        )
    });
    assert!(desktop.state.staged_attachments.is_empty());

    let session = SessionFile::open(Path::new(
        desktop.state.session.session_file.as_ref().unwrap(),
    ))
    .unwrap();
    let messages = session.messages();
    let submitted = messages
        .iter()
        .find(|m| m["role"] == "user")
        .expect("persisted user message");
    let parts = submitted["content"].as_array().unwrap();
    let text = parts.iter().find(|part| part["type"] == "text").unwrap()["text"]
        .as_str()
        .unwrap();
    assert!(text.contains("garden-notes.md") && text.contains(ORIGINAL_TEXT));
    assert!(!text.contains("Changed on disk") && !text.contains(daemon.work_dir.to_str().unwrap()));
    let image = parts
        .iter()
        .find(|part| part["type"] == "image")
        .expect("persisted image part");
    assert_eq!(image["mimeType"], "image/png");
    assert_eq!(image["data"], PNG);
}

#[test]
fn choose_model_and_reasoning_before_sending_the_first_prompt() {
    let daemon = IsolatedDaemon::spawn();
    // Register a local catalog only. Switching models and effort never needs a completion.
    let catalog = serde_json::json!({"providers": {"fixture": {
        "api": "openai-completions", "baseUrl": "http://127.0.0.1:9/v1", "apiKey": "fixture",
        "models": [
            {"id": "writer", "name": "Fixture Writer", "reasoning": false},
            {"id": "planner", "name": "Fixture Planner", "reasoning": true,
             "thinkingLevelMap": {"off":null,"minimal":null,"low":"low","medium":"medium","high":"high"}}
        ]
    }}});
    std::fs::write(daemon.agent_dir.join("models.json"), catalog.to_string()).unwrap();
    let settings = serde_json::json!({"defaultProvider":"fixture","defaultModel":"writer","defaultThinkingLevel":"low"});
    std::fs::write(daemon.agent_dir.join("settings.json"), settings.to_string()).unwrap();
    let mut desktop = daemon.open_workspace(&daemon.agent_dir);
    wait_for_workspace(&mut desktop, |w| w.daemon_connected);
    assert!(desktop.state.session.active_session_id.is_empty());
    click(&mut desktop, "composer-input");
    press(&mut desktop, Key::Text("Unsent plan".into()));
    click(&mut desktop, "composer-model-pill");
    wait_for_workspace(&mut desktop, |w| {
        w.catalog_models
            .iter()
            .any(|m| m.provider == "fixture" && m.id == "planner")
            && w.model_picker_ready()
    });
    click(&mut desktop, "modal-search");
    press(&mut desktop, Key::Text("Fixture Planner".into()));
    press(&mut desktop, Key::Enter);
    wait_for_workspace(&mut desktop, |w| {
        w.session.model_provider.as_deref() == Some("fixture")
            && w.session.model_id.as_deref() == Some("planner")
            && w.model_picker_ready()
    });
    let session_id = desktop.state.session.active_session_id.clone();
    click(&mut desktop, "composer-effort-pill");
    assert!(desktop.find("reasoning-level-high").is_some());
    assert!(desktop.find("reasoning-level-max").is_none());
    click(&mut desktop, "reasoning-level-high");
    wait_for_workspace(&mut desktop, |w| {
        w.session.thinking_level == "high" && w.model_picker_ready()
    });
    assert_eq!(desktop.state.session.active_session_id, session_id);
    assert_eq!(desktop.state.composer_input.text, "Unsent plan");
    assert!(!desktop.state.session.entries.iter().any(|e| matches!(
        e,
        TranscriptEntry::User { .. } | TranscriptEntry::Assistant { .. }
    )));
}

#[test]
fn connecting_a_provider_retries_the_selected_model_in_the_same_unsent_chat() {
    let daemon = IsolatedDaemon::spawn();
    // A local model for a built-in, key-only provider exercises the CLI's auth store.
    // A new custom provider would require an apiKey in models.json itself.
    let models = serde_json::json!({"providers": {
        "local-writer": {
            "api":"openai-completions", "baseUrl":"http://127.0.0.1:9/v1", "apiKey":"local-fixture",
            "models":[{"id":"writer","name":"Offline Writer"}]
        },
        "mistral": {
            "api":"openai-completions", "baseUrl":"http://127.0.0.1:9/v1",
            "models":[{"id":"editor","name":"Offline Editor"}]
        }
    }});
    std::fs::write(daemon.agent_dir.join("models.json"), models.to_string()).unwrap();
    let settings = serde_json::json!({"defaultProvider":"local-writer","defaultModel":"writer"});
    std::fs::write(daemon.agent_dir.join("settings.json"), settings.to_string()).unwrap();
    set_provider_api_key(&daemon.agent_dir, "mistral", "initial-test-value").unwrap();

    let mut desktop = daemon.open_workspace(&daemon.agent_dir);
    wait_for_workspace(&mut desktop, |w| w.daemon_connected);
    click(&mut desktop, "composer-input");
    press(&mut desktop, Key::Text("/login".into()));
    press(&mut desktop, Key::Enter);
    assert!(desktop.find("connect-provider").is_some());
    click(&mut desktop, "provider-connect-cancel");
    assert!(desktop.state.session.active_session_id.is_empty());
    click(&mut desktop, "composer-input");
    press(&mut desktop, Key::Text("Unsent edit".into()));
    click(&mut desktop, "composer-model-pill");
    wait_for_workspace(&mut desktop, |w| {
        w.model_picker_ready()
            && w.catalog_models
                .iter()
                .any(|m| m.provider == "mistral" && m.id == "editor" && m.configured)
    });
    let sid = desktop.state.session.active_session_id.clone();

    // A second CLI can remove a credential after the desktop has rendered its catalog.
    set_provider_api_key(&daemon.agent_dir, "mistral", "").unwrap();
    click(&mut desktop, "modal-search");
    press(&mut desktop, Key::Text("Offline Editor".into()));
    press(&mut desktop, Key::Enter);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        desktop.frame(1. / 60.);
        if desktop.find("provider-connect-input").is_some() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "daemon refusal did not open API-key entry"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    click(&mut desktop, "provider-connect-input");
    press(&mut desktop, Key::Text("replacement-test-value".into()));
    assert!(!desktop.elements().iter().any(|e| {
        e.text
            .as_deref()
            .is_some_and(|s| s.contains("replacement-test-value"))
    }));
    press(&mut desktop, Key::Enter);
    wait_for_workspace(&mut desktop, |w| {
        w.session.model_provider.as_deref() == Some("mistral")
            && w.session.model_id.as_deref() == Some("editor")
            && w.model_picker_ready()
    });
    assert_eq!(desktop.state.session.active_session_id, sid);
    assert_eq!(desktop.state.composer_input.text, "Unsent edit");
    let stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(daemon.agent_dir.join("auth.json")).unwrap())
            .unwrap();
    assert_eq!(stored["mistral"]["key"], "replacement-test-value");
    assert!(!desktop.state.session.entries.iter().any(|e| matches!(
        e,
        TranscriptEntry::User { .. } | TranscriptEntry::Assistant { .. }
    )));
}

#[test]
fn project_new_chat_preserves_drafts_without_creating_sessions() {
    let daemon = IsolatedDaemon::spawn();
    let first = &daemon.work_dir;
    let second = daemon.root.path().join("second-project");
    std::fs::create_dir(&second).unwrap();
    std::fs::write(
        daemon.agent_dir.join("desktop-ui.json"),
        serde_json::json!({"projectFolders": [first, second]}).to_string(),
    )
    .unwrap();

    let mut desktop = daemon.open_workspace(&daemon.agent_dir);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    wait_for_workspace(&mut desktop, |v| v.daemon_connected);
    let first_button = format!("project-new-chat-{}", first.display());
    let second_button = format!("project-new-chat-{}", second.display());
    for (button, cwd, expected, keys) in [
        (&first_button, first, "", "plan"),
        (&second_button, &second, "", "ship"),
        (&first_button, first, "plan", ""),
        (&second_button, &second, "ship", ""),
    ] {
        click(&mut desktop, button);
        {
            let view = &desktop.state;
            assert_eq!(Path::new(&view.working_cwd), cwd);
            assert_eq!(view.composer_input.text, expected);
        }
        if !keys.is_empty() {
            press(&mut desktop, Key::Text(keys.into()));
        }
    }

    // Check the supervisor itself; the sidebar can lag a worker creation.
    let link = pa_daemon::supervisor_link::SupervisorLink::new(daemon.socket.clone());
    let roster = rt
        .block_on(link.request_success(
            serde_json::json!({"type": "list", "all": true}),
            Duration::from_secs(5),
        ))
        .expect("list daemon sessions");
    assert_eq!(roster["sessions"], serde_json::json!([]));
}

#[test]
fn an_older_cli_chat_opens_from_another_project_without_copying_it() {
    use pa_daemon::session_store::{SessionFile, session_file_name};

    let daemon = IsolatedDaemon::spawn();
    let project = daemon.root.path().join("other-project");
    let checkout = project.join("checkout");
    std::fs::create_dir_all(&checkout).unwrap();
    // A Finder launch can select a CLI profile outside its default agent directory.
    let desktop_dir = daemon.root.path().join("desktop");
    std::fs::create_dir(&desktop_dir).unwrap();
    std::fs::write(
        desktop_dir.join("desktop-ui.json"),
        serde_json::json!({"cliDataDir": daemon.agent_dir, "projectFolders": [project]})
            .to_string(),
    )
    .unwrap();
    let credentials = br#"{"custom-saved":{"type":"api_key","key":"local-test-only"}}"#;
    std::fs::write(daemon.agent_dir.join("auth.json"), credentials).unwrap();
    let sessions = daemon.agent_dir.join("sessions");
    let mut older = SessionFile::create(checkout.to_str().unwrap(), None, 0);
    older.append_message(&serde_json::json!({
        "role": "user", "content": "A note from the CLI", "timestamp": 1
    }));
    let path = sessions.join(session_file_name(older.session_id()));
    older.set_path(path.clone());
    older.rewrite().unwrap();
    let history = std::fs::read(&path).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH)
        .unwrap();

    // Fill the first page with a different project's sessions.
    for _ in 0..120 {
        let mut file = SessionFile::create(daemon.work_dir.to_str().unwrap(), None, 0);
        file.set_path(sessions.join(session_file_name(file.session_id())));
        file.rewrite().unwrap();
    }
    let mut desktop = daemon.open_workspace(&desktop_dir);
    wait_for_workspace(&mut desktop, |view| !view.saved_sessions.is_empty());
    {
        let view = &desktop.state;
        assert!(
            !view
                .saved_sessions
                .iter()
                .any(|s| s.id == older.session_id())
        );
    }
    click(&mut desktop, "sidebar-search");
    press(&mut desktop, Key::Text("other-project".into()));
    click(&mut desktop, "load-older-chats");
    wait_for_workspace(&mut desktop, |view| {
        view.saved_sessions
            .iter()
            .any(|s| s.id == older.session_id())
    });
    click(&mut desktop, &format!("saved-chat-{}", path.display()));
    wait_for_workspace(&mut desktop, |view| {
        view.session.session_id == older.session_id()
    });
    {
        let view = &desktop.state;
        assert_eq!(view.session.session_file.as_deref(), path.to_str());
        assert_eq!(view.working_cwd, checkout.to_str().unwrap());
        assert!(view.session.entries.iter().any(|entry| matches!(entry,
            TranscriptEntry::User { text, .. } if text == "A note from the CLI")));
    }
    {
        let view = &desktop.state;
        assert_eq!(view.agent_dir, daemon.agent_dir);
        assert!(
            view.provider_auth
                .iter()
                .any(|item| item.provider_id == "custom-saved" && item.has_stored_api_key)
        );
    }
    // Reopening may append worker state, but never changes prior journal bytes.
    assert!(std::fs::read(&path).unwrap().starts_with(&history));
    assert!(!desktop_dir.join("auth.json").exists());
    assert_eq!(
        std::fs::read(daemon.agent_dir.join("auth.json")).unwrap(),
        credentials
    );
}

#[test]
fn temporary_chat_can_be_found_and_reopened_after_its_folder_is_removed() {
    use pa_daemon::session_store::{SessionFile, session_file_name};

    let daemon = IsolatedDaemon::spawn();
    let removed = daemon.root.path().join("former-test-project");
    let mut file = SessionFile::create(removed.to_str().unwrap(), None, 0);
    file.append_message(
        &serde_json::json!({"role":"user", "content":"A test chat", "timestamp": 1}),
    );
    let log = daemon
        .agent_dir
        .join("sessions")
        .join(session_file_name(file.session_id()));
    file.set_path(log.clone());
    file.rewrite().unwrap();
    let mut desktop = daemon.open_workspace(&daemon.agent_dir);
    wait_for_workspace(&mut desktop, |w| {
        w.saved_sessions
            .iter()
            .any(|s| s.path == log.to_string_lossy())
    });
    let saved_chat = format!("saved-chat-{}", log.display());
    assert!(
        !desktop
            .elements()
            .iter()
            .any(|el| el.key.as_deref() == Some(&saved_chat))
    );
    click(&mut desktop, "sidebar-history");
    click(&mut desktop, "project-toggle-\0temporary");
    click(&mut desktop, &saved_chat);
    wait_for_workspace(&mut desktop, |w| w.session.session_id == file.session_id());
    {
        let w = &desktop.state;
        assert_eq!(w.session.session_file.as_deref(), log.to_str());
        assert_eq!(w.working_cwd, removed.to_string_lossy());
        assert!(
            w.session
                .entries
                .iter()
                .any(|e| matches!(e, TranscriptEntry::User {text, ..} if text == "A test chat"))
        );
    }
    assert!(
        !removed.exists(),
        "viewing history must not recreate the working directory"
    );
}

/// Requires an installed kernel runtime; the workspace kernel-backed daemon tests use the same override.
#[test]
#[ignore = "set PA_E2E_KERNEL_PYTHON to a provisioned interpreter and run with --ignored"]
fn a_spawned_subagent_updates_its_card_and_opens_its_own_transcript() {
    let kernel = std::env::var_os("PA_E2E_KERNEL_PYTHON").expect("set PA_E2E_KERNEL_PYTHON");
    let daemon = IsolatedDaemon::with_kernel(Some(Path::new(&kernel)));
    let parent_script = daemon.root.path().join("parent.json");
    let child_script = daemon.root.path().join("child.json");
    // Keep the parent's tool turn open until the desktop observes the running child. A refresh
    // at turn_end alone cannot pass this test. The child prompt starts after parent settle.
    let release = daemon.root.path().join("observed");
    let code = format!(
        "import asyncio, os\nchild = await rlm.spawn('Check the module', name='reviewer')\nwhile not os.path.exists({:?}):\n    await asyncio.sleep(0.02)",
        release.to_str().unwrap()
    );
    std::fs::write(
        &parent_script,
        serde_json::json!({"engine":"faux", "responses":[
            {"content":[{"type":"toolCall","name":"ipython","arguments":{"code":code}}]},
            {"text":"Review assigned."}, {"text":"Acknowledged."}
        ]})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        &child_script,
        r#"{"engine":"faux","responses":[{"text":"Review complete."}]}"#,
    )
    .unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let link = pa_daemon::supervisor_link::SupervisorLink::new(daemon.socket.clone());
    let created = rt.block_on(link.request_success(serde_json::json!({"type":"create", "name":"parent", "telemetryDisabled":true,
        "lifecycle":"resident", "config":{"cwd":daemon.work_dir, "script": parent_script, "childScript":child_script}}), Duration::from_secs(30))).unwrap();
    let sid = created["activeSessionId"].as_str().unwrap();
    let mut desktop = daemon.open_workspace(&daemon.agent_dir);
    wait_for_workspace(&mut desktop, |w| {
        w.active_sessions
            .iter()
            .any(|row| row.active_session_id == sid)
    });
    click(&mut desktop, &format!("live-session-{sid}"));
    wait_for_workspace(&mut desktop, |w| {
        w.session.active_session_id == sid && w.rlm_children.loaded
    });
    rt.block_on(link.request_success(serde_json::json!({"type":"prompt", "activeSessionId":sid,"message":"Delegate the review."}), Duration::from_secs(30))).unwrap();
    wait_for_workspace(&mut desktop, |w| {
        w.rlm_children
            .rows
            .iter()
            .any(|c| c.name == "reviewer" && c.status == "running")
    });
    click(&mut desktop, "session-subagents");
    std::fs::write(release, b"observed").unwrap();
    wait_for_workspace(&mut desktop, |w| {
        w.rlm_children.rows.iter().any(|c| {
            c.name == "reviewer"
                && c.status == "done"
                && c.answer_preview.as_deref() == Some("Review complete.")
        })
    });
    click(&mut desktop, "subagent-inspect");
    wait_for_workspace(&mut desktop, |w| {
        w.session.session_name.as_deref() == Some("reviewer")
    });
    {
        let w = &desktop.state;
        assert_ne!(w.session.active_session_id, sid);
        assert!(w.session.entries.iter().any(|e| matches!(e, TranscriptEntry::Assistant { blocks, .. } if blocks.contains(&AssistantBlock::Text("Review complete.".into())))));
    }
    click(&mut desktop, "subagent-parent");
    wait_for_workspace(&mut desktop, |w| {
        w.session.active_session_id == sid && w.rlm_children.rows.len() == 1
    });
}
