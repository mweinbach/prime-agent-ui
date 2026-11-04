//! Center conversation stage: frameless header, Goal banner, editorial empty state,
//! scrollable rich transcript, and `/btw` side-question scratchpad.

use gpui::{AnyElement, Context, FontWeight, SharedString, div, prelude::*, px};

use crate::assets::Icon;
use crate::bridge::BridgeRequest;
use crate::state::projects::folder_label;
use crate::state::transcript::{AssistantBlock, TranscriptEntry};
use crate::state::{format_tokens_compact, truncate_chars};
use crate::theme::{DetailLevel, InspectorTab, Palette, font_mono, font_sans, font_serif};
use crate::ui::composer::render_composer;
use crate::ui::markdown::render_markdown;
use crate::ui::sidebar::project_tooltip;
use crate::ui::tool_cards::{pill, render_agent_mail_card, render_tool_card};
use crate::ui::workspace::PrimeDesktopView;

#[must_use]
#[allow(clippy::too_many_lines)]
pub fn render_stage(
    view: &PrimeDesktopView,
    palette: Palette,
    sidebar_docked: bool,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let attached = !view.session.active_session_id.is_empty();
    let empty = view.session.entries.is_empty() && !view.session.is_streaming;
    let clear_traffic_lights = cfg!(target_os = "macos") && !sidebar_docked;
    let project_cwd = view.current_project();
    let subagent_count = view.rlm_children.rows.len();

    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_hidden()
        .flex()
        .flex_col()
        .bg(palette.bg_canvas)
        .child(
            div()
                .h(px(52.0))
                .flex_none()
                .pl(px(if clear_traffic_lights { 82.0 } else { 20.0 }))
                .pr(px(16.0))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(6.0))
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .overflow_hidden()
                        .when(!sidebar_docked, |d| {
                            d.child(
                                div()
                                    .id("stage-open-sidebar-btn")
                                    .p(px(7.0))
                                    .rounded_md()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(palette.bg_hover))
                                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)))
                                    .child(Icon::Sidebar.render(16.0, palette.text_secondary)),
                            )
                            .child(
                                div()
                                    .id("stage-new-chat")
                                    .p(px(7.0))
                                    .rounded_md()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(palette.bg_hover))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.start_fresh_session(window, cx);
                                    }))
                                    .child(Icon::Plus.render(16.0, palette.text_secondary)),
                            )
                        })
                        .child(if let Some(parent_id) = view.parent_session_target() {
                            div()
                                .id("subagent-parent")
                                .debug_selector(|| "subagent-parent".to_owned())
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap(px(5.0))
                                .px(px(6.0))
                                .py(px(4.0))
                                .rounded_md()
                                .cursor_pointer()
                                .hover(|s| s.bg(palette.bg_hover))
                                .text_size(px(12.0))
                                .text_color(palette.text_secondary)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_session(
                                        BridgeRequest::AttachSession {
                                            active_session_id: parent_id.clone(),
                                        },
                                        cx,
                                    );
                                }))
                                .child(Icon::GitBranch.render(13.0, palette.text_muted))
                                .child("Parent chat")
                                .into_any_element()
                        } else {
                            div()
                                .id("stage-project-folder")
                                .debug_selector(|| "stage-project-folder".to_owned())
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .when(!project_cwd.is_empty(), |d| {
                                    d.tooltip(project_tooltip(
                                        project_cwd.to_owned().into(),
                                        palette,
                                    ))
                                })
                                .child(Icon::Folder.render(13.0, palette.text_muted))
                                .child(
                                    div()
                                        .min_w_0()
                                        .max_w(px(140.0))
                                        .truncate()
                                        .text_size(px(12.0))
                                        .text_color(palette.text_secondary)
                                        .child(folder_label(project_cwd)),
                                )
                                .into_any_element()
                        })
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(palette.text_muted)
                                .child("/"),
                        )
                        .child(if attached && view.renaming_session {
                            div()
                                .w(px(220.0))
                                .min_w_0()
                                .px(px(8.0))
                                .py(px(3.0))
                                .rounded_md()
                                .border_1()
                                .border_color(palette.accent)
                                .bg(palette.bg_elevated)
                                .child(view.rename_input.clone())
                                .into_any_element()
                        } else if attached {
                            div()
                                .id("stage-session-title")
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .px(px(6.0))
                                .py(px(4.0))
                                .rounded_md()
                                .cursor_pointer()
                                .hover(|s| s.bg(palette.bg_hover))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.begin_rename_session(window, cx);
                                }))
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(13.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(view.session.display_title()),
                                )
                                .child(Icon::EditPen.render(11.0, palette.text_muted))
                                .into_any_element()
                        } else {
                            div()
                                .truncate()
                                .px(px(4.0))
                                .text_size(px(13.0))
                                .text_color(palette.text_secondary)
                                .child("New chat")
                                .into_any_element()
                        }),
                )
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .when(!empty, |d| {
                            d.child(
                                div()
                                    .id("stage-detail-pill")
                                    .px(px(7.0))
                                    .py(px(5.0))
                                    .rounded_md()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(palette.bg_hover))
                                    .text_size(px(12.0))
                                    .text_color(palette.text_secondary)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.cycle_detail_level(cx)),
                                    )
                                    .child(view.prefs.detail_level.label()),
                            )
                            .child(
                                div()
                                    .id("stage-side-question-btn")
                                    .px(px(7.0))
                                    .py(px(5.0))
                                    .rounded_md()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(palette.bg_hover))
                                    .when(view.session.side_question.open, |d| {
                                        d.bg(palette.bg_selected)
                                    })
                                    .text_size(px(12.0))
                                    .text_color(palette.text_secondary)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_side_question(window, cx);
                                    }))
                                    .child("/btw"),
                            )
                        })
                        .when(attached && subagent_count > 0, |d| {
                            d.child(
                                div()
                                    .id("session-subagents")
                                    .debug_selector(|| "session-subagents".to_owned())
                                    .px(px(7.0))
                                    .py(px(5.0))
                                    .rounded_md()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.0))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(palette.bg_hover))
                                    .text_size(px(12.0))
                                    .text_color(palette.text_secondary)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_inspector_tab(InspectorTab::Agents, cx);
                                    }))
                                    .child(Icon::GitBranch.render(13.0, palette.text_secondary))
                                    .child(format!("Subagents {subagent_count}")),
                            )
                        })
                        .child(
                            div()
                                .id("stage-toggle-inspector-btn")
                                .px(px(8.0))
                                .py(px(5.0))
                                .rounded_md()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .cursor_pointer()
                                .hover(|s| s.bg(palette.bg_hover))
                                .when(view.prefs.inspector_open, |d| d.bg(palette.bg_selected))
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_inspector(cx)))
                                .child(Icon::Layers.render(14.0, palette.text_secondary))
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(palette.text_secondary)
                                        .child("Inspector"),
                                ),
                        ),
                ),
        )
        // Active Goal Banner (when active)
        .when_some(
            view.session
                .goal
                .as_ref()
                .filter(|g| g.active || g.objective.is_some()),
            |d, goal| {
                let status_txt = goal.status.slug();
                let used = goal.tokens_used;
                let budget = goal.token_budget;
                let obj_text = goal.objective.as_deref().unwrap_or("Active goal");
                d.child(
                    div()
                        .w_full()
                        .px(px(20.0))
                        .py(px(7.0))
                        .border_b_1()
                        .border_color(palette.border_subtle)
                        .bg(palette.accent_soft)
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .overflow_hidden()
                                .child(Icon::Target.render(14.0, palette.accent))
                                .child(
                                    div()
                                        .text_size(px(12.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(palette.text_primary)
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .child(truncate_chars(obj_text, 76)),
                                ),
                        )
                        .child(
                            div()
                                .font_family(font_mono())
                                .text_size(px(11.0))
                                .text_color(palette.accent)
                                .child(match budget {
                                    Some(b) if b > 0 => format!(
                                        "{status_txt} · {} / {}",
                                        format_tokens_compact(used),
                                        format_tokens_compact(b)
                                    ),
                                    _ => format!("{status_txt} · {}", format_tokens_compact(used)),
                                }),
                        ),
                )
            },
        )
        // `/btw` Parallel Side-Question Drawer
        .when(view.session.side_question.open, |d| {
            d.child(render_side_question_bar(view, palette, cx))
        })
        .when(empty, |stage| {
            stage.child(render_welcome_empty_state(view, palette, cx))
        })
        .when(!empty, |stage| {
            stage
                .child(render_transcript_scroll(view, palette, cx))
                .child(
                    div()
                        .flex_none()
                        .px(px(24.0))
                        .pt(px(8.0))
                        .pb(px(20.0))
                        .child(render_composer(view, palette, cx)),
                )
        })
        .into_any_element()
}

