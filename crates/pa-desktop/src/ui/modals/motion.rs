//! A short, one-shot entrance shared by the workspace modals.

use std::time::{Duration, Instant};

use gpui::{App, Div, Hsla, IntoElement, Stateful, Window, prelude::*, px};

const DURATION: Duration = Duration::from_millis(140);

#[derive(IntoElement)]
pub(super) struct ModalEntrance {
    pub(super) entrance_id: &'static str,
    pub(super) backdrop: Stateful<Div>,
    pub(super) sheet: Stateful<Div>,
    pub(super) scrim: Hsla,
    pub(super) reduce_motion: bool,
}

enum Phase {
    Opening(Instant),
    Settled,
}

impl RenderOnce for ModalEntrance {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let now = cx.background_executor().now();
        let progress = window.with_global_id(self.entrance_id.into(), |id, window| {
            // Once settled, stay there for the lifetime of this modal. Changing a setting
            // or typing into its search field must not replay the entrance.
            window.with_element_state(id, |state: Option<Phase>, window| {
                if !self.reduce_motion
                    && let Phase::Opening(start) = state.unwrap_or(Phase::Opening(now))
                {
                    let elapsed = now.duration_since(start);
                    if elapsed < DURATION {
                        window.request_animation_frame();
                        let t = elapsed.as_secs_f32() / DURATION.as_secs_f32();
                        return (1.0 - (1.0 - t).powi(3), Phase::Opening(start));
                    }
                }
                (1.0, Phase::Settled)
            })
        });

        self.backdrop
            .bg(self.scrim.opacity(progress))
            .child(self.sheet.relative().top(px(6.0 * (1.0 - progress))))
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Context, TestAppContext, VisualTestContext, div};

    use super::*;

    struct Modal {
        reduce: bool,
        id: &'static str,
    }

    impl Render for Modal {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            ModalEntrance {
                entrance_id: self.id,
                backdrop: div().id("backdrop").p(px(12.0)),
                sheet: div()
                    .id("sheet")
                    .debug_selector(|| "sheet".into())
                    .size(px(60.0)),
                scrim: gpui::black(),
                reduce_motion: self.reduce,
            }
        }
    }

    fn top(cx: &mut VisualTestContext) -> gpui::Pixels {
        cx.update(|window, cx| window.draw(cx).clear());
        cx.debug_bounds("sheet").unwrap().top()
    }

    #[test]
    fn entrance_continues_on_redraw_and_reduced_motion_opens_in_place() {
        let mut app = TestAppContext::single();
        let (modal, cx) = app.add_window_view(|_, _| Modal {
            reduce: false,
            id: "first",
        });
        let initial = top(cx);
        cx.executor().advance_clock(Duration::from_millis(70));
        let middle = top(cx);
        cx.executor().advance_clock(Duration::from_millis(70));
        let settled = top(cx);
        assert!(initial > middle && middle >= settled);
        assert_eq!(initial - settled, px(6.0));

        modal.update_in(cx, |modal, _, cx| {
            modal.id = "second";
            modal.reduce = true;
            cx.notify();
        });
        assert_eq!(top(cx), settled);
        modal.update_in(cx, |modal, _, cx| {
            modal.reduce = false;
            cx.notify();
        });
        assert_eq!(top(cx), settled);
        app.quit();
    }
}
