//! Interactive transcript cards for `python_repl`, `bash`, `edit` diffs,
//! inter-agent mail, subagent outcomes, compactions, and continual-harness refinements.

use gpui::{AnyElement, ClipboardItem, Context, FontWeight, SharedString, div, prelude::*, px};
use pa_types::ai::is_python_tool_name;

use crate::assets::Icon;
use crate::state::transcript::{DiffView, MailKind, ToolCard};
use crate::state::truncate_chars;
use crate::theme::{DetailLevel, Palette, font_mono, font_sans};
use crate::ui::markdown::render_code_block;
use crate::ui::workspace::PrimeDesktopView;

/// Render a tool invocation card (`python_repl`, `bash`, `edit`, or generic).
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn render_tool_card(
    card: &ToolCard,
    detail: DetailLevel,
    expanded: bool,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let key = format!("tool-{}", card.id);
    let toggle_key = key.clone();

    let (icon, badge_label) = match card.name.as_str() {
        name if is_python_tool_name(name) => (Icon::Cpu, "Python"),
        "bash" => (Icon::Terminal, "Shell"),
        "edit" => (Icon::EditPen, "Edit"),
        "attach_image" => (Icon::Image, "Image"),
        other => (Icon::Zap, other),
    };

    let first_line = card
        .code_or_cmd
        .lines()
        .find(|l| !l.trim().is_empty())
        .map_or_else(|| card.name.clone(), |l| truncate_chars(l.trim(), 64));

    let (status_label, status_color) = if card.running {
        ("Running…".to_string(), palette.warning)
    } else if card.is_error || card.python.as_ref().is_some_and(|p| p.error_name.is_some()) {
        ("Error".to_string(), palette.danger)
    } else if let Some(ms) = card.python.as_ref().and_then(|p| p.duration_ms) {
        if ms >= 1000.0 {
            (format!("{:.2}s", ms / 1000.0), palette.success)
        } else {
            (format!("{ms:.0}ms"), palette.success)
        }
    } else if card.settled {
        ("Done".to_string(), palette.text_muted)
    } else {
        ("Streaming…".to_string(), palette.accent)
    };

    let py_bash_count = card.python.as_ref().map_or(0, |p| p.bash_count);
    let py_diff_count =
        card.python.as_ref().map_or(0, |p| p.diffs.len()) + usize::from(card.edit_diff.is_some());
    let py_mail_count = card.python.as_ref().map_or(0, |p| p.sent_mail.len());
    let copy_code = card.code_or_cmd.clone();

    div()
        .w_full()
        .rounded_lg()
        .border_1()
        .border_color(if card.is_error {
            palette.danger
        } else {
            palette.border_subtle
        })
        .bg(palette.bg_elevated)
        .overflow_hidden()
        .child(
            div()
                .id(SharedString::from(format!("{key}-hdr")))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .px(px(12.0))
                .py(px(8.0))
                .cursor_pointer()
                .hover(|s| s.bg(palette.bg_hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.session.toggle_expanded(&toggle_key);
                    cx.notify();
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .overflow_hidden()
                        .child(
                            if expanded {
                                Icon::ChevronDown
                            } else {
                                Icon::ChevronRight
                            }
                            .render(13.0, palette.text_muted),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.0))
                                .child(icon.render(13.0, palette.text_secondary))
                                .child(
                                    div()
                                        .text_size(px(11.5))
                                        .font_family(font_mono())
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(palette.text_primary)
                                        .child(badge_label.to_string()),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(12.5))
                                .font_family(font_mono())
                                .text_color(palette.text_secondary)
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(first_line),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .flex_none()
                        .when(detail != DetailLevel::Overview && py_bash_count > 0, |d| {
                            d.child(pill(
                                &format!("$ {py_bash_count}"),
                                palette.warning_soft,
                                palette.warning,
                            ))
                        })
                        .when(detail != DetailLevel::Overview && py_diff_count > 0, |d| {
                            d.child(pill(
                                &format!("± {py_diff_count}"),
                                palette.accent_soft,
                                palette.accent,
                            ))
                        })
                        .when(detail != DetailLevel::Overview && py_mail_count > 0, |d| {
                            d.child(pill(
                                &format!("↑ {py_mail_count}"),
                                palette.info_soft,
                                palette.info,
                            ))
                        })
                        .when(expanded && !copy_code.is_empty(), |d| {
                            d.child(
                                div()
                                    .id(SharedString::from(format!("{key}-copy")))
                                    .p(px(3.0))
                                    .rounded_sm()
                                    .hover(|s| s.bg(palette.bg_hover))
                                    .on_click(move |_, _, cx| {
                                        cx.stop_propagation();
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy_code.clone(),
                                        ));
                                    })
                                    .child(Icon::Copy.render(12.0, palette.text_muted)),
                            )
                        })
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_family(font_mono())
                                .text_color(status_color)
                                .child(status_label),
                        ),
                ),
        )
        .when(expanded, |container| {
            let mut body = div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .px(px(12.0))
                .pb(px(12.0))
                .pt(px(8.0))
                .border_t_1()
                .border_color(palette.border_subtle);

            if !card.code_or_cmd.is_empty() && card.name != "edit" {
                let lang = match card.name.as_str() {
                    name if is_python_tool_name(name) => "python",
                    "bash" => "bash",
                    _ => "json",
                };
                body = body.child(render_code_block(
                    &format!("{key}-src"),
                    lang,
                    &card.code_or_cmd,
                    palette,
                ));
            }

            if let Some(ref diff) = card.edit_diff {
                body = body.child(render_diff_card(diff, palette));
            }

            if let Some(ref py) = card.python {
                if let Some(ref cmd) = py.bash_first {
                    body = body.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .px(px(10.0))
                            .py(px(5.0))
                            .rounded_md()
                            .bg(palette.bg_code)
                            .font_family(font_mono())
                            .text_size(px(12.0))
                            .text_color(palette.warning)
                            .child(Icon::Terminal.render(12.0, palette.warning))
                            .child(truncate_chars(cmd, 80)),
                    );
                }
                for d in &py.diffs {
                    body = body.child(render_diff_card(d, palette));
                }
                if !py.sent_mail.is_empty() {
                    let mut mail_row = div().flex().flex_wrap().gap(px(6.0));
                    for m in &py.sent_mail {
                        mail_row = mail_row.child(pill(
                            &format!("↑ {} ({})", m.target, m.status),
                            palette.info_soft,
                            palette.info,
                        ));
                    }
                    body = body.child(mail_row);
                }

                let combined_out = [
                    py.stdout.trim_end(),
                    py.return_value.as_deref().unwrap_or_default(),
                    py.background_output.as_deref().unwrap_or_default(),
                ]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n");

                if !combined_out.is_empty() {
                    body = body.child(render_console_output(
                        "stdout",
                        &combined_out,
                        detail.expand_tool_output(),
                        false,
                        palette,
                    ));
                }
                if !py.stderr.trim().is_empty() {
                    body = body.child(render_console_output(
                        "stderr",
                        py.stderr.trim_end(),
                        true,
                        true,
                        palette,
                    ));
                }
                if let Some(ref ename) = py.error_name {
                    let tb = if py.traceback.is_empty() {
                        format!("{ename}: {}", py.error_value.as_deref().unwrap_or_default())
                    } else {
                        py.traceback.join("\n")
                    };
                    body = body.child(render_console_output("traceback", &tb, true, true, palette));
                }
            } else if !card.text_output.trim().is_empty() {
                body = body.child(render_console_output(
                    "output",
                    card.text_output.trim_end(),
                    detail.expand_tool_output() || card.is_error,
                    card.is_error,
                    palette,
                ));
            }

            if card.image_count > 0 {
                body = body.child(pill(
                    &format!("Attached {} image(s) to model context", card.image_count),
                    palette.info_soft,
                    palette.info,
                ));
            }

            container.child(body)
        })
        .when(!expanded && detail.expand_diffs(), |container| {
            // In `Details` mode, surface edit diffs inline even when REPL output stays compact.
            let mut diffs: Vec<&DiffView> = Vec::new();
            if let Some(ref d) = card.edit_diff {
                diffs.push(d);
            }
            if let Some(ref py) = card.python {
                diffs.extend(py.diffs.iter());
            }
            if diffs.is_empty() {
                container
            } else {
                container.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .px(px(12.0))
                        .pb(px(10.0))
                        .children(diffs.into_iter().map(|d| render_diff_card(d, palette))),
                )
            }
        })
        .into_any_element()
}