fn render_welcome_empty_state(
    view: &PrimeDesktopView,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let starters = [
        (
            Icon::Layers,
            "Explore this project",
            "Walk me through this project's structure and its main entry points.",
        ),
        (
            Icon::GitBranch,
            "Review changes",
            "Review my uncommitted changes. What should I look at before committing?",
        ),
    ];
    let has_draft =
        !view.composer_input.read(cx).text().is_empty() || !view.staged_images.is_empty();

    // Put the flexible space on the scroll viewport itself. The content keeps its
    // intrinsic height when a short window or the command list needs scrolling.
    let mut bottom_space = div().flex_1();
    bottom_space.style().flex_grow = Some(1.6);
    div()
        .id("welcome-stage")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .items_center()
        .px(px(24.0))
        .py(px(20.0))
        .child(div().flex_1())
        .child(
            div()
                .w_full()
                .max_w(px(720.0))
                .flex_none()
                .flex()
                .flex_col()
                .gap(px(18.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .child(Icon::Prime.render(30.0, palette.accent))
                        .child(
                            div()
                                .font_family(font_serif())
                                .text_size(px(34.0))
                                .line_height(px(42.0))
                                .text_color(palette.text_primary)
                                .child("What are we working on?"),
                        ),
                )
                .child(render_composer(view, palette, cx))
                .child(
                    div()
                        .min_h(px(32.0))
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.0))
                        .when(!has_draft, |row| {
                            row.children(starters.into_iter().enumerate().map(
                                |(i, (icon, label, prompt))| {
                                    div()
                                        .id(("starter", i))
                                        .px(px(9.0))
                                        .py(px(6.0))
                                        .rounded_md()
                                        .flex()
                                        .items_center()
                                        .gap(px(7.0))
                                        .cursor_pointer()
                                        .hover(|s| s.bg(palette.bg_hover))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.populate_draft(prompt, window, cx);
                                        }))
                                        .child(icon.render(14.0, palette.text_muted))
                                        .child(
                                            div()
                                                .text_size(px(12.5))
                                                .text_color(palette.text_secondary)
                                                .child(label),
                                        )
                                },
                            ))
                        }),
                ),
        )
        .child(bottom_space)
        .into_any_element()
}

