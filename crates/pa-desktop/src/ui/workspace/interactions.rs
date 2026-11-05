//! Input submission, transient editors, and workspace keyboard interactions.

mod viewport;

use gpui::{Context, Focusable as _, Window, px};
use pa_core::refinement::HarnessScope;

use super::{DismissOverlay, PrimeDesktopView};
use crate::bridge::BridgeRequest;
use crate::theme::{font_mono, font_sans};
use crate::ui::composer::slash_matches;
use crate::ui::text_input::{TextChanged, TextInput, TextSubmitted};
use crate::ui::workspace::panel_motion::{INSPECTOR_BREAKPOINT, SIDEBAR_BREAKPOINT};

impl PrimeDesktopView {
    pub(super) fn focus_active_surface(&self, window: &mut Window, cx: &Context<Self>) {
        use crate::state::ActiveModal;
        let handle = match self.active_modal {
            Some(ActiveModal::ModelPicker | ActiveModal::QuickSwitcher) => {
                self.modal_search_input.focus_handle(cx)
            }
            Some(ActiveModal::SettingsAuth)
                if self.provider_auth.iter().any(|item| {
                    item.can_edit_api_key && item.provider_id == self.selected_auth_provider
                }) =>
            {
                self.api_key_input.focus_handle(cx)
            }
            // The credential field is conditional. The motion control is always mounted.
            Some(ActiveModal::SettingsAuth) => self.settings_motion_focus.clone(),
            None => self.composer_input.focus_handle(cx),
        };
        window.focus(&handle);
    }

    pub(in crate::ui) fn toggle_reduce_motion(&mut self, cx: &mut Context<Self>) {
        self.prefs.reduce_motion = !self.prefs.reduce_motion;
        self.save_preferences();
        cx.notify();
    }

    pub(in crate::ui) fn tab_settings(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let stroke = &event.keystroke;
        if stroke.key != "tab"
            || !(stroke.modifiers == gpui::Modifiers::none()
                || stroke.modifiers == gpui::Modifiers::shift())
        {
            return;
        }
        if self.settings_motion_focus.is_focused(window) {
            self.focus_active_surface(window, cx);
        } else {
            window.focus(&self.settings_motion_focus);
        }
        cx.stop_propagation();
    }

    pub(super) fn bind_input_interactions(&self, window: &mut Window, cx: &mut Context<Self>) {
        type Submit = fn(&mut PrimeDesktopView, &mut Window, &mut Context<PrimeDesktopView>);
        let forms: &[(_, Submit)] = &[
            (&self.composer_input, |view, window, cx| {
                if !view.complete_slash(window, cx) {
                    view.submit_composer(window, cx);
                }
            }),
            (&self.modal_search_input, Self::submit_modal_selection),
            (&self.rename_input, |view, window, cx| {
                let name = view.rename_input.read(cx).text().to_owned();
                view.commit_rename_session(&name, window, cx);
            }),
            (&self.side_question_input, Self::submit_side_question),
            (&self.agent_mail_input, Self::submit_agent_mail),
            (&self.api_key_input, |view, _, cx| {
                // Removal has its own confirmation prompt. Enter saves a populated key.
                if !view.api_key_input.read(cx).text().trim().is_empty() {
                    view.save_selected_provider_key(cx);
                }
            }),
            (&self.refine_input, |view, _, cx| view.submit_refinement(cx)),
            (&self.quick_bash_input, |view, _, cx| {
                view.submit_quick_bash(cx);
            }),
            (&self.schedule_interval_input, |view, _, cx| {
                view.submit_heartbeat(cx);
            }),
            (&self.schedule_prompt_input, |view, _, cx| {
                view.submit_heartbeat(cx);
            }),
            (&self.mcp_server_input, |view, _, cx| {
                view.submit_mcp_token(cx);
            }),
            (&self.mcp_token_input, |view, _, cx| {
                view.submit_mcp_token(cx);
            }),
            (&self.harness_title_input, |view, window, cx| {
                window.focus(&view.harness_body_input.focus_handle(cx));
            }),
            (&self.harness_body_input, Self::save_harness_editor),
        ];
        for &(input, submit) in forms {
            cx.subscribe_in(
                input,
                window,
                move |view, _, _: &TextSubmitted, window, cx| {
                    submit(view, window, cx);
                },
            )
            .detach();
        }
        cx.on_blur(
            &self.rename_input.focus_handle(cx),
            window,
            |view, _, cx| {
                // A click into another field keeps focus there. Return explicitly commits.
                if view.renaming_session {
                    view.renaming_session = false;
                    cx.notify();
                }
            },
        )
        .detach();
        cx.subscribe(&self.composer_input, |view, input, _: &TextChanged, cx| {
            let current = input.read(cx).text().to_owned();
            input.update(cx, |input, _| {
                input.set_font_family(if input.text().starts_with('!') {
                    font_mono()
                } else {
                    font_sans()
                });
            });
            if view.dismissed_slash_draft.as_deref() != Some(current.as_str()) {
                view.dismissed_slash_draft = None;
            }
            view.slash_selected_idx = 0;
            cx.notify();
        })
        .detach();
        cx.subscribe(&self.modal_search_input, |view, _, _: &TextChanged, cx| {
            view.modal_selected_idx = 0;
            view.modal_scroll.scroll_to_item(0);
            cx.notify();
        })
        .detach();
        for input in [&self.sidebar_search_input, &self.api_key_input] {
            cx.subscribe(input, |_, _, _: &TextChanged, cx| cx.notify())
                .detach();
        }
    }

