//! Reactive application state models: sessions, subagent roster, runtime drawers, and modals.

pub mod harness_store;
pub mod models;
pub(crate) mod projects;
pub mod subagents;
pub mod transcript;
pub use subagents::RlmChildRow;

use serde_json::Value;

/// An unsent prompt belongs to either a new conversation in a folder or an attached session.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum DraftKey {
    Project(String),
    Session(String),
}

/// Active modal overlay in the desktop window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveModal {
    ModelPicker,
    QuickSwitcher,
    SettingsAuth,
}

/// Live active/resident session row from the daemon `list` / `roster_update` feed.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveSessionRow {
    pub agent_id: String,
    pub progress_note: Option<String>,
    pub tool_use_count: Option<u64>,
    pub parent_session_id: Option<String>,
    pub parent_session_path: Option<String>,
    pub active_session_id: String,
    pub session_id: Option<String>,
    pub name: String,
    pub cwd: String,
    pub status: String,
    pub activity: Option<String>,
    pub model_label: Option<String>,
    pub parent_active_session_id: Option<String>,
    pub rlm_depth: u64,
    pub is_streaming: bool,
    pub session_file: Option<String>,
}

impl ActiveSessionRow {
    #[must_use]
    pub fn from_wire(value: &Value) -> Option<Self> {
        let obj = value.get("summary").unwrap_or(value).as_object()?;
        let active_session_id = obj
            .get("activeSessionId")
            .or_else(|| obj.get("id"))
            .and_then(Value::as_str)?
            .to_string();
        if active_session_id.is_empty() {
            return None;
        }
        let session_id = obj
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let raw_name = obj
            .get("name")
            .or_else(|| obj.get("sessionName"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let first_msg = obj
            .get("firstMessage")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let name = if !raw_name.is_empty() {
            raw_name.to_string()
        } else if !first_msg.is_empty() {
            truncate_chars(first_msg, 52)
        } else {
            format!("Session {}", short_id(&active_session_id))
        };
        let cwd = obj
            .get("cwd")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let status = value
            .get("status")
            .or_else(|| obj.get("activity"))
            .and_then(Value::as_str)
            .unwrap_or("idle")
            .to_string();
        let activity = obj
            .get("activity")
            .or_else(|| obj.get("workingMessage"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let is_streaming = obj
            .get("isStreaming")
            .and_then(Value::as_bool)
            .unwrap_or(matches!(
                status.as_str(),
                "running" | "streaming" | "working" | "busy"
            ));
        let model_label = obj.get("model").and_then(extract_model_label);
        let parent_active_session_id = obj
            .get("parentActiveSessionId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let rlm_depth = obj.get("rlmDepth").and_then(Value::as_u64).unwrap_or(0);
        let session_file = obj
            .get("sessionFile")
            .or_else(|| obj.get("sessionPath"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let roster_id = value.get("agentId").and_then(Value::as_str).map_or_else(
            || {
                pa_types::daemon::agent_roster::roster_agent_id_for_summary(
                    value.get("summary").unwrap_or(value),
                )
            },
            str::to_owned,
        );
        let string = |field| {
            obj.get(field)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        Some(Self {
            agent_id: if roster_id.is_empty() {
                active_session_id.clone()
            } else {
                roster_id
            },
            progress_note: string("progressNote"),
            tool_use_count: obj.get("toolUseCount").and_then(Value::as_u64),
            parent_session_id: string("parentSessionId"),
            parent_session_path: string("parentSessionPath"),
            active_session_id,
            session_id,
            name,
            cwd,
            status,
            activity,
            model_label,
            parent_active_session_id,
            rlm_depth,
            is_streaming,
            session_file,
        })
    }
}

/// Persisted session entry on disk (`list_saved_sessions`).
#[derive(Debug, Clone, PartialEq)]
pub struct SavedSessionRow {
    pub path: String,
    pub id: String,
    pub name: String,
    pub cwd: String,
    pub created: String,
    pub modified: String,
    pub message_count: u64,
    pub first_message: String,
    pub rlm_depth: u64,
    pub model_label: Option<String>,
}

impl SavedSessionRow {
    #[must_use]
    pub fn from_wire(value: &Value) -> Option<Self> {
        let obj = value.as_object()?;
        let path = obj.get("path").and_then(Value::as_str)?.to_string();
        let id = obj
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let first_message = obj
            .get("firstMessage")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        let explicit_name = obj
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let name = if !explicit_name.is_empty() {
            explicit_name.to_string()
        } else if !first_message.is_empty() {
            truncate_chars(first_message.lines().next().unwrap_or_default(), 54)
        } else {
            format!("Conversation {}", short_id(&id))
        };
        Some(Self {
            path,
            id,
            name,
            cwd: obj
                .get("cwd")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            created: obj
                .get("created")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            modified: obj
                .get("modified")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            message_count: obj.get("messageCount").and_then(Value::as_u64).unwrap_or(0),
            first_message,
            rlm_depth: obj.get("rlmDepth").and_then(Value::as_u64).unwrap_or(0),
            model_label: obj.get("model").and_then(extract_model_label),
        })
    }
}

/// Live kernel or persistent bash shell activity row.
#[derive(Debug, Clone, PartialEq)]
pub struct ShellProcessRow {
    pub id: String,
    pub command: String,
    pub pid: Option<u64>,
    pub running: bool,
    pub persistent: bool,
    pub exit_code: Option<i64>,
    pub tail_preview: Option<String>,
}

impl ShellProcessRow {
    #[must_use]
    pub fn from_wire(value: &Value, persistent: bool) -> Option<Self> {
        let obj = value.as_object()?;
        let id = obj
            .get("activityId")
            .or_else(|| obj.get("shellId"))
            .or_else(|| obj.get("id"))
            .and_then(Value::as_str)?
            .to_string();
        Some(Self {
            id,
            command: obj
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("(shell)")
                .to_string(),
            pid: obj.get("pid").and_then(Value::as_u64),
            running: obj.get("running").and_then(Value::as_bool).unwrap_or(true),
            persistent,
            exit_code: obj.get("exitCode").and_then(Value::as_i64),
            tail_preview: obj
                .get("output")
                .or_else(|| obj.get("tail"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        })
    }
}

/// Recurring heartbeat or cron schedule entry.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleRow {
    pub id: String,
    pub label: String,
    pub schedule: String,
    pub prompt: String,
    pub paused: bool,
    pub is_heartbeat: bool,
}

impl ScheduleRow {
    #[must_use]
    pub fn from_wire(value: &Value, is_heartbeat: bool) -> Option<Self> {
        let obj = value.as_object()?;
        let id = obj
            .get("jobId")
            .or_else(|| obj.get("id"))
            .and_then(Value::as_str)?
            .to_string();
        let schedule = obj
            .get("interval")
            .or_else(|| obj.get("schedule"))
            .and_then(Value::as_str)
            .unwrap_or("every 5m")
            .to_string();
        let prompt = obj
            .get("instruction")
            .or_else(|| obj.get("prompt"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let label = obj
            .get("label")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map_or_else(|| truncate_chars(&prompt, 36), str::to_string);
        let status = obj
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("active");
        let paused = status == "paused"
            || obj
                .get("enabled")
                .and_then(Value::as_bool)
                .is_some_and(|enabled| !enabled);
        Some(Self {
            id,
            label,
            schedule,
            prompt,
            paused,
            is_heartbeat,
        })
    }
}

/// MCP connection status row (`get_mcp_connections`).
#[derive(Debug, Clone, PartialEq)]
pub struct McpRow {
    pub server: String,
    pub status: String,
    pub endpoint: Option<String>,
    pub tool_count: Option<u64>,
}

impl McpRow {
    #[must_use]
    pub fn from_wire(value: &Value) -> Option<Self> {
        let obj = value.as_object()?;
        let server = obj
            .get("server")
            .or_else(|| obj.get("id"))
            .or_else(|| obj.get("name"))
            .and_then(Value::as_str)?
            .to_string();
        Some(Self {
            server,
            status: obj
                .get("status")
                .or_else(|| obj.get("connectionStatus"))
                .and_then(Value::as_str)
                .unwrap_or("connected")
                .to_string(),
            endpoint: obj
                .get("endpoint")
                .or_else(|| obj.get("url"))
                .and_then(Value::as_str)
                .map(str::to_string),
            tool_count: obj.get("toolCount").and_then(Value::as_u64),
        })
    }
}

/// Token & cost telemetry summary for the active session.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionTelemetry {
    pub context_tokens: Option<u64>,
    pub context_window: Option<u64>,
    pub context_percent: Option<f64>,
    pub total_input_tokens: Option<u64>,
    pub total_output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub total_cost_usd: Option<f64>,
    pub auto_compaction: Option<bool>,
    pub auto_retry: Option<bool>,
}

impl SessionTelemetry {
    pub fn update_from_state(&mut self, state: &Value) {
        if let Some(usage) = state.get("contextUsage") {
            self.context_tokens = usage
                .get("tokens")
                .or_else(|| usage.get("usedTokens"))
                .and_then(Value::as_u64);
            self.context_window = usage
                .get("contextWindow")
                .or_else(|| usage.get("maxTokens"))
                .and_then(Value::as_u64)
                .filter(|&n| n > 0);
            self.context_percent = usage.get("percent").and_then(Value::as_f64).or_else(|| {
                Some(100.0 * self.context_tokens? as f64 / self.context_window? as f64)
            });
        }
        if let Some(value) = state
            .get("autoCompactionEnabled")
            .or_else(|| state.get("autoCompaction"))
        {
            self.auto_compaction = value.as_bool();
        }
        if let Some(value) = state
            .get("autoRetryEnabled")
            .or_else(|| state.get("autoRetry"))
        {
            self.auto_retry = value.as_bool();
        }
    }

    pub fn update_from_stats(&mut self, stats: &Value) {
        let tokens = stats.get("tokens").unwrap_or(stats);
        self.total_input_tokens = tokens.get("input").and_then(Value::as_u64);
        self.total_output_tokens = tokens.get("output").and_then(Value::as_u64);
        self.cache_read_tokens = tokens.get("cacheRead").and_then(Value::as_u64);
        self.total_cost_usd = stats
            .get("cost")
            .and_then(|cost| cost.get("total").unwrap_or(cost).as_f64());
        // Stats are a snapshot: the daemon omits contextUsage for unknown windows.
        if stats.get("contextUsage").is_none() {
            self.context_window = None;
            self.context_tokens = None;
            self.context_percent = None;
        }
        self.update_from_state(stats);
    }
}

/// Image staged in the composer for the next `Prompt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedImage {
    pub filename: String,
    pub mime_type: String,
    pub base64_data: String,
    pub byte_len: usize,
}

#[must_use]
pub fn extract_model_label(model: &Value) -> Option<String> {
    match model {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Object(map) => {
            let id = map.get("id").and_then(Value::as_str)?;
            if id.is_empty() {
                return None;
            }
            match map.get("provider").and_then(Value::as_str) {
                Some(p) if !p.is_empty() => Some(format!("{p}/{id}")),
                _ => Some(id.to_string()),
            }
        }
        _ => None,
    }
}

#[must_use]
pub fn short_id(id: &str) -> &str {
    let end = id.char_indices().nth(8).map_or(id.len(), |(idx, _)| idx);
    &id[..end]
}

#[must_use]
pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    let clean = text.trim();
    let count = clean.chars().count();
    if count <= max_chars {
        return clean.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let prefix: String = clean.chars().take(keep).collect();
    format!("{prefix}…")
}

#[must_use]
pub fn compact_path(path: &str) -> String {
    if let Some(home) = pa_types::platform::home_dir() {
        let home_str = home.to_string_lossy();
        if path == home_str {
            return "~".to_string();
        }
        let prefix = format!("{home_str}/");
        if let Some(rest) = path.strip_prefix(&prefix) {
            return format!("~/{rest}");
        }
    }
    path.to_string()
}

#[must_use]
pub fn format_tokens_compact(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        format!("{tokens}")
    }
}

#[cfg(test)]
mod tests {
    use super::SessionTelemetry;
    use serde_json::json;

    #[test]
    fn after_compaction_missing_usage_is_not_the_previous_estimate_or_zero() {
        let mut stats = SessionTelemetry::default();
        stats.update_from_stats(
            &json!({ "contextUsage": { "contextWindow": 8192, "tokens": 1024 } }),
        );
        stats.update_from_stats(&json!({ "contextUsage": { "contextWindow": 8192, "tokens": null, "percent": null }, "cost": 0.0 }));
        assert_eq!(
            stats,
            SessionTelemetry {
                context_window: Some(8192),
                total_cost_usd: Some(0.0),
                ..SessionTelemetry::default()
            }
        );
    }
}
