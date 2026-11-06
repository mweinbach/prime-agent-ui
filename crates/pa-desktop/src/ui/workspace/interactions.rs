//! Editor, keyboard and panel actions.
use super::layout::{INSPECTOR_BREAKPOINT, SIDEBAR_BREAKPOINT};
use super::{ActiveModal, BridgeRequest, HarnessScope, InspectorTab, PrimeDesktopView, TextEdit};
use crate::ui::composer::slash_matches;

impl PrimeDesktopView {
    pub(in crate::ui) fn focus_input(&mut self, target: fn(&mut Self) -> &mut TextEdit) {
        for input in [
            &mut self.composer_input,
            &mut self.sidebar_search_input,
            &mut self.modal_search_input,
            &mut self.rename_input,
            &mut self.side_question_input,
            &mut self.agent_mail_input,
            &mut self.refine_input,
            &mut self.harness_title_input,
            &mut self.harness_body_input,
            &mut self.quick_bash_input,
            &mut self.schedule_interval_input,
            &mut self.schedule_prompt_input,
            &mut self.mcp_server_input,
            &mut self.mcp_token_input,
            &mut self.api_key_input,
        ] {
            input.focused = false;
        }
        target(self).focused = true;
    }

    pub(super) fn focus_active_surface(&mut self) {
        match self.active_modal {
            Some(ActiveModal::QuickSwitcher | ActiveModal::ModelPicker) => {
                self.focus_input(|s| &mut s.modal_search_input);
            }
            Some(ActiveModal::SettingsAuth) => self.focus_input(|s| &mut s.api_key_input),
            None => self.focus_input(|s| &mut s.composer_input),
        }
    }

    pub(in crate::ui) fn toggle_reduce_motion(&mut self) {
        self.prefs.reduce_motion = !self.prefs.reduce_motion;
        self.save_preferences();
    }

    pub(in crate::ui) fn composer_edited(&mut self) {
        self.slash_selected_idx = 0;
        self.history_cursor = None;
    }

    pub(super) fn record_history(&mut self, entry: &str) {
        let entry = entry.trim();
        if !entry.is_empty() && self.history.last().is_none_or(|last| last != entry) {
            if self.history.len() == 80 {
                self.history.remove(0);
            }
            self.history.push(entry.into());
        }
        self.history_cursor = None;
    }

    pub(in crate::ui) fn composer_history(&mut self, older: bool) -> bool {
        let last = self.history.len();
        let Some(next) = (match self.history_cursor {
            None if older => last.checked_sub(1),
            Some(i) if older => i.checked_sub(1),
            Some(i) if i < last => Some(i + 1),
            _ => None,
        }) else {
            return false;
        };
        if self.history_cursor.is_none() {
            self.history_draft.clone_from(&self.composer_input.text);
        }
        let text = self.history.get(next).unwrap_or(&self.history_draft);
        self.composer_input.replace_text(text);
        self.history_cursor = (next < last).then_some(next);
        true
    }

    pub(in crate::ui) fn complete_slash(&mut self) -> bool {
        let matches = slash_matches(self.composer_input.text.as_str());
        let Some(&(command, _)) = matches.get(self.slash_selected_idx) else {
            return false;
        };
        self.populate_draft(command);
        true
    }

    pub(in crate::ui) fn close_mail(&mut self) {
        self.agent_mail_target = None;
        self.focus_input(|s| &mut s.composer_input);
    }

    pub(in crate::ui) fn submit_agent_mail(&mut self) {
        let Some(target) = &self.agent_mail_target else {
            return;
        };
        let message = self.agent_mail_input.text.trim();
        if message.is_empty() || self.session.active_session_id.is_empty() {
            return;
        }
        self.bridge.send(BridgeRequest::SendAgentMessage {
            from_active_session_id: Some(self.session.active_session_id.clone()),
            target_active_session_id: target.clone(),
            message: message.to_owned(),
        });
        self.agent_mail_input.replace_text("");
        self.close_mail();
    }

    pub(in crate::ui) fn submit_refinement(&mut self) {
        let instructions = self.refine_input.text.trim();
        let global = self.harness_scope == HarnessScope::Global;
        if self.request_refinement(
            (!instructions.is_empty()).then(|| instructions.to_owned()),
            global,
        ) {
            self.refine_input.replace_text("");
        }
    }

    pub(super) fn request_refinement(
        &mut self,
        instructions: Option<String>,
        global: bool,
    ) -> bool {
        if self.session.active_session_id.is_empty() {
            self.toast = Some(("Open a session before starting a refinement.".into(), true));

            return false;
        }
        self.bridge.send(BridgeRequest::Refine {
            active_session_id: self.session.active_session_id.clone(),
            instructions,
            rollback_id: None,
            global,
        });
        self.toast = Some(("Running Continual Harness refinement…".into(), false));

        true
    }

