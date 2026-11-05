//! Transient workspace toast notifications with auto-dismissal, hover-to-pause,
//! topic replacement, smart stage-aware placement, and reduced-motion support.

use std::time::{Duration, Instant};

use gpui::{AnyElement, Context, FontWeight, Task, Window, div, prelude::*, px};

use super::PrimeDesktopView;
use crate::assets::Icon;
use crate::state::truncate_chars;
use crate::theme::Palette;

const ENTER_DURATION: Duration = Duration::from_millis(150);
const EXIT_DURATION: Duration = Duration::from_millis(130);
const HOVER_RESUME_MIN: Duration = Duration::from_millis(1_800);

/// Visual severity and default lifetime for a workspace toast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Progress,
    Error,
}

impl ToastKind {
    #[must_use]
    pub const fn ttl(self) -> Duration {
        match self {
            Self::Info => Duration::from_millis(3_800),
            Self::Error => Duration::from_millis(6_500),
            Self::Progress => Duration::from_millis(8_000),
        }
    }
}

/// Logical lifecycle bucket so resolved states clear stale banners automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastTopic {
    General,
    Connection,
    SessionOpening,
    Compaction,
    Refinement,
}

/// Active toast notification model stored on [`PrimeDesktopView`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub id: u64,
    pub message: String,
    pub kind: ToastKind,
    pub topic: ToastTopic,
}

impl Toast {
    #[must_use]
    pub const fn is_error(&self) -> bool {
        matches!(self.kind, ToastKind::Error)
    }
}

#[derive(Default)]
enum ToastPhase {
    #[default]
    Idle,
    Entering {
        id: u64,
        start: Instant,
    },
    Settled {
        id: u64,
    },
    Exiting {
        snapshot: Toast,
        start: Instant,
    },
}

#[derive(Default)]
pub(super) struct ToastController {
    next_id: u64,
    timer: Option<Task<()>>,
    hovered: bool,
    shown_at: Option<Instant>,
    pending_timer_for_id: Option<u64>,
    phase: ToastPhase,
}

impl ToastController {
    fn clear_immediately(&mut self) {
        self.timer = None;
        self.hovered = false;
        self.shown_at = None;
        self.pending_timer_for_id = None;
        self.phase = ToastPhase::Idle;
    }
}

impl PrimeDesktopView {
    #[cfg(test)]
    pub(in crate::ui) fn is_toast_mounted(&self) -> bool {
        self.toast.is_some() || !matches!(self.toast_ctl.phase, ToastPhase::Idle)
    }

    /// Display an info or error toast in the general topic with auto-dismissal.
    pub fn show_toast(
        &mut self,
        message: impl Into<String>,
        is_error: bool,
        cx: &mut Context<Self>,
    ) {
        let kind = if is_error {
            ToastKind::Error
        } else {
            ToastKind::Info
        };
        self.show_topic_toast(message, kind, ToastTopic::General, cx);
    }

    /// Display a categorized toast that can also be cleared when its operation settles.
    pub fn show_topic_toast(
        &mut self,
        message: impl Into<String>,
        kind: ToastKind,
        topic: ToastTopic,
        cx: &mut Context<Self>,
    ) {
        let message = message.into();
        if message.trim().is_empty() {
            return;
        }
        self.toast_ctl.next_id = self.toast_ctl.next_id.wrapping_add(1).max(1);
        let id = self.toast_ctl.next_id;
        let now = cx.background_executor().now();
        self.toast = Some(Toast {
            id,
            message,
            kind,
            topic,
        });
        self.toast_ctl.shown_at = Some(now);
        self.toast_ctl.pending_timer_for_id = None;
        self.toast_ctl.phase = if self.prefs.reduce_motion {
            ToastPhase::Settled { id }
        } else {
            ToastPhase::Entering { id, start: now }
        };
        if self.toast_ctl.hovered {
            self.toast_ctl.timer = None;
        } else {
            self.arm_toast_timer(id, kind.ttl(), cx);
        }
        cx.notify();
    }

