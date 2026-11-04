//! The attached session's direct children. Live changes can overtake a roster request.

use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct RlmChildRow {
    pub rlm_child_id: String,
    pub active_session_id: Option<String>,
    pub name: String,
    pub label: Option<String>,
    pub status: String,
    pub model: Option<String>,
    pub answer_preview: Option<String>,
    pub error: Option<String>,
}

impl RlmChildRow {
    #[must_use]
    pub fn from_wire(value: &Value) -> Option<Self> {
        let text = |field| {
            value
                .get(field)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        };
        let id = text("id")?;
        Some(Self {
            rlm_child_id: id.to_owned(),
            active_session_id: text("activeSessionId").map(str::to_owned),
            name: text("sessionName").unwrap_or(id).to_owned(),
            label: text("label").map(str::to_owned),
            status: text("status").unwrap_or("unknown").to_owned(),
            model: value.get("model").and_then(super::extract_model_label),
            answer_preview: text("answerPreview").map(str::to_owned),
            error: text("error").map(str::to_owned),
        })
    }
}

#[derive(Default)]
pub struct Subagents {
    pub rows: Vec<RlmChildRow>,
    pub loaded: bool,
    pub load_error: Option<String>,
    snapshot_sequence: u64,
    updates: HashMap<String, u64>,
}

impl Subagents {
    pub(crate) fn update(&mut self, row: RlmChildRow, sequence: u64) {
        if sequence <= self.snapshot_sequence
            || self
                .updates
                .get(&row.rlm_child_id)
                .is_some_and(|seq| *seq >= sequence)
        {
            return;
        }
        self.updates.insert(row.rlm_child_id.clone(), sequence);
        if let Some(existing) = self
            .rows
            .iter_mut()
            .find(|r| r.rlm_child_id == row.rlm_child_id)
        {
            *existing = row;
        } else {
            self.rows.push(row);
        }
    }

    pub(crate) fn apply_snapshot(&mut self, mut rows: Vec<RlmChildRow>, sequence: u64) {
        if self.loaded && sequence < self.snapshot_sequence {
            return;
        }
        self.updates.retain(|_, seq| *seq > sequence);
        // A child may have spawned or settled while the snapshot was in flight.
        rows.retain(|row| !self.updates.contains_key(&row.rlm_child_id));
        rows.extend(
            self.rows
                .drain(..)
                .filter(|row| self.updates.contains_key(&row.rlm_child_id)),
        );
        self.rows = rows;
        self.loaded = true;
        self.load_error = None;
        self.snapshot_sequence = sequence;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_spawn_and_a_completion_overtaking_a_snapshot_are_preserved() {
        let row = |id, status| RlmChildRow::from_wire(&json!({"id":id, "status":status})).unwrap();
        let mut agents = Subagents::default();
        agents.update(row("new", "running"), 12);
        agents.update(row("first", "done"), 13);
        agents.apply_snapshot(vec![row("first", "running"), row("other", "cancelled")], 11);
        assert_eq!(
            agents.rows,
            [
                row("other", "cancelled"),
                row("new", "running"),
                row("first", "done")
            ]
        );
        agents.apply_snapshot(vec![row("first", "done")], 20);
        agents.update(row("other", "running"), 19);
        assert_eq!(agents.rows, [row("first", "done")]);
    }
}
