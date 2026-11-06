//! Session, harness, runtime and system controls in the right-hand drawer.

use pa_core::refinement::{HarnessScope, RefinementKind};
use rune::{Color, Cursor, Cx, El, ScrollSpec, TextStyle, col, el, row, text};

use crate::assets::Icon;
use crate::bridge::BridgeRequest;
use crate::state::{
    ActiveModal, ActiveSessionRow, RlmChildRow, format_tokens_compact, truncate_chars,
};
use crate::theme::{InspectorTab, Palette};
use crate::ui::panel_controls::{button, icon_button, label};
use crate::ui::text_input::field;
use crate::ui::workspace::PrimeDesktopView;

pub(super) const INSPECTOR_WIDTH: f32 = 328.0;
const KINDS: [(RefinementKind, &str); 4] = [
    (RefinementKind::Memory, "Memory"),
    (RefinementKind::Prompt, "Prompt"),
    (RefinementKind::Skill, "Skill"),
    (RefinementKind::Subagent, "Subagent"),
];

fn caption(value: impl Into<String>, color: Color) -> El {
    label(value, 12.0, 400.0, color).wrap_anywhere()
}

fn heading(value: impl Into<String>, p: Palette) -> El {
    label(value, 11.0, 600.0, p.text_muted)
}

fn stack() -> El {
    col().w_full().gap(10.0)
}

fn card(p: Palette) -> El {
    col()
        .w_full()
        .padding(11.0)
        .gap(8.0)
        .radius(10.0)
        .border(1.0, p.border_subtle)
        .fill(p.bg_elevated)
}

fn actions() -> El {
    row().center_y().flex_wrap().gap(5.0)
}

fn action_base(key: impl Into<String>, label: &str, p: Palette) -> El {
    button(key, label, p).fill(p.bg_hover).hover(p.bg_selected)
}

fn action(
    key: impl Into<String>,
    label: &str,
    p: Palette,
    press: impl Fn(&mut PrimeDesktopView) + 'static,
) -> El {
    action_base(key, label, p).on_press(press)
}

fn option(
    key: impl Into<String>,
    label: &str,
    selected: bool,
    p: Palette,
    press: impl Fn(&mut PrimeDesktopView) + 'static,
) -> El {
    let fill = if selected { p.bg_selected } else { p.bg_hover };
    action(key, label, p, press)
        .role("radio")
        .checked(selected)
        .fill(fill)
}

fn confirm_request(
    view: &mut PrimeDesktopView,
    message: &str,
    label: &str,
    request: BridgeRequest,
) {
    view.confirm(message, label, move |workspace| {
        workspace.bridge.send(request)
    });
}

fn stat(label: &str, value: impl Into<String>, p: Palette) -> El {
    row()
        .w_full()
        .center_y()
        .justify_between()
        .gap(8.0)
        .child(caption(label, p.text_secondary).shrink())
        .child(caption(value, p.text_primary).shrink())
}

impl PrimeDesktopView {
    pub(super) fn inspector(&self, _: &mut Cx<Self>) -> El {
        let p = Palette::for_mode(self.prefs.theme);
        let active = self.prefs.inspector_tab;
        let tabs = row()
            .gap(3.0)
            .radius(8.0)
            .padding(3.0)
            .fill(p.bg_code)
            .children(InspectorTab::ALL.into_iter().map(|tab| {
                option(
                    format!("insp-tab-{}", tab.label()),
                    tab.label(),
                    tab == active,
                    p,
                    move |s| s.set_inspector_tab(tab),
                )
            }));
        let refresh = icon_button("insp-refresh-btn", "Refresh inspector", Icon::Refresh, p)
            .on_press(|s: &mut Self| {
                s.reload_local_stores();
                if !s.session.active_session_id.is_empty() {
                    s.bridge.send(BridgeRequest::RefreshInspector {
                        active_session_id: s.session.active_session_id.clone(),
                    });
                }
            });
        let body = if self.session.active_session_id.is_empty() && active != InspectorTab::Harness {
            caption(
                "Start or open a chat to inspect this session.",
                p.text_muted,
            )
        } else {
            match active {
                InspectorTab::Agents => self.inspector_agents(p),
                InspectorTab::Harness => self.inspector_harness(p),
                InspectorTab::Runtime => self.inspector_runtime(p),
                InspectorTab::System => self.inspector_system(p),
            }
        };
        col()
            .key("inspector")
            .role("group")
            .label("Inspector")
            .h_full()
            .w(INSPECTOR_WIDTH)
            .fill(p.bg_inspector)
            .child(
                row()
                    .h(52.0)
                    .px(10.0)
                    .center_y()
                    .justify_between()
                    .child(tabs)
                    .child(refresh),
            )
            .child(el().w_full().h(1.0).fill(p.border_subtle))
            .child(
                col()
                    .key(format!("inspector-scroll-{}", active.label()))
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(col().w_full().padding(12.0).child(body)),
            )
    }

