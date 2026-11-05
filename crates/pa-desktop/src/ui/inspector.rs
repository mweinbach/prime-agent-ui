//! Right-hand Agentic Harness Studio drawer (`Agents`, `Harness`, `Runtime`, `System`).

use gpui::{
    AnyElement, ClipboardItem, Context, Focusable, FontWeight, SharedString, div, prelude::*, px,
};
use pa_core::refinement::{HarnessScope, RefinementKind};

use crate::assets::Icon;
use crate::bridge::BridgeRequest;
use crate::state::{
    ActiveModal, ActiveSessionRow, RlmChildRow, format_tokens_compact, truncate_chars,
};
use crate::theme::{InspectorTab, Palette, font_mono, font_sans};
use crate::ui::modals::confirm_action;
use crate::ui::tool_cards::pill;
use crate::ui::workspace::PrimeDesktopView;

pub(super) const INSPECTOR_WIDTH: f32 = 328.0;

#[must_use]
pub fn render_inspector(
    view: &PrimeDesktopView,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let active_tab = view.prefs.inspector_tab;
    let sid_empty = view.session.active_session_id.is_empty();
    let sid = view.session.active_session_id.clone();

    div()
        .w(px(INSPECTOR_WIDTH))
        .max_w_full()
        .min_w_0()
        .h_full()
        .flex_none()
        .flex()
        .flex_col()
        .bg(palette.bg_inspector)
        .border_l_1()
        .border_color(palette.border_subtle)
        // Segmented Tab Header
        .child(
            div()
                .h(px(52.0))
                .flex_none()
                .px(px(10.0))
                .border_b_1()
                .border_color(palette.border_subtle)
                .flex()
                .items_center()
                .justify_between()
                .gap(px(4.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(3.0))
                        .p(px(3.0))
                        .rounded_lg()
                        .bg(palette.bg_code)
                        .children(InspectorTab::ALL.into_iter().map(|tab| {
                            let selected = tab == active_tab;
                            div()
                                .id(SharedString::from(format!("insp-tab-{}", tab.label())))
                                .px(px(9.0))
                                .py(px(4.0))
                                .rounded_md()
                                .cursor_pointer()
                                .bg(if selected {
                                    palette.bg_elevated
                                } else {
                                    gpui::transparent_black()
                                })
                                .hover(|s| {
                                    if selected {
                                        s.bg(palette.bg_elevated)
                                    } else {
                                        s.bg(palette.bg_hover)
                                    }
                                })
                                .text_size(px(12.0))
                                .font_weight(if selected {
                                    FontWeight::MEDIUM
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(if selected {
                                    palette.text_primary
                                } else {
                                    palette.text_secondary
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.set_inspector_tab(tab, window, cx);
                                }))
                                .child(tab.label())
                        })),
                )
                .child(
                    div()
                        .id("insp-refresh-btn")
                        .p(px(5.0))
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|s| s.bg(palette.bg_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.reload_local_stores(cx);
                            if !sid.is_empty() {
                                this.bridge.send(BridgeRequest::RefreshInspector {
                                    active_session_id: sid.clone(),
                                });
                            }
                            cx.notify();
                        }))
                        .child(Icon::Refresh.render(13.5, palette.text_muted)),
                ),
        )
        // Scrollable Tab Body
        .child(
            div()
                .id("inspector-scroll-body")
                .flex_1()
                .overflow_y_scroll()
                .p(px(12.0))
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(if sid_empty && active_tab != InspectorTab::Harness {
                    empty_box("Start or open a chat to inspect this session.", palette)
                } else {
                    match active_tab {
                        InspectorTab::Agents => render_agents_tab(view, palette, cx),
                        InspectorTab::Harness => render_harness_tab(view, palette, cx),
                        InspectorTab::Runtime => render_runtime_tab(view, palette, cx),
                        InspectorTab::System => render_system_tab(view, palette, cx),
                    }
                }),
        )
        .into_any_element()
}