    pub(in crate::ui) fn close_harness_editor(&mut self) {
        self.harness_editor_open = false;
        self.harness_editing_id = None;
        self.focus_input(|s| &mut s.composer_input);
    }

    pub(in crate::ui) fn submit_quick_bash(&mut self) {
        let command = self.quick_bash_input.text.trim();
        if command.is_empty() || self.session.active_session_id.is_empty() {
            return;
        }
        self.bridge.send(BridgeRequest::ExecuteBash {
            active_session_id: self.session.active_session_id.clone(),
            command: command.to_owned(),
        });
        self.quick_bash_input.replace_text("");
    }

    pub(in crate::ui) fn submit_heartbeat(&mut self) {
        let schedule = self.schedule_interval_input.text.trim();
        let prompt = self.schedule_prompt_input.text.trim();
        if schedule.is_empty() || prompt.is_empty() || self.session.active_session_id.is_empty() {
            return;
        }
        self.bridge.send(BridgeRequest::CreateHeartbeat {
            active_session_id: self.session.active_session_id.clone(),
            schedule: schedule.to_owned(),
            prompt: prompt.to_owned(),
        });
        self.schedule_prompt_input.replace_text("");
    }

    pub(in crate::ui) fn submit_mcp_token(&mut self) {
        let server = self.mcp_server_input.text.trim();
        let token = self.mcp_token_input.text.trim();
        if server.is_empty() || token.is_empty() || self.session.active_session_id.is_empty() {
            return;
        }
        self.bridge.send(BridgeRequest::SetMcpToken {
            active_session_id: self.session.active_session_id.clone(),
            server: server.to_owned(),
            token: token.to_owned(),
        });
        self.mcp_server_input.replace_text("");
        self.mcp_token_input.replace_text("");
    }

    pub fn dismiss_overlay(&mut self) -> bool {
        let width = self.viewport_width;
        if self.confirm.take().is_some() {
            return true;
        }
        if self.reasoning_picker_open {
            self.close_reasoning_picker();
        } else if self.active_modal.is_some() {
            self.close_modal();
        } else if self.prefs.inspector_open && width < INSPECTOR_BREAKPOINT {
            self.toggle_inspector();
        } else if self.prefs.sidebar_open && width < SIDEBAR_BREAKPOINT {
            self.toggle_sidebar();
        } else if self.renaming_session {
            self.renaming_session = false;
            self.focus_input(|s| &mut s.composer_input);
        } else if self.agent_mail_target.is_some() {
            self.close_mail();
        } else if self.harness_editor_open {
            self.close_harness_editor();
        } else if self.session.side_question.open {
            self.toggle_side_question();
        } else if self.session.is_streaming && !self.session.active_session_id.is_empty() {
            self.bridge.send(BridgeRequest::AbortSession {
                active_session_id: self.session.active_session_id.clone(),
            });
        } else if self.sidebar_search_input.focused {
            if self.sidebar_search_input.text.is_empty() {
                self.focus_input(|s| &mut s.composer_input);
            } else {
                self.sidebar_search_input.replace_text("");
            }
        } else {
            return false;
        }
        true
    }

    pub(crate) fn close_sidebar_overlay(&mut self) {
        if self.viewport_width < SIDEBAR_BREAKPOINT && self.prefs.sidebar_open {
            self.toggle_sidebar();
        }
    }

    pub fn toggle_sidebar(&mut self) {
        self.prefs.sidebar_open = !self.prefs.sidebar_open;
        if self.prefs.sidebar_open && self.viewport_width < SIDEBAR_BREAKPOINT {
            self.prefs.inspector_open = false;
        }
        self.focus_input(|s| &mut s.composer_input);
        self.save_preferences();
    }

    pub fn toggle_inspector(&mut self) {
        self.prefs.inspector_open = !self.prefs.inspector_open;
        if self.prefs.inspector_open && self.viewport_width < SIDEBAR_BREAKPOINT {
            self.prefs.sidebar_open = false;
        }
        self.focus_input(|s| &mut s.composer_input);
        self.save_preferences();
    }

    pub fn set_inspector_tab(&mut self, tab: InspectorTab) {
        self.prefs.inspector_open = true;
        if self.viewport_width < SIDEBAR_BREAKPOINT {
            self.prefs.sidebar_open = false;
        }
        self.prefs.inspector_tab = tab;
        self.save_preferences();
        if tab == InspectorTab::Harness || tab == InspectorTab::System {
            self.reload_local_stores();
        }
        self.focus_input(|s| &mut s.composer_input);
    }
}
