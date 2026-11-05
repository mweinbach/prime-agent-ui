//! Keep the reader's place during streaming and focus the composer when a drawer closes.
use gpui::{Context, Focusable as _, ScrollWheelEvent, Window, px};

use crate::theme::InspectorTab;
use crate::ui::workspace::PrimeDesktopView;

impl PrimeDesktopView {
    pub(in crate::ui::workspace) fn schedule_tail_scroll(
        &self,
        window: &mut Window,
        cx: &Context<Self>,
    ) {
        if self.follow_tail && !self.session.entries.is_empty() {
            // GPUI resolves this request against the *new* bounds during prepaint. This
            // covers the first frame after attach as well as height changes within a row.
            self.transcript_scroll.scroll_to_bottom();
            // Reconcile after layout for content that was remeasured in the same frame.
            cx.on_next_frame(window, |view, _, cx| {
                let bottom = -view.transcript_scroll.max_offset().height;
                let mut offset = view.transcript_scroll.offset();
                if view.follow_tail && offset.y != bottom {
                    offset.y = bottom;
                    view.transcript_scroll.set_offset(offset);
                    cx.notify();
                }
            });
        }
    }

    pub(crate) fn on_transcript_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(window.line_height()).y;
        if delta == px(0.0) {
            return;
        }
        // GPUI's scroll listener has applied the wheel delta in the bubble phase.
        let distance =
            self.transcript_scroll.max_offset().height + self.transcript_scroll.offset().y;
        let following = distance <= px(1.0) || (delta < px(0.0) && distance <= px(40.0));
        if self.follow_tail != following {
            self.follow_tail = following;
            cx.notify();
        }
    }

    pub(crate) fn jump_to_bottom(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.follow_tail = true;
        cx.notify();
    }

    pub(crate) fn close_sidebar_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.viewport_size().width < px(760.0) && self.prefs.sidebar_open {
            self.toggle_sidebar(window, cx);
        }
    }

    pub fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prefs.sidebar_open = !self.prefs.sidebar_open;
        if self.prefs.sidebar_open && window.viewport_size().width < px(760.0) {
            self.prefs.inspector_open = false;
        }
        window.focus(&self.composer_input.focus_handle(cx));
        self.save_preferences();
        cx.notify();
    }

    pub fn toggle_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prefs.inspector_open = !self.prefs.inspector_open;
        if self.prefs.inspector_open && window.viewport_size().width < px(760.0) {
            self.prefs.sidebar_open = false;
        }
        window.focus(&self.composer_input.focus_handle(cx));
        self.save_preferences();
        cx.notify();
    }

    pub fn set_inspector_tab(
        &mut self,
        tab: InspectorTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prefs.inspector_open = true;
        if window.viewport_size().width < px(760.0) {
            self.prefs.sidebar_open = false;
        }
        self.prefs.inspector_tab = tab;
        self.save_preferences();
        if tab == InspectorTab::Harness || tab == InspectorTab::System {
            self.reload_local_stores(cx);
        }
        window.focus(&self.composer_input.focus_handle(cx));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::BridgeEvent;
    use crate::state::transcript::TranscriptEntry;
    use crate::ui::workspace::{LaunchOptions, SessionSelection};
    use gpui::{Modifiers, ScrollDelta, TestAppContext, point};
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn attached_history_follows_until_scrolled_and_jump_resumes_following() {
        let profile = TempDir::new().unwrap();
        let mut app = TestAppContext::single();
        app.dispatcher.allow_parking();
        let (view, cx) = app.add_window_view(|window, cx| {
            PrimeDesktopView::new(
                LaunchOptions {
                    daemon_socket: Some(profile.path().join("offline.sock")),
                    initial_cwd: Some(profile.path().into()),
                    ..Default::default()
                },
                profile.path().into(),
                window,
                cx,
            )
        });
        // Drain the real bridge's initial offline status before injecting the snapshot.
        cx.executor()
            .block_test(cx.condition(&view, |view, _| view.toast.is_some()));
        view.update_in(cx, |view, _, cx| {
            view.toast = None;
            let messages = (0..60)
                .map(|i| json!({"role": "user", "content": format!("Question {i}")}))
                .collect::<Vec<_>>();
            view.selection = SessionSelection::WaitingFor("history".into());
            view.follow_tail = false;
            view.apply_bridge_event(
                BridgeEvent::SessionAttached {
                    active_session_id: "history".into(),
                    snapshot: json!({"messages": messages}),
                },
                cx,
            );
        });
        cx.update(|window, cx| window.draw(cx).clear());
        let handle = view.read_with(cx, |view, _| view.transcript_scroll.clone());
        let bottom = -handle.max_offset().height;
        assert!(bottom < px(-100.0));
        assert_eq!(handle.offset().y, bottom);

        cx.simulate_event(ScrollWheelEvent {
            position: handle.bounds().center(),
            delta: ScrollDelta::Pixels(point(px(0.0), px(180.0))),
            ..Default::default()
        });
        cx.update(|window, cx| window.draw(cx).clear());
        let reading_at = handle.offset().y;
        assert!(reading_at > bottom);
        view.update_in(cx, |view, _, cx| {
            assert!(!view.follow_tail);
            view.session.entries.push(TranscriptEntry::User {
                key: "later".into(),
                text: "A new message while reading history".into(),
                skill_name: None,
                image_count: 0,
            });
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert_eq!(handle.offset().y, reading_at);
        let jump = cx
            .debug_bounds("jump-to-bottom")
            .expect("reader can resume following");
        cx.simulate_click(jump.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(view.read_with(cx, |view, _| view.follow_tail));
        assert_eq!(handle.offset().y, -handle.max_offset().height);
        app.quit();
    }
}