#[allow(clippy::too_many_lines)]
fn render_agents_tab(
    view: &PrimeDesktopView,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let children = &view.rlm_children;
    let children_heading = if children.loaded || !children.rows.is_empty() {
        format!("DIRECT SUBAGENTS ({})", children.rows.len())
    } else {
        "DIRECT SUBAGENTS".to_owned()
    };
    let sid = view.session.active_session_id.clone();
    let sid_dec = sid.clone();
    let sid_inc = sid;

    div()
        .flex()
        .flex_col()
        .gap(px(12.0))
        // RLM Recursion Depth Stepper Card
        .when_some(view.rlm_max_depth, |d, max_depth| {
            d.child(
                div()
                    .p(px(11.0))
                    .rounded_xl()
                    .border_1()
                    .border_color(palette.border_subtle)
                    .bg(palette.bg_elevated)
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(palette.text_primary)
                                    .child("Maximum agent depth"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(palette.text_muted)
                                    .child("Allowed levels of nested agents"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .id("rlm-depth-dec")
                                    .px(px(8.0))
                                    .py(px(3.0))
                                    .rounded_md()
                                    .border_1()
                                    .border_color(palette.border_subtle)
                                    .cursor_pointer()
                                    .hover(|s| s.bg(palette.bg_hover))
                                    .font_family(font_mono())
                                    .text_size(px(12.0))
                                    .text_color(palette.text_primary)
                                    .on_click(cx.listener(move |this, _, _, _cx| {
                                        if !sid_dec.is_empty() && max_depth > 0 {
                                            this.bridge.send(BridgeRequest::SetRlmMaxDepth {
                                                active_session_id: sid_dec.clone(),
                                                max_depth: max_depth - 1,
                                            });
                                        }
                                    }))
                                    .child("−"),
                            )
                            .child(
                                div()
                                    .font_family(font_mono())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(13.0))
                                    .text_color(palette.accent)
                                    .child(format!("{max_depth}")),
                            )
                            .child(
                                div()
                                    .id("rlm-depth-inc")
                                    .px(px(8.0))
                                    .py(px(3.0))
                                    .rounded_md()
                                    .border_1()
                                    .border_color(palette.border_subtle)
                                    .cursor_pointer()
                                    .hover(|s| s.bg(palette.bg_hover))
                                    .font_family(font_mono())
                                    .text_size(px(12.0))
                                    .text_color(palette.text_primary)
                                    .on_click(cx.listener(move |this, _, _, _cx| {
                                        if !sid_inc.is_empty() && max_depth < 8 {
                                            this.bridge.send(BridgeRequest::SetRlmMaxDepth {
                                                active_session_id: sid_inc.clone(),
                                                max_depth: max_depth + 1,
                                            });
                                        }
                                    }))
                                    .child("+"),
                            ),
                    ),
            )
        })
        // Direct Subagents Section
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(7.0))
                .child(label_caps(&children_heading, palette))
                .when_some(children.load_error.as_ref(), |d, error| {
                    d.child(empty_box(
                        &format!("Couldn’t load subagents: {error}"),
                        palette,
                    ))
                })
                .when(
                    children.rows.is_empty() && children.load_error.is_none(),
                    |d| {
                        d.child(empty_box(
                            if children.loaded {
                                "No subagents in this chat."
                            } else {
                                "Loading subagents…"
                            },
                            palette,
                        ))
                    },
                )
                .children(children.rows.iter().map(|child| {
                    let live = child.active_session_id.as_deref().and_then(|id| {
                        view.active_sessions.iter().find(|row| {
                            row.status != "inactive"
                                && (row.active_session_id == id
                                    || row.session_id.as_deref() == Some(id))
                        })
                    });
                    render_subagent(child, live, view, palette, cx)
                })),
        )
        .into_any_element()
}