    fn inspector_agents(&self, p: Palette) -> El {
        let children = &self.rlm_children;
        let mut body = stack();
        if let Some(depth) = self.rlm_max_depth {
            let step = |label: &str, value| {
                let button = action_base(format!("rlm-depth-{label}"), label, p);
                if value == depth {
                    return button.disabled(true).cursor(Cursor::Default).opacity(0.5);
                }
                button.on_press(move |s: &mut Self| {
                    s.bridge.send(BridgeRequest::SetRlmMaxDepth {
                        active_session_id: s.session.active_session_id.clone(),
                        max_depth: value,
                    });
                })
            };
            body = body.child(
                card(p)
                    .child(heading("Maximum agent depth", p))
                    .child(caption("Allowed levels of nested agents", p.text_secondary))
                    .child(
                        actions()
                            .child(step("−", depth.saturating_sub(1)))
                            .child(caption(depth.to_string(), p.text_primary))
                            .child(step("+", depth.saturating_add(1).min(8))),
                    ),
            );
        }
        body = body.child(heading(
            format!("DIRECT SUBAGENTS ({})", children.rows.len()),
            p,
        ));
        if let Some(error) = &children.load_error {
            body = body.child(caption(
                format!("Couldn’t load subagents: {error}"),
                p.danger,
            ));
        } else if children.rows.is_empty() {
            body = body.child(caption(
                if children.loaded {
                    "No subagents in this chat."
                } else {
                    "Loading subagents…"
                },
                p.text_muted,
            ));
        }
        // The first live child has a common headless selector; further children use their IDs.
        let mut first_live = true;
        for child in &children.rows {
            let live = child.active_session_id.as_ref().and_then(|id| {
                self.active_sessions.iter().find(|session| {
                    session.status != "inactive"
                        && (&session.active_session_id == id
                            || session.session_id.as_ref() == Some(id))
                })
            });
            let inspect_key = if live.is_some() && first_live {
                first_live = false;
                "subagent-inspect".to_owned()
            } else {
                format!("subagent-inspect-{}", child.rlm_child_id)
            };
            body = body.child(self.inspector_child(child, live, inspect_key, p));
        }
        body
    }

