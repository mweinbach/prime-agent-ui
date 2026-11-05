//! Session transcript state: snapshot reconstruction and live `session_event` folding.

mod edit_diff;

use std::collections::HashMap;
use std::time::Instant;

use edit_diff::build_edit_arg_diff;

use pa_types::ai::is_python_tool_name;
use pa_types::goal::GoalState;
use serde_json::Value;

/// Structured reasoning trace block inside an assistant response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThinkingBlock {
    pub text: String,
    pub streaming: bool,
    pub redacted: bool,
    pub duration_ms: Option<u64>,
}

impl ThinkingBlock {
    #[must_use]
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            streaming: false,
            redacted: false,
            duration_ms: None,
        }
    }

    /// Human-readable elapsed reasoning duration once settled (omitted under 0.5s).
    #[must_use]
    pub fn formatted_duration(&self) -> Option<String> {
        let ms = self.duration_ms?;
        if ms < 500 {
            return None;
        }
        let secs = ms.saturating_add(500) / 1_000;
        let secs = secs.max(1);
        if secs < 60 {
            Some(format!("{secs}s"))
        } else {
            let mins = secs / 60;
            let rem = secs % 60;
            if rem == 0 {
                Some(format!("{mins}m"))
            } else {
                Some(format!("{mins}m {rem}s"))
            }
        }
    }

    /// Concise single-line step title or topic extracted from the trace.
    #[must_use]
    pub fn headline(&self) -> Option<String> {
        let non_empty: Vec<&str> = self
            .text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !is_fence_marker(line))
            .collect();
        if non_empty.is_empty() {
            return None;
        }
        let last_idx = non_empty.len() - 1;
        if self.streaming {
            for (idx, line) in non_empty.iter().enumerate().rev() {
                if let Some(heading) = extract_step_heading(line, idx == last_idx) {
                    return Some(crate::state::truncate_chars(&heading, 64));
                }
            }
            let tail = strip_inline_markdown(non_empty[last_idx]);
            (!tail.is_empty()).then(|| crate::state::truncate_chars(&tail, 64))
        } else {
            for line in &non_empty {
                if let Some(heading) = extract_step_heading(line, false) {
                    return Some(crate::state::truncate_chars(&heading, 64));
                }
            }
            let first = strip_inline_markdown(non_empty[0]);
            (!first.is_empty()).then(|| crate::state::truncate_chars(&first, 64))
        }
    }

    /// Compact trailing prose lines for smooth `Overview` live preview while streaming.
    #[must_use]
    pub fn live_preview_lines(&self, max_lines: usize) -> Vec<String> {
        if max_lines == 0 {
            return Vec::new();
        }
        let headline = self.headline();
        let mut collected: Vec<String> = Vec::with_capacity(max_lines);
        for raw in self.text.lines().rev() {
            let trimmed = raw.trim();
            if trimmed.is_empty() || is_fence_marker(trimmed) {
                continue;
            }
            if extract_step_heading(trimmed, true).is_some() {
                continue;
            }
            let cleaned = strip_inline_markdown(trimmed);
            if cleaned.is_empty() {
                continue;
            }
            if headline.as_deref() == Some(cleaned.as_str()) {
                continue;
            }
            collected.push(crate::state::truncate_chars(&cleaned, 112));
            if collected.len() >= max_lines {
                break;
            }
        }
        collected.reverse();
        collected
    }
}

/// One block inside an assistant response.
#[derive(Debug, Clone, PartialEq)]
pub enum AssistantBlock {
    Thinking(ThinkingBlock),
    Text(String),
}

/// Structured file diff emitted by `edit(...)` inside `python_repl` or `edit` tool calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffView {
    pub path: String,
    pub unified: String,
    pub added_lines: usize,
    pub removed_lines: usize,
}

/// Sent/queued agent-to-agent message receipt inside a `python_repl` cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentMailReceipt {
    id: String,
    pub target: String,
    pub status: String,
}

impl SentMailReceipt {
    fn from_wire(value: &Value) -> Option<Self> {
        let id = value.get("id")?.as_str()?;
        let status = value.get("deliveryStatus")?.as_str()?;
        if !matches!(status, "queued" | "delivered") {
            return None;
        }
        let target = value
            .get("target")
            .and_then(agent_endpoint_name)
            .unwrap_or("unknown agent");
        Some(Self {
            id: id.to_string(),
            target: target.to_string(),
            status: status.to_string(),
        })
    }
}

/// Parsed execution details for a `python_repl` cell.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PythonCellDetails {
    pub duration_ms: Option<f64>,
    pub status: Option<String>,
    pub stdout: String,
    pub stderr: String,
    pub return_value: Option<String>,
    pub background_output: Option<String>,
    pub error_name: Option<String>,
    pub error_value: Option<String>,
    pub traceback: Vec<String>,
    pub bash_first: Option<String>,
    pub bash_count: usize,
    pub diffs: Vec<DiffView>,
    pub sent_mail: Vec<SentMailReceipt>,
}

impl PythonCellDetails {
    fn add_sent_mail(&mut self, receipt: SentMailReceipt) {
        if !self
            .sent_mail
            .iter()
            .any(|existing| existing.id == receipt.id)
        {
            self.sent_mail.push(receipt);
        }
    }
}

/// One tool invocation card (`python_repl`, `bash`, `edit`, `attach_image`, or generic).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCard {
    pub id: String,
    pub name: String,
    pub code_or_cmd: String,
    pub args: Value,
    pub running: bool,
    pub settled: bool,
    pub is_error: bool,
    pub text_output: String,
    pub python: Option<PythonCellDetails>,
    pub edit_diff: Option<DiffView>,
    pub image_count: usize,
}

/// Direction/kind of an inter-agent or subagent lifecycle row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailKind {
    Received,
    Sent,
    ChildCompleted,
    ChildCancelled,
    ChildFailed,
}

/// Rich transcript row rendered in the conversation stage.
#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptEntry {
    User {
        key: String,
        text: String,
        skill_name: Option<String>,
        image_count: usize,
    },
    Assistant {
        key: String,
        blocks: Vec<AssistantBlock>,
        streaming: bool,
        model_label: Option<String>,
        error: Option<String>,
    },
    Tool(Box<ToolCard>),
    UserBash {
        key: String,
        command: String,
        output: String,
        running: bool,
        exit_code: Option<i64>,
        cancelled: bool,
    },
    AgentMail {
        key: String,
        kind: MailKind,
        counterpart: String,
        body: String,
    },
    Compaction {
        key: String,
        streaming: bool,
        reason: String,
        summary: String,
        tokens_before: Option<u64>,
    },
    Refinement {
        key: String,
        summary: String,
        changes: Vec<String>,
    },
    Notice {
        key: String,
        title: String,
        body: String,
        is_error: bool,
    },
}

/// Queued prompts parked behind an in-flight turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueuedLaneState {
    pub steering: Vec<String>,
    pub follow_ups: Vec<String>,
    pub preparing: Option<String>,
}

impl QueuedLaneState {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steering.is_empty() && self.follow_ups.is_empty() && self.preparing.is_none()
    }
}

/// Active `/btw` side-question thread state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SideQuestionState {
    pub open: bool,
    pub running_id: Option<String>,
    pub question: String,
    pub answer: String,
    pub error: Option<String>,
}

/// Complete live state of the currently attached session in the center stage.
#[derive(Debug, Clone, Default)]
pub struct AttachedSessionState {
    pub active_session_id: String,
    pub session_id: String,
    pub session_name: Option<String>,
    pub session_file: Option<String>,
    pub cwd: String,
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    pub thinking_level: String,
    pub is_streaming: bool,
    pub working_note: Option<String>,
    pub goal: Option<GoalState>,
    pub queued: QueuedLaneState,
    pub entries: Vec<TranscriptEntry>,
    expansion_overrides: HashMap<String, bool>,
    pub side_question: SideQuestionState,
    next_seq: u64,
    snapshot_buffer: Vec<Value>,
    thinking_started_at: Option<Instant>,
}