fn render_subagent(
    child: &RlmChildRow,
    live: Option<&ActiveSessionRow>,
    view: &PrimeDesktopView,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let (status, background, foreground) = match child.status.as_str() {
        "running" => ("Running", palette.warning_soft, palette.warning),
        "done" | "completed" => ("Completed", palette.success_soft, palette.success),
        "error" => ("Error", palette.danger_soft, palette.danger),
        "cancelled" => ("Cancelled", palette.bg_hover, palette.text_secondary),
        "" | "unknown" => ("Unknown", palette.bg_hover, palette.text_secondary),
        other => (other, palette.bg_hover, palette.text_secondary),
    };
    let mut metadata = Vec::new();
    if let Some(model) = child
        .model
        .as_ref()
        .or(live.and_then(|row| row.model_label.as_ref()))
    {
        metadata.push(model.clone());
    }
    if let Some(count) = live.and_then(|row| row.tool_use_count) {
        metadata.push(format!(
            "{count} tool {}",
            if count == 1 { "call" } else { "calls" }
        ));
    }
    let note = live.and_then(|row| row.progress_note.as_ref().or(row.activity.as_ref()));
    let child_id = &child.rlm_child_id;
    let mut actions = div().flex().flex_wrap().items_center().gap(px(6.0));
    if let Some(row) = live {
        let attach_id = row.active_session_id.clone();
        let target_id = attach_id.clone();
        actions = actions
            .child(
                small_btn(
                    &format!("open-child-{child_id}"),
                    "Inspect live",
                    palette.info_soft,
                    palette.info,
                    cx.listener(move |this, _, _, cx| {
                        this.open_session(
                            BridgeRequest::AttachSession {
                                active_session_id: attach_id.clone(),
                            },
                            cx,
                        );
                    }),
                )
                .debug_selector(|| "subagent-inspect".to_owned()),
            )
            .child(small_btn(
                &format!("msg-child-{child_id}"),
                "Message",
                palette.accent_soft,
                palette.accent,
                cx.listener(move |this, _, window, cx| {
                    if this.agent_mail_target.as_ref() != Some(&target_id) {
                        this.agent_mail_input
                            .update(cx, crate::ui::text_input::TextInput::clear);
                    }
                    this.agent_mail_target = Some(target_id.clone());
                    window.focus(&this.agent_mail_input.focus_handle(cx));
                    cx.notify();
                }),
            ));
    } else {
        actions = actions.child(
            div()
                .text_size(px(11.5))
                .text_color(palette.text_muted)
                .child("Live view unavailable"),
        );
    }
    if child.status == "running" {
        let active_session_id = view.session.active_session_id.clone();
        let cid = child_id.clone();
        let name = child.name.clone();
        actions = actions.child(small_btn(
            &format!("cancel-child-{child_id}"),
            "Cancel",
            palette.danger_soft,
            palette.danger,
            cx.listener(move |_this, _, window, cx| {
                let request = BridgeRequest::CancelRlmChild {
                    active_session_id: active_session_id.clone(),
                    child_id: cid.clone(),
                };
                confirm_action(
                    &format!("Stop {name}? Its running turn will end."),
                    "Stop agent",
                    window,
                    cx,
                    move |this, _| this.bridge.send(request),
                );
            }),
        ));
    } else if matches!(
        child.status.as_str(),
        "done" | "completed" | "error" | "cancelled"
    ) {
        let active_session_id = view.session.active_session_id.clone();
        let cid = child_id.clone();
        let name = child.name.clone();
        actions = actions.child(small_btn(
            &format!("del-child-{child_id}"),
            "Remove",
            palette.bg_hover,
            palette.text_secondary,
            cx.listener(move |_this, _, window, cx| {
                let request = BridgeRequest::DeleteRlmSubagent {
                    active_session_id: active_session_id.clone(),
                    child_id: cid.clone(),
                };
                confirm_action(
                    &format!("Remove {name} from this chat?"),
                    "Remove",
                    window,
                    cx,
                    move |this, _| this.bridge.send(request),
                );
            }),
        ));
    }

    div()
        .min_w_0()
        .p(px(10.0))
        .rounded_xl()
        .border_1()
        .border_color(palette.border_subtle)
        .bg(palette.bg_elevated)
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            div()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(Icon::GitBranch.render(13.0, palette.info))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .text_size(px(12.5))
                        .text_color(palette.text_primary)
                        .child(child.name.clone()),
                )
                .child(pill(status, background, foreground)),
        )
        .when_some(
            child.label.as_ref().filter(|label| **label != child.name),
            |d, label| {
                d.child(
                    div()
                        .text_size(px(12.0))
                        .text_color(palette.text_secondary)
                        .child(label.clone()),
                )
            },
        )
        .when(!metadata.is_empty(), |d| {
            d.child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_family(font_mono())
                    .text_size(px(11.0))
                    .text_color(palette.text_muted)
                    .child(metadata.join(" · ")),
            )
        })
        .when_some(note, |d, note| {
            d.child(
                div()
                    .text_size(px(11.5))
                    .text_color(palette.text_secondary)
                    .child(truncate_chars(note, 160)),
            )
        })
        .when_some(child.answer_preview.as_ref(), |d, preview| {
            d.child(
                div()
                    .text_size(px(12.0))
                    .line_height(px(17.0))
                    .text_color(palette.text_primary)
                    .child(preview.clone()),
            )
        })
        .when_some(child.error.as_ref(), |d, error| {
            d.child(
                div()
                    .text_size(px(12.0))
                    .text_color(if child.status == "error" {
                        palette.danger
                    } else {
                        palette.text_secondary
                    })
                    .child(error.clone()),
            )
        })
        .child(actions)
        .when_some(
            live.filter(|row| {
                view.agent_mail_target.as_deref() == Some(row.active_session_id.as_str())
            }),
            |card, _| {
                card.child(
                    div()
                        .mt(px(3.0))
                        .pt(px(8.0))
                        .border_t_1()
                        .border_color(palette.border_subtle)
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            div()
                                .p(px(7.0))
                                .rounded_md()
                                .bg(palette.bg_code)
                                .child(view.agent_mail_input.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_end()
                                .gap(px(6.0))
                                .child(small_btn(
                                    "close-mail-box",
                                    "Dismiss",
                                    palette.bg_hover,
                                    palette.text_secondary,
                                    cx.listener(|this, _, window, cx| this.close_mail(window, cx)),
                                ))
                                .child(small_btn(
                                    "send-agent-mail-btn",
                                    "Send message",
                                    palette.accent,
                                    palette.text_on_accent,
                                    cx.listener(|this, _, window, cx| {
                                        this.submit_agent_mail(window, cx);
                                    }),
                                )),
                        ),
                )
            },
        )
        .into_any_element()
}