fn render_diff_card(diff: &DiffView, palette: Palette) -> AnyElement {
    let lines: Vec<AnyElement> = diff
        .unified
        .lines()
        .take(80)
        .map(|line| {
            let (bg, fg) = if line.starts_with('+') && !line.starts_with("+++") {
                (palette.success_soft, palette.success)
            } else if line.starts_with('-') && !line.starts_with("---") {
                (palette.danger_soft, palette.danger)
            } else {
                (gpui::transparent_black(), palette.text_muted)
            };
            div()
                .w_full()
                .px(px(8.0))
                .bg(bg)
                .text_color(fg)
                .font_family(font_mono())
                .text_size(px(12.0))
                .line_height(px(18.0))
                .whitespace_nowrap()
                .child(if line.is_empty() {
                    " ".to_string()
                } else {
                    line.to_string()
                })
                .into_any_element()
        })
        .collect();

    div()
        .w_full()
        .rounded_lg()
        .border_1()
        .border_color(palette.border_subtle)
        .bg(palette.bg_code)
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px(px(10.0))
                .py(px(5.0))
                .border_b_1()
                .border_color(palette.border_subtle)
                .bg(palette.bg_elevated)
                .child(
                    div()
                        .font_family(font_mono())
                        .text_size(px(12.0))
                        .text_color(palette.text_primary)
                        .child(diff.path.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .font_family(font_mono())
                        .text_size(px(11.5))
                        .child(
                            div()
                                .text_color(palette.success)
                                .child(format!("+{}", diff.added_lines)),
                        )
                        .child(
                            div()
                                .text_color(palette.danger)
                                .child(format!("-{}", diff.removed_lines)),
                        ),
                ),
        )
        .child(
            div()
                .id(SharedString::from(format!("diff-scroll-{}", diff.path)))
                .py(px(4.0))
                .overflow_x_scroll()
                .flex()
                .flex_col()
                .children(lines),
        )
        .into_any_element()
}

