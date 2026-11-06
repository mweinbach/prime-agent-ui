//! The conversation stage and its Rune timeline. Offscreen turns are built on demand.

use std::borrow::Cow;

use rune::{Cx, El, Family, GrowthSpec, ScrollSpec, TextStyle, col, el, row, text};

use super::composer::composer;
use super::markdown::render_markdown;
use super::selectable_text::{control, copy_button, icon_control, label};
use super::text_input::field;
use super::tool_cards::{console, disclosure, pill, render_tool_card};
use super::workspace::PrimeDesktopView;
use crate::assets::Icon;
use crate::bridge::BridgeRequest;
use crate::state::{
    format_tokens_compact,
    projects::folder_label,
    transcript::{AssistantBlock, MailKind, TranscriptEntry},
};
use crate::theme::{DetailLevel, InspectorTab, Palette, font_display};

impl PrimeDesktopView {
    pub(super) fn stage(&self, cx: &mut Cx<Self>) -> El {
        let p = Palette::for_mode(self.prefs.theme);
        let mut stage = col()
            .key("workspace-stage")
            .grow(1.)
            .h_full()
            .min_h(0.)
            .min_w(0.)
            .fill(p.bg_canvas)
            .child(self.stage_header(cx, p))
            .child(el().w_full().h(1.).fill(p.border_subtle));
        if let Some(goal) = self
            .session
            .goal
            .as_ref()
            .filter(|g| g.active || g.objective.is_some())
        {
            let usage = format_tokens_compact(goal.tokens_used);
            let status = match goal.token_budget.filter(|b| *b > 0) {
                Some(budget) => format!(
                    "{} · {usage} / {}",
                    goal.status.slug(),
                    format_tokens_compact(budget)
                ),
                None => format!("{} · {usage}", goal.status.slug()),
            };
            stage = stage.child(
                row()
                    .w_full()
                    .center_y()
                    .gap(8.)
                    .px(18.)
                    .py(7.)
                    .fill(p.accent_soft)
                    .child(Icon::Target.render(14., p.accent))
                    .child(
                        label(
                            goal.objective.as_deref().unwrap_or("Active goal"),
                            12.5,
                            p.text_primary,
                        )
                        .grow(1.)
                        .min_w(0.)
                        .truncate(),
                    )
                    .child(label(status, 11., p.accent)),
            );
        }
        if self.session.side_question.open {
            stage = stage.child(self.side_question_bar(p));
        }
        if self.session.entries.is_empty() && !self.session.is_streaming {
            return stage.child(self.welcome(p, cx));
        }
        let viewport = self.transcript_viewport_key();
        // Rune retains the reading position by turn key and follows streaming text only
        // while the reader is at the bottom. A newly attached session gets a new viewport.
        let count = self.session.entries.len() + usize::from(self.session.is_streaming);
        let turns = col()
            .key("transcript-rows")
            .growth(GrowthSpec::default())
            .virtual_rows(count, 90., turn_key, move |s: &Self, i| {
                let content = s.session.entries.get(i).map_or_else(
                    || {
                        row()
                            .key("live-working-row")
                            .center_y()
                            .gap(8.)
                            .child(el().size(7., 7.).radius(4.).fill(p.accent))
                            .child(label(
                                s.session
                                    .working_note
                                    .as_deref()
                                    .unwrap_or("Prime Agent is working…"),
                                12.5,
                                p.text_secondary,
                            ))
                    },
                    |entry| s.turn(entry, p),
                );
                col()
                    .w_full()
                    .items_center()
                    .px(24.)
                    .py(7.)
                    .child(col().w_full().max_w(704.).min_w(0.).child(content))
            });
        stage = stage.child(
            col()
                .key(&viewport)
                .grow(1.)
                .min_h(0.)
                .w_full()
                .scroll(ScrollSpec { stick_bottom: true })
                .scroll_anchor()
                .on_at_bottom(|s: &mut Self, at_bottom| s.follow_tail = at_bottom)
                .child(turns),
        );
        if !self.follow_tail {
            stage = stage.child(
                control("jump-to-bottom", "Jump to bottom", p)
                    .self_center()
                    .fill(p.bg_elevated)
                    .child(Icon::ArrowDown.render(14., p.text_secondary))
                    .child(label("Jump to bottom", 12., p.text_secondary))
                    .on_press_tx(move |s: &mut Self, tx| {
                        s.follow_tail = true;
                        tx.scroll_to_bottom(&viewport);
                    }),
            );
        }
        stage.child(
            col()
                .w_full()
                .items_center()
                .px(24.)
                .pt(8.)
                .pb(16.)
                .child(col().w_full().max_w(704.).child(composer(self, p, cx))),
        )
    }