#[allow(clippy::too_many_lines)]
fn render_harness_tab(
    view: &PrimeDesktopView,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let scope = view.harness_scope;
    let entries = view
        .harness
        .entries_for(scope)
        .iter()
        .filter(|e| view.harness_kind_filter.is_none_or(|k| e.kind == k))
        .collect::<Vec<_>>();
    let kinds: [(Option<RefinementKind>, &str); 5] = [
        (None, "All"),
        (Some(RefinementKind::Memory), "Memory"),
        (Some(RefinementKind::Prompt), "Prompt"),
        (Some(RefinementKind::Skill), "Skill"),
        (Some(RefinementKind::Subagent), "Subagent"),
    ];

    div()
        .flex()
        .flex_col()
        .gap(px(11.0))
        // Scope Switcher (`Global` vs `Local`) + `+ New`
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .p(px(3.0))
                        .rounded_lg()
                        .bg(palette.bg_code)
                        .children(
                            [
                                (HarnessScope::Global, "Global"),
                                (HarnessScope::Local, "Session Local"),
                            ]
                            .into_iter()
                            .map(|(sc, lbl)| {
                                let active = sc == scope;
                                div()
                                    .id(SharedString::from(format!("h-scope-{lbl}")))
                                    .px(px(9.0))
                                    .py(px(3.5))
                                    .rounded_md()
                                    .cursor_pointer()
                                    .bg(if active {
                                        palette.bg_elevated
                                    } else {
                                        gpui::transparent_black()
                                    })
                                    .text_size(px(11.5))
                                    .text_color(if active {
                                        palette.text_primary
                                    } else {
                                        palette.text_muted
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.harness_scope = sc;
                                        cx.notify();
                                    }))
                                    .child(lbl)
                            }),
                        ),
                )
                .child(small_btn(
                    "harness-new-entry-btn",
                    "+ Entry",
                    palette.accent,
                    palette.text_on_accent,
                    cx.listener(|this, _, window, cx| {
                        this.open_new_harness_editor(window, cx);
                    }),
                )),
        )
        // `✦ Refine` Bar
        .child(
            div()
                .p(px(9.0))
                .rounded_xl()
                .border_1()
                .border_color(palette.border_subtle)
                .bg(palette.bg_elevated)
                .flex()
                .items_center()
                .gap(px(7.0))
                .child(Icon::Sparkles.render(13.5, palette.accent))
                .child(div().flex_1().min_w_0().child(view.refine_input.clone()))
                .child(small_btn(
                    "trigger-refine-btn",
                    "Refine",
                    palette.accent_soft,
                    palette.accent,
                    cx.listener(|this, _, _, cx| this.submit_refinement(cx)),
                )),
        )
        // Inline Create / Edit Form
        .when(view.harness_editor_open, |d| {
            d.child(
                div()
                    .p(px(10.0))
                    .rounded_xl()
                    .border_1()
                    .border_color(palette.accent)
                    .bg(palette.bg_elevated)
                    .flex()
                    .flex_col()
                    .gap(px(7.0))
                    .child(
                        div().flex().items_center().gap(px(4.0)).children(
                            [
                                (RefinementKind::Memory, "memory"),
                                (RefinementKind::Prompt, "prompt"),
                                (RefinementKind::Skill, "skill"),
                                (RefinementKind::Subagent, "subagent"),
                            ]
                            .into_iter()
                            .map(|(k, name)| {
                                let active = view.harness_editor_kind == k;
                                div()
                                    .id(SharedString::from(format!("edit-kind-{name}")))
                                    .px(px(7.0))
                                    .py(px(2.5))
                                    .rounded_md()
                                    .cursor_pointer()
                                    .bg(if active {
                                        palette.accent_soft
                                    } else {
                                        palette.bg_code
                                    })
                                    .font_family(font_mono())
                                    .text_size(px(11.0))
                                    .text_color(if active {
                                        palette.accent
                                    } else {
                                        palette.text_muted
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.harness_editor_kind = k;
                                        cx.notify();
                                    }))
                                    .child(name)
                            }),
                        ),
                    )
                    .child(
                        div()
                            .px(px(8.0))
                            .py(px(5.0))
                            .rounded_md()
                            .bg(palette.bg_code)
                            .child(view.harness_title_input.clone()),
                    )
                    .child(
                        div()
                            .p(px(8.0))
                            .rounded_md()
                            .bg(palette.bg_code)
                            .child(view.harness_body_input.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(6.0))
                            .child(small_btn(
                                "cancel-harness-edit",
                                "Cancel",
                                palette.bg_hover,
                                palette.text_secondary,
                                cx.listener(|this, _, window, cx| this.close_harness_editor(window, cx)),
                            ))
                            .child(small_btn(
                                "save-harness-edit",
                                "Save entry",
                                palette.accent,
                                palette.text_on_accent,
                                cx.listener(|this, _, window, cx| this.save_harness_editor(window, cx)),
                            )),
                    ),
            )
        })
        // Kind Filter Pills
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(4.0))
                .children(kinds.into_iter().map(|(opt_k, lbl)| {
                    let active = view.harness_kind_filter == opt_k;
                    div()
                        .id(SharedString::from(format!("h-filter-{lbl}")))
                        .px(px(7.5))
                        .py(px(2.5))
                        .rounded_md()
                        .cursor_pointer()
                        .bg(if active {
                            palette.accent_soft
                        } else {
                            palette.bg_elevated
                        })
                        .text_size(px(11.0))
                        .text_color(if active {
                            palette.accent
                        } else {
                            palette.text_secondary
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.harness_kind_filter = opt_k;
                            cx.notify();
                        }))
                        .child(lbl)
                })),
        )
        // Entries List
        .when(entries.is_empty(), |d| {
            d.child(empty_box(
                "No continual harness entries in this scope yet.",
                palette,
            ))
        })
        .children(entries.into_iter().map(|entry| {
            let eid = entry.id.clone();
            let eid_del = eid.clone();
            let sid_del = view.session.active_session_id.clone();
            let msg = format!("Delete the harness entry “{}”?", entry.title);
            let kind = entry.kind;
            let title_clone = entry.title.clone();
            let content_clone = entry.content.clone();
            let kind_str = format!("{:?}", entry.kind).to_lowercase();

            div()
                .p(px(10.0))
                .rounded_xl()
                .border_1()
                .border_color(palette.border_subtle)
                .bg(palette.bg_elevated)
                .flex()
                .flex_col()
                .gap(px(5.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(6.0))
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_size(px(12.5))
                                .text_color(palette.text_primary)
                                .child(truncate_chars(&entry.title, 34)),
                        )
                        .child(pill(
                            &format!("{kind_str} v{}", entry.version),
                            palette.info_soft,
                            palette.info,
                        )),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .line_height(px(17.0))
                        .text_color(palette.text_secondary)
                        .child(truncate_chars(&entry.content, 160)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(px(6.0))
                        .child(small_btn(
                            &format!("edit-he-{eid}"),
                            "Edit",
                            palette.bg_hover,
                            palette.text_secondary,
                            cx.listener(move |this, _, window, cx| {
                                this.edit_existing_harness_entry(
                                    kind,
                                    &eid,
                                    &title_clone,
                                    &content_clone,
                                    window,
                                    cx,
                                );
                            }),
                        ))
                        .child(small_btn(
                            &format!("del-he-{eid_del}"),
                            "Delete",
                            palette.danger_soft,
                            palette.danger,
                            cx.listener(move |_this, _, window, cx| {
                                let id = eid_del.clone(); let sid = sid_del.clone();
                                confirm_action(&msg, "Delete entry", window, cx, move |this, cx| {
                                    if this.session.active_session_id == sid && this.harness_scope == scope {
                                        this.remove_harness_entry(kind, &id, cx);
                                    }
                                });
                            }),
                        )),
                )
        }))
        // Refinement History Timeline
        .when(!view.harness.history.is_empty(), |d| {
            let sid_rb = view.session.active_session_id.clone();
            d.child(
                div()
                    .pt(px(6.0))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(label_caps("RECENT REFINEMENTS", palette))
                    .children(view.harness.history.iter().take(6).map(|rev| {
                        let rb_id = rev.id.clone();
                        let sid_call = sid_rb.clone();
                        let global = rev.scope == Some(HarnessScope::Global);
                        div()
                            .p(px(9.0))
                            .rounded_lg()
                            .border_1()
                            .border_color(palette.border_subtle)
                            .bg(palette.bg_elevated)
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .font_family(font_mono())
                                            .text_size(px(10.5))
                                            .text_color(palette.accent)
                                            .child(truncate_chars(&rev.id, 24)),
                                    )
                                    .when(!sid_call.is_empty() && rev.scope.is_some(), |row| {
                                        row.child(small_btn(
                                            &format!("rb-{}", rev.id),
                                            "Rollback",
                                            palette.bg_hover,
                                            palette.text_muted,
                                            cx.listener(move |_this, _, window, cx| {
                let request = BridgeRequest::Refine {
                                                    active_session_id: sid_call.clone(),
                                                    instructions: None,
                                                    rollback_id: Some(rb_id.clone()),
                                                    global,
                                                };
                confirm_action(&format!("Roll back refinement {rb_id}? This changes saved harness entries."), "Roll back", window, cx, move |this, _| this.bridge.send(request));
            }),
                                        ))
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(palette.text_secondary)
                                    .child(truncate_chars(&rev.summary, 110)),
                            )
                    })),
            )
        })
        .into_any_element()
}