    /// Record an error toast from a synchronous helper that lacks a GPUI `Context`;
    /// the auto-dismiss timer is armed automatically on the next render pass.
    pub(super) fn record_toast_without_cx(&mut self, message: impl Into<String>, kind: ToastKind) {
        let message = message.into();
        if message.trim().is_empty() {
            return;
        }
        self.toast_ctl.next_id = self.toast_ctl.next_id.wrapping_add(1).max(1);
        let id = self.toast_ctl.next_id;
        self.toast = Some(Toast {
            id,
            message,
            kind,
            topic: ToastTopic::General,
        });
        self.toast_ctl.shown_at = None;
        self.toast_ctl.pending_timer_for_id = Some(id);
        self.toast_ctl.phase = ToastPhase::Settled { id };
    }

    /// Dismiss the active toast (starting a brief non-occluding exit fade unless reduced motion is on).
    pub fn dismiss_toast(&mut self, cx: &mut Context<Self>) {
        let Some(current) = self.toast.take() else {
            return;
        };
        self.toast_ctl.timer = None;
        self.toast_ctl.hovered = false;
        self.toast_ctl.shown_at = None;
        self.toast_ctl.pending_timer_for_id = None;
        self.toast_ctl.phase = if self.prefs.reduce_motion {
            ToastPhase::Idle
        } else {
            ToastPhase::Exiting {
                snapshot: current,
                start: cx.background_executor().now(),
            }
        };
        cx.notify();
    }

    /// Dismiss the active toast only if it belongs to `topic`.
    pub fn clear_toast_topic(&mut self, topic: ToastTopic, cx: &mut Context<Self>) {
        if self.toast.as_ref().is_some_and(|t| t.topic == topic) {
            self.dismiss_toast(cx);
        }
    }