#[allow(clippy::too_many_lines)]
fn render_transcript_scroll(
    view: &PrimeDesktopView,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let detail = view.prefs.detail_level;
    let mut rows: Vec<AnyElement> = Vec::with_capacity(view.session.entries.len() + 1);

    for entry in &view.session.entries {
        match entry {
            TranscriptEntry::User {
                key,
                text,
                skill_name,
                image_count,
            } => {
                rows.push(
                    div()
                        .id(SharedString::from(key.clone()))
                        .w_full()
                        .flex()
                        .justify_end()
                        .pt(px(6.0))
                        .child(
                            div()
                                .max_w(px(620.0))
                                .px(px(15.0))
                                .py(px(10.5))
                                .rounded_2xl()
                                .bg(palette.bg_user_bubble)
                                .border_1()
                                .border_color(palette.border_subtle)
                                .flex()
                                .flex_col()
                                .gap(px(6.0))
                                .when_some(skill_name.as_ref(), |d, sname| {
                                    d.child(pill(
                                        &format!("skill · {sname}"),
                                        palette.accent_soft,
                                        palette.accent,
                                    ))
                                })
                                .when(*image_count > 0, |d| {
                                    d.child(pill(
                                        &format!("{image_count} image(s) attached"),
                                        palette.info_soft,
                                        palette.info,
                                    ))
                                })
                                .child(
                                    div()
                                        .font_family(font_sans())
                                        .text_size(px(14.5))
                                        .line_height(px(22.0))
                                        .text_color(palette.text_primary)
                                        .child(text.clone()),
                                ),
                        )
                        .into_any_element(),
                );
            }
            TranscriptEntry::Assistant {
                key,
                blocks,
                streaming,
                error,
                ..
            } => {
                let mut block_els: Vec<AnyElement> = Vec::new();
                for (b_idx, block) in blocks.iter().enumerate() {
                    match block {
                        AssistantBlock::Thinking(thought) => {
                            let t_key = format!("{key}-think-{b_idx}");
                            let toggle_t_key = t_key.clone();
                            let open = view.session.is_expanded(&t_key, detail.show_thinking());
                            block_els.push(
                                div()
                                    .w_full()
                                    .rounded_lg()
                                    .border_1()
                                    .border_color(palette.border_subtle)
                                    .bg(palette.bg_elevated.opacity(0.6))
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .id(SharedString::from(format!("{t_key}-hdr")))
                                            .px(px(11.0))
                                            .py(px(6.0))
                                            .cursor_pointer()
                                            .hover(|s| s.bg(palette.bg_hover))
                                            .flex()
                                            .items_center()
                                            .gap(px(6.0))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.session.toggle_expanded(&toggle_t_key);
                                                cx.notify();
                                            }))
                                            .child(
                                                if open {
                                                    Icon::ChevronDown
                                                } else {
                                                    Icon::ChevronRight
                                                }
                                                .render(12.5, palette.text_muted),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(12.0))
                                                    .font_family(font_sans())
                                                    .text_color(palette.text_muted)
                                                    .child(if *streaming {
                                                        "Thinking…"
                                                    } else {
                                                        "Thought process"
                                                    }),
                                            ),
                                    )
                                    .when(open, |d| {
                                        d.child(
                                            div()
                                                .px(px(13.0))
                                                .pb(px(10.0))
                                                .pt(px(4.0))
                                                .border_t_1()
                                                .border_color(palette.border_subtle)
                                                .text_size(px(13.0))
                                                .line_height(px(20.0))
                                                .text_color(palette.text_secondary)
                                                .child(thought.trim().to_string()),
                                        )
                                    })
                                    .into_any_element(),
                            );
                        }
                        AssistantBlock::Text(md) => {
                            block_els.push(render_markdown(
                                &format!("{key}-md-{b_idx}"),
                                md,
                                palette,
                            ));
                        }
                    }
                }
                if let Some(err_msg) = error {
                    block_els.push(
                        div()
                            .px(px(12.0))
                            .py(px(8.0))
                            .rounded_lg()
                            .border_1()
                            .border_color(palette.danger)
                            .bg(palette.danger_soft)
                            .text_size(px(13.0))
                            .text_color(palette.danger)
                            .child(err_msg.clone())
                            .into_any_element(),
                    );
                }
                rows.push(
                    div()
                        .id(SharedString::from(key.clone()))
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .children(block_els)
                        .into_any_element(),
                );
            }
            TranscriptEntry::Tool(card) => {
                let t_key = format!("tool-{}", card.id);
                let default_open = detail.expand_tool_output() || card.is_error;
                let expanded = view.session.is_expanded(&t_key, default_open);
                rows.push(render_tool_card(card, detail, expanded, palette, cx));
            }
            TranscriptEntry::UserBash {
                key,
                command,
                output,
                running,
                exit_code,
                cancelled,
            } => {
                let toggle_k = key.clone();
                let open = view.session.is_expanded(key, true);
                let badge = if *running {
                    ("Running…".to_string(), palette.warning)
                } else if *cancelled {
                    ("Cancelled".to_string(), palette.danger)
                } else {
                    match exit_code {
                        Some(0) => ("exit 0".to_string(), palette.success),
                        Some(c) => (format!("exit {c}"), palette.danger),
                        None => ("done".to_string(), palette.text_muted),
                    }
                };
                rows.push(
                    div()
                        .w_full()
                        .rounded_xl()
                        .border_1()
                        .border_color(palette.border_subtle)
                        .bg(palette.bg_elevated)
                        .overflow_hidden()
                        .child(
                            div()
                                .id(SharedString::from(format!("{key}-hdr")))
                                .px(px(12.0))
                                .py(px(7.5))
                                .cursor_pointer()
                                .hover(|s| s.bg(palette.bg_hover))
                                .flex()
                                .items_center()
                                .justify_between()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.session.toggle_expanded(&toggle_k);
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(8.0))
                                        .child(Icon::Terminal.render(13.0, palette.warning))
                                        .child(
                                            div()
                                                .font_family(font_mono())
                                                .text_size(px(12.5))
                                                .text_color(palette.text_primary)
                                                .child(format!("$ {command}")),
                                        ),
                                )
                                .child(
                                    div()
                                        .font_family(font_mono())
                                        .text_size(px(11.5))
                                        .text_color(badge.1)
                                        .child(badge.0),
                                ),
                        )
                        .when(open && !output.trim().is_empty(), |d| {
                            d.child(
                                div()
                                    .p(px(11.0))
                                    .border_t_1()
                                    .border_color(palette.border_subtle)
                                    .bg(palette.bg_code)
                                    .font_family(font_mono())
                                    .text_size(px(12.0))
                                    .line_height(px(18.0))
                                    .text_color(palette.text_secondary)
                                    .child(output.trim_end().to_string()),
                            )
                        })
                        .into_any_element(),
                );
            }
            TranscriptEntry::AgentMail {
                key,
                kind,
                counterpart,
                body,
            } => {
                let open = view
                    .session
                    .is_expanded(key, detail != DetailLevel::Overview);
                rows.push(render_agent_mail_card(
                    key,
                    *kind,
                    counterpart,
                    body,
                    open,
                    palette,
                    cx,
                ));
            }
            TranscriptEntry::Compaction {
                key,
                streaming,
                reason,
                summary,
                tokens_before,
            } => {
                let toggle_k = key.clone();
                let open = view.session.is_expanded(key, *streaming);
                rows.push(
                    div()
                        .w_full()
                        .rounded_xl()
                        .border_1()
                        .border_color(palette.border_subtle)
                        .bg(palette.bg_elevated)
                        .overflow_hidden()
                        .child(
                            div()
                                .id(SharedString::from(format!("{key}-hdr")))
                                .px(px(12.0))
                                .py(px(7.5))
                                .cursor_pointer()
                                .hover(|s| s.bg(palette.bg_hover))
                                .flex()
                                .items_center()
                                .justify_between()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.session.toggle_expanded(&toggle_k);
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(8.0))
                                        .child(pill(
                                            "Compact",
                                            palette.warning_soft,
                                            palette.warning,
                                        ))
                                        .child(
                                            div()
                                                .text_size(px(12.5))
                                                .text_color(palette.text_primary)
                                                .child(if *streaming {
                                                    format!("Compacting context ({reason})…")
                                                } else {
                                                    "Context compacted".to_string()
                                                }),
                                        ),
                                )
                                .when_some(*tokens_before, |d, tb| {
                                    d.child(
                                        div()
                                            .font_family(font_mono())
                                            .text_size(px(11.0))
                                            .text_color(palette.text_muted)
                                            .child(format!("was {}", format_tokens_compact(tb))),
                                    )
                                }),
                        )
                        .when(open && !summary.trim().is_empty(), |d| {
                            d.child(
                                div()
                                    .p(px(12.0))
                                    .border_t_1()
                                    .border_color(palette.border_subtle)
                                    .child(render_markdown(
                                        &format!("{key}-sum"),
                                        summary,
                                        palette,
                                    )),
                            )
                        })
                        .into_any_element(),
                );
            }
            TranscriptEntry::Refinement {
                key,
                summary,
                changes,
            } => {
                rows.push(
                    div()
                        .id(SharedString::from(key.clone()))
                        .w_full()
                        .p(px(12.0))
                        .rounded_xl()
                        .border_1()
                        .border_color(palette.accent.opacity(0.4))
                        .bg(palette.accent_soft)
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(7.0))
                                .child(Icon::Sparkles.render(13.5, palette.accent))
                                .child(
                                    div()
                                        .text_size(px(12.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(palette.text_primary)
                                        .child("Continual Harness refined"),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(12.5))
                                .text_color(palette.text_secondary)
                                .child(summary.clone()),
                        )
                        .when(!changes.is_empty(), |d| {
                            d.child(
                                div().flex().flex_wrap().gap(px(5.0)).children(
                                    changes
                                        .iter()
                                        .map(|c| pill(c, palette.bg_elevated, palette.accent)),
                                ),
                            )
                        })
                        .into_any_element(),
                );
            }
            TranscriptEntry::Notice {
                key,
                title,
                body,
                is_error,
            } => {
                rows.push(
                    div()
                        .id(SharedString::from(key.clone()))
                        .w_full()
                        .px(px(12.0))
                        .py(px(8.0))
                        .rounded_lg()
                        .border_1()
                        .border_color(if *is_error {
                            palette.danger
                        } else {
                            palette.border_subtle
                        })
                        .bg(if *is_error {
                            palette.danger_soft
                        } else {
                            palette.bg_elevated
                        })
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .child(
                            div()
                                .font_family(font_mono())
                                .text_size(px(11.0))
                                .text_color(if *is_error {
                                    palette.danger
                                } else {
                                    palette.text_muted
                                })
                                .child(title.to_uppercase()),
                        )
                        .child(
                            div()
                                .text_size(px(12.5))
                                .text_color(if *is_error {
                                    palette.danger
                                } else {
                                    palette.text_secondary
                                })
                                .child(body.clone()),
                        )
                        .into_any_element(),
                );
            }
        }
    }

    // Live working indicator row at the tail of an active turn.
    if view.session.is_streaming {
        let note = view
            .session
            .working_note
            .as_deref()
            .unwrap_or("Prime Agent is working…");
        rows.push(
            div()
                .id("live-working-row")
                .flex()
                .items_center()
                .gap(px(8.0))
                .py(px(6.0))
                .child(
                    div()
                        .w(px(8.0))
                        .h(px(8.0))
                        .rounded_full()
                        .bg(palette.accent),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_family(font_sans())
                        .text_color(palette.text_secondary)
                        .child(note.to_string()),
                )
                .into_any_element(),
        );
    }

    div()
        .id("transcript-scroll-container")
        .flex_1()
        .overflow_y_scroll()
        .track_scroll(&view.transcript_scroll)
        .min_h_0()
        .px(px(24.0))
        .py(px(20.0))
        .flex()
        .flex_col()
        .items_center()
        .child(
            div()
                .w_full()
                .max_w(px(720.0))
                .flex()
                .flex_col()
                .gap(px(18.0))
                .children(rows),
        )
        .into_any_element()
}