#[allow(clippy::too_many_lines)]
fn render_runtime_tab(
    view: &PrimeDesktopView,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(12.0))
        // Quick `!bash` Runner
        .child(
            div()
                .p(px(10.0))
                .rounded_xl()
                .border_1()
                .border_color(palette.border_subtle)
                .bg(palette.bg_elevated)
                .flex()
                .flex_col()
                .gap(px(7.0))
                .child(label_caps("RUN SHELL COMMAND IN SESSION CWD", palette))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .flex_1()
                                .px(px(8.0))
                                .py(px(4.5))
                                .rounded_md()
                                .bg(palette.bg_code)
                                .child(view.quick_bash_input.clone()),
                        )
                        .child(small_btn(
                            "run-quick-bash-btn",
                            "Run",
                            palette.accent,
                            palette.text_on_accent,
                            cx.listener(|this, _, _, cx| this.submit_quick_bash(cx)),
                        )),
                ),
        )
        // Live Kernel & Persistent Bash Shells
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(label_caps(
                    &format!("KERNEL & PERSISTENT SHELLS ({})", view.shells.len()),
                    palette,
                ))
                .when(view.shells.is_empty(), |d| {
                    d.child(empty_box(
                        "No background or persistent `bash()` shells running.",
                        palette,
                    ))
                })
                .children(view.shells.iter().map(|sh| {
                    let sh_id = sh.id.clone();
                    let sh_id_kill = sh.id.clone();
                    let persist = sh.persistent;
                    let sid_tail = view.session.active_session_id.clone();
                    let sid_kill = sid_tail.clone();
                    let tail_txt = view
                        .shell_tails
                        .get(&sh.id)
                        .or(sh.tail_preview.as_ref())
                        .cloned();

                    div()
                        .p(px(10.0))
                        .rounded_xl()
                        .border_1()
                        .border_color(palette.border_subtle)
                        .bg(palette.bg_elevated)
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .font_family(font_mono())
                                        .text_size(px(11.5))
                                        .text_color(palette.text_primary)
                                        .child(truncate_chars(&sh.command, 32)),
                                )
                                .child(pill(
                                    if sh.persistent {
                                        "persistent"
                                    } else if sh.running {
                                        "running"
                                    } else {
                                        "exited"
                                    },
                                    if sh.running {
                                        palette.warning_soft
                                    } else {
                                        palette.bg_hover
                                    },
                                    if sh.running {
                                        palette.warning
                                    } else {
                                        palette.text_muted
                                    },
                                )),
                        )
                        .when_some(tail_txt, |d, out| {
                            d.child(
                                div()
                                    .p(px(7.0))
                                    .rounded_md()
                                    .bg(palette.bg_code)
                                    .font_family(font_mono())
                                    .text_size(px(11.0))
                                    .text_color(palette.text_secondary)
                                    .child(truncate_chars(&out, 240)),
                            )
                        })
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap(px(6.0))
                                .child(small_btn(
                                    &format!("tail-sh-{sh_id}"),
                                    "Tail",
                                    palette.bg_hover,
                                    palette.text_secondary,
                                    cx.listener(move |this, _, _, _cx| {
                                        this.bridge.send(BridgeRequest::TailShell {
                                            active_session_id: sid_tail.clone(),
                                            id: sh_id.clone(),
                                            persistent: persist,
                                        });
                                    }),
                                ))
                                .when(sh.running, |d| {
                                    d.child(small_btn(
                                        &format!("kill-sh-{sh_id_kill}"),
                                        "Kill",
                                        palette.danger_soft,
                                        palette.danger,
                                        cx.listener(move |_this, _, window, cx| {
                let request = BridgeRequest::KillShell {
                                                active_session_id: sid_kill.clone(),
                                                id: sh_id_kill.clone(),
                                                persistent: persist,
                                            };
                confirm_action(&format!("Stop shell {sh_id_kill}? The process will be terminated."), "Stop shell", window, cx, move |this, _| this.bridge.send(request));
            }),
                                    ))
                                }),
                        )
                })),
        )
        // Heartbeats & Cron Schedules
        .child(
            div()
                .p(px(10.0))
                .rounded_xl()
                .border_1()
                .border_color(palette.border_subtle)
                .bg(palette.bg_elevated)
                .flex()
                .flex_col()
                .gap(px(7.0))
                .child(label_caps(
                    &format!("HEARTBEATS & SCHEDULES ({})", view.schedules.len()),
                    palette,
                ))
                .child(
                    div()
                        .flex()
                        .gap(px(6.0))
                        .child(
                            div()
                                .w(px(104.0))
                                .px(px(7.0))
                                .py(px(4.5))
                                .rounded_md()
                                .bg(palette.bg_code)
                                .child(view.schedule_interval_input.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .px(px(7.0))
                                .py(px(4.5))
                                .rounded_md()
                                .bg(palette.bg_code)
                                .child(view.schedule_prompt_input.clone()),
                        ),
                )
                .child(div().flex().justify_end().child(small_btn(
                    "add-heartbeat-btn",
                    "+ Heartbeat",
                    palette.accent_soft,
                    palette.accent,
                    cx.listener(|this, _, _, cx| this.submit_heartbeat(cx)),
                )))
                .children(view.schedules.iter().map(|job| {
                    let jid = job.id.clone();
                    let is_hb = job.is_heartbeat;
                    let sid_job = view.session.active_session_id.clone();
                    div()
                        .pt(px(6.0))
                        .border_t_1()
                        .border_color(palette.border_subtle)
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(6.0))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .overflow_hidden()
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(palette.text_primary)
                                        .child(truncate_chars(&job.label, 30)),
                                )
                                .child(
                                    div()
                                        .font_family(font_mono())
                                        .text_size(px(10.5))
                                        .text_color(palette.text_muted)
                                        .child(job.schedule.clone()),
                                ),
                        )
                        .child(small_btn(
                            &format!("rm-job-{jid}"),
                            "Remove",
                            palette.danger_soft,
                            palette.danger,
                            cx.listener(move |_this, _, window, cx| {
                                let request = if is_hb { BridgeRequest::ManageHeartbeat {
                                    active_session_id: sid_job.clone(), job_id: jid.clone(), action: "delete".to_string(),
                                } } else { BridgeRequest::CancelCron { active_session_id: Some(sid_job.clone()), job_id: jid.clone() } };
                                confirm_action(&format!("Remove schedule {jid}?"), "Remove", window, cx, move |this, _| this.bridge.send(request));
                            }),
                        ))
                })),
        )
        .into_any_element()
}