impl AttachedSessionState {
    #[must_use]
    pub fn fresh_in(cwd: impl Into<String>) -> Self {
        Self {
            cwd: cwd.into(),
            ..Self::default()
        }
    }

    fn mint_key(&mut self, prefix: &str) -> String {
        self.next_seq = self.next_seq.saturating_add(1);
        format!("{prefix}-{}", self.next_seq)
    }

    #[must_use]
    pub fn display_title(&self) -> String {
        if let Some(name) = self
            .session_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return name.to_string();
        }
        for entry in &self.entries {
            if let TranscriptEntry::User { text, .. } = entry {
                let first_line = text.lines().next().unwrap_or_default().trim();
                if !first_line.is_empty() {
                    return crate::state::truncate_chars(first_line, 56);
                }
            }
        }
        if self.active_session_id.is_empty() {
            "New conversation".to_string()
        } else {
            format!(
                "Session {}",
                crate::state::short_id(&self.active_session_id)
            )
        }
    }

    #[must_use]
    pub fn model_selector_label(&self) -> String {
        match (&self.model_provider, &self.model_id) {
            (Some(p), Some(id)) if !p.is_empty() && !id.is_empty() => format!("{p}/{id}"),
            (_, Some(id)) if !id.is_empty() => id.clone(),
            _ => "Select model".to_string(),
        }
    }

    pub fn set_expanded(&mut self, key: &str, open: bool) {
        self.expansion_overrides.insert(key.to_owned(), open);
    }

    #[must_use]
    pub fn explicit_expansion(&self, key: &str) -> Option<bool> {
        self.expansion_overrides.get(key).copied()
    }

    #[must_use]
    pub fn is_expanded(&self, key: &str, default_open: bool) -> bool {
        self.expansion_overrides
            .get(key)
            .copied()
            .unwrap_or(default_open)
    }

    /// Reconstruct from an `attach` / `create` response payload or `SessionAttached` snapshot.
    pub fn load_snapshot(&mut self, active_session_id: &str, snapshot: &Value) {
        if self.active_session_id != active_session_id {
            *self = Self::default();
        }
        self.active_session_id = active_session_id.to_string();
        self.entries.clear();
        self.snapshot_buffer.clear();
        self.working_note = None;
        self.thinking_started_at = None;

        if let Some(state) = snapshot.get("state") {
            self.apply_state_object(state);
        }
        if let Some(summary) = snapshot.get("summary") {
            if self.cwd.is_empty()
                && let Some(cwd) = summary.get("cwd").and_then(Value::as_str)
            {
                self.cwd = cwd.to_string();
            }
            if self.session_file.is_none()
                && let Some(sf) = summary
                    .get("sessionFile")
                    .or_else(|| summary.get("sessionPath"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
            {
                self.session_file = Some(sf.to_string());
            }
        }
        if let Some(messages) = snapshot.get("messages").and_then(Value::as_array) {
            self.rebuild_entries_from_messages(messages);
        }
    }

    pub fn begin_chunked_snapshot(&mut self, active_session_id: &str, header: &Value) {
        if self.active_session_id != active_session_id {
            *self = Self::default();
        }
        self.active_session_id = active_session_id.to_string();
        self.entries.clear();
        self.snapshot_buffer.clear();
        self.thinking_started_at = None;
        if let Some(state) = header.get("state") {
            self.apply_state_object(state);
        }
    }

    pub fn append_snapshot_chunk(&mut self, chunk: &[Value]) {
        self.snapshot_buffer.extend_from_slice(chunk);
    }

    pub fn finish_chunked_snapshot(&mut self) {
        let buffered = std::mem::take(&mut self.snapshot_buffer);
        self.rebuild_entries_from_messages(&buffered);
    }

    pub fn apply_state_object(&mut self, state: &Value) {
        if let Some(sid) = state.get("sessionId").and_then(Value::as_str) {
            self.session_id = sid.to_string();
        }
        if let Some(name) = state.get("sessionName").and_then(Value::as_str) {
            self.session_name = (!name.trim().is_empty()).then(|| name.to_string());
        }
        if let Some(sf) = state
            .get("sessionFile")
            .or_else(|| state.get("sessionPath"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            self.session_file = Some(sf.to_string());
        }
        if let Some(cwd) = state
            .get("cwd")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            self.cwd = cwd.to_string();
        }
        if let Some(model) = state.get("model") {
            match model {
                Value::String(s) if !s.is_empty() => {
                    if let Some((prov, id)) = s.split_once('/') {
                        self.model_provider = Some(prov.to_string());
                        self.model_id = Some(id.to_string());
                    } else {
                        self.model_id = Some(s.clone());
                    }
                }
                Value::Object(map) => {
                    if let Some(id) = map
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        self.model_id = Some(id.to_string());
                    }
                    if let Some(p) = map
                        .get("provider")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        self.model_provider = Some(p.to_string());
                    }
                }
                _ => {}
            }
        }
        if let Some(level) = state
            .get("thinkingLevel")
            .or_else(|| state.get("reasoningEffort"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            self.thinking_level = level.to_string();
        }
        if let Some(streaming) = state.get("isStreaming").and_then(Value::as_bool) {
            self.is_streaming = streaming;
        }
        if let Some(goal_val) = state.get("goal") {
            self.goal = if goal_val.is_null() {
                None
            } else {
                serde_json::from_value::<GoalState>(goal_val.clone()).ok()
            };
        }
        if let Some(actions) = state.get("sessionActions") {
            self.apply_queue_actions(actions);
        }
    }

    fn apply_queue_actions(&mut self, actions: &Value) {
        let lane = |key: &str| -> Vec<String> {
            actions
                .get(key)
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        };
        self.queued.steering = lane("steering");
        self.queued.follow_ups = lane("followUps");
        self.queued.preparing = actions
            .get("active")
            .filter(|a| {
                a.get("kind").and_then(Value::as_str) == Some("turn")
                    && a.get("phase").and_then(Value::as_str) == Some("preparing")
            })
            .and_then(|a| a.get("label"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
    }

    pub fn rebuild_entries_from_messages(&mut self, messages: &[Value]) {
        self.entries.clear();
        for msg in messages {
            self.push_replay_message(msg);
        }
    }

    fn push_replay_message(&mut self, message: &Value) {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match role {
            "user" => {
                if let Some(entry) = self.decode_user_message(message) {
                    self.entries.push(entry);
                }
            }
            "assistant" => {
                self.push_or_update_assistant(message, false, None, None);
            }
            "toolResult" => {
                let call_id = message
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let tool_name = message
                    .get("toolName")
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                let is_error = message
                    .get("isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                self.settle_tool_result(call_id, tool_name, message, is_error, false);
            }
            "bashExecution" => {
                let key = self.mint_key("bash");
                self.entries.push(TranscriptEntry::UserBash {
                    key,
                    command: message
                        .get("command")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    output: message
                        .get("output")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    running: false,
                    exit_code: message.get("exitCode").and_then(Value::as_i64),
                    cancelled: message
                        .get("cancelled")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                });
            }
            "compactionSummary" => {
                let key = self.mint_key("compact");
                self.entries.push(TranscriptEntry::Compaction {
                    key,
                    streaming: false,
                    reason: "compacted".to_string(),
                    summary: message
                        .get("summary")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    tokens_before: message.get("tokensBefore").and_then(Value::as_u64),
                });
            }
            "custom" => {
                if let Some(custom_entry) = self.decode_custom_message(message) {
                    self.entries.push(custom_entry);
                }
            }
            _ => {}
        }
    }

    /// Fold one live `session_event` JSON object into the active transcript.
    pub fn apply_live_event(&mut self, event: &Value) {
        self.apply_live_event_at(event, Instant::now());
    }

    /// Fold one live `session_event` JSON object using a caller-supplied timestamp.
    pub fn apply_live_event_at(&mut self, event: &Value, now: Instant) {
        let Some(event_type) = event.get("type").and_then(Value::as_str) else {
            return;
        };
        match event_type {
            "agent_start" | "turn_start" => {
                self.is_streaming = true;
                self.queued.preparing = None;
            }
            "agent_end" => {
                self.is_streaming = false;
                self.working_note = None;
                self.queued.preparing = None;
                self.finish_streaming_assistant(Some(now));
            }
            "turn_end" => {
                self.working_note = None;
                self.queued.preparing = None;
                self.finish_streaming_assistant(Some(now));
                if let Some(err) = event
                    .get("error")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    let key = self.mint_key("err");
                    self.entries.push(TranscriptEntry::Notice {
                        key,
                        title: "Turn error".to_string(),
                        body: err.to_string(),
                        is_error: true,
                    });
                }
            }
            "session_info_changed" => {
                self.session_name = event
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
            }
            "message_start" | "message_update" | "message_end" => {
                let Some(message) = event.get("message") else {
                    return;
                };
                let streaming = event_type != "message_end";
                let sub_event = event
                    .get("assistantMessageEvent")
                    .and_then(|sub| sub.get("type"))
                    .and_then(Value::as_str);
                match message.get("role").and_then(Value::as_str) {
                    Some("user") if event_type == "message_start" => {
                        self.queued.preparing = None;
                        if let Some(entry) = self.decode_user_message(message) {
                            if let TranscriptEntry::User { text, .. } = &entry {
                                let trimmed = text.trim();
                                if self
                                    .queued
                                    .steering
                                    .first()
                                    .is_some_and(|s| s.trim() == trimmed)
                                {
                                    self.queued.steering.remove(0);
                                } else if self
                                    .queued
                                    .follow_ups
                                    .first()
                                    .is_some_and(|s| s.trim() == trimmed)
                                {
                                    self.queued.follow_ups.remove(0);
                                }
                            }
                            self.entries.push(entry);
                        }
                    }
                    Some("assistant") => {
                        if event_type == "message_start" {
                            self.queued.preparing = None;
                        }
                        if streaming {
                            self.is_streaming = true;
                        }
                        self.push_or_update_assistant(message, streaming, sub_event, Some(now));
                    }
                    Some("custom") if event_type == "message_start" => {
                        if let Some(entry) = self.decode_custom_message(message) {
                            self.entries.push(entry);
                        }
                    }
                    _ => {}
                }
            }
            "tool_execution_start" => {
                self.is_streaming = true;
                self.settle_open_thinking_blocks(Some(now));
                let id = event
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let name = event
                    .get("toolName")
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                let args = event.get("args").cloned().unwrap_or(Value::Null);
                self.upsert_tool_card(id, name, &args, true);
            }
            "tool_execution_update" => {
                let id = event
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if let Some(partial) = event.get("partialResult") {
                    if partial
                        .get("details")
                        .and_then(|d| d.get("status"))
                        .and_then(Value::as_str)
                        == Some("starting")
                    {
                        self.working_note = extract_content_text(partial);
                    } else {
                        self.working_note = None;
                        self.settle_tool_result(id, "tool", partial, false, true);
                    }
                }
            }
            "tool_execution_end" => {
                self.working_note = None;
                let id = event
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let is_error = event
                    .get("isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let result = event.get("result").cloned().unwrap_or(Value::Null);
                self.settle_tool_result(id, "tool", &result, is_error, false);
            }
            "ipython_sent_agent_message" => {
                if let Some(id) = event.get("toolCallId").and_then(Value::as_str)
                    && let Some(receipt) = event.get("message").and_then(SentMailReceipt::from_wire)
                    && let Some(TranscriptEntry::Tool(card)) = self.entries.iter_mut().rev().find(
                        |entry| matches!(entry, TranscriptEntry::Tool(card) if card.id == id && is_python_tool_name(&card.name)),
                    )
                {
                    card.python.get_or_insert_with(PythonCellDetails::default).add_sent_mail(receipt);
                }
            }
            "compaction_start" => {
                let reason = event
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("manual")
                    .to_string();
                let key = self.mint_key("compact");
                self.entries.push(TranscriptEntry::Compaction {
                    key,
                    streaming: true,
                    reason,
                    summary: String::new(),
                    tokens_before: None,
                });
            }
            "compaction_summary_delta" => {
                let delta = event
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if let Some(TranscriptEntry::Compaction { summary, .. }) =
                    self.entries.iter_mut().rev().find(|e| {
                        matches!(
                            e,
                            TranscriptEntry::Compaction {
                                streaming: true,
                                ..
                            }
                        )
                    })
                {
                    summary.push_str(delta);
                }
            }
            "compaction_end" => {
                let summary_text = event
                    .get("result")
                    .and_then(|r| r.get("summary"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let tokens_before = event
                    .get("result")
                    .and_then(|r| r.get("tokensBefore"))
                    .and_then(Value::as_u64);
                if let Some(TranscriptEntry::Compaction {
                    streaming,
                    summary,
                    tokens_before: tb,
                    ..
                }) = self.entries.iter_mut().rev().find(|e| {
                    matches!(
                        e,
                        TranscriptEntry::Compaction {
                            streaming: true,
                            ..
                        }
                    )
                }) {
                    *streaming = false;
                    if !summary_text.is_empty() {
                        *summary = summary_text;
                    }
                    *tb = tokens_before;
                }
            }
            "goal_update" => {
                let g = event.get("goal").cloned().unwrap_or(Value::Null);
                self.goal = if g.is_null() {
                    None
                } else {
                    serde_json::from_value::<GoalState>(g).ok()
                };
            }
            "session_action_update" => {
                if let Some(actions) = event.get("actions") {
                    self.apply_queue_actions(actions);
                }
            }
            "bash_start" => {
                let key = self.mint_key("bash");
                self.entries.push(TranscriptEntry::UserBash {
                    key,
                    command: event
                        .get("command")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    output: String::new(),
                    running: true,
                    exit_code: None,
                    cancelled: false,
                });
            }
            "bash_output" => {
                let chunk = event
                    .get("chunk")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if let Some(TranscriptEntry::UserBash { output, .. }) = self
                    .entries
                    .iter_mut()
                    .rev()
                    .find(|e| matches!(e, TranscriptEntry::UserBash { running: true, .. }))
                {
                    output.push_str(chunk);
                }
            }
            "bash_end" => {
                let code = event.get("exitCode").and_then(Value::as_i64);
                let cancelled = event
                    .get("cancelled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if let Some(TranscriptEntry::UserBash {
                    running,
                    exit_code,
                    cancelled: c,
                    ..
                }) = self
                    .entries
                    .iter_mut()
                    .rev()
                    .find(|e| matches!(e, TranscriptEntry::UserBash { running: true, .. }))
                {
                    *running = false;
                    *exit_code = code;
                    *c = cancelled;
                }
            }
            "auto_retry_start" => {
                let attempt = event.get("attempt").and_then(Value::as_u64).unwrap_or(1);
                let max = event
                    .get("maxAttempts")
                    .and_then(Value::as_u64)
                    .unwrap_or(3);
                let msg = event
                    .get("errorMessage")
                    .and_then(Value::as_str)
                    .unwrap_or("transient provider error");
                self.working_note = Some(format!("Retrying ({attempt}/{max}): {msg}"));
            }
            "auto_retry_end" => {
                self.working_note = None;
            }
            _ => {}
        }
    }

    pub fn apply_side_question_event(&mut self, event: &Value) {
        let Some(id) = event.get("id").and_then(Value::as_str) else {
            return;
        };
        if self.side_question.running_id.as_deref() != Some(id) {
            return;
        }
        if let Some(answer) = event.get("answer").and_then(Value::as_str) {
            answer.clone_into(&mut self.side_question.answer);
        }
        if event.get("status").and_then(Value::as_str) != Some("running") {
            self.side_question.running_id = None;
            self.side_question.error = event
                .get("errorMessage")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
    }

    fn take_elapsed_thinking_ms(&mut self, now: Option<Instant>) -> Option<u64> {
        let started = self.thinking_started_at.take()?;
        let now_ts = now?;
        let ms = u64::try_from(now_ts.saturating_duration_since(started).as_millis())
            .unwrap_or(u64::MAX);
        (ms >= 500).then_some(ms)
    }

    fn settle_open_thinking_blocks(&mut self, now: Option<Instant>) {
        let elapsed = self.take_elapsed_thinking_ms(now);
        for entry in self.entries.iter_mut().rev() {
            if let TranscriptEntry::Assistant { blocks, .. } = entry {
                for block in blocks.iter_mut() {
                    if let AssistantBlock::Thinking(tb) = block
                        && tb.streaming
                    {
                        tb.streaming = false;
                        if tb.duration_ms.is_none() {
                            tb.duration_ms = elapsed;
                        }
                    }
                }
                blocks.retain(|b| {
                    !matches!(
                        b,
                        AssistantBlock::Thinking(tb) if tb.text.trim().is_empty() && !tb.redacted
                    )
                });
            }
        }
        self.entries.retain(|entry| {
            !matches!(
                entry,
                TranscriptEntry::Assistant { blocks, error: None, .. } if blocks.is_empty()
            )
        });
    }

    fn finish_streaming_assistant(&mut self, now: Option<Instant>) {
        self.settle_open_thinking_blocks(now);
        for entry in self.entries.iter_mut().rev() {
            if let TranscriptEntry::Assistant { streaming, .. } = entry
                && *streaming
            {
                *streaming = false;
            }
        }
    }

    fn decode_user_message(&mut self, message: &Value) -> Option<TranscriptEntry> {
        let mut text_parts: Vec<String> = Vec::new();
        let mut image_count = 0usize;

        match message.get("content") {
            Some(Value::String(s)) => text_parts.push(s.clone()),
            Some(Value::Array(blocks)) => {
                for block in blocks {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") | None => {
                            if let Some(t) = block.get("text").and_then(Value::as_str) {
                                text_parts.push(t.to_string());
                            }
                        }
                        Some("image" | "image_url") => {
                            image_count += 1;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }

        let raw_text = text_parts.join("\n");
        let trimmed = raw_text.trim();
        if trimmed.starts_with("[harness-digest]") {
            // Persisted cold-start digest context is harness plumbing; surfaced via the Harness Studio tab.
            return None;
        }
        if trimmed.is_empty() && image_count == 0 {
            return None;
        }

        let (skill_name, clean_text) = extract_skill_tag(trimmed);
        let key = self.mint_key("user");
        Some(TranscriptEntry::User {
            key,
            text: clean_text,
            skill_name,
            image_count,
        })
    }

    #[allow(clippy::too_many_lines)]
    fn push_or_update_assistant(
        &mut self,
        message: &Value,
        streaming: bool,
        sub_event: Option<&str>,
        now: Option<Instant>,
    ) {
        let mut blocks: Vec<AssistantBlock> = Vec::new();
        let mut tool_calls: Vec<(String, String, Value)> = Vec::new();
        let sub_ended = matches!(sub_event, Some("thinking_end" | "done" | "error"));

        match message.get("content") {
            Some(Value::String(s)) if !s.trim().is_empty() => {
                blocks.push(AssistantBlock::Text(s.clone()));
            }
            Some(Value::Array(items)) => {
                let len = items.len();
                for (idx, item) in items.iter().enumerate() {
                    match item.get("type").and_then(Value::as_str) {
                        Some("thinking") => {
                            let raw_thought = item
                                .get("thinking")
                                .or_else(|| item.get("text"))
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            let clean_text = raw_thought.trim_end_matches(['\n', '\r']).to_string();
                            let redacted = item
                                .get("redacted")
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            let item_streaming = streaming && (idx + 1 == len) && !sub_ended;
                            if !clean_text.trim().is_empty() || redacted || item_streaming {
                                if let Some(AssistantBlock::Thinking(prev)) = blocks.last_mut() {
                                    if !clean_text.trim().is_empty() {
                                        if !prev.text.is_empty() {
                                            prev.text.push_str("\n\n");
                                        }
                                        prev.text.push_str(&clean_text);
                                    }
                                    prev.streaming = item_streaming;
                                    prev.redacted |= redacted;
                                } else {
                                    blocks.push(AssistantBlock::Thinking(ThinkingBlock {
                                        text: clean_text,
                                        streaming: item_streaming,
                                        redacted,
                                        duration_ms: None,
                                    }));
                                }
                            }
                        }
                        Some("text") => {
                            let t = item.get("text").and_then(Value::as_str).unwrap_or_default();
                            if !t.is_empty() {
                                blocks.push(AssistantBlock::Text(t.to_string()));
                            }
                        }
                        Some("toolCall" | "tool_use") => {
                            let id = item
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                            let name = item
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                            let args = item
                                .get("arguments")
                                .or_else(|| item.get("input"))
                                .cloned()
                                .unwrap_or(Value::Null);
                            if !id.is_empty() && !name.is_empty() {
                                tool_calls.push((id, name, args));
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }

        let error = match message.get("stopReason").and_then(Value::as_str) {
            Some("error") => message
                .get("errorMessage")
                .and_then(Value::as_str)
                .map_or_else(
                    || Some("Model request failed".to_string()),
                    |e| Some(e.to_string()),
                ),
            Some("aborted") => Some("Generation stopped".to_string()),
            _ => None,
        };

        let model_label = message
            .get("model")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let last_user_idx = self
            .entries
            .iter()
            .rposition(|e| matches!(e, TranscriptEntry::User { .. }));
        let search_start = last_user_idx.map_or(0, |idx| idx + 1);
        let existing_idx = self.entries[search_start..]
            .iter()
            .rposition(|e| {
                matches!(
                    e,
                    TranscriptEntry::Assistant {
                        streaming: true,
                        ..
                    }
                )
            })
            .map(|rel| search_start + rel);

        if let Some(idx) = existing_idx {
            type PreservedThinking = Vec<(usize, ThinkingBlock)>;
            type PrevThinkingMeta = Vec<(bool, Option<u64>)>;
            let (preserved_thinking, prev_thinking_meta): (PreservedThinking, PrevThinkingMeta) =
                if let TranscriptEntry::Assistant {
                    blocks: existing_blocks,
                    ..
                } = &self.entries[idx]
                {
                    let preserved = existing_blocks
                        .iter()
                        .enumerate()
                        .filter_map(|(pos, block)| match block {
                            AssistantBlock::Thinking(tb)
                                if !tb.text.trim().is_empty() || tb.redacted =>
                            {
                                let mut settled = tb.clone();
                                settled.streaming = false;
                                Some((pos, settled))
                            }
                            _ => None,
                        })
                        .collect();
                    let meta = existing_blocks
                        .iter()
                        .filter_map(|block| match block {
                            AssistantBlock::Thinking(tb) => Some((tb.streaming, tb.duration_ms)),
                            AssistantBlock::Text(_) => None,
                        })
                        .collect();
                    (preserved, meta)
                } else {
                    (Vec::new(), Vec::new())
                };

            // Some providers finish with only the answer or only tool calls after streaming a
            // separate trace. Keep that trace at its original position so disclosures stay stable.
            if !blocks
                .iter()
                .any(|block| matches!(block, AssistantBlock::Thinking(_)))
                && (!streaming
                    || !tool_calls.is_empty()
                    || blocks
                        .iter()
                        .any(|block| matches!(block, AssistantBlock::Text(_))))
            {
                for (pos, tb) in preserved_thinking {
                    blocks.insert(pos.min(blocks.len()), AssistantBlock::Thinking(tb));
                }
            }

            let mut think_ord = 0usize;
            for block in &mut blocks {
                if let AssistantBlock::Thinking(curr) = block {
                    let (was_streaming, prev_dur) = prev_thinking_meta
                        .get(think_ord)
                        .copied()
                        .unwrap_or((false, None));
                    think_ord += 1;
                    if curr.duration_ms.is_none() {
                        curr.duration_ms = prev_dur;
                    }
                    if curr.streaming {
                        if self.thinking_started_at.is_none() {
                            self.thinking_started_at = now;
                        }
                    } else if curr.duration_ms.is_none()
                        && (was_streaming || self.thinking_started_at.is_some())
                    {
                        curr.duration_ms = self.take_elapsed_thinking_ms(now);
                    }
                }
            }

            if blocks.is_empty() && error.is_none() {
                if !streaming || !tool_calls.is_empty() {
                    self.thinking_started_at = None;
                    self.entries.remove(idx);
                }
            } else if let TranscriptEntry::Assistant {
                blocks: existing_blocks,
                streaming: existing_stream,
                model_label: existing_model,
                error: existing_err,
                ..
            } = &mut self.entries[idx]
            {
                *existing_blocks = blocks;
                *existing_stream = streaming;
                if model_label.is_some() {
                    *existing_model = model_label;
                }
                if error.is_some() {
                    *existing_err = error;
                }
            }
        } else if !blocks.is_empty() || error.is_some() {
            if blocks
                .iter()
                .any(|b| matches!(b, AssistantBlock::Thinking(tb) if tb.streaming))
                && self.thinking_started_at.is_none()
            {
                self.thinking_started_at = now;
            }
            let key = self.mint_key("asst");
            self.entries.push(TranscriptEntry::Assistant {
                key,
                blocks,
                streaming,
                model_label,
                error,
            });
        } else if !streaming {
            self.finish_streaming_assistant(now);
        }

        for (id, name, args) in tool_calls {
            self.upsert_tool_card(&id, &name, &args, false);
        }
    }

    fn upsert_tool_card(&mut self, id: &str, name: &str, args: &Value, mark_running: bool) {
        if id.is_empty() {
            return;
        }
        let code_or_cmd = extract_tool_primary_source(name, args);
        let edit_diff = (name == "edit")
            .then(|| build_edit_arg_diff(args))
            .flatten();

        if let Some(TranscriptEntry::Tool(card)) = self
            .entries
            .iter_mut()
            .rev()
            .find(|e| matches!(e, TranscriptEntry::Tool(c) if c.id == id && !c.settled))
        {
            if !name.is_empty() && name != "tool" {
                card.name = name.to_string();
            }
            if !args.is_null() {
                card.args = args.clone();
                if !code_or_cmd.is_empty() {
                    card.code_or_cmd = code_or_cmd;
                }
                if edit_diff.is_some() {
                    card.edit_diff = edit_diff;
                }
            }
            if mark_running {
                card.running = true;
            }
            return;
        }

        self.entries.push(TranscriptEntry::Tool(Box::new(ToolCard {
            id: id.to_string(),
            name: name.to_string(),
            code_or_cmd,
            args: args.clone(),
            running: mark_running,
            settled: false,
            is_error: false,
            text_output: String::new(),
            python: None,
            edit_diff,
            image_count: 0,
        })));
    }

    fn settle_tool_result(
        &mut self,
        id: &str,
        fallback_name: &str,
        payload: &Value,
        is_error: bool,
        partial: bool,
    ) {
        let text_out = extract_content_text(payload).unwrap_or_default();
        let image_count = count_content_images(payload);
        let details = payload.get("details").unwrap_or(&Value::Null);

        let target_idx = self
            .entries
            .iter()
            .rposition(|e| matches!(e, TranscriptEntry::Tool(c) if c.id == id && !c.settled));

        if let Some(idx) = target_idx {
            if let Some(TranscriptEntry::Tool(card)) = self.entries.get_mut(idx) {
                card.running = partial;
                card.settled = !partial;
                card.is_error = is_error;
                card.text_output = text_out;
                card.image_count = image_count;
                if is_python_tool_name(&card.name)
                    || details.get("stdout").is_some()
                    || details.get("durationMs").is_some()
                {
                    let mut python = parse_python_details(details, &card.text_output);
                    if let Some(previous) = card.python.take() {
                        for receipt in previous.sent_mail {
                            python.add_sent_mail(receipt);
                        }
                    }
                    card.python = Some(python);
                }
            }
        } else if !id.is_empty() {
            let is_py = is_python_tool_name(fallback_name)
                || details.get("stdout").is_some()
                || details.get("durationMs").is_some();
            self.entries.push(TranscriptEntry::Tool(Box::new(ToolCard {
                id: id.to_string(),
                name: fallback_name.to_string(),
                code_or_cmd: String::new(),
                args: Value::Null,
                running: partial,
                settled: !partial,
                is_error,
                python: is_py.then(|| parse_python_details(details, &text_out)),
                text_output: text_out,
                edit_diff: None,
                image_count,
            })));
        }
    }

    fn decode_custom_message(&mut self, message: &Value) -> Option<TranscriptEntry> {
        if message.get("display").and_then(Value::as_bool) == Some(false) {
            return None;
        }
        let custom_type = message
            .get("customType")
            .and_then(Value::as_str)
            .unwrap_or("notice");
        let content_str = extract_custom_text(message);
        let details = message.get("details").unwrap_or(&Value::Null);

        match custom_type {
            "agent_message" => {
                let counterpart = details
                    .get("from")
                    .and_then(agent_endpoint_name)
                    .unwrap_or("unknown agent")
                    .to_string();
                let body = details
                    .get("message")
                    .and_then(Value::as_str)
                    .map_or(content_str, str::to_string);
                let direction = details
                    .get("direction")
                    .and_then(Value::as_str)
                    .unwrap_or("received");
                let kind = if direction == "sent" || direction == "queued" {
                    MailKind::Sent
                } else {
                    MailKind::Received
                };
                let key = self.mint_key("mail");
                Some(TranscriptEntry::AgentMail {
                    key,
                    kind,
                    counterpart,
                    body,
                })
            }
            "rlm_child_terminal_notice" | "rlm_child_failure" => {
                let counterpart = ["sessionName", "childId"]
                    .into_iter()
                    .find_map(|key| {
                        details
                            .get(key)
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                    })
                    .unwrap_or("unknown")
                    .to_string();
                let kind = if custom_type == "rlm_child_failure" {
                    MailKind::ChildFailed
                } else if details.get("kind").and_then(Value::as_str) == Some("cancelled") {
                    MailKind::ChildCancelled
                } else {
                    MailKind::ChildCompleted
                };
                let key = self.mint_key("child");
                Some(TranscriptEntry::AgentMail {
                    key,
                    kind,
                    counterpart,
                    body: content_str,
                })
            }
            "refinement_outcome" => {
                let summary = details
                    .get("summary")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map_or_else(|| content_str.clone(), str::to_string);
                let changes = details
                    .get("appliedEdits")
                    .and_then(Value::as_array)
                    .map(|edits| {
                        edits
                            .iter()
                            .filter_map(|e| {
                                let act = e.get("action")?.as_str()?;
                                let k = e.get("kind")?.as_str()?;
                                let id = e.get("id")?.as_str()?;
                                Some(format!("{act} {k}:{id}"))
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let key = self.mint_key("refine");
                Some(TranscriptEntry::Refinement {
                    key,
                    summary,
                    changes,
                })
            }
            "compaction_outcome" => {
                let key = self.mint_key("compact");
                Some(TranscriptEntry::Compaction {
                    key,
                    streaming: false,
                    reason: "compacted".to_string(),
                    summary: content_str,
                    tokens_before: details.get("tokensBefore").and_then(Value::as_u64),
                })
            }
            "session_slash_command" => {
                let key = self.mint_key("slash");
                Some(TranscriptEntry::User {
                    key,
                    text: content_str,
                    skill_name: None,
                    image_count: 0,
                })
            }
            "harness_digest" => None,
            other => {
                if content_str.trim().is_empty() {
                    return None;
                }
                let key = self.mint_key("note");
                Some(TranscriptEntry::Notice {
                    key,
                    title: other.replace('_', " "),
                    body: content_str,
                    is_error: other.contains("error") || other.contains("fail"),
                })
            }
        }
    }
}

fn extract_skill_tag(text: &str) -> (Option<String>, String) {
    if let Some(rest) = text.strip_prefix("<skill")
        && let Some(close_open) = rest.find('>')
    {
        let header = &rest[..close_open];
        let skill_name = header
            .split_once("name=\"")
            .and_then(|(_, after)| after.split_once('"'))
            .map(|(n, _)| n.to_string());
        if let Some((_, trailing)) = rest.split_once("</skill>") {
            let trimmed = trailing.trim();
            return (
                skill_name.clone(),
                if trimmed.is_empty() {
                    format!("Invoked skill {}", skill_name.as_deref().unwrap_or("skill"))
                } else {
                    trimmed.to_string()
                },
            );
        }
    }
    (None, text.to_string())
}

fn agent_endpoint_name(endpoint: &Value) -> Option<&str> {
    ["sessionName", "activeSessionId", "sessionId", "clientId"]
        .into_iter()
        .find_map(|key| {
            endpoint
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
        })
}

fn extract_tool_primary_source(name: &str, args: &Value) -> String {
    let parsed_obj: Option<Value> = match args {
        Value::String(raw) => serde_json::from_str(raw).ok(),
        Value::Object(_) => Some(args.clone()),
        _ => None,
    };
    let Some(obj) = parsed_obj.as_ref().and_then(Value::as_object) else {
        return args.as_str().unwrap_or_default().to_string();
    };
    match name {
        name if is_python_tool_name(name) => obj
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        "bash" => obj
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        "edit" => obj
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => serde_json::to_string_pretty(args).unwrap_or_default(),
    }
}

fn extract_content_text(payload: &Value) -> Option<String> {
    match payload.get("content") {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Array(arr)) => {
            let joined: Vec<&str> = arr
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str).unwrap_or("text") == "text")
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect();
            let text = joined.join("\n");
            (!text.is_empty()).then_some(text)
        }
        _ => None,
    }
}

fn extract_custom_text(message: &Value) -> String {
    extract_content_text(message)
        .or_else(|| {
            message
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default()
}

fn count_content_images(payload: &Value) -> usize {
    payload
        .get("content")
        .and_then(Value::as_array)
        .map_or(0, |arr| {
            arr.iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("image"))
                .count()
        })
}

fn parse_python_details(details: &Value, fallback_text: &str) -> PythonCellDetails {
    let Some(obj) = details.as_object() else {
        return PythonCellDetails {
            stdout: fallback_text.to_string(),
            ..Default::default()
        };
    };

    let mut diffs = Vec::new();
    if let Some(raw_diffs) = obj.get("diffs").and_then(Value::as_array) {
        for d in raw_diffs {
            let path = d
                .get("path")
                .or_else(|| d.get("filePath"))
                .and_then(Value::as_str)
                .unwrap_or("file")
                .to_string();
            let unified = d
                .get("diff")
                .or_else(|| d.get("patch"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let added_lines = unified
                .lines()
                .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
                .count();
            let removed_lines = unified
                .lines()
                .filter(|l| l.starts_with('-') && !l.starts_with("---"))
                .count();
            diffs.push(DiffView {
                path,
                unified,
                added_lines,
                removed_lines,
            });
        }
    }

    let sent_mail = obj
        .get("sentAgentMessages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(SentMailReceipt::from_wire)
        .collect();

    let (bash_first, bash_count) =
        obj.get("bashCommands")
            .and_then(Value::as_object)
            .map_or((None, 0), |b| {
                (
                    b.get("first").and_then(Value::as_str).map(str::to_string),
                    b.get("count").and_then(Value::as_u64).unwrap_or(0) as usize,
                )
            });

    let err_obj = obj.get("error").and_then(Value::as_object);
    let error_name = err_obj
        .and_then(|e| e.get("ename"))
        .or_else(|| obj.get("errorEname"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let error_value = err_obj
        .and_then(|e| e.get("evalue"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let traceback = err_obj
        .and_then(|e| e.get("traceback"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let stdout = obj
        .get("stdout")
        .and_then(Value::as_str)
        .map_or_else(|| fallback_text.to_string(), str::to_string);

    PythonCellDetails {
        duration_ms: obj.get("durationMs").and_then(Value::as_f64),
        status: obj
            .get("status")
            .and_then(Value::as_str)
            .map(str::to_string),
        stdout,
        stderr: obj
            .get("stderr")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        return_value: obj
            .get("result")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && *s != "None")
            .map(str::to_string),
        background_output: obj
            .get("backgroundOutput")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        error_name,
        error_value,
        traceback,
        bash_first,
        bash_count,
        diffs,
        sent_mail,
    }
}

fn is_fence_marker(trimmed: &str) -> bool {
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

fn extract_step_heading(trimmed_line: &str, allow_unclosed_tail: bool) -> Option<String> {
    if let Some(rest) = trimmed_line.strip_prefix('#') {
        let heading = strip_inline_markdown(rest.trim_start_matches('#').trim());
        return (!heading.is_empty()).then_some(heading);
    }
    if let Some(rest) = trimmed_line.strip_prefix("**") {
        if let Some(end) = rest.find("**") {
            let title = rest[..end].trim();
            let after = rest[end + 2..].trim();
            if !title.is_empty()
                && (after.is_empty()
                    || after.starts_with(':')
                    || after.starts_with('—')
                    || after.starts_with('-'))
            {
                let cleaned = strip_inline_markdown(title);
                if !cleaned.is_empty() {
                    return Some(cleaned);
                }
            }
        } else if allow_unclosed_tail {
            let title = strip_inline_markdown(rest.trim_end_matches('*').trim());
            if !title.is_empty() {
                return Some(title);
            }
        }
    }
    None
}

fn strip_inline_markdown(line: &str) -> String {
    let mut s = line.trim();
    if s.starts_with('#') {
        s = s.trim_start_matches('#').trim_start();
    }
    while let Some(rest) = s.strip_prefix('>') {
        s = rest.trim_start();
    }
    if let Some(rest) = s
        .strip_prefix("- ")
        .or_else(|| s.strip_prefix("* "))
        .or_else(|| s.strip_prefix("+ "))
    {
        s = rest.trim_start();
    } else if let Some((digits, rest)) = s.split_once(". ")
        && !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
    {
        s = rest.trim_start();
    }

    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(ch) = rest.chars().next() {
        let ch_len = ch.len_utf8();
        if ch == '\\' {
            rest = &rest[ch_len..];
            if let Some(escaped) = rest.chars().next() {
                out.push(escaped);
                rest = &rest[escaped.len_utf8()..];
            }
            continue;
        }
        if ch == '['
            && let Some(close_bracket) = rest[1..].find("](")
        {
            let label = &rest[1..=close_bracket];
            out.push_str(&strip_inline_markdown(label));
            let after_open_paren = &rest[close_bracket + 3..];
            rest = after_open_paren
                .find(')')
                .map_or("", |close_paren| &after_open_paren[close_paren + 1..]);
            continue;
        }
        rest = &rest[ch_len..];
        if matches!(ch, '*' | '`' | '~') {
            continue;
        }
        if ch.is_whitespace() {
            if !out.ends_with(' ') && !out.is_empty() {
                out.push(' ');
            }
        } else {
            out.push(ch);
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn completed_answer_keeps_the_open_streamed_thinking_trace() {
        let mut session = AttachedSessionState::default();
        session.apply_live_event(
            &json!({"type":"message_start","message":{"role":"assistant",
            "content":[{"type":"thinking","thinking":"Reading **files**"}]}}),
        );
        let TranscriptEntry::Assistant { key, .. } = &session.entries[0] else {
            panic!()
        };
        let trace_key = format!("{key}-think-0");
        session.set_expanded(&trace_key, true);
        session.apply_live_event(&json!({"type":"message_end","message":{"role":"assistant",
            "content":[{"type":"text","text":"Found it"}]}}));
        let TranscriptEntry::Assistant {
            key,
            blocks,
            streaming,
            ..
        } = &session.entries[0]
        else {
            panic!()
        };
        assert!(!streaming);
        assert!(session.is_expanded(&format!("{key}-think-0"), false));
        assert_eq!(
            blocks,
            &[
                AssistantBlock::Thinking(ThinkingBlock::plain("Reading **files**")),
                AssistantBlock::Text("Found it".into())
            ]
        );
    }

    #[test]
    fn thinking_settles_on_text_start_with_duration_and_preserves_tool_only_traces() {
        let t0 = Instant::now();
        let mut session = AttachedSessionState::default();

        // 1. Responses-style empty ThinkingStart shows a live thinking block immediately.
        session.apply_live_event_at(
            &json!({
                "type": "message_update",
                "assistantMessageEvent": {"type": "thinking_start"},
                "message": {"role": "assistant", "content": [{"type": "thinking", "thinking": ""}]}
            }),
            t0,
        );
        let TranscriptEntry::Assistant { blocks, .. } = &session.entries[0] else {
            panic!("expected assistant entry");
        };
        assert_eq!(
            blocks,
            &[AssistantBlock::Thinking(ThinkingBlock {
                text: String::new(),
                streaming: true,
                redacted: false,
                duration_ms: None,
            })]
        );

        // 2. Streaming step headings update headline and live preview cleanly without raw `**`.
        session.apply_live_event_at(
            &json!({
                "type": "message_update",
                "assistantMessageEvent": {"type": "thinking_delta"},
                "message": {"role": "assistant", "content": [{
                    "type": "thinking",
                    "thinking": "**Inspecting workspace**\n\nChecking `stage.rs` layout.\n\n**Refining toast"
                }]}
            }),
            t0 + Duration::from_millis(900),
        );
        let TranscriptEntry::Assistant { blocks, .. } = &session.entries[0] else {
            panic!("expected assistant entry");
        };
        let AssistantBlock::Thinking(live_tb) = &blocks[0] else {
            panic!("expected thinking block");
        };
        assert!(live_tb.streaming);
        assert_eq!(live_tb.headline().as_deref(), Some("Refining toast"));
        assert_eq!(
            live_tb.live_preview_lines(2),
            vec!["Checking stage.rs layout."]
        );

        // 3. When TextStart arrives, the thinking block settles immediately (before message_end)
        //    and records elapsed duration.
        session.apply_live_event_at(
            &json!({
                "type": "message_update",
                "assistantMessageEvent": {"type": "text_start"},
                "message": {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "**Inspecting workspace**\n\nChecking `stage.rs` layout.\n\n"},
                    {"type": "text", "text": "Here "}
                ]}
            }),
            t0 + Duration::from_millis(2_700),
        );
        let TranscriptEntry::Assistant {
            blocks, streaming, ..
        } = &session.entries[0]
        else {
            panic!("expected assistant entry");
        };
        assert!(*streaming);
        let AssistantBlock::Thinking(settled_tb) = &blocks[0] else {
            panic!("expected thinking block");
        };
        assert!(!settled_tb.streaming);
        assert_eq!(settled_tb.formatted_duration().as_deref(), Some("3s"));
        assert_eq!(
            settled_tb.headline().as_deref(),
            Some("Inspecting workspace")
        );
        assert_eq!(
            settled_tb.text,
            "**Inspecting workspace**\n\nChecking `stage.rs` layout."
        );

        session.apply_live_event_at(
            &json!({
                "type": "message_end",
                "message": {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "**Inspecting workspace**\n\nChecking `stage.rs` layout.\n\n"},
                    {"type": "text", "text": "Here is the fix."}
                ]}
            }),
            t0 + Duration::from_millis(3_500),
        );

        // 4. Next step streams thinking and finishes with ONLY a toolCall (omitting thinking):
        //    the streamed thinking trace must be preserved!
        let mut tool_session = AttachedSessionState::default();
        tool_session.apply_live_event_at(
            &json!({
                "type": "message_update",
                "message": {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "**Running verification**\n\nExecuting test suite."}
                ]}
            }),
            t0,
        );
        tool_session.apply_live_event_at(
            &json!({
                "type": "message_end",
                "message": {"role": "assistant", "content": [
                    {"type": "toolCall", "id": "tc-9", "name": "python_repl", "arguments": {"code": "1 + 1"}}
                ]}
            }),
            t0 + Duration::from_millis(1_600),
        );
        assert_eq!(tool_session.entries.len(), 2);
        let TranscriptEntry::Assistant {
            blocks, streaming, ..
        } = &tool_session.entries[0]
        else {
            panic!("expected preserved assistant thinking entry");
        };
        assert!(!streaming);
        assert_eq!(
            blocks,
            &[AssistantBlock::Thinking(ThinkingBlock {
                text: "**Running verification**\n\nExecuting test suite.".into(),
                streaming: false,
                redacted: false,
                duration_ms: Some(1_600),
            })]
        );
        assert!(matches!(&tool_session.entries[1], TranscriptEntry::Tool(c) if c.id == "tc-9"));

        // 5. Silent ThinkingStart ("") followed by toolCall-only message_end leaves no ghost row.
        let mut silent_session = AttachedSessionState::default();
        silent_session.apply_live_event_at(
            &json!({
                "type": "message_update",
                "assistantMessageEvent": {"type": "thinking_start"},
                "message": {"role": "assistant", "content": [{"type": "thinking", "thinking": ""}]}
            }),
            t0,
        );
        assert_eq!(silent_session.entries.len(), 1);
        silent_session.apply_live_event_at(
            &json!({
                "type": "message_end",
                "message": {"role": "assistant", "content": [
                    {"type": "toolCall", "id": "tc-silent", "name": "python_repl", "arguments": {"code": "2 + 2"}}
                ]}
            }),
            t0 + Duration::from_millis(1_200),
        );
        assert_eq!(silent_session.entries.len(), 1);
        assert!(
            matches!(&silent_session.entries[0], TranscriptEntry::Tool(c) if c.id == "tc-silent")
        );
    }

    #[test]
    fn folds_streaming_assistant_and_python_repl_lifecycle() {
        let mut session = AttachedSessionState::default();
        session.load_snapshot(
            "sess-1",
            &json!({
                "state": {
                    "sessionId": "uuid-1",
                    "cwd": "/tmp/demo",
                    "model": {"provider": "openai-codex", "id": "gpt-5.3-codex"},
                    "thinkingLevel": "high"
                },
                "messages": []
            }),
        );
        assert_eq!(session.model_selector_label(), "openai-codex/gpt-5.3-codex");

        session.apply_live_event(&json!({
            "type": "message_start",
            "message": {"role": "user", "content": "Inspect the workspace"}
        }));
        session.apply_live_event(&json!({
            "type": "message_update",
            "message": {
                "role": "assistant",
                "content": [
                    {"type": "thinking", "thinking": "Checking files"},
                    {"type": "text", "text": "Running a quick check."},
                    {"type": "toolCall", "id": "tc-1", "name": "python_repl", "arguments": {"code": "print(42)"}}
                ]
            }
        }));
        session.apply_live_event(&json!({
            "type": "tool_execution_start",
            "toolCallId": "tc-1",
            "toolName": "python_repl",
            "args": {"code": "print(42)"}
        }));
        session.apply_live_event(&json!({
            "type": "tool_execution_end",
            "toolCallId": "tc-1",
            "isError": false,
            "result": {
                "content": [{"type": "text", "text": "42\n"}],
                "details": {
                    "durationMs": 12.5,
                    "stdout": "42\n",
                    "diffs": [{"path": "src/lib.rs", "diff": "+pub fn ok() {}\n"}]
                }
            }
        }));

        assert_eq!(session.entries.len(), 3);
        let TranscriptEntry::Tool(card) = &session.entries[2] else {
            panic!("expected tool card");
        };
        assert!(card.settled);
        assert_eq!(card.code_or_cmd, "print(42)");
        let py = card.python.as_ref().expect("python details");
        assert_eq!(py.stdout, "42\n");
        assert_eq!(py.diffs.len(), 1);
        assert_eq!(py.diffs[0].added_lines, 1);
    }

    #[test]
    fn agent_mail_uses_the_sender_endpoint_and_raw_body() {
        use pa_core::session_engine::agent_messaging::{
            AgentFamilyRelationship, AgentSessionMessageRowPayload,
            create_agent_session_message_row,
        };

        let message = create_agent_session_message_row(&AgentSessionMessageRowPayload {
            id: "spawn:sub-1",
            prompt: "[task from parent]\n\nCheck the docs.",
            message: "Check the docs.",
            from: &json!({"sessionName": "root", "activeSessionId": "worker-1"}),
            from_relationship: Some(AgentFamilyRelationship::Parent),
            target: None,
            timestamp: 1_000,
        });
        let mut session = AttachedSessionState::default();
        session.rebuild_entries_from_messages(&[message]);
        assert_eq!(
            session.entries,
            vec![TranscriptEntry::AgentMail {
                key: "mail-1".to_string(),
                kind: MailKind::Received,
                counterpart: "root".to_string(),
                body: "Check the docs.".to_string(),
            }],
        );
    }

    #[test]
    fn child_notices_distinguish_cancellation_from_completion_and_failure() {
        use pa_core::session_engine::rlm_notices::{
            RlmChildTerminalNotice, create_rlm_child_failure_message,
            create_rlm_child_terminal_notice,
        };

        let notices = [
            create_rlm_child_terminal_notice(
                &RlmChildTerminalNotice::CompletedWithoutReply {
                    child_id: "sub-1".to_string(),
                    session_name: "docs".to_string(),
                    last_assistant_text_preview: Some("Docs checked.".to_string()),
                },
                1_000,
            ),
            create_rlm_child_terminal_notice(
                &RlmChildTerminalNotice::Cancelled {
                    child_id: "sub-2".to_string(),
                    session_name: "tests".to_string(),
                    reason: Some("Deleted by parent".to_string()),
                },
                2_000,
            ),
            create_rlm_child_failure_message("sub-3", "review", "Provider failed", 3_000),
        ];
        let mut session = AttachedSessionState::default();
        for notice in notices {
            let mut message = serde_json::to_value(notice).unwrap();
            message["role"] = json!("custom");
            session.apply_live_event(&json!({"type": "message_start", "message": message}));
        }
        let outcomes: Vec<_> = session
            .entries
            .iter()
            .map(|entry| match entry {
                TranscriptEntry::AgentMail {
                    kind, counterpart, ..
                } => (*kind, counterpart.as_str()),
                other => panic!("expected child notice, got {other:?}"),
            })
            .collect();
        assert_eq!(
            outcomes,
            [
                (MailKind::ChildCompleted, "docs"),
                (MailKind::ChildCancelled, "tests"),
                (MailKind::ChildFailed, "review"),
            ],
        );
    }

    #[test]
    fn python_mail_merges_late_and_result_receipts_by_call_and_message_id() {
        let mut session = AttachedSessionState::default();
        for (id, name) in [("tc-1", "ipython"), ("tc-2", "python_repl")] {
            session.apply_live_event(&json!({
                "type": "tool_execution_start", "toolCallId": id, "toolName": name,
                "args": {"code": "await agent_message.send(...)"}
            }));
        }
        let queued = json!({
            "id": "agentmsg_1", "message": "Hold", "deliveryStatus": "queued",
            "receiverRole": "child",
            "target": {"sessionName": "docs", "activeSessionId": "worker-1", "sessionId": "s1"}
        });
        let delivered = json!({
            "id": "agentmsg_2", "message": "Proceed", "deliveryStatus": "delivered",
            "target": {"activeSessionId": "worker-2", "sessionId": "s2"}
        });
        let late_only = json!({
            "id": "agentmsg_3", "message": "Done", "deliveryStatus": "queued",
            "target": {"sessionName": "root", "activeSessionId": "worker-3", "sessionId": "s3"}
        });
        session.apply_live_event(&json!({
            "type": "ipython_sent_agent_message", "toolCallId": "tc-1", "message": queued,
        }));
        session.apply_live_event(&json!({
            "type": "tool_execution_end", "toolCallId": "tc-1", "isError": false,
            "result": {"content": [], "details": {"sentAgentMessages": [delivered]}},
        }));
        for message in [late_only, delivered, queued] {
            session.apply_live_event(&json!({
                "type": "ipython_sent_agent_message", "toolCallId": "tc-1", "message": message,
            }));
        }
        let [TranscriptEntry::Tool(first), TranscriptEntry::Tool(second)] =
            session.entries.as_slice()
        else {
            panic!("expected two tool cards");
        };
        assert!(second.python.is_none());
        let receipts: Vec<_> = first
            .python
            .as_ref()
            .expect("Python details")
            .sent_mail
            .iter()
            .map(|receipt| (receipt.target.as_str(), receipt.status.as_str()))
            .collect();
        assert_eq!(
            receipts,
            [
                ("worker-2", "delivered"),
                ("docs", "queued"),
                ("root", "queued")
            ]
        );
    }

    #[test]
    fn queued_preparing_and_steering_clear_as_live_events_progress() {
        let mut session = AttachedSessionState::default();
        session.apply_state_object(&json!({
            "sessionActions": {
                "steering": ["Focus on stage.rs"],
                "followUps": ["Run make check"],
                "active": {
                    "kind": "turn",
                    "phase": "preparing",
                    "label": "Preparing next turn…"
                }
            }
        }));
        assert_eq!(session.queued.steering, ["Focus on stage.rs"]);
        assert_eq!(session.queued.follow_ups, ["Run make check"]);
        assert_eq!(
            session.queued.preparing.as_deref(),
            Some("Preparing next turn…")
        );

        session.apply_live_event(&json!({
            "type": "message_start",
            "message": {
                "role": "user",
                "content": "Focus on stage.rs"
            }
        }));
        assert!(session.queued.preparing.is_none());
        assert_eq!(session.queued.steering, [] as [String; 0]);
        assert_eq!(session.queued.follow_ups, ["Run make check"]);

        session.queued.preparing = Some("Still preparing".to_string());
        session.apply_live_event(&json!({ "type": "agent_end" }));
        assert!(session.queued.preparing.is_none());

        session.apply_live_event(&json!({
            "type": "message_start",
            "message": {
                "role": "user",
                "content": "Run make check"
            }
        }));
        assert!(session.queued.is_empty());
    }
}