fn render_side_question_bar(
    view: &PrimeDesktopView,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let sq = &view.session.side_question;
    div()
        .w_full()
        .px(px(20.0))
        .py(px(10.0))
        .border_b_1()
        .border_color(palette.border_subtle)
        .bg(palette.bg_elevated)
        .flex()
        .justify_center()
        .child(
            div()
                .w_full()
                .max_w(px(720.0))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(pill("/btw", palette.accent_soft, palette.accent))
                        .child(div().flex_1().child(view.side_question_input.clone()))
                        .child(
                            div()
                                .id("sq-ask-btn")
                                .px(px(10.0))
                                .py(px(4.5))
                                .rounded_md()
                                .bg(palette.accent)
                                .cursor_pointer()
                                .text_size(px(12.0))
                                .text_color(palette.text_on_accent)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.submit_side_question(window, cx);
                                }))
                                .child("Ask"),
                        )
                        .when_some(sq.running_id.as_ref(), |d, run_id| {
                            let rid = run_id.clone();
                            let sid = view.session.active_session_id.clone();
                            d.child(
                                div()
                                    .id("sq-stop-btn")
                                    .px(px(8.0))
                                    .py(px(4.5))
                                    .rounded_md()
                                    .bg(palette.danger_soft)
                                    .cursor_pointer()
                                    .text_size(px(12.0))
                                    .text_color(palette.danger)
                                    .on_click(cx.listener(move |this, _, _, _cx| {
                                        this.bridge.send(BridgeRequest::AbortSideQuestion {
                                            active_session_id: sid.clone(),
                                            side_question_id: rid.clone(),
                                        });
                                    }))
                                    .child("Stop"),
                            )
                        }),
                )
                .when(!sq.answer.is_empty(), |d| {
                    d.child(
                        div()
                            .p(px(10.0))
                            .rounded_lg()
                            .bg(palette.bg_canvas)
                            .border_1()
                            .border_color(palette.border_subtle)
                            .child(render_markdown("sq-ans", &sq.answer, palette)),
                    )
                })
                .when_some(sq.error.as_ref(), |d, err| {
                    d.child(
                        div()
                            .text_size(px(12.0))
                            .text_color(palette.danger)
                            .child(err.clone()),
                    )
                }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use gpui::{Modifiers, TestAppContext, px, size};

    use crate::ui::workspace::{LaunchOptions, PrimeDesktopView};

    #[test]
    fn hovering_the_project_breadcrumb_reveals_the_working_directory() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = PathBuf::from("/fixture")
            .join("a-long-ancestor-directory/".repeat(12))
            .join("project");
        let options = LaunchOptions {
            initial_cwd: Some(cwd),
            daemon_socket: Some(temp.path().join("no-listener")),
            ..Default::default()
        };
        let mut app = TestAppContext::single();
        let (_, cx) = app.add_window_view(|window, cx| {
            PrimeDesktopView::new(options, temp.path().to_path_buf(), window, cx)
        });
        cx.simulate_resize(size(px(960.0), px(640.0)));
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("project-tooltip").is_none());
        let breadcrumb = cx.debug_bounds("stage-project-folder").unwrap();
        cx.simulate_mouse_move(breadcrumb.center(), None, Modifiers::none());
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.update(|window, cx| window.draw(cx).clear());
        let tooltip = cx
            .debug_bounds("project-tooltip")
            .expect("project path tooltip");
        assert!(tooltip.size.height > px(32.0), "the full path should wrap");
        app.quit();
    }
}
