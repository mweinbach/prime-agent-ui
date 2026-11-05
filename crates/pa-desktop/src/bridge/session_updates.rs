//! Forward live daemon events and refresh the session rosters shown in the workspace.

use std::time::Duration;

use futures::channel::mpsc as futures_mpsc;
use pa_types::daemon::DaemonCommand;
use serde_json::{Map, Value};
use tokio::sync::mpsc;

use super::{BridgeEvent, RawClient, catalog, check_daemon_ok};
use crate::state::{ActiveSessionRow, RlmChildRow};

pub(super) async fn forward_daemon_frames(
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

pub(super) async fn refresh_sessions(
    client: &RawClient,
    cwd: Option<String>,
    evt_tx: &futures_mpsc::UnboundedSender<BridgeEvent>,
) {
    refresh_roster(client, evt_tx).await;
    refresh_saved(client, cwd, evt_tx).await;
}

pub(super) async fn refresh_roster(
    client: &RawClient,
    evt_tx: &futures_mpsc::UnboundedSender<BridgeEvent>,
) {
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

pub(super) async fn refresh_saved(
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