    fn inspector_child(
        &self,
        child: &RlmChildRow,
        live: Option<&ActiveSessionRow>,
        inspect_key: String,
        p: Palette,
    ) -> El {
        let status = match child.status.as_str() {
            "running" => ("Running", p.warning),
            "done" | "completed" => ("Completed", p.success),
            "error" => ("Error", p.danger),
            "cancelled" => ("Cancelled", p.text_secondary),
            "" | "unknown" => ("Unknown", p.text_muted),
            other => (other, p.text_secondary),
        };
        let id = &child.rlm_child_id;
        let mut content = card(p).key(format!("child-{id}")).child(
            row()
                .center_y()
                .gap(6.0)
                .child(Icon::GitBranch.render(14.0, p.info))
                .child(caption(child.name.clone(), p.text_primary).grow(1.0))
                .child(caption(status.0, status.1)),
        );
        if let Some(label) = child.label.as_ref().filter(|label| *label != &child.name) {
            content = content.child(caption(label.clone(), p.text_secondary));
        }
        let mut metadata = Vec::new();
        if let Some(model) = child
            .model
            .as_ref()
            .or_else(|| live.and_then(|row| row.model_label.as_ref()))
        {
            metadata.push(model.clone());
        }
        if let Some(count) = live.and_then(|row| row.tool_use_count) {
            metadata.push(format!(
                "{count} tool {}",
                if count == 1 { "call" } else { "calls" }
            ));
        }
        if !metadata.is_empty() {
            content = content.child(caption(metadata.join(" · "), p.text_muted));
        }
        if let Some(note) =
            live.and_then(|row| row.progress_note.as_ref().or(row.activity.as_ref()))
        {
            content = content.child(caption(truncate_chars(note, 160), p.text_muted));
        }
        if let Some(answer) = &child.answer_preview {
            content = content.child(caption(answer.clone(), p.text_primary).selectable(true));
        }
        if let Some(error) = &child.error {
            content = content.child(caption(error.clone(), p.danger).selectable(true));
        }
        let mut controls = actions();
        if let Some(live) = live {
            let attach = live.active_session_id.clone();
            let target = attach.clone();
            controls = controls
                .child(action(inspect_key, "Inspect live", p, move |s| {
                    s.open_session(BridgeRequest::AttachSession {
                        active_session_id: attach.clone(),
                    });
                }))
                .child(action(format!("msg-child-{id}"), "Message", p, move |s| {
                    if s.agent_mail_target.as_ref() != Some(&target) {
                        s.agent_mail_input.clear();
                    }
                    s.agent_mail_target = Some(target.clone());
                    s.focus_input(|s| &mut s.agent_mail_input);
                }));
        } else {
            controls = controls.child(caption("Live view unavailable", p.text_muted));
        }
        let active_session_id = self.session.active_session_id.clone();
        let child_id = id.clone();
        let destructive = match child.status.as_str() {
            "running" => Some((
                "Cancel",
                format!("Stop {}? Its running turn will end.", child.name),
                "Stop agent",
                BridgeRequest::CancelRlmChild {
                    active_session_id,
                    child_id,
                },
            )),
            "done" | "completed" | "error" | "cancelled" => Some((
                "Remove",
                format!("Remove {} from this chat?", child.name),
                "Remove",
                BridgeRequest::DeleteRlmSubagent {
                    active_session_id,
                    child_id,
                },
            )),
            _ => None,
        };
        if let Some((title, message, confirm, request)) = destructive {
            controls = controls.child(action(format!("child-{title}-{id}"), title, p, move |s| {
                confirm_request(s, &message, confirm, request.clone())
            }));
        }
        content = content.child(controls);
        if live.is_some_and(|row| self.agent_mail_target.as_deref() == Some(&row.active_session_id))
        {
            content = content
                .child(field(
                    "agent-mail",
                    &self.agent_mail_input,
                    "Message to subagent",
                    p,
                    |s| &mut s.agent_mail_input,
                ))
                .child(
                    actions()
                        .child(action("mail-cancel", "Cancel", p, Self::close_mail))
                        .child(action("mail-send", "Send", p, Self::submit_agent_mail)),
                );
        }
        content
    }