fn render_console_output(
    label: &str,
    output: &str,
    show_full: bool,
    is_error: bool,
    palette: Palette,
) -> AnyElement {
    let max_lines = if show_full { 120 } else { 18 };
    let all_lines: Vec<&str> = output.lines().collect();
    let shown = all_lines.iter().take(max_lines);

    div()
        .w_full()
        .rounded_lg()
        .border_1()
        .border_color(if is_error {
            palette.danger
        } else {
            palette.border_subtle
        })
        .bg(palette.bg_code)
        .p(px(10.0))
        .flex()
        .flex_col()
        .gap(px(3.0))
        .child(
            div()
                .text_size(px(10.5))
                .font_family(font_mono())
                .text_color(if is_error {
                    palette.danger
                } else {
                    palette.text_muted
                })
                .child(label.to_uppercase()),
        )
        .children(shown.map(|line| {
            div()
                .font_family(font_mono())
                .text_size(px(12.0))
                .line_height(px(17.5))
                .text_color(if is_error {
                    palette.danger
                } else {
                    palette.text_secondary
                })
                .child((*line).to_string())
        }))
        .when(all_lines.len() > max_lines, |d| {
            let shortcut = if cfg!(target_os = "macos") {
                "⌘O"
            } else {
                "Ctrl+O"
            };
            d.child(
                div()
                    .pt(px(4.0))
                    .font_family(font_mono())
                    .text_size(px(11.0))
                    .text_color(palette.text_muted)
                    .child(format!(
                        "… {} more lines (switch view to All with {shortcut})",
                        all_lines.len() - max_lines
                    )),
            )
        })
        .into_any_element()
}

/// Render an inter-agent `agent_message` or RLM subagent terminal outcome row.
#[must_use]
pub fn render_agent_mail_card(
    key: &str,
    kind: MailKind,
    counterpart: &str,
    body: &str,
    expanded: bool,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let toggle_key = key.to_string();
    let (symbol, headline, chip_bg, chip_ink) = match kind {
        MailKind::Received => (
            "↓",
            format!("Agent message from {counterpart}"),
            palette.info_soft,
            palette.info,
        ),
        MailKind::Sent => (
            "↑",
            format!("Agent message to {counterpart}"),
            palette.info_soft,
            palette.info,
        ),
        MailKind::ChildCompleted => (
            "◆",
            format!("Subagent `{counterpart}` completed"),
            palette.success_soft,
            palette.success,
        ),
        MailKind::ChildCancelled => (
            "◆",
            format!("Subagent `{counterpart}` cancelled"),
            palette.warning_soft,
            palette.warning,
        ),
        MailKind::ChildFailed => (
            "◆",
            format!("Subagent `{counterpart}` failed"),
            palette.danger_soft,
            palette.danger,
        ),
    };

    div()
        .w_full()
        .rounded_lg()
        .border_1()
        .border_color(palette.border_subtle)
        .bg(palette.bg_elevated)
        .overflow_hidden()
        .child(
            div()
                .id(SharedString::from(format!("{key}-hdr")))
                .flex()
                .items_center()
                .justify_between()
                .px(px(12.0))
                .py(px(7.0))
                .cursor_pointer()
                .hover(|s| s.bg(palette.bg_hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.session.toggle_expanded(&toggle_key);
                    cx.notify();
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(pill(symbol, chip_bg, chip_ink))
                        .child(
                            div()
                                .text_size(px(13.0))
                                .font_family(font_sans())
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.text_primary)
                                .child(headline),
                        ),
                )
                .child(
                    if expanded {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    }
                    .render(13.0, palette.text_muted),
                ),
        )
        .when(expanded && !body.trim().is_empty(), |d| {
            d.child(
                div()
                    .px(px(12.0))
                    .py(px(9.0))
                    .border_t_1()
                    .border_color(palette.border_subtle)
                    .text_size(px(13.0))
                    .line_height(px(20.0))
                    .text_color(palette.text_secondary)
                    .child(body.trim().to_string()),
            )
        })
        .into_any_element()
}

#[must_use]
pub fn pill(text: &str, bg: gpui::Hsla, fg: gpui::Hsla) -> AnyElement {
    div()
        .px(px(7.0))
        .py(px(2.0))
        .rounded_md()
        .bg(bg)
        .font_family(font_mono())
        .text_size(px(11.0))
        .text_color(fg)
        .child(text.to_string())
        .into_any_element()
}