    fn stage_header(&self, cx: &Cx<Self>, p: Palette) -> El {
        let attached = !self.session.active_session_id.is_empty();
        let mut left = row().grow(1.).min_w(0.).items_center().gap(4.);
        if !self.prefs.sidebar_open {
            left = left
                .child(
                    icon_control("stage-open-sidebar-btn", "Show sidebar", Icon::Sidebar, p)
                        .on_press(Self::toggle_sidebar),
                )
                .child(
                    icon_control("stage-new-chat", "New chat", Icon::Compose, p)
                        .on_press(Self::start_fresh_session),
                );
        }
        if let Some(parent) = self.parent_session_target() {
            left = left.child(
                control("subagent-parent", "Parent chat", p)
                    .child(Icon::GitBranch.render(13., p.text_muted))
                    .child(label("Parent chat", 12., p.text_secondary))
                    .on_press(move |s: &mut Self| {
                        s.open_session(BridgeRequest::AttachSession {
                            active_session_id: parent.clone(),
                        });
                    }),
            );
        } else {
            let cwd = self.current_project();
            left = left.child(
                control("stage-project-folder", "Show project", p)
                    .help(format!("Show project: {cwd}"))
                    .disabled(cwd.is_empty())
                    .child(Icon::Folder.render(13., p.text_muted))
                    .child(
                        label(folder_label(cwd), 12., p.text_secondary)
                            .max_w(130.)
                            .truncate(),
                    )
                    .on_press(Self::reveal_current_project),
            );
        }
        left = left.child(Icon::ChevronRight.render(12., p.text_muted));
        left = if self.renaming_session && attached {
            left.child(
                field("rename-input", &self.rename_input, "Session name", p, |s| {
                    &mut s.rename_input
                })
                .single_line()
                .grow(1.)
                .min_w(0.)
                .on_submit(|s: &mut Self, _| s.commit_rename_session(&s.rename_input.text.clone())),
            )
        } else if attached {
            let title = self.session.display_title();
            left.child(
                control("stage-session-title", "Rename chat", p)
                    .shrink()
                    .min_w(0.)
                    .help(title.clone())
                    .child(
                        label(title, 13., p.text_primary)
                            .font(TextStyle::new(13., 600.))
                            .shrink()
                            .min_w(0.)
                            .truncate(),
                    )
                    .child(Icon::EditPen.render(11., p.text_muted))
                    .on_press(Self::begin_rename_session),
            )
        } else {
            left.child(
                label("New chat", 13., p.text_primary)
                    .font(TextStyle::new(13., 550.))
                    .truncate(),
            )
        };

        let mut right = row().items_center().gap(4.);
        if !self.session.entries.is_empty() || self.session.is_streaming {
            right = right
                .child(
                    control("stage-detail-pill", "Change transcript detail", p)
                        .radius(8.)
                        .border(1., p.border_subtle)
                        .child(label(
                            self.prefs.detail_level.label(),
                            12.,
                            p.text_secondary,
                        ))
                        .on_press(Self::cycle_detail_level),
                )
                .child(
                    control("stage-side-question-btn", "Side question", p)
                        .radius(8.)
                        .border(1., p.border_subtle)
                        .child(label("/btw", 12., p.text_secondary))
                        .on_press(Self::toggle_side_question),
                );
        }
        if attached && !self.rlm_children.rows.is_empty() {
            right = right.child(
                control("session-subagents", "Inspect subagents", p)
                    .child(Icon::GitBranch.render(13., p.text_secondary))
                    .child(label(
                        format!("Subagents {}", self.rlm_children.rows.len()),
                        12.,
                        p.text_secondary,
                    ))
                    .on_press(|s: &mut Self| s.set_inspector_tab(InspectorTab::Agents)),
            );
        }
        let inspect = control("stage-toggle-inspector-btn", "Toggle inspector", p)
            .radius(8.)
            .px(9.)
            .border(1., p.border_subtle)
            .child(Icon::Layers.render(14., p.text_secondary))
            .child(label("Inspector", 12., p.text_secondary))
            .on_press(Self::toggle_inspector);
        right = right.child(if self.prefs.inspector_open {
            inspect.fill(p.bg_selected)
        } else {
            inspect
        });
        let inset = if self.prefs.sidebar_open {
            0.
        } else {
            cx.titlebar_inset()
        };
        row()
            .w_full()
            .h(52.)
            .px(15.)
            .pl(inset.max(15.))
            .items_center()
            .gap(8.)
            .child(left)
            .child(right)
    }