    pub(super) fn set_toast_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if self.toast_ctl.hovered == hovered {
            return;
        }
        self.toast_ctl.hovered = hovered;
        let Some(toast) = self.toast.as_ref() else {
            self.toast_ctl.timer = None;
            return;
        };
        let id = toast.id;
        let ttl = toast.kind.ttl();
        if hovered {
            self.toast_ctl.timer = None;
        } else {
            let now = cx.background_executor().now();
            let elapsed = self
                .toast_ctl
                .shown_at
                .map_or(Duration::ZERO, |start| now.saturating_duration_since(start));
            let remaining = ttl.saturating_sub(elapsed).max(HOVER_RESUME_MIN);
            self.arm_toast_timer(id, remaining, cx);
        }
        cx.notify();
    }

    fn arm_toast_timer(&mut self, id: u64, duration: Duration, cx: &mut Context<Self>) {
        let delay = cx.background_executor().timer(duration);
        self.toast_ctl.timer = Some(cx.spawn(async move |this, cx| {
            delay.await;
            let _ = this.update(cx, |view, cx| {
                if view.toast.as_ref().is_some_and(|t| t.id == id) && !view.toast_ctl.hovered {
                    view.dismiss_toast(cx);
                }
            });
        }));
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn render_toast_overlay(
        &mut self,
        window: &mut Window,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let now = cx.background_executor().now();

        if let Some(toast) = self.toast.as_ref()
            && self.toast_ctl.pending_timer_for_id == Some(toast.id)
        {
            let id = toast.id;
            let ttl = toast.kind.ttl();
            self.toast_ctl.pending_timer_for_id = None;
            self.toast_ctl.shown_at = Some(now);
            if !self.prefs.reduce_motion {
                self.toast_ctl.phase = ToastPhase::Entering { id, start: now };
            }
            if !self.toast_ctl.hovered {
                self.arm_toast_timer(id, ttl, cx);
            }
        }

        let (display_toast, opacity, slide_px, interactive) = if let Some(active) = &self.toast {
            let id = active.id;
            let (alpha, offset) = if self.prefs.reduce_motion {
                self.toast_ctl.phase = ToastPhase::Settled { id };
                (1.0, 0.0)
            } else {
                match self.toast_ctl.phase {
                    ToastPhase::Entering {
                        id: phase_id,
                        start,
                    } if phase_id == id => {
                        let elapsed = now.saturating_duration_since(start);
                        if elapsed < ENTER_DURATION {
                            window.request_animation_frame();
                            let t = elapsed.as_secs_f32() / ENTER_DURATION.as_secs_f32();
                            let eased = 1.0 - (1.0 - t).powi(3);
                            (eased, 6.0 * (1.0 - eased))
                        } else {
                            self.toast_ctl.phase = ToastPhase::Settled { id };
                            (1.0, 0.0)
                        }
                    }
                    ToastPhase::Settled { id: phase_id } if phase_id == id => (1.0, 0.0),
                    _ => {
                        self.toast_ctl.phase = ToastPhase::Entering { id, start: now };
                        window.request_animation_frame();
                        (0.05, 6.0)
                    }
                }
            };
            (active.clone(), alpha, offset, true)
        } else if !self.prefs.reduce_motion
            && let ToastPhase::Exiting { snapshot, start } = &self.toast_ctl.phase
        {
            let elapsed = now.saturating_duration_since(*start);
            if elapsed < EXIT_DURATION {
                window.request_animation_frame();
                let t = elapsed.as_secs_f32() / EXIT_DURATION.as_secs_f32();
                let eased = (1.0 - t).powi(2);
                (snapshot.clone(), eased, 4.0 * t, false)
            } else {
                self.toast_ctl.clear_immediately();
                return None;
            }
        } else {
            self.toast_ctl.clear_immediately();
            return None;
        };

        let viewport = window.viewport_size();
        let modal_open = self.active_modal.is_some();
        let inspector_docked = !modal_open
            && self.prefs.inspector_open
            && viewport.width >= px(super::panel_motion::INSPECTOR_BREAKPOINT);
        let right_offset = if inspector_docked {
            px(crate::ui::inspector::INSPECTOR_WIDTH + 18.0)
        } else {
            px(18.0)
        };
        let base_top = if self.session.side_question.open {
            px(106.0)
        } else {
            px(58.0)
        };
        let max_offset_y = (viewport.height - px(68.0)).max(px(12.0));
        let top_offset = (base_top - px(slide_px)).max(px(12.0)).min(max_offset_y);
        let bottom_offset = (px(18.0) - px(slide_px)).max(px(10.0)).min(max_offset_y);
        let max_width = (viewport.width - right_offset - px(18.0))
            .min(px(396.0))
            .max(px(160.0));

        let is_err = display_toast.is_error();
        let (icon, icon_color) = match display_toast.kind {
            ToastKind::Error => (Icon::Shield, palette.danger),
            ToastKind::Progress => (Icon::Sparkles, palette.warning),
            ToastKind::Info => (Icon::Check, palette.accent),
        };
        let border_color = if is_err {
            palette.danger.opacity(0.65 * opacity)
        } else {
            palette.border_strong.opacity(opacity)
        };
        let text_color = if is_err {
            palette.danger.opacity(opacity)
        } else {
            palette.text_primary.opacity(opacity)
        };

        let banner = div()
            .id("toast-banner")
            .debug_selector(|| "toast-banner".to_owned())
            .when(interactive, gpui::InteractiveElement::occlude)
            .min_w_0()
            .absolute()
            .when(modal_open, |d| d.bottom(bottom_offset))
            .when(!modal_open, |d| d.top(top_offset))
            .right(right_offset)
            .max_w(max_width)
            .pl(px(12.0))
            .pr(px(8.0))
            .py(px(8.5))
            .rounded_xl()
            .border_1()
            .border_color(border_color)
            .bg(palette.bg_elevated.opacity(opacity))
            .shadow_lg()
            .flex()
            .items_start()
            .gap(px(8.5))
            .when(interactive, |d| {
                d.cursor_pointer()
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        this.set_toast_hovered(*hovered, cx);
                    }))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.dismiss_toast(cx);
                    }))
            })
            .child(
                div()
                    .flex_none()
                    .pt(px(2.0))
                    .child(icon.render(14.0, icon_color.opacity(opacity))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.5))
                    .line_height(px(18.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(text_color)
                    .child(truncate_chars(&display_toast.message, 240)),
            )
            .child(
                div()
                    .id("toast-close-btn")
                    .debug_selector(|| "toast-close-btn".to_owned())
                    .flex_none()
                    .p(px(4.0))
                    .rounded_md()
                    .when(interactive, |d| {
                        d.cursor_pointer()
                            .hover(|s| s.bg(palette.bg_hover.opacity(opacity)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.dismiss_toast(cx);
                            }))
                    })
                    .child(Icon::Close.render(12.0, palette.text_muted.opacity(opacity))),
            );

        Some(banner.into_any_element())
    }
}
