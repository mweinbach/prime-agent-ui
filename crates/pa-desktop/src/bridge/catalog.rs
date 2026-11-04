//! Saved chats are paged by the supervisor in file modification order. The
//! terminal response is authoritative; streamed rows can omit rollup metadata.

use super::{BridgeEvent, RawClient, check_daemon_ok};
use crate::state::SavedSessionRow;
use anyhow::{Context as _, Result};
use futures::channel::mpsc::UnboundedSender;
use pa_types::daemon::DaemonCommand;
use serde_json::{Map, Value, json};
use std::time::Duration;

pub(super) async fn load_saved_page(
    client: &RawClient,
    cwd: Option<String>,
    cursor: Option<String>,
    events: &UnboundedSender<BridgeEvent>,
) -> Result<()> {
    let response = client
        .request(
            DaemonCommand::ListSavedSessions {
                id: None,
                active_session_id: None,
                // A non-session-addressed request requires a cwd even for scope=all.
                cwd: Some(cwd.unwrap_or_else(|| ".".into())),
                session_dir: None,
                scope: json!("all"),
                limit: Some(120),
                cursor: cursor.clone(),
                rest: Map::new(),
            },
            Duration::from_secs(15),
        )
        .await?;
    let data = check_daemon_ok(response)?;
    let rows = data
        .get("sessions")
        .and_then(Value::as_array)
        .context("list_saved_sessions returned no sessions array")?
        .iter()
        .filter_map(SavedSessionRow::from_wire)
        .collect();
    let next_cursor = data
        .get("nextCursor")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let _ = events.unbounded_send(BridgeEvent::SavedSessionsPage {
        cursor,
        next_cursor,
        rows,
    });
    Ok(())
}