    fn welcome(&self, p: Palette, cx: &Cx<Self>) -> El {
        let mark = row()
            .key("welcome-mark")
            .size(44., 44.)
            .radius(13.)
            .fill(p.accent_soft)
            .center()
            .child(Icon::Prime.render(28., p.accent));
        let heading = col().w_full().items_start().gap(15.).child(mark).child(
            text("What are we working on?")
                .font(
                    TextStyle::new(39., 480.)
                        .family(Family::Named(font_display()))
                        .lh(46.)
                        .tracking(-1.3),
                )
                .color(p.text_primary)
                .min_w(0.)
                .wrap(),
        );
        let mut starters = row().min_h(32.).gap(8.).flex_wrap();
        if self.composer_input.text.is_empty() && self.staged_images.is_empty() {
            for (key, icon, title, prompt) in [
                (
                    "explore-project",
                    Icon::Layers,
                    "Explore this project",
                    "Walk me through this project's structure and its main entry points.",
                ),
                (
                    "review-changes",
                    Icon::GitBranch,
                    "Review changes",
                    "Review my uncommitted changes. What should I look at before committing?",
                ),
            ] {
                starters = starters.child(
                    control(key, title, p)
                        .h(34.)
                        .px(11.)
                        .gap(7.)
                        .radius(10.)
                        .border(1., p.border_strong)
                        .child(icon.render(15., p.accent))
                        .child(label(title, 13., p.text_primary))
                        .on_press(move |s: &mut Self| s.populate_draft(prompt)),
                );
            }
        }
        let content = col()
            .w_full()
            .max_w(704.)
            .gap(20.)
            .child(heading)
            .child(composer(self, p, cx))
            .child(starters);
        col()
            .key("welcome-stage")
            .grow(1.)
            .min_h(0.)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .items_center()
            .px(24.)
            .py(20.)
            .child(el().grow(1.))
            .child(content)
            .child(el().grow(1.6))
    }

    fn side_question_bar(&self, p: Palette) -> El {
        let q = &self.session.side_question;
        let mut form = row()
            .w_full()
            .items_center()
            .gap(6.)
            .child(label("/btw", 13., p.info))
            .child(
                field(
                    "side-question-input",
                    &self.side_question_input,
                    "Ask a side question…",
                    p,
                    |s| &mut s.side_question_input,
                )
                .single_line()
                .grow(1.)
                .on_submit(|s: &mut Self, _| s.submit_side_question()),
            )
            .child(
                control("sq-ask-btn", "Ask side question", p)
                    .fill(p.info_soft)
                    .child(label("Ask", 12., p.info))
                    .on_press(Self::submit_side_question),
            );
        if let Some(id) = &q.running_id {
            let id = id.clone();
            form = form.child(
                control("sq-stop-btn", "Stop side question", p)
                    .child(label("Stop", 12., p.danger))
                    .on_press(move |s: &mut Self| {
                        s.bridge.send(BridgeRequest::AbortSideQuestion {
                            active_session_id: s.session.active_session_id.clone(),
                            side_question_id: id.clone(),
                        });
                    }),
            );
        }
        let mut bar = col()
            .w_full()
            .px(18.)
            .py(6.)
            .gap(5.)
            .fill(p.bg_elevated)
            .child(
                form.child(
                    icon_control("sq-close", "Close side question", Icon::Close, p)
                        .on_press(Self::toggle_side_question),
                ),
            );
        if !q.answer.is_empty() {
            bar = bar
                .child(
                    col()
                        .key("sq-answer-scroll")
                        .max_h(150.)
                        .min_h(22.)
                        .scroll(ScrollSpec { stick_bottom: true })
                        .child(render_markdown(
                            "sq-answer",
                            &q.answer,
                            q.running_id.is_some(),
                            p,
                        )),
                )
                .child(copy_button("sq-copy", &q.answer, p).self_end());
        }
        if let Some(error) = &q.error {
            bar = bar.child(label(error, 12., p.danger).wrap_anywhere().selectable(true));
        }
        bar
    }