    fn inspector_harness(&self, p: Palette) -> El {
        let scope = self.harness_scope;
        let mut body = stack()
            .child(
                actions()
                    .children(
                        [
                            (HarnessScope::Global, "Global"),
                            (HarnessScope::Local, "Session Local"),
                        ]
                        .map(|(value, label)| {
                            option(
                                format!("scope-{label}"),
                                label,
                                value == scope,
                                p,
                                move |s| s.harness_scope = value,
                            )
                        }),
                    )
                    .child(action(
                        "harness-new",
                        "+ New",
                        p,
                        Self::open_new_harness_editor,
                    )),
            )
            .child(
                card(p)
                    .child(field(
                        "refine-input",
                        &self.refine_input,
                        "Refinement instructions",
                        p,
                        |s| &mut s.refine_input,
                    ))
                    .child(action(
                        "harness-refine",
                        "Refine",
                        p,
                        Self::submit_refinement,
                    )),
            );
        if self.harness_editor_open {
            body = body.child(
                card(p)
                    .child(actions().children(KINDS.map(|(kind, label)| {
                        option(
                            format!("editor-kind-{label}"),
                            label,
                            self.harness_editor_kind == kind,
                            p,
                            move |s| s.harness_editor_kind = kind,
                        )
                    })))
                    .child(
                        field(
                            "harness-title",
                            &self.harness_title_input,
                            "Entry title",
                            p,
                            |s| &mut s.harness_title_input,
                        )
                        .single_line(),
                    )
                    .child(
                        field(
                            "harness-body",
                            &self.harness_body_input,
                            "Entry content",
                            p,
                            |s| &mut s.harness_body_input,
                        )
                        .min_h(64.0),
                    )
                    .child(
                        actions()
                            .child(action(
                                "editor-cancel",
                                "Cancel",
                                p,
                                Self::close_harness_editor,
                            ))
                            .child(action("editor-save", "Save", p, Self::save_harness_editor)),
                    ),
            );
        }
        body = body.child(
            actions()
                .child(option(
                    "filter-all",
                    "All",
                    self.harness_kind_filter.is_none(),
                    p,
                    |s| s.harness_kind_filter = None,
                ))
                .children(KINDS.map(|(kind, label)| {
                    option(
                        format!("filter-{label}"),
                        label,
                        self.harness_kind_filter == Some(kind),
                        p,
                        move |s| s.harness_kind_filter = Some(kind),
                    )
                })),
        );
        let mut entries = self
            .harness
            .entries_for(scope)
            .iter()
            .filter(|entry| {
                self.harness_kind_filter
                    .is_none_or(|kind| entry.kind == kind)
            })
            .peekable();
        if entries.peek().is_none() {
            body = body.child(caption("No harness entries in this scope.", p.text_muted));
        }
        for entry in entries {
            let kind = entry.kind;
            let edit_id = entry.id.clone();
            let title = entry.title.clone();
            let content = entry.content.clone();
            let id = entry.id.clone();
            let msg = format!("Delete the harness entry “{}”?", entry.title);
            let sid = self.session.active_session_id.clone();
            body = body.child(
                card(p)
                    .child(stat(
                        &entry.title,
                        format!("{} v{}", format!("{kind:?}").to_lowercase(), entry.version),
                        p,
                    ))
                    .child(caption(
                        truncate_chars(&entry.content, 160),
                        p.text_secondary,
                    ))
                    .child(
                        actions()
                            .child(action(format!("edit-he-{edit_id}"), "Edit", p, move |s| {
                                s.edit_existing_harness_entry(kind, &edit_id, &title, &content);
                            }))
                            .child(action(format!("del-he-{id}"), "Delete", p, move |s| {
                                let id = id.clone();
                                let sid = sid.clone();
                                s.confirm(&msg, "Delete entry", move |s| {
                                    if s.session.active_session_id == sid
                                        && s.harness_scope == scope
                                    {
                                        s.remove_harness_entry(kind, &id);
                                    }
                                });
                            })),
                    ),
            );
        }
        if !self.harness.history.is_empty() {
            body = body.child(heading("RECENT REFINEMENTS", p));
        }
        for revision in self.harness.history.iter().take(6) {
            let mut revision_card = card(p)
                .child(caption(truncate_chars(&revision.id, 24), p.accent))
                .child(caption(
                    truncate_chars(&revision.summary, 110),
                    p.text_secondary,
                ));
            if let Some(revision_scope) = revision
                .scope
                .filter(|_| !self.session.active_session_id.is_empty())
            {
                let message = format!(
                    "Roll back refinement {}? This changes saved harness entries.",
                    revision.id
                );
                let request = BridgeRequest::Refine {
                    active_session_id: self.session.active_session_id.clone(),
                    instructions: None,
                    rollback_id: Some(revision.id.clone()),
                    global: revision_scope == HarnessScope::Global,
                };
                revision_card = revision_card.child(action(
                    format!("rb-{}", revision.id),
                    "Roll back",
                    p,
                    move |s| confirm_request(s, &message, "Roll back", request.clone()),
                ));
            }
            body = body.child(revision_card);
        }
        body
    }

