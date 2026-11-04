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
use pa_desktop::ui::text_input::bind_text_input_keys;
use pa_desktop::ui::workspace::{
    LaunchOptions, PrimeDesktopView, register_workspace_bindings_and_menus,
};

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

    fn open_workspace<'a>(
        &self,
        app: &'a mut gpui::TestAppContext,
        preferences_dir: &Path,
    ) -> (
        gpui::Entity<PrimeDesktopView>,
        &'a mut gpui::VisualTestContext,
    ) {
        app.dispatcher.allow_parking(); // Socket traffic wakes the GPUI test dispatcher.
        app.update(|cx| {
            bind_text_input_keys(cx);
            register_workspace_bindings_and_menus(cx);
        });
        app.add_window_view(|window, cx| {
            PrimeDesktopView::new(
                LaunchOptions {
                    daemon_socket: Some(self.socket.clone()),
                    initial_cwd: Some(self.work_dir.clone()),
                    ..LaunchOptions::default()
                },
                preferences_dir.to_path_buf(),
                window,
                cx,
            )
        })
    }
}

fn wait_for_workspace(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<PrimeDesktopView>,
    rt: &tokio::runtime::Runtime,
    predicate: impl Fn(&PrimeDesktopView) -> bool,
) {
    let _clock = rt.enter();
    cx.executor()
        .block_test(tokio::time::timeout(
            Duration::from_secs(15),
            cx.condition(view, |view, _| predicate(view)),
        ))
        .expect("timed out waiting for workspace update");
}

fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    cx.update(|window, cx| window.draw(cx).clear());
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
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
            id_override: Some("gpui_smoke_mem"),
            title: "GPUI Desktop Smoke Memory",
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
            "gpui_smoke_mem"
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
                        name: Some("GPUI Integration Session".to_string()),
                        initial_prompt: None,
                        images: Vec::new(),
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
                        command: "printf 'gpui-bridge-ok'".to_string(),
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
                } => {
                    if attached_sid.as_deref() == Some(active_session_id.as_str()) {
                        session_state.apply_live_event(&event);
                        if session_state.entries.iter().any(|e| {
                            matches!(
                                e,
                                TranscriptEntry::UserBash {
                                    running: false,
                                    output,
                                    ..
                                } if output.contains("gpui-bridge-ok")
                            )
                        }) {
                            saw_bash_done = true;
                        }
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
                    assert_eq!(restored.display_title(), "GPUI Integration Session");
                    assert!(
                        restored.entries.iter().any(|e| matches!(
                            e,
                            TranscriptEntry::UserBash { output, .. } if output.contains("gpui-bridge-ok")
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
    let script = daemon.root.path().join("faux.json");
    std::fs::write(
        &script,
        serde_json::json!({"engine": "faux", "responses": [REPLY]}).to_string(),
    )
    .expect("write faux provider script");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    // Only test setup uses the raw create command. The desktop bridge does not
    // expose the daemon's config.script verification seam to users.
    let link = pa_daemon::supervisor_link::SupervisorLink::new(daemon.socket.clone());
    let created = rt
        .block_on(link.request_success(
            serde_json::json!({
                "type": "create",
                "telemetryDisabled": true,
                "lifecycle": "resident",
                "config": { "cwd": daemon.work_dir, "script": script }
            }),
            Duration::from_secs(25),
        ))
        .expect("create faux session");
    let sid = created["activeSessionId"]
        .as_str()
        .expect("active session id");
    let mut app = gpui::TestAppContext::single();
    let (view, cx) = daemon.open_workspace(&mut app, &daemon.agent_dir);
    wait_for_workspace(cx, &view, &rt, |workspace| {
        workspace
            .active_sessions
            .iter()
            .any(|row| row.active_session_id == sid)
    });
    click(cx, "live-session");
    wait_for_workspace(cx, &view, &rt, |workspace| {
        workspace.session.active_session_id == sid
    });
    view.read_with(cx, |workspace, _| {
        assert_eq!(workspace.session.model_provider.as_deref(), Some("faux"));
    });
    click(cx, "composer-input");
    cx.update(|window, cx| window.draw(cx).clear());
    cx.simulate_keystrokes("c h e c k space n o t e s");
    view.read_with(cx, |workspace, cx| {
        assert_eq!(workspace.composer_input.read(cx).text(), PROMPT);
    });
    cx.simulate_keystrokes("enter");
    wait_for_workspace(cx, &view, &rt, |workspace| {
        matches!(
            workspace.session.entries.last(),
            Some(TranscriptEntry::Assistant {
                streaming: false,
                ..
            })
        )
    });
    view.read_with(cx, |workspace, cx| {
        assert!(workspace.composer_input.read(cx).text().is_empty());
        let messages = &workspace.session.entries;
        assert!(matches!(messages.as_slice(), [
            TranscriptEntry::User { text, .. },
            TranscriptEntry::Assistant { blocks, streaming: false, error: None, .. }
        ] if text == PROMPT && matches!(blocks.as_slice(), [AssistantBlock::Text(reply)] if reply == REPLY)),
            "expected the typed prompt and settled reply, got {messages:?}");
    });
    app.quit();
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

    let mut app = gpui::TestAppContext::single();
    let (view, cx) = daemon.open_workspace(&mut app, &daemon.agent_dir);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    wait_for_workspace(cx, &view, &rt, |v| v.daemon_connected);
    // GPUI requires 'static debug selectors, but the fixture paths are unique per run.
    let first_button: &'static str = format!("project-new-chat-{}", first.display()).leak();
    let second_button: &'static str = format!("project-new-chat-{}", second.display()).leak();
    for (button, cwd, expected, keys) in [
        (first_button, first, "", "p l a n"),
        (second_button, &second, "", "s h i p"),
        (first_button, first, "plan", ""),
        (second_button, &second, "ship", ""),
    ] {
        click(cx, button);
        view.read_with(cx, |view, cx| {
            assert_eq!(Path::new(&view.working_cwd), cwd);
            assert_eq!(view.composer_input.read(cx).text(), expected);
        });
        if !keys.is_empty() {
            cx.update(|window, cx| window.draw(cx).clear());
            cx.simulate_keystrokes(keys);
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
    app.quit();
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
    let mut app = gpui::TestAppContext::single();
    let (view, cx) = daemon.open_workspace(&mut app, &desktop_dir);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .build()
        .unwrap();
    wait_for_workspace(cx, &view, &rt, |view| !view.saved_sessions.is_empty());
    view.update(cx, |view, cx| {
        assert!(
            !view
                .saved_sessions
                .iter()
                .any(|s| s.id == older.session_id())
        );
        view.sidebar_search_input
            .update(cx, |input, cx| input.set_text("other-project", cx));
    });
    click(cx, "load-older-chats");
    wait_for_workspace(cx, &view, &rt, |view| {
        view.saved_sessions
            .iter()
            .any(|s| s.id == older.session_id())
    });
    click(cx, "saved-chat");
    wait_for_workspace(cx, &view, &rt, |view| {
        view.session.session_id == older.session_id()
    });
    view.read_with(cx, |view, _| {
        assert_eq!(view.session.session_file.as_deref(), path.to_str());
        assert_eq!(view.working_cwd, checkout.to_str().unwrap());
        assert!(view.session.entries.iter().any(|entry| matches!(entry,
            TranscriptEntry::User { text, .. } if text == "A note from the CLI")));
    });
    view.read_with(cx, |view, _| {
        assert_eq!(view.agent_dir, daemon.agent_dir);
        assert!(
            view.provider_auth
                .iter()
                .any(|item| item.provider_id == "custom-saved" && item.has_stored_api_key)
        );
    });
    // Reopening may append worker state, but never changes prior journal bytes.
    assert!(std::fs::read(&path).unwrap().starts_with(&history));
    assert!(!desktop_dir.join("auth.json").exists());
    assert_eq!(
        std::fs::read(daemon.agent_dir.join("auth.json")).unwrap(),
        credentials
    );
    app.quit();
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
    let mut app = gpui::TestAppContext::single();
    let (view, cx) = daemon.open_workspace(&mut app, &daemon.agent_dir);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .build()
        .unwrap();
    wait_for_workspace(cx, &view, &rt, |w| {
        w.saved_sessions
            .iter()
            .any(|s| s.path == log.to_string_lossy())
    });
    cx.update(|window, cx| window.draw(cx).clear());
    assert!(cx.debug_bounds("saved-chat").is_none());
    click(cx, "sidebar-history");
    click(cx, "project-toggle-\0temporary");
    click(cx, "saved-chat");
    wait_for_workspace(cx, &view, &rt, |w| {
        w.session.session_id == file.session_id()
    });
    view.read_with(cx, |w, _| {
        assert_eq!(w.session.session_file.as_deref(), log.to_str());
        assert_eq!(w.working_cwd, removed.to_string_lossy());
        assert!(
            w.session
                .entries
                .iter()
                .any(|e| matches!(e, TranscriptEntry::User {text, ..} if text == "A test chat"))
        );
    });
    assert!(
        !removed.exists(),
        "viewing history must not recreate the working directory"
    );
    app.quit();
}

/// Requires an installed kernel runtime; the workspace kernel-backed daemon tests use the same override.
#[test]
#[ignore = "set PA_E2E_KERNEL_PYTHON to a provisioned interpreter and run with --ignored"]
fn a_spawned_subagent_updates_its_card_and_opens_its_own_transcript() {
    let kernel = std::env::var_os("PA_E2E_KERNEL_PYTHON").expect("set PA_E2E_KERNEL_PYTHON");
    let daemon = IsolatedDaemon::with_kernel(Some(Path::new(&kernel)));
    let parent_script = daemon.root.path().join("parent.json");
    let child_script = daemon.root.path().join("child.json");
    // Keep the parent's tool turn open until GPUI observes the running child. A refresh
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
    let mut app = gpui::TestAppContext::single();
    let (view, cx) = daemon.open_workspace(&mut app, &daemon.agent_dir);
    wait_for_workspace(cx, &view, &rt, |w| {
        w.active_sessions
            .iter()
            .any(|row| row.active_session_id == sid)
    });
    click(cx, "live-session");
    wait_for_workspace(cx, &view, &rt, |w| {
        w.session.active_session_id == sid && w.rlm_children.loaded
    });
    rt.block_on(link.request_success(serde_json::json!({"type":"prompt", "activeSessionId":sid,"message":"Delegate the review."}), Duration::from_secs(30))).unwrap();
    wait_for_workspace(cx, &view, &rt, |w| {
        w.rlm_children
            .rows
            .iter()
            .any(|c| c.name == "reviewer" && c.status == "running")
    });
    click(cx, "session-subagents");
    std::fs::write(release, b"observed").unwrap();
    wait_for_workspace(cx, &view, &rt, |w| {
        w.rlm_children.rows.iter().any(|c| {
            c.name == "reviewer"
                && c.status == "done"
                && c.answer_preview.as_deref() == Some("Review complete.")
        })
    });
    click(cx, "subagent-inspect");
    wait_for_workspace(cx, &view, &rt, |w| {
        w.session.session_name.as_deref() == Some("reviewer")
    });
    view.read_with(cx, |w, _| {
        assert_ne!(w.session.active_session_id, sid);
        assert!(w.session.entries.iter().any(|e| matches!(e, TranscriptEntry::Assistant { blocks, .. } if blocks.contains(&AssistantBlock::Text("Review complete.".into())))));
    });
    click(cx, "subagent-parent");
    wait_for_workspace(cx, &view, &rt, |w| {
        w.session.active_session_id == sid && w.rlm_children.rows.len() == 1
    });
    app.quit();
}