#[allow(clippy::too_many_lines)]
fn render_system_tab(
    view: &PrimeDesktopView,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let t = &view.telemetry;
    let sid = view.session.active_session_id.clone();
    let sid_compact_policy = sid.clone();
    let sid_retry_policy = sid.clone();
    let missing = "Not reported";

    div()
        .flex()
        .flex_col()
        .gap(px(12.0))
        // Telemetry Summary Card
        .child(
            div()
                .p(px(11.0))
                .rounded_xl()
                .border_1()
                .border_color(palette.border_subtle)
                .bg(palette.bg_elevated)
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(label_caps("SESSION TELEMETRY", palette))
                .child(stat_row(
                    "Context window",
                    &match (t.context_tokens, t.context_window) {
                        (Some(tokens), Some(window)) => format!(
                            "{} / {}",
                            format_tokens_compact(tokens),
                            format_tokens_compact(window)
                        ),
                        (None, Some(window)) => {
                            format!("{} capacity", format_tokens_compact(window))
                        }
                        (Some(tokens), None) => format!("{} used", format_tokens_compact(tokens)),
                        (None, None) => missing.to_string(),
                    },
                    palette,
                ))
                .child(stat_row(
                    "Input / Output",
                    &format!(
                        "{} in · {} out",
                        t.total_input_tokens
                            .map_or_else(|| "—".to_owned(), format_tokens_compact),
                        t.total_output_tokens
                            .map_or_else(|| "—".to_owned(), format_tokens_compact)
                    ),
                    palette,
                ))
                .child(stat_row(
                    "Cache read",
                    &t.cache_read_tokens
                        .map_or_else(|| missing.to_string(), format_tokens_compact),
                    palette,
                ))
                .child(stat_row(
                    "Session cost",
                    &t.total_cost_usd
                        .map_or_else(|| missing.to_string(), |cost| format!("${cost:.4}")),
                    palette,
                ))
                .child(
                    div()
                        .pt(px(4.0))
                        .flex()
                        .justify_between()
                        .gap(px(6.0))
                        .child(small_btn(
                            "sys-compact-now",
                            "Compact context",
                            palette.accent_soft,
                            palette.accent,
                            cx.listener(|this, _, _, cx| {
                                this.trigger_compact(cx);
                            }),
                        ))
                        .child(small_btn(
                            "sys-reload-session",
                            "Reload session",
                            palette.bg_hover,
                            palette.text_secondary,
                            cx.listener(move |this, _, _, _cx| {
                                if !sid.is_empty() {
                                    this.bridge.send(BridgeRequest::ReloadSession {
                                        active_session_id: sid.clone(),
                                    });
                                }
                            }),
                        )),
                ),
        )
        // Session Policies
        .child(
            div()
                .p(px(11.0))
                .rounded_xl()
                .border_1()
                .border_color(palette.border_subtle)
                .bg(palette.bg_elevated)
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(label_caps("POLICIES", palette))
                .when_some(t.auto_compaction, |d, enabled| {
                    d.child(toggle_row(
                        "toggle-autocompact",
                        "Auto-compaction",
                        enabled,
                        palette,
                        cx.listener(move |this, _, _, _cx| {
                            if !sid_compact_policy.is_empty() {
                                this.bridge.send(BridgeRequest::SetAutoCompaction {
                                    active_session_id: sid_compact_policy.clone(),
                                    enabled: !enabled,
                                });
                            }
                        }),
                    ))
                })
                .when_some(t.auto_retry, |d, enabled| {
                    d.child(toggle_row(
                        "toggle-autoretry",
                        "Auto-retry on transient errors",
                        enabled,
                        palette,
                        cx.listener(move |this, _, _, _cx| {
                            if !sid_retry_policy.is_empty() {
                                this.bridge.send(BridgeRequest::SetAutoRetry {
                                    active_session_id: sid_retry_policy.clone(),
                                    enabled: !enabled,
                                });
                            }
                        }),
                    ))
                })
                .when(t.auto_retry.is_none() && t.auto_compaction.is_none(), |d| {
                    d.child(
                        div()
                            .text_size(px(12.0))
                            .text_color(palette.text_secondary)
                            .child(missing),
                    )
                }),
        )
        // Provider Auth Summary
        .child(
            div()
                .p(px(11.0))
                .rounded_xl()
                .border_1()
                .border_color(palette.border_subtle)
                .bg(palette.bg_elevated)
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(label_caps("PROVIDERS", palette))
                        .child(small_btn(
                            "sys-open-auth-modal",
                            "Manage keys",
                            palette.accent_soft,
                            palette.accent,
                            cx.listener(|this, _, window, cx| {
                                this.open_modal(ActiveModal::SettingsAuth, window, cx);
                            }),
                        )),
                )
                .children(view.provider_auth.iter().take(5).map(|item| {
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(palette.text_primary)
                                .child(item.display_name.clone()),
                        )
                        .child(pill(
                            &item.auth_kind,
                            if item.configured {
                                palette.success_soft
                            } else {
                                palette.bg_hover
                            },
                            if item.configured {
                                palette.success
                            } else {
                                palette.text_muted
                            },
                        ))
                })),
        )
        // MCP Connections
        .child(
            div()
                .p(px(11.0))
                .rounded_xl()
                .border_1()
                .border_color(palette.border_subtle)
                .bg(palette.bg_elevated)
                .flex()
                .flex_col()
                .gap(px(7.0))
                .child(label_caps(
                    &format!("MCP SERVERS ({})", view.mcp_rows.len()),
                    palette,
                ))
                .children(view.mcp_rows.iter().map(|mcp| {
                    let srv = mcp.server.clone();
                    let sid_rm = view.session.active_session_id.clone();
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .font_family(font_mono())
                                .text_size(px(12.0))
                                .text_color(palette.text_primary)
                                .child(mcp.server.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.0))
                                .child(pill(&mcp.status, palette.bg_hover, palette.text_secondary))
                                .child(small_btn(
                                    &format!("rm-mcp-{srv}"),
                                    "×",
                                    palette.danger_soft,
                                    palette.danger,
                                    cx.listener(move |_this, _, window, cx| {
                                        let request = BridgeRequest::RemoveMcp {
                                            active_session_id: sid_rm.clone(),
                                            server: srv.clone(),
                                        };
                                        confirm_action(
                                            &format!("Remove the connection to {srv}?"),
                                            "Remove",
                                            window,
                                            cx,
                                            move |this, _| this.bridge.send(request),
                                        );
                                    }),
                                )),
                        )
                }))
                .child(
                    div()
                        .flex()
                        .gap(px(5.0))
                        .child(
                            div()
                                .w(px(108.0))
                                .px(px(7.0))
                                .py(px(4.0))
                                .rounded_md()
                                .bg(palette.bg_code)
                                .child(view.mcp_server_input.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .px(px(7.0))
                                .py(px(4.0))
                                .rounded_md()
                                .bg(palette.bg_code)
                                .child(view.mcp_token_input.clone()),
                        )
                        .child(small_btn(
                            "add-mcp-token-btn",
                            "Add",
                            palette.accent_soft,
                            palette.accent,
                            cx.listener(|this, _, _, cx| this.submit_mcp_token(cx)),
                        )),
                ),
        )
        // Assembled System Prompt Inspector
        .when_some(view.system_prompt.as_ref(), |d, prompt| {
            let prompt_copy = prompt.clone();
            d.child(
                div()
                    .p(px(11.0))
                    .rounded_xl()
                    .border_1()
                    .border_color(palette.border_subtle)
                    .bg(palette.bg_elevated)
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(label_caps(
                                &format!("SYSTEM PROMPT ({} chars)", prompt.len()),
                                palette,
                            ))
                            .child(
                                div()
                                    .id("copy-sys-prompt")
                                    .px(px(7.0))
                                    .py(px(2.5))
                                    .rounded_md()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(palette.bg_hover))
                                    .font_family(font_mono())
                                    .text_size(px(11.0))
                                    .text_color(palette.accent)
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            prompt_copy.clone(),
                                        ));
                                    })
                                    .child("Copy"),
                            ),
                    )
                    .child(
                        div()
                            .p(px(8.0))
                            .rounded_md()
                            .bg(palette.bg_code)
                            .font_family(font_mono())
                            .text_size(px(11.0))
                            .line_height(px(16.0))
                            .text_color(palette.text_secondary)
                            .child(truncate_chars(prompt, 420)),
                    ),
            )
        })
        .into_any_element()
}