    fn inspector_runtime(&self, p: Palette) -> El {
        let mut body = stack()
            .child(
                card(p)
                    .child(heading("RUN SHELL COMMAND IN SESSION CWD", p))
                    .child(
                        field(
                            "quick-bash",
                            &self.quick_bash_input,
                            "Shell command",
                            p,
                            |s| &mut s.quick_bash_input,
                        )
                        .single_line()
                        .on_submit(|s: &mut Self, _| s.submit_quick_bash()),
                    )
                    .child(action(
                        "run-shell-command",
                        "Run",
                        p,
                        Self::submit_quick_bash,
                    )),
            )
            .child(heading(
                format!("KERNEL & PERSISTENT SHELLS ({})", self.shells.len()),
                p,
            ));
        if self.shells.is_empty() {
            body = body.child(caption("No shells in this session.", p.text_muted));
        }
        for shell in &self.shells {
            let status = if shell.persistent {
                "persistent"
            } else if shell.running {
                "running"
            } else {
                "exited"
            };
            let mut shell_card = card(p).child(stat(&shell.command, status, p));
            if let Some(output) = self
                .shell_tails
                .get(&shell.id)
                .or(shell.tail_preview.as_ref())
            {
                shell_card = shell_card.child(
                    text(truncate_chars(output, 240))
                        .font(TextStyle::new(11.0, 400.0).mono())
                        .color(p.text_secondary)
                        .wrap_anywhere()
                        .selectable(true),
                );
            }
            let tail = BridgeRequest::TailShell {
                active_session_id: self.session.active_session_id.clone(),
                id: shell.id.clone(),
                persistent: shell.persistent,
            };
            let mut controls = actions().child(action(
                format!("tail-sh-{}", shell.id),
                "Tail",
                p,
                move |s| s.bridge.send(tail.clone()),
            ));
            if shell.running {
                let request = BridgeRequest::KillShell {
                    active_session_id: self.session.active_session_id.clone(),
                    id: shell.id.clone(),
                    persistent: shell.persistent,
                };
                let message = format!("Stop shell {}? The process will be terminated.", shell.id);
                controls = controls.child(action(
                    format!("kill-sh-{}", shell.id),
                    "Stop",
                    p,
                    move |s| confirm_request(s, &message, "Stop shell", request.clone()),
                ));
            }
            body = body.child(shell_card.child(controls));
        }
        let mut schedules = card(p)
            .child(heading(
                format!("HEARTBEATS & SCHEDULES ({})", self.schedules.len()),
                p,
            ))
            .child(
                field(
                    "schedule-interval",
                    &self.schedule_interval_input,
                    "Interval",
                    p,
                    |s| &mut s.schedule_interval_input,
                )
                .single_line(),
            )
            .child(field(
                "schedule-prompt",
                &self.schedule_prompt_input,
                "Heartbeat prompt",
                p,
                |s| &mut s.schedule_prompt_input,
            ))
            .child(action(
                "add-heartbeat",
                "Add heartbeat",
                p,
                Self::submit_heartbeat,
            ));
        for job in &self.schedules {
            let request = if job.is_heartbeat {
                BridgeRequest::ManageHeartbeat {
                    active_session_id: self.session.active_session_id.clone(),
                    job_id: job.id.clone(),
                    action: "delete".to_owned(),
                }
            } else {
                BridgeRequest::CancelCron {
                    active_session_id: Some(self.session.active_session_id.clone()),
                    job_id: job.id.clone(),
                }
            };
            let message = format!("Remove schedule {}?", job.id);
            schedules = schedules.child(
                row()
                    .w_full()
                    .center_y()
                    .justify_between()
                    .gap(6.0)
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .child(caption(job.label.clone(), p.text_primary))
                            .child(caption(job.schedule.clone(), p.text_muted)),
                    )
                    .child(action(
                        format!("rm-job-{}", job.id),
                        "Remove",
                        p,
                        move |s| confirm_request(s, &message, "Remove", request.clone()),
                    )),
            );
        }
        body.child(schedules)
    }

    fn inspector_system(&self, p: Palette) -> El {
        let telemetry = &self.telemetry;
        let missing = "Not reported";
        let context = match (telemetry.context_tokens, telemetry.context_window) {
            (Some(tokens), Some(window)) => format!(
                "{} / {}",
                format_tokens_compact(tokens),
                format_tokens_compact(window)
            ),
            (Some(tokens), None) => format!("{} used", format_tokens_compact(tokens)),
            (None, Some(window)) => format!("{} capacity", format_tokens_compact(window)),
            (None, None) => missing.to_owned(),
        };
        let input = telemetry
            .total_input_tokens
            .map_or_else(|| "—".to_owned(), format_tokens_compact);
        let output = telemetry
            .total_output_tokens
            .map_or_else(|| "—".to_owned(), format_tokens_compact);
        let mut body = stack().child(
            card(p)
                .child(heading("SESSION TELEMETRY", p))
                .child(stat("Context window", context, p))
                .child(stat(
                    "Input / Output",
                    format!("{input} in · {output} out"),
                    p,
                ))
                .child(stat(
                    "Cache read",
                    telemetry
                        .cache_read_tokens
                        .map_or_else(|| missing.to_owned(), format_tokens_compact),
                    p,
                ))
                .child(stat(
                    "Session cost",
                    telemetry
                        .total_cost_usd
                        .map_or_else(|| missing.to_owned(), |cost| format!("${cost:.4}")),
                    p,
                ))
                .child(
                    actions()
                        .child(action(
                            "system-compact",
                            "Compact",
                            p,
                            Self::trigger_compact,
                        ))
                        .child(action("system-reload", "Reload session", p, |s| {
                            s.bridge.send(BridgeRequest::ReloadSession {
                                active_session_id: s.session.active_session_id.clone(),
                            })
                        })),
                ),
        );
        let mut policies = card(p).child(heading("POLICIES", p));
        for (is_retry, label, value) in [
            (false, "Auto-compaction", telemetry.auto_compaction),
            (true, "Auto-retry on transient errors", telemetry.auto_retry),
        ] {
            if let Some(enabled) = value {
                let request = if is_retry {
                    BridgeRequest::SetAutoRetry {
                        active_session_id: self.session.active_session_id.clone(),
                        enabled: !enabled,
                    }
                } else {
                    BridgeRequest::SetAutoCompaction {
                        active_session_id: self.session.active_session_id.clone(),
                        enabled: !enabled,
                    }
                };
                policies = policies.child(
                    row()
                        .w_full()
                        .center_y()
                        .justify_between()
                        .gap(6.0)
                        .child(caption(label, p.text_secondary))
                        .child(
                            action(
                                format!("policy-{label}"),
                                if enabled { "On" } else { "Off" },
                                p,
                                move |s| s.bridge.send(request.clone()),
                            )
                            .role("switch")
                            .label(label)
                            .checked(enabled),
                        ),
                );
            }
        }
        if telemetry.auto_compaction.is_none() && telemetry.auto_retry.is_none() {
            policies = policies.child(caption(missing, p.text_muted));
        }
        let mut providers = card(p).child(
            row()
                .w_full()
                .center_y()
                .justify_between()
                .child(heading("PROVIDERS", p))
                .child(action("system-provider-settings", "Manage", p, |s| {
                    s.open_modal(ActiveModal::SettingsAuth)
                })),
        );
        for provider in self.provider_auth.iter().take(5) {
            providers = providers.child(
                row()
                    .w_full()
                    .justify_between()
                    .gap(6.0)
                    .child(caption(provider.display_name.clone(), p.text_primary))
                    .child(caption(
                        provider.auth_kind.clone(),
                        if provider.configured {
                            p.success
                        } else {
                            p.text_muted
                        },
                    )),
            );
        }
        let mut mcp = card(p).child(heading(format!("MCP SERVERS ({})", self.mcp_rows.len()), p));
        for item in &self.mcp_rows {
            let request = BridgeRequest::RemoveMcp {
                active_session_id: self.session.active_session_id.clone(),
                server: item.server.clone(),
            };
            let message = format!("Remove the connection to {}?", item.server);
            mcp = mcp.child(
                row()
                    .w_full()
                    .center_y()
                    .justify_between()
                    .gap(6.0)
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .child(caption(item.server.clone(), p.text_primary))
                            .child(caption(item.status.clone(), p.text_muted)),
                    )
                    .child(action(
                        format!("rm-mcp-{}", item.server),
                        "Remove",
                        p,
                        move |s| confirm_request(s, &message, "Remove", request.clone()),
                    )),
            );
        }
        mcp = mcp
            .child(
                field("mcp-server", &self.mcp_server_input, "Server", p, |s| {
                    &mut s.mcp_server_input
                })
                .single_line(),
            )
            .child(
                field("mcp-token", &self.mcp_token_input, "Access token", p, |s| {
                    &mut s.mcp_token_input
                })
                .single_line(),
            )
            .child(action(
                "set-mcp-token",
                "Set token",
                p,
                Self::submit_mcp_token,
            ));
        body = body.child(policies).child(providers).child(mcp);
        if let Some(prompt) = &self.system_prompt {
            let copy = prompt.clone();
            body = body.child(
                card(p)
                    .child(
                        row()
                            .w_full()
                            .center_y()
                            .justify_between()
                            .child(heading(
                                format!("SYSTEM PROMPT ({} chars)", prompt.chars().count()),
                                p,
                            ))
                            .child(
                                action_base("copy-system-prompt", "Copy", p)
                                    .on_press_tx(move |_: &mut Self, tx| tx.copy(copy.clone())),
                            ),
                    )
                    .child(
                        text(truncate_chars(prompt, 420))
                            .font(TextStyle::new(11.0, 400.0).mono())
                            .wrap_anywhere()
                            .color(p.text_secondary)
                            .selectable(true),
                    ),
            );
        }
        body
    }
}