    fn turn(&self, entry: &TranscriptEntry, p: Palette) -> El {
        match entry {
            TranscriptEntry::User {
                key,
                text,
                skill_name,
                image_count,
            } => {
                let mut bubble = col()
                    .key(key)
                    .self_end()
                    .max_w(640.)
                    .gap(5.)
                    .min_w(0.)
                    .shrink()
                    .corners([16., 16., 5., 16.])
                    .px(16.)
                    .py(11.)
                    .fill(p.bg_user_bubble)
                    .border(1., p.border_subtle);
                if let Some(skill) = skill_name {
                    bubble = bubble.child(
                        pill(&format!("skill · {skill}"), p.accent_soft, p.accent).self_start(),
                    );
                }
                if *image_count > 0 {
                    bubble = bubble.child(label(
                        format!("{image_count} image(s) attached"),
                        11.5,
                        p.info,
                    ));
                }
                if !text.is_empty() {
                    bubble = bubble.child(
                        label(text, 15., p.text_primary)
                            .key(format!("{key}-text"))
                            .font(TextStyle::new(15., 400.).lh(22.))
                            .wrap_anywhere()
                            .selectable(true)
                            .selection_color(p.selection_bg),
                    );
                }
                let mut line = row()
                    .hover_group()
                    .max_w(668.)
                    .min_w(0.)
                    .items_center()
                    .gap(6.);
                if !text.is_empty() {
                    line = line.child(
                        copy_button(&format!("{key}-copy"), text, p).group_opacity(0.32, 1.),
                    );
                }
                col().w_full().items_end().child(line.child(bubble))
            }
            TranscriptEntry::Assistant {
                key,
                blocks,
                streaming,
                error,
                ..
            } => {
                let mut message = col().w_full().min_w(0.).gap(8.);
                let mut reply = Vec::new();
                for (i, block) in blocks.iter().enumerate() {
                    match block {
                        AssistantBlock::Text(body) => {
                            reply.push(body.as_str());
                            message = message.child(render_markdown(
                                &format!("{key}-body-{i}"),
                                body,
                                *streaming,
                                p,
                            ));
                        }
                        AssistantBlock::Thinking(body) => {
                            let id = format!("{key}-think-{i}");
                            let open = self
                                .session
                                .is_expanded(&id, self.prefs.detail_level.show_thinking());
                            message = message.child(
                                disclosure(
                                    &id,
                                    if *streaming {
                                        "Thinking…"
                                    } else {
                                        "Thinking"
                                    },
                                    open,
                                    p,
                                )
                                .self_start(),
                            );
                            if open {
                                message = message.child(
                                    col()
                                        .px(12.)
                                        .child(render_markdown(&id, body, *streaming, p)),
                                );
                            }
                        }
                    }
                }
                if let Some(error) = error {
                    message =
                        message.child(label(error, 13., p.danger).wrap_anywhere().selectable(true));
                }
                let mut author = row()
                    .w_full()
                    .items_center()
                    .gap(7.)
                    .child(Icon::Prime.render(15., p.accent))
                    .child(
                        label("Prime Agent", 12.5, p.text_secondary)
                            .font(TextStyle::new(12.5, 600.)),
                    );
                if !reply.is_empty() && !streaming {
                    author = author.child(el().grow(1.)).child(
                        copy_button(&format!("{key}-copy"), &reply.join("\n\n"), p)
                            .group_opacity(0.4, 1.),
                    );
                }
                col()
                    .key(key)
                    .hover_group()
                    .gap(4.)
                    .w_full()
                    .min_w(0.)
                    .child(author)
                    .child(message)
            }
            TranscriptEntry::Tool(card) => render_tool_card(self, card, p),
            TranscriptEntry::UserBash {
                key,
                command,
                output,
                running,
                exit_code,
                cancelled,
            } => {
                let open = self.session.is_expanded(key, true);
                let status = if *running {
                    "Running…".into()
                } else if *cancelled {
                    "Cancelled".into()
                } else {
                    exit_code.map_or_else(|| "Done".into(), |code| format!("Exit {code}"))
                };
                let mut card = col()
                    .key(key)
                    .padding(8.)
                    .radius(8.)
                    .border(1., p.border_subtle)
                    .fill(p.bg_elevated)
                    .child(
                        row()
                            .center_y()
                            .justify_between()
                            .child(
                                disclosure(key, &format!("$ {command}"), open, p)
                                    .min_w(0.)
                                    .shrink(),
                            )
                            .child(label(status, 11.5, p.warning)),
                    );
                if open && !output.is_empty() {
                    card = card.child(console(
                        self,
                        &format!("{key}-output"),
                        "Output",
                        output,
                        None,
                        if self.prefs.detail_level.expand_tool_output() {
                            120
                        } else {
                            18
                        },
                        p,
                    ));
                }
                card
            }
            TranscriptEntry::AgentMail {
                key,
                kind,
                counterpart,
                body,
            } => {
                let headline = match kind {
                    MailKind::Received => format!("Agent message from {counterpart}"),
                    MailKind::Sent => format!("Agent message to {counterpart}"),
                    MailKind::ChildCompleted => format!("Subagent {counterpart} completed"),
                    MailKind::ChildCancelled => format!("Subagent {counterpart} cancelled"),
                    MailKind::ChildFailed => format!("Subagent {counterpart} failed"),
                };
                let open = self
                    .session
                    .is_expanded(key, self.prefs.detail_level != DetailLevel::Overview);
                let mut card = col()
                    .key(key)
                    .gap(6.)
                    .px(9.)
                    .py(5.)
                    .radius(7.)
                    .fill(p.info_soft)
                    .child(disclosure(key, &headline, open, p).self_start());
                if open {
                    card = card.child(render_markdown(&format!("{key}-mail"), body, false, p));
                }
                card
            }
            TranscriptEntry::Compaction {
                key,
                streaming,
                reason,
                summary,
                tokens_before,
            } => {
                let open = self.session.is_expanded(key, *streaming);
                let title = if *streaming {
                    format!("Compacting context ({reason})…")
                } else {
                    "Context compacted".into()
                };
                let mut header = row()
                    .center_y()
                    .gap(8.)
                    .child(disclosure(key, &title, open, p));
                if let Some(tokens) = tokens_before {
                    header = header.child(label(
                        format!("was {}", format_tokens_compact(*tokens)),
                        11.5,
                        p.text_muted,
                    ));
                }
                let mut card = col()
                    .key(key)
                    .gap(6.)
                    .padding(7.)
                    .radius(8.)
                    .fill(p.bg_sidebar)
                    .child(header);
                if open {
                    card = card.child(render_markdown(
                        &format!("{key}-summary"),
                        summary,
                        *streaming,
                        p,
                    ));
                }
                card
            }
            TranscriptEntry::Refinement {
                key,
                summary,
                changes,
            } => {
                let mut card = col()
                    .key(key)
                    .gap(5.)
                    .padding(10.)
                    .radius(8.)
                    .fill(p.accent_soft)
                    .child(label("Continual Harness refined", 12., p.accent))
                    .child(render_markdown(
                        &format!("{key}-summary"),
                        summary,
                        false,
                        p,
                    ));
                for change in changes {
                    card = card.child(
                        label(change, 12.5, p.text_secondary)
                            .wrap_anywhere()
                            .selectable(true),
                    );
                }
                card
            }
            TranscriptEntry::Notice {
                key,
                title,
                body,
                is_error,
            } => {
                let mut notice = col()
                    .key(key)
                    .gap(5.)
                    .padding(10.)
                    .radius(8.)
                    .fill(if *is_error {
                        p.danger_soft
                    } else {
                        p.bg_sidebar
                    })
                    .child(label(
                        title,
                        12.5,
                        if *is_error {
                            p.danger
                        } else {
                            p.text_secondary
                        },
                    ));
                if !body.is_empty() {
                    notice =
                        notice.child(render_markdown(&format!("{key}-notice"), body, false, p));
                }
                notice
            }
        }
    }
}

fn turn_key(s: &PrimeDesktopView, i: usize) -> Cow<'_, str> {
    match s.session.entries.get(i) {
        Some(
            TranscriptEntry::User { key, .. }
            | TranscriptEntry::Assistant { key, .. }
            | TranscriptEntry::UserBash { key, .. }
            | TranscriptEntry::AgentMail { key, .. }
            | TranscriptEntry::Compaction { key, .. }
            | TranscriptEntry::Refinement { key, .. }
            | TranscriptEntry::Notice { key, .. },
        ) => Cow::Borrowed(key),
        Some(TranscriptEntry::Tool(card)) => Cow::Owned(format!("tool-{}", card.id)),
        None => Cow::Borrowed("live-working-row"),
    }
}
