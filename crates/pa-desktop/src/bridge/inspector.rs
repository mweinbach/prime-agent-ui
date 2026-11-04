//! Read the attached session's inspector data without delaying child lifecycle updates.

use super::{BridgeEvent, RawClient, check_daemon_ok};
use crate::state::{McpRow, RlmChildRow, ScheduleRow, ShellProcessRow};
use anyhow::Context as _;
use futures::channel::mpsc as futures_mpsc;
use pa_types::daemon::DaemonCommand;
use serde_json::{Map, Value};
use std::time::Duration;

pub(super) async fn load_inspector_snapshot(
    client: &RawClient,
    active_session_id: &str,
    evt_tx: &futures_mpsc::UnboundedSender<BridgeEvent>,
) {
    let t = Duration::from_secs(8);
    let sid = active_session_id.to_string();

    let ((), depth_res, kbash_res, pshell_res, hb_res, cron_res, mcp_res, stats_res, prompt_res) = tokio::join!(
        async {
            let result = async {
                let data = check_daemon_ok(
                    client
                        .request(
                            DaemonCommand::GetRlmChildren {
                                id: None,
                                active_session_id: sid.clone(),
                                rest: Map::new(),
                            },
                            t,
                        )
                        .await?,
                )?;
                let sequence = data
                    .get("eventSequence")
                    .and_then(Value::as_u64)
                    .context("Missing child roster sequence")?;
                let rows = data
                    .get("children")
                    .and_then(Value::as_array)
                    .context("Missing child roster")?
                    .iter()
                    .map(RlmChildRow::from_wire)
                    .collect::<Option<Vec<_>>>()
                    .context("Invalid child roster row")?;
                anyhow::Ok((sequence, rows))
            }
            .await
            .map_err(|error| error.to_string());
            let _ = evt_tx.unbounded_send(BridgeEvent::SubagentsFetched {
                active_session_id: sid.clone(),
                result,
            });
        },
        client.request(
            DaemonCommand::GetRlmMaxDepthStatus {
                id: None,
                active_session_id: sid.clone(),
                rest: Map::new(),
            },
            t
        ),
        client.request(
            DaemonCommand::ListKernelBash {
                id: None,
                active_session_id: sid.clone(),
                rest: Map::new(),
            },
            t
        ),
        client.request(
            DaemonCommand::ListPersistentShells {
                id: None,
                rest: Map::new(),
            },
            t
        ),
        client.request(
            DaemonCommand::HeartbeatsList {
                id: None,
                active_session_id: Some(sid.clone()),
                rest: Map::new(),
            },
            t
        ),
        client.request(
            DaemonCommand::CronList {
                id: None,
                active_session_id: Some(sid.clone()),
                include_inactive: Some(false),
                rest: Map::new(),
            },
            t
        ),
        client.request(
            DaemonCommand::GetMcpConnections {
                id: None,
                active_session_id: sid.clone(),
                rest: Map::new(),
            },
            t
        ),
        client.request(
            DaemonCommand::GetSessionStats {
                id: None,
                active_session_id: sid.clone(),
                rest: Map::new(),
            },
            t
        ),
        client.request(
            DaemonCommand::GetSystemPrompt {
                id: None,
                active_session_id: sid.clone(),
                rest: Map::new(),
            },
            t
        ),
    );

    let rlm_max_depth = depth_res
        .ok()
        .filter(|r| r.success)
        .and_then(|r| r.data)
        .and_then(|d| {
            d.get("maxDepth")
                .or_else(|| d.get("effectiveMaxDepth"))
                .and_then(Value::as_u64)
        });

    let mut shells: Vec<ShellProcessRow> = Vec::new();
    if let Ok(r) = kbash_res
        && r.success
        && let Some(d) = r.data
        && let Some(arr) = d
            .get("activities")
            .or_else(|| d.get("shells"))
            .and_then(Value::as_array)
    {
        shells.extend(
            arr.iter()
                .filter_map(|v| ShellProcessRow::from_wire(v, false)),
        );
    }
    if let Ok(r) = pshell_res
        && r.success
        && let Some(d) = r.data
        && let Some(arr) = d.get("shells").and_then(Value::as_array)
    {
        shells.extend(
            arr.iter()
                .filter_map(|v| ShellProcessRow::from_wire(v, true)),
        );
    }

    let mut schedules: Vec<ScheduleRow> = Vec::new();
    if let Ok(r) = hb_res
        && r.success
        && let Some(d) = r.data
        && let Some(arr) = d
            .get("heartbeats")
            .or_else(|| d.get("jobs"))
            .and_then(Value::as_array)
    {
        schedules.extend(arr.iter().filter_map(|v| ScheduleRow::from_wire(v, true)));
    }
    if let Ok(r) = cron_res
        && r.success
        && let Some(d) = r.data
        && let Some(arr) = d.get("jobs").and_then(Value::as_array)
    {
        schedules.extend(arr.iter().filter_map(|v| ScheduleRow::from_wire(v, false)));
    }

    let mcp = mcp_res
        .ok()
        .filter(|r| r.success)
        .and_then(|r| r.data)
        .and_then(|d| {
            d.get("connections")
                .or_else(|| d.get("servers"))
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(McpRow::from_wire).collect())
        })
        .unwrap_or_default();

    let stats = stats_res.ok().filter(|r| r.success).and_then(|r| r.data);
    let system_prompt = prompt_res
        .ok()
        .filter(|r| r.success)
        .and_then(|r| r.data)
        .and_then(|d| {
            d.get("prompt")
                .or_else(|| d.get("systemPrompt"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });

    let _ = evt_tx.unbounded_send(BridgeEvent::InspectorSnapshot {
        active_session_id: sid,
        rlm_max_depth,
        shells,
        schedules,
        mcp,
        stats,
        system_prompt,
    });
}