fn label_caps(text: &str, palette: Palette) -> AnyElement {
    div()
        .font_family(font_mono())
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(10.5))
        .text_color(palette.text_muted)
        .child(text.to_string())
        .into_any_element()
}

fn empty_box(text: &str, palette: Palette) -> AnyElement {
    div()
        .p(px(11.0))
        .rounded_xl()
        .border_1()
        .border_color(palette.border_subtle)
        .bg(palette.bg_elevated.opacity(0.6))
        .text_size(px(12.0))
        .line_height(px(17.5))
        .text_color(palette.text_muted)
        .font_family(font_sans())
        .child(text.to_string())
        .into_any_element()
}

fn stat_row(label: &str, value: &str, palette: Palette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(px(12.0))
                .text_color(palette.text_secondary)
                .child(label.to_string()),
        )
        .child(
            div()
                .font_family(font_mono())
                .text_size(px(12.0))
                .text_color(palette.text_primary)
                .truncate()
                .min_w_0()
                .child(value.to_string()),
        )
        .into_any_element()
}

fn toggle_row(
    id: &str,
    label: &str,
    enabled: bool,
    palette: Palette,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> AnyElement {
    div()
        .id(SharedString::from(id.to_string()))
        .flex()
        .items_center()
        .justify_between()
        .cursor_pointer()
        .on_click(on_click)
        .child(
            div()
                .text_size(px(12.0))
                .text_color(palette.text_primary)
                .child(label.to_string()),
        )
        .child(pill(
            if enabled { "ON" } else { "OFF" },
            if enabled {
                palette.success_soft
            } else {
                palette.bg_hover
            },
            if enabled {
                palette.success
            } else {
                palette.text_muted
            },
        ))
        .into_any_element()
}

fn small_btn(
    id: &str,
    label: &str,
    bg: gpui::Hsla,
    fg: gpui::Hsla,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(id.to_string()))
        .flex_none()
        .whitespace_nowrap()
        .px(px(8.5))
        .py(px(4.0))
        .rounded_md()
        .bg(bg)
        .hover(|s| s.opacity(0.88))
        .cursor_pointer()
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .on_click(on_click)
        .child(label.to_string())
}
