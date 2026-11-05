//! Prompt composer, staged attachments, command suggestions and live turn controls.

use gpui::{AnyElement, Context, FontWeight, SharedString, div, prelude::*, px};

use crate::assets::Icon;
use crate::bridge::BridgeRequest;
use crate::state::{ActiveModal, truncate_chars};
use crate::theme::{Palette, font_mono};
use crate::ui::text_input::{Down, Up};
use crate::ui::tool_cards::pill;
use crate::ui::workspace::{CompleteSlash, PrimeDesktopView};

const SLASH_SUGGESTIONS: [(&str, &str); 8] = [
    ("/compact", "Summarize & compact conversation context"),
    ("/refine", "Refine the session harness"),
    (
        "/new",
        "Start a clean session in the current project folder",
    ),
    (
        "/reload",
        "Reload skills, system prompt, and session configuration",
    ),
    ("/model", "Choose a model"),
    (
        "/btw ",
        "Ask a parallel side question without interrupting the turn",
    ),
    (
        "/export",
        "Export current session transcript to standalone HTML",
    ),
    (
        "/harness",
        "Open Continual Harness Studio (memories, skills, prompts)",
    ),
];

pub(super) fn slash_matches(draft: &str) -> Vec<(&'static str, &'static str)> {
    if !draft.starts_with('/') || draft.contains(char::is_whitespace) {
        return Vec::new();
    }
    SLASH_SUGGESTIONS
        .iter()
        .copied()
        .filter(|(command, _)| command.starts_with(draft) && *command != draft)
        .collect()
}