    pub(in crate::ui) fn visible_slash_matches(
        &self,
        cx: &gpui::App,
    ) -> Vec<(&'static str, &'static str)> {
        let draft = self.composer_input.read(cx).text();
        if self.dismissed_slash_draft.as_deref() == Some(draft) {
            return Vec::new();
        }
        slash_matches(draft)
    }

    pub(in crate::ui) fn dismiss_slash_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.visible_slash_matches(cx).is_empty() {
            return false;
        }
        let draft = self.composer_input.read(cx).text().to_owned();
        self.dismissed_slash_draft = Some(draft);
        self.slash_selected_idx = 0;
        cx.notify();
        true
    }

    pub(in crate::ui) fn complete_slash(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let matches = self.visible_slash_matches(cx);
        let idx = self.slash_selected_idx.min(matches.len().saturating_sub(1));
        let Some(&(command, _)) = matches.get(idx) else {
            return false;
        };
        self.dismissed_slash_draft = None;
        self.populate_draft(command, window, cx);
        true
    }

    pub(in crate::ui) fn close_mail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.agent_mail_target = None;
        window.focus(&self.composer_input.focus_handle(cx));
        cx.notify();
    }

    pub(in crate::ui) fn submit_agent_mail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = &self.agent_mail_target else {
            return;
        };
        let message = self.agent_mail_input.read(cx).text().trim();
        if message.is_empty() || self.session.active_session_id.is_empty() {
            return;
        }
        self.bridge.send(BridgeRequest::SendAgentMessage {
            from_active_session_id: Some(self.session.active_session_id.clone()),
            target_active_session_id: target.clone(),
            message: message.to_owned(),
        });
        self.agent_mail_input.update(cx, TextInput::clear);
        self.close_mail(window, cx);
    }

    pub(in crate::ui) fn submit_refinement(&mut self, cx: &mut Context<Self>) {
        let instructions = self.refine_input.read(cx).text().trim();
        let global = self.harness_scope == HarnessScope::Global;
        if self.request_refinement(
            (!instructions.is_empty()).then(|| instructions.to_owned()),
            global,
            cx,
        ) {
            self.refine_input.update(cx, TextInput::clear);
        }
    }

    pub(super) fn request_refinement(
        &mut self,
        instructions: Option<String>,
        global: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.session.active_session_id.is_empty() {
            self.show_toast("Open a session before starting a refinement.", true, cx);
            return false;
        }
        self.bridge.send(BridgeRequest::Refine {
            active_session_id: self.session.active_session_id.clone(),
            instructions,
            rollback_id: None,
            global,
        });
        self.show_topic_toast(
            "Running Continual Harness refinement…",
            super::ToastKind::Progress,
            super::ToastTopic::Refinement,
            cx,
        );
        true
    }

    pub(in crate::ui) fn close_harness_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.harness_editor_open = false;
        self.harness_editing_id = None;
        window.focus(&self.composer_input.focus_handle(cx));
        cx.notify();
    }

    pub(in crate::ui) fn submit_quick_bash(&mut self, cx: &mut Context<Self>) {
        let command = self.quick_bash_input.read(cx).text().trim();
        if command.is_empty() || self.session.active_session_id.is_empty() {
            return;
        }
        self.bridge.send(BridgeRequest::ExecuteBash {
            active_session_id: self.session.active_session_id.clone(),
            command: command.to_owned(),
        });
        self.quick_bash_input.update(cx, TextInput::clear);
    }

    pub(in crate::ui) fn submit_heartbeat(&mut self, cx: &mut Context<Self>) {
        let schedule = self.schedule_interval_input.read(cx).text().trim();
        let prompt = self.schedule_prompt_input.read(cx).text().trim();
        if schedule.is_empty() || prompt.is_empty() || self.session.active_session_id.is_empty() {
            return;
        }
        self.bridge.send(BridgeRequest::CreateHeartbeat {
            active_session_id: self.session.active_session_id.clone(),
            schedule: schedule.to_owned(),
            prompt: prompt.to_owned(),
        });
        self.schedule_prompt_input.update(cx, TextInput::clear);
    }

    pub(in crate::ui) fn submit_mcp_token(&mut self, cx: &mut Context<Self>) {
        let server = self.mcp_server_input.read(cx).text().trim();
        let token = self.mcp_token_input.read(cx).text().trim();
        if server.is_empty() || token.is_empty() || self.session.active_session_id.is_empty() {
            return;
        }
        self.bridge.send(BridgeRequest::SetMcpToken {
            active_session_id: self.session.active_session_id.clone(),
            server: server.to_owned(),
            token: token.to_owned(),
        });
        self.mcp_server_input.update(cx, TextInput::clear);
        self.mcp_token_input.update(cx, TextInput::clear);
    }

    pub(super) fn dismiss_overlay(
        &mut self,
        _: &DismissOverlay,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let width = window.viewport_size().width;
        if self.active_modal.is_some() {
            self.close_modal(window, cx);
            return;
        }
        if self.renaming_session {
            self.renaming_session = false;
            window.focus(&self.composer_input.focus_handle(cx));
            cx.notify();
            return;
        }
        if self.composer_input.focus_handle(cx).is_focused(window) && self.dismiss_slash_menu(cx) {
            return;
        }
        if self.prefs.inspector_open && width < px(INSPECTOR_BREAKPOINT) {
            self.toggle_inspector(window, cx);
            return;
        }
        if self.prefs.sidebar_open && width < px(SIDEBAR_BREAKPOINT) {
            self.toggle_sidebar(window, cx);
            return;
        }
        if self.dismiss_slash_menu(cx) {
            return;
        }
        if self.agent_mail_target.is_some() {
            self.close_mail(window, cx);
            return;
        }
        if self.harness_editor_open {
            self.close_harness_editor(window, cx);
            return;
        }
        if self.session.side_question.open {
            self.toggle_side_question(window, cx);
            return;
        }
        if self.session.is_streaming && !self.session.active_session_id.is_empty() {
            self.bridge.send(BridgeRequest::AbortSession {
                active_session_id: self.session.active_session_id.clone(),
            });
            return;
        }
        if self
            .sidebar_search_input
            .focus_handle(cx)
            .is_focused(window)
        {
            if self.sidebar_search_input.read(cx).text().is_empty() {
                window.focus(&self.composer_input.focus_handle(cx));
            } else {
                self.sidebar_search_input.update(cx, TextInput::clear);
            }
            return;
        }
        if self.toast.is_some() {
            self.dismiss_toast(cx);
        }
    }

    pub fn cycle_thinking_level(&mut self, cx: &mut Context<Self>) {
        if self.session.active_session_id.is_empty() {
            return;
        }
        let model = self.catalog_models.iter().find(|model| {
            self.session.model_provider.as_deref() == Some(&model.provider)
                && self.session.model_id.as_deref() == Some(&model.id)
        });
        if let Some(level) =
            model.and_then(|model| model.next_thinking_level(&self.session.thinking_level))
        {
            self.bridge.send(BridgeRequest::SetThinkingLevel {
                active_session_id: self.session.active_session_id.clone(),
                level: level.to_owned(),
            });
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ActiveModal, models::parse_model_catalog};
    use crate::ui::text_input::bind_text_input_keys;
    use crate::ui::workspace::{LaunchOptions, register_workspace_bindings_and_menus};
    use gpui::{Entity, TestAppContext, VisualTestContext};
    use serde_json::json;
    use tempfile::TempDir;

    fn workspace<'a>(
        app: &'a mut TestAppContext,
        profile: &TempDir,
    ) -> (Entity<PrimeDesktopView>, &'a mut VisualTestContext) {
        app.dispatcher.allow_parking();
        app.update(|cx| {
            bind_text_input_keys(cx);
            register_workspace_bindings_and_menus(cx);
        });
        let (view, cx) = app.add_window_view(|window, cx| {
            PrimeDesktopView::new(
                LaunchOptions {
                    daemon_socket: Some(profile.path().join("isolated.sock")),
                    initial_cwd: Some(profile.path().to_owned()),
                    ..Default::default()
                },
                profile.path().to_owned(),
                window,
                cx,
            )
        });
        // The failed attach resets editors and session state. Start the test once it has landed.
        cx.executor()
            .block_test(cx.condition(&view, |view, _| view.toast.is_some()));
        (view, cx)
    }

    #[test]
    fn reduce_motion_toggles_from_settings_and_survives_relaunch() {
        let profile = TempDir::new().unwrap();
        // A preference file from before Reduce motion retains its other values.
        std::fs::write(
            profile.path().join("desktop-ui.json"),
            r#"{"sidebarOpen":false}"#,
        )
        .unwrap();
        let mut app = TestAppContext::single();
        let (view, cx) = workspace(&mut app, &profile);
        view.update_in(cx, |view, _, cx| view.reload_local_stores(cx));
        cx.executor()
            .block_test(cx.condition(&view, |view, _| !view.provider_auth.is_empty()));
        view.update_in(cx, |view, window, cx| {
            assert!(!view.prefs.reduce_motion && !view.prefs.sidebar_open);
            view.open_settings_modal(window, cx);
            assert!(view.api_key_input.focus_handle(cx).is_focused(window));
        });
        cx.update(|window, cx| window.draw(cx).clear());
        cx.simulate_keystrokes("tab");
        cx.update(|window, cx| window.draw(cx).clear());
        view.update_in(cx, |view, window, _| {
            assert!(view.settings_motion_focus.is_focused(window));
        });
        cx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("space").unwrap(),
        });
        let saved = crate::theme::DesktopPreferences::load(profile.path());
        assert!(saved.reduce_motion && !saved.sidebar_open);
        cx.simulate_keystrokes("shift-tab");
        view.update_in(cx, |view, window, cx| {
            assert!(view.api_key_input.focus_handle(cx).is_focused(window));
        });
        cx.update(|window, cx| window.draw(cx).clear());
        let bounds = cx
            .debug_bounds("reduce-motion")
            .expect("motion control is visible");
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        assert!(!crate::theme::DesktopPreferences::load(profile.path()).reduce_motion);
        app.quit();
    }

    #[test]
    fn search_and_completion_keys_act_on_the_highlighted_row() {
        let profile = TempDir::new().unwrap();
        let mut app = TestAppContext::single();
        let (view, cx) = workspace(&mut app, &profile);
        view.update_in(cx, |view, window, cx| {
            view.populate_draft("/", window, cx);
        });
        cx.update(|window, cx| window.draw(cx).clear());
        cx.simulate_keystrokes("down down up tab");
        view.read_with(cx, |view, cx| {
            assert_eq!(view.composer_input.read(cx).text(), "/refine");
        });
        view.update_in(cx, |view, window, cx| {
            view.populate_draft("/mo", window, cx);
        });
        cx.update(|window, cx| window.draw(cx).clear());
        cx.simulate_keystrokes("enter");
        view.read_with(cx, |view, cx| {
            assert_eq!(view.composer_input.read(cx).text(), "/model");
        });

        view.update_in(cx, |view, window, cx| {
            view.catalog_models = parse_model_catalog(&json!({"models": [
                {"provider":"example","id":"one"}, {"provider":"example","id":"two"}
            ]}))
            .0;
            view.open_modal(ActiveModal::QuickSwitcher, window, cx);
        });
        cx.update(|window, cx| window.draw(cx).clear());
        cx.simulate_keystrokes("down enter"); // Switch model
        view.read_with(cx, |view, _| {
            assert_eq!(view.active_modal, Some(ActiveModal::ModelPicker));
        });
        cx.update(|window, cx| window.draw(cx).clear());
        cx.simulate_keystrokes("down");
        view.read_with(cx, |view, _| assert_eq!(view.modal_selected_idx, 1));
        cx.simulate_keystrokes("t");
        view.read_with(cx, |view, _| assert_eq!(view.modal_selected_idx, 0));
        cx.simulate_keystrokes("enter");
        view.update_in(cx, |view, window, cx| {
            assert_eq!(view.active_modal, None);
            assert!(view.composer_input.focus_handle(cx).is_focused(window));
        });
        app.quit();
    }

    #[test]
    fn editor_return_advances_fields_and_modified_return_saves() {
        let profile = TempDir::new().unwrap();
        let mut app = TestAppContext::single();
        let (view, cx) = workspace(&mut app, &profile);
        view.update_in(cx, |view, window, cx| {
            view.open_harness_tab(window, cx);
            view.open_new_harness_editor(window, cx);
        });
        cx.update(|window, cx| window.draw(cx).clear());
        cx.simulate_keystrokes("n o t e enter");
        view.update_in(cx, |view, window, cx| {
            assert!(view.harness_body_input.focus_handle(cx).is_focused(window));
        });
        cx.update(|window, cx| window.draw(cx).clear());
        cx.simulate_keystrokes("a enter b");
        view.read_with(cx, |view, cx| {
            assert_eq!(view.harness_body_input.read(cx).text(), "a\nb");
        });
        cx.simulate_keystrokes("secondary-enter");
        view.update_in(cx, |view, window, cx| {
            assert!(!view.harness_editor_open);
            assert!(view.composer_input.focus_handle(cx).is_focused(window));
        });
        app.quit();
    }

    #[test]
    fn escape_dismisses_editors_in_order_during_a_running_turn() {
        let profile = TempDir::new().unwrap();
        let mut app = TestAppContext::single();
        let (view, cx) = workspace(&mut app, &profile);
        view.update_in(cx, |view, window, cx| {
            view.prefs.inspector_open = false;
            view.prefs.sidebar_open = false;
            view.session.active_session_id = "running".into();
            view.session.is_streaming = true;
            view.session.side_question.open = true;
            view.harness_editor_open = true;
            view.agent_mail_target = Some("child".into());
            window.focus(&view.composer_input.focus_handle(cx));
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear());
        cx.simulate_keystrokes("escape");
        view.read_with(cx, |view, _| {
            assert!(view.agent_mail_target.is_none() && view.harness_editor_open);
        });
        cx.simulate_keystrokes("escape");
        view.read_with(cx, |view, _| {
            assert!(!view.harness_editor_open && view.session.side_question.open);
        });
        cx.simulate_keystrokes("escape");
        view.update_in(cx, |view, window, cx| {
            assert!(!view.session.side_question.open && view.session.is_streaming);
            assert!(view.composer_input.focus_handle(cx).is_focused(window));
        });
        app.quit();
    }

    #[test]
    fn toasts_auto_dismiss_pause_on_hover_and_close_via_x_without_covering_controls() {
        let profile = TempDir::new().unwrap();
        let mut app = TestAppContext::single();
        let (view, cx) = workspace(&mut app, &profile);
        cx.simulate_resize(gpui::size(gpui::px(1100.0), gpui::px(720.0)));
        cx.update(|window, cx| window.draw(cx).clear());

        let banner = cx.debug_bounds("toast-banner").expect("toast banner");
        let close_btn = cx.debug_bounds("toast-close-btn").expect("toast close x");
        let send_btn = cx
            .debug_bounds("composer-send-btn")
            .expect("composer send button");
        assert!(
            !banner.intersects(&send_btn),
            "toast banner must never cover composer send button"
        );
        assert!(banner.intersects(&close_btn));

        // Hovering pauses the auto-dismiss countdown.
        cx.simulate_mouse_move(banner.center(), None, gpui::Modifiers::none());
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(10));
        cx.update(|window, cx| window.draw(cx).clear());
        view.read_with(cx, |view, _| assert!(view.toast.is_some()));

        // Moving away resumes countdown and auto-dismisses cleanly (including exit fade).
        cx.simulate_mouse_move(
            gpui::point(gpui::px(24.0), gpui::px(24.0)),
            None,
            gpui::Modifiers::none(),
        );
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(8));
        cx.update(|window, cx| window.draw(cx).clear());
        view.read_with(cx, |view, _| assert!(view.toast.is_none()));
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(300));
        cx.update(|window, cx| window.draw(cx).clear());
        view.read_with(cx, |view, _| assert!(!view.is_toast_mounted()));

        // Connection topic toast clears immediately when daemon reconnects.
        view.update_in(cx, |view, _, cx| {
            let target = view.prefs.daemon_target;
            view.apply_bridge_event(
                crate::bridge::BridgeEvent::ConnectionStatus {
                    connected: false,
                    target,
                    socket_path: profile.path().join("isolated.sock"),
                    daemon_version: None,
                    error: Some("Daemon offline".into()),
                },
                cx,
            );
            assert!(view.toast.is_some());
            view.apply_bridge_event(
                crate::bridge::BridgeEvent::ConnectionStatus {
                    connected: true,
                    target,
                    socket_path: profile.path().join("isolated.sock"),
                    daemon_version: Some("0.1.0".into()),
                    error: None,
                },
                cx,
            );
        });
        cx.update(|window, cx| window.draw(cx).clear());
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(300));
        cx.update(|window, cx| window.draw(cx).clear());
        view.read_with(cx, |view, _| assert!(!view.is_toast_mounted()));

        // Explicit close `×` button dismisses immediately.
        view.update_in(cx, |view, _, cx| {
            view.show_toast("Switched model to openai-codex/aurora-alpha", false, cx);
        });
        cx.update(|window, cx| window.draw(cx).clear());
        let close_btn = cx.debug_bounds("toast-close-btn").expect("toast close x");
        cx.simulate_click(close_btn.center(), gpui::Modifiers::none());
        cx.update(|window, cx| window.draw(cx).clear());
        view.read_with(cx, |view, _| assert!(view.toast.is_none()));
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(300));
        cx.update(|window, cx| window.draw(cx).clear());
        view.read_with(cx, |view, _| assert!(!view.is_toast_mounted()));
        app.quit();
    }

    #[test]
    fn popup_close_buttons_queued_strip_and_slash_escape_handle_edge_cases_cleanly() {
        let profile = TempDir::new().unwrap();
        let mut app = TestAppContext::single();
        let (view, cx) = workspace(&mut app, &profile);
        cx.simulate_resize(gpui::size(gpui::px(1100.0), gpui::px(720.0)));

        // Dismiss initial connection toast so Escape tests slash menu directly.
        view.update_in(cx, |view, window, cx| {
            view.dismiss_toast(cx);
            view.session.active_session_id = "s-live".into();
            view.session.is_streaming = true;
            view.populate_draft("/co", window, cx);
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("slash-popover").is_some());
        view.read_with(cx, |view, cx| {
            assert_ne!(view.visible_slash_matches(cx), [] as [(&str, &str); 0]);
        });

        // Escape dismisses slash menu without clearing "/co" or aborting the running turn.
        cx.simulate_keystrokes("escape");
        cx.update(|window, cx| window.draw(cx).clear());
        view.read_with(cx, |view, cx| {
            assert_eq!(view.visible_slash_matches(cx), [] as [(&str, &str); 0]);
            assert_eq!(view.composer_input.read(cx).text(), "/co");
            assert!(view.session.is_streaming);
        });

        // Typing re-opens the slash popover; clicking its `×` closes it cleanly.
        cx.simulate_keystrokes("m");
        cx.update(|window, cx| window.draw(cx).clear());
        view.read_with(cx, |view, cx| {
            assert_ne!(view.visible_slash_matches(cx), [] as [(&str, &str); 0]);
        });
        let slash_close = cx
            .debug_bounds("slash-popover-close")
            .expect("slash close button");
        cx.simulate_click(slash_close.center(), gpui::Modifiers::none());
        cx.update(|window, cx| window.draw(cx).clear());
        view.read_with(cx, |view, cx| {
            assert_eq!(view.visible_slash_matches(cx), [] as [(&str, &str); 0]);
            assert_eq!(view.composer_input.read(cx).text(), "/com");
        });

        // Preparing-only queued strip shows close `×` without phantom Interrupt/Clear buttons.
        view.update_in(cx, |view, _, cx| {
            view.session.queued.preparing = Some("Preparing follow-up…".into());
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("queued-strip").is_some());
        // `queue-send-now-btn` has never been drawn while `total == 0`.
        assert!(cx.debug_bounds("queue-send-now-btn").is_none());
        let q_close = cx.debug_bounds("queue-close-btn").expect("queue close x");
        cx.simulate_click(q_close.center(), gpui::Modifiers::none());
        cx.update(|window, cx| window.draw(cx).clear());
        view.read_with(cx, |view, _| assert!(view.session.queued.is_empty()));

        // Queued steering strip clears optimistically via Clear button.
        view.update_in(cx, |view, _, cx| {
            view.session.queued.steering = vec!["Steer now".into()];
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("queue-send-now-btn").is_some());
        let q_clear = cx.debug_bounds("queue-clear-btn").expect("queue clear");
        cx.simulate_click(q_clear.center(), gpui::Modifiers::none());
        cx.update(|window, cx| window.draw(cx).clear());
        view.read_with(cx, |view, _| assert!(view.session.queued.is_empty()));

        // Quick Switcher `×` button closes modal.
        view.update_in(cx, |view, window, cx| {
            view.open_modal(ActiveModal::QuickSwitcher, window, cx);
        });
        cx.update(|window, cx| window.draw(cx).clear());
        let close_sw = cx
            .debug_bounds("close-switcher-modal")
            .expect("switcher close x");
        cx.simulate_click(close_sw.center(), gpui::Modifiers::none());
        view.read_with(cx, |view, _| assert!(view.active_modal.is_none()));

        // Harness editor `×` button closes inline editor card.
        view.update_in(cx, |view, window, cx| {
            view.open_harness_tab(window, cx);
            view.open_new_harness_editor(window, cx);
        });
        cx.update(|window, cx| window.draw(cx).clear());
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(220));
        cx.update(|window, cx| window.draw(cx).clear());
        let close_ed = cx
            .debug_bounds("close-harness-editor-x")
            .expect("harness editor close x");
        cx.simulate_click(close_ed.center(), gpui::Modifiers::none());
        view.read_with(cx, |view, _| assert!(!view.harness_editor_open));

        // Submitting a new /btw side question clears stale error state.
        view.update_in(cx, |view, window, cx| {
            view.session.side_question.open = true;
            view.session.side_question.error = Some("Previous /btw failed".into());
            view.side_question_input
                .update(cx, |input, cx| input.set_text("Fresh question?", cx));
            view.submit_side_question(window, cx);
            assert!(view.session.side_question.error.is_none());
        });
        app.quit();
    }
}