#[must_use]
#[allow(clippy::too_many_lines)]
pub fn render_composer(
    view: &PrimeDesktopView,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let draft = view.composer_input.read(cx).text().to_string();
    let trimmed = draft.trim();
    let is_bash_bang = trimmed.starts_with('!');
    let is_streaming = view.session.is_streaming;

    let slash_matches = slash_matches(&draft);
    let match_count = slash_matches.len();

    div()
        .when(match_count > 0, |d| {
            d.key_context("PaCompletions")
                .capture_action(cx.listener(move |view, _: &Up, _, cx| {
                    view.slash_selected_idx = view.slash_selected_idx.saturating_sub(1);
                    cx.stop_propagation();
                    cx.notify();
                }))
                .capture_action(cx.listener(move |view, _: &Down, _, cx| {
                    view.slash_selected_idx = (view.slash_selected_idx + 1).min(match_count - 1);
                    cx.stop_propagation();
                    cx.notify();
                }))
                .on_action(cx.listener(|view, _: &CompleteSlash, window, cx| {
                    view.complete_slash(window, cx);
                }))
        })
        .w_full()
        .flex()
        .justify_center()
        .child(
            div()
                .w_full()
                .max_w(px(720.0))
                .flex()
                .flex_col()
                .gap(px(8.0))
                // Queued Steering / Follow-Up Strip
                .when(!view.session.queued.is_empty(), |d| {
                    d.child(render_queued_strip(view, palette, cx))
                })
                // Slash Autocomplete Popover
                .when(!slash_matches.is_empty(), |d| {
                    d.child(
                        div()
                            .w_full()
                            .rounded_xl()
                            .border_1()
                            .border_color(palette.border_strong)
                            .bg(palette.bg_elevated)
                            .p(px(6.0))
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .children(slash_matches.into_iter().enumerate().map(
                                |(index, (cmd, desc))| {
                                    div()
                                        .id(SharedString::from(format!("slash-opt-{cmd}")))
                                        .when(index == view.slash_selected_idx, |d| {
                                            d.bg(palette.bg_selected)
                                        })
                                        .px(px(10.0))
                                        .py(px(6.0))
                                        .rounded_lg()
                                        .cursor_pointer()
                                        .hover(|s| s.bg(palette.bg_hover))
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.populate_draft(cmd, window, cx);
                                        }))
                                        .child(
                                            div()
                                                .font_family(font_mono())
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_size(px(12.5))
                                                .text_color(palette.accent)
                                                .child(cmd),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(12.0))
                                                .text_color(palette.text_secondary)
                                                .min_w_0()
                                                .truncate()
                                                .child(desc),
                                        )
                                },
                            )),
                    )
                })
                // Main Floating Composer Card
                .child(
                    div()
                        .w_full()
                        .rounded_2xl()
                        .border_1()
                        .border_color(if is_bash_bang {
                            palette.warning
                        } else {
                            palette.border_strong
                        })
                        .bg(palette.bg_elevated)
                        .shadow_sm()
                        .px(px(16.0))
                        .pt(px(16.0))
                        .pb(px(12.0))
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        // Staged images strip
                        .when(!view.staged_images.is_empty(), |d| {
                            d.child(div().flex().flex_wrap().gap(px(6.0)).children(
                                view.staged_images.iter().enumerate().map(|(idx, img)| {
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(6.0))
                                        .px(px(8.0))
                                        .py(px(3.5))
                                        .rounded_lg()
                                        .bg(palette.info_soft)
                                        .child(Icon::Image.render(12.5, palette.info))
                                        .child(
                                            div()
                                                .font_family(font_mono())
                                                .text_size(px(11.5))
                                                .text_color(palette.info)
                                                .child(truncate_chars(&img.filename, 24)),
                                        )
                                        .child(
                                            div()
                                                .id(SharedString::from(format!("rm-img-{idx}")))
                                                .cursor_pointer()
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    if idx < this.staged_images.len() {
                                                        this.staged_images.remove(idx);
                                                        cx.notify();
                                                    }
                                                }))
                                                .child(Icon::Close.render(11.0, palette.info)),
                                        )
                                }),
                            ))
                        })
                        // Multi-line GPUI Editor
                        .child(view.composer_input.clone())
                        // Bottom Control Bar
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(8.0))
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex()
                                        .flex_wrap()
                                        .items_center()
                                        .gap(px(6.0))
                                        .child(
                                            div()
                                                .id("composer-attach-img-btn")
                                                .p(px(6.0))
                                                .rounded_md()
                                                .cursor_pointer()
                                                .hover(|s| s.bg(palette.bg_hover))
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.prompt_attach_images(window, cx);
                                                }))
                                                .child(
                                                    Icon::Image
                                                        .render(17.0, palette.text_secondary),
                                                ),
                                        )
                                        .child(
                                            div().w(px(1.0)).h(px(15.0)).bg(palette.border_subtle),
                                        )
                                        .child(
                                            div()
                                                .id("composer-model-pill")
                                                .min_w_0()
                                                .px(px(7.0))
                                                .py(px(5.0))
                                                .rounded_md()
                                                .flex()
                                                .items_center()
                                                .gap(px(5.0))
                                                .cursor_pointer()
                                                .hover(|s| s.bg(palette.bg_hover))
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.open_modal(
                                                        ActiveModal::ModelPicker,
                                                        window,
                                                        cx,
                                                    );
                                                }))
                                                .child(
                                                    div()
                                                        .truncate()
                                                        .text_size(px(12.5))
                                                        .text_color(palette.text_primary)
                                                        .child(
                                                            view.session
                                                                .model_id
                                                                .as_deref()
                                                                .unwrap_or("Select model")
                                                                .to_string(),
                                                        ),
                                                )
                                                .child(
                                                    Icon::ChevronDown
                                                        .render(12.0, palette.text_muted),
                                                ),
                                        )
                                        .when(!view.session.thinking_level.is_empty(), |d| {
                                            d.child(
                                                div()
                                                    .id("composer-effort-pill")
                                                    .flex_none()
                                                    .whitespace_nowrap()
                                                    .px(px(7.0))
                                                    .py(px(5.0))
                                                    .rounded_md()
                                                    .cursor_pointer()
                                                    .hover(|s| s.bg(palette.bg_hover))
                                                    .text_size(px(12.0))
                                                    .text_color(palette.text_secondary)
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.cycle_thinking_level(cx);
                                                    }))
                                                    .child(format!(
                                                        "Effort: {}",
                                                        view.session.thinking_level
                                                    )),
                                            )
                                        })
                                        .when(is_bash_bang, |d| {
                                            d.child(
                                                div()
                                                    .id("composer-bash-prefix-btn")
                                                    .px(px(7.0))
                                                    .py(px(4.0))
                                                    .rounded_md()
                                                    .cursor_pointer()
                                                    .bg(palette.warning_soft)
                                                    .font_family(font_mono())
                                                    .text_size(px(11.5))
                                                    .text_color(palette.warning)
                                                    .on_click(cx.listener(|this, _, window, cx| {
                                                        this.toggle_bash_prefix(window, cx);
                                                    }))
                                                    .child("Shell ×"),
                                            )
                                        }),
                                )
                                // Right Controls: Context usage meter + Steer / Queue / Stop / Send
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(7.0))
                                        .when_some(view.telemetry.context_percent, |d, percent| {
                                            d.child(
                                                div()
                                                    .text_size(px(11.5))
                                                    .text_color(palette.text_muted)
                                                    .font_family(font_mono())
                                                    .flex_none()
                                                    .whitespace_nowrap()
                                                    .child(format!("{percent:.0}% context")),
                                            )
                                        })
                                        .children(if is_streaming {
                                            vec![
                                                // Steer mid-turn
                                                div()
                                                    .id("composer-steer-btn")
                                                    .px(px(9.0))
                                                    .py(px(5.0))
                                                    .rounded_lg()
                                                    .bg(palette.accent_soft)
                                                    .hover(|s| s.bg(palette.accent.opacity(0.28)))
                                                    .cursor_pointer()
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(5.0))
                                                    .on_click(cx.listener(|this, _, window, cx| {
                                                        this.submit_steer(window, cx);
                                                    }))
                                                    .child(Icon::Steer.render(12.5, palette.accent))
                                                    .child(
                                                        div()
                                                            .text_size(px(12.0))
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .text_color(palette.accent)
                                                            .child("Steer"),
                                                    )
                                                    .into_any_element(),
                                                // Queue follow-up
                                                div()
                                                    .id("composer-queue-btn")
                                                    .px(px(9.0))
                                                    .py(px(5.0))
                                                    .rounded_lg()
                                                    .border_1()
                                                    .border_color(palette.border_subtle)
                                                    .hover(|s| s.bg(palette.bg_hover))
                                                    .cursor_pointer()
                                                    .text_size(px(12.0))
                                                    .text_color(palette.text_secondary)
                                                    .on_click(cx.listener(|this, _, window, cx| {
                                                        this.submit_follow_up(window, cx);
                                                    }))
                                                    .child("Queue")
                                                    .into_any_element(),
                                                // Stop / Abort
                                                div()
                                                    .id("composer-stop-btn")
                                                    .w(px(30.0))
                                                    .h(px(30.0))
                                                    .rounded_full()
                                                    .bg(palette.danger)
                                                    .hover(|s| s.opacity(0.88))
                                                    .cursor_pointer()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .on_click(cx.listener(|this, _, _, _cx| {
                                                        if !this
                                                            .session
                                                            .active_session_id
                                                            .is_empty()
                                                        {
                                                            this.bridge.send(
                                                                BridgeRequest::AbortSession {
                                                                    active_session_id: this
                                                                        .session
                                                                        .active_session_id
                                                                        .clone(),
                                                                },
                                                            );
                                                        }
                                                    }))
                                                    .child(
                                                        Icon::Stop
                                                            .render(13.5, palette.text_on_accent),
                                                    )
                                                    .into_any_element(),
                                            ]
                                        } else {
                                            let can_send = !trimmed.is_empty()
                                                || !view.staged_images.is_empty();
                                            vec![
                                                div()
                                                    .id("composer-send-btn")
                                                    .w(px(34.0))
                                                    .h(px(34.0))
                                                    .rounded_full()
                                                    .bg(if can_send {
                                                        palette.accent
                                                    } else {
                                                        palette.bg_hover
                                                    })
                                                    .when(can_send, |d| {
                                                        d.hover(|s| s.bg(palette.accent_hover))
                                                            .cursor_pointer()
                                                    })
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .on_click(cx.listener(|this, _, window, cx| {
                                                        this.submit_composer(window, cx);
                                                    }))
                                                    .child(Icon::Send.render(
                                                        15.0,
                                                        if can_send {
                                                            palette.text_on_accent
                                                        } else {
                                                            palette.text_muted
                                                        },
                                                    ))
                                                    .into_any_element(),
                                            ]
                                        }),
                                ),
                        ),
                ),
        )
        .into_any_element()
}

fn render_queued_strip(
    view: &PrimeDesktopView,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let sid = view.session.active_session_id.clone();
    let sid_clear = sid.clone();
    let total = view.session.queued.steering.len() + view.session.queued.follow_ups.len();

    div()
        .w_full()
        .px(px(12.0))
        .py(px(7.0))
        .rounded_xl()
        .border_1()
        .border_color(palette.border_subtle)
        .bg(palette.bg_elevated)
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .child(pill(
                    &format!("Queued ({total})"),
                    palette.warning_soft,
                    palette.warning,
                ))
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(palette.text_secondary)
                        .min_w_0()
                        .truncate()
                        .child(
                            view.session
                                .queued
                                .steering
                                .first()
                                .or_else(|| view.session.queued.follow_ups.first())
                                .or(view.session.queued.preparing.as_ref())
                                .map_or_else(String::new, |s| truncate_chars(s, 54)),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .flex_none()
                .whitespace_nowrap()
                .child(
                    div()
                        .id("queue-send-now-btn")
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded_md()
                        .cursor_pointer()
                        .bg(palette.accent_soft)
                        .text_size(px(11.5))
                        .text_color(palette.accent)
                        .on_click(cx.listener(move |this, _, _, _cx| {
                            if !sid.is_empty() {
                                this.bridge.send(BridgeRequest::AbortAndSendQueued {
                                    active_session_id: sid.clone(),
                                });
                            }
                        }))
                        .child("Interrupt & send"),
                )
                .child(
                    div()
                        .id("queue-clear-btn")
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|s| s.bg(palette.bg_hover))
                        .text_size(px(11.5))
                        .text_color(palette.text_muted)
                        .on_click(cx.listener(move |this, _, _, _cx| {
                            if !sid_clear.is_empty() {
                                this.bridge.send(BridgeRequest::ClearQueue {
                                    active_session_id: sid_clear.clone(),
                                });
                            }
                        }))
                        .child("Clear"),
                ),
        )
        .into_any_element()
}
