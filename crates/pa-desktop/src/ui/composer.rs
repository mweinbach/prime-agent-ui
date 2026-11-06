//! Prompt editing, slash completion, staged attachments and live turn controls.

use rune::{Cx, El, Key, KeyEvent, auto_grow, col, row};

use super::selectable_text::{control, icon_control, label};
use super::text_input::{LINE_HEIGHT, dismiss, field};
use super::workspace::PrimeDesktopView;
use crate::assets::Icon;
use crate::bridge::BridgeRequest;
use crate::state::{ActiveModal, truncate_chars};
use crate::theme::Palette;

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

pub(super) fn composer(view: &PrimeDesktopView, p: Palette, cx: &Cx<PrimeDesktopView>) -> El {
    let draft = &view.composer_input.text;
    let shell = draft.trim_start().starts_with('!');
    let mut section = col()
        .key("composer")
        .w_full()
        .gap(8.)
        .on_drop_files(|s: &mut PrimeDesktopView, files, _| s.attach_files(files.to_vec()));
    if !view.session.queued.is_empty() {
        section = section.child(queued(view, p));
    }
    let matches = slash_matches(draft);
    if !matches.is_empty() {
        let selected = view.slash_selected_idx.min(matches.len() - 1);
        let mut options = col()
            .role("list")
            .gap(2.)
            .padding(5.)
            .fill(p.bg_elevated)
            .border(1., p.border_strong)
            .radius(10.);
        for (i, (command, description)) in matches.into_iter().enumerate() {
            let mut option = control(&format!("slash-opt-{command}"), command, p)
                .justify_between()
                .w_full()
                .child(label(command, 13., p.accent))
                .child(
                    label(description, 12., p.text_secondary)
                        .min_w(0.)
                        .truncate(),
                )
                .on_press(move |s: &mut PrimeDesktopView| s.populate_draft(command));
            if i == selected {
                option = option.fill(p.bg_selected);
            }
            options = options.child(option);
        }
        section = section.child(options);
    }
    let focused = view.composer_input.focused;
    let border = if shell {
        p.warning
    } else if focused {
        p.accent.mix(p.bg_elevated, 0.25)
    } else {
        p.border_strong
    };
    let mut card = col()
        .w_full()
        .gap(12.)
        .px(18.)
        .pt(18.)
        .pb(12.)
        .fill(p.bg_elevated)
        .radius(19.)
        .border(1., border)
        .shadow(3., 10., rune::Color::BLACK.alpha(0.09));
    if focused {
        card = card.outline(2., 1., p.accent.alpha(0.09));
    }
    if !view.staged_images.is_empty() {
        let mut attachments = row().flex_wrap().gap(6.);
        for (i, image) in view.staged_images.iter().enumerate() {
            attachments = attachments.child(
                control(
                    &format!("rm-img-{i}"),
                    &format!("Remove {}", image.filename),
                    p,
                )
                .fill(p.info_soft)
                .child(Icon::Image.render(13., p.info))
                .child(label(truncate_chars(&image.filename, 28), 12., p.info))
                .child(Icon::Close.render(11., p.info))
                .on_press(move |s: &mut PrimeDesktopView| {
                    if i < s.staged_images.len() {
                        s.staged_images.remove(i);
                    }
                }),
            );
        }
        card = card.child(attachments);
    }
    // Up/Down move through wrapped text normally. Only the outermost visual lines
    // recall history, including the ends of a long scrolled draft.
    let edge_lines = cx
        .window_layout("composer-input")
        .zip(cx.caret("composer-input"))
        .map(|([_, top, _, height], [_, caret, _, caret_height])| {
            let center = caret + caret_height / 2.;
            (
                center < top + LINE_HEIGHT,
                center > top + height - LINE_HEIGHT,
            )
        })
        .unwrap_or_default();
    let input = field(
        "composer-input",
        &view.composer_input,
        "Message Prime Agent…",
        p,
        |s| &mut s.composer_input,
    )
    .font(rune::TextStyle::new(15., 400.).lh(LINE_HEIGHT))
    .no_focus_ring()
    .on_input_change(|s: &mut PrimeDesktopView, _| s.composer_edited())
    .on_submit(|s: &mut PrimeDesktopView, _| submit(s))
    .on_input_key(move |s: &mut PrimeDesktopView, event, _| composer_key(s, event, edge_lines));

    let mut tools = row()
        .min_w(0.)
        .items_center()
        .gap(6.)
        .flex_wrap()
        .child(
            control("composer-attach-img-btn", "Attach images", p)
                .help("Attach images")
                .size(32., 32.)
                .padding(0.)
                .radius(10.)
                .border(1., p.border_strong)
                .child(Icon::Plus.render(17., p.text_secondary))
                .on_press(PrimeDesktopView::prompt_attach_images),
        )
        .child(
            control("composer-model-pill", "Select model", p)
                .h(32.)
                .gap(6.)
                .px(10.)
                .radius(10.)
                .fill(p.bg_sidebar)
                .child(Icon::Cpu.render(13., p.text_secondary))
                .child(
                    label(
                        view.session.model_id.as_deref().unwrap_or("Select model"),
                        12.5,
                        p.text_primary,
                    )
                    .truncate()
                    .max_w(200.),
                )
                .child(Icon::ChevronDown.render(12., p.text_muted))
                .on_press(|s: &mut PrimeDesktopView| s.open_modal(ActiveModal::ModelPicker)),
        );
    if !view.session.thinking_level.is_empty() {
        tools = tools.child(
            control("composer-effort-pill", "Change reasoning effort", p)
                .child(label(
                    format!("Effort: {}", view.session.thinking_level),
                    12.,
                    p.text_secondary,
                ))
                .on_press(PrimeDesktopView::cycle_thinking_level),
        );
    }
    if shell {
        tools = tools.child(
            control("composer-bash-prefix-btn", "Remove shell prefix", p)
                .fill(p.warning_soft)
                .child(label("Shell ×", 12., p.warning))
                .on_press(PrimeDesktopView::toggle_bash_prefix),
        );
    }
    let mut actions = row().center_y().gap(4.);
    if let Some(percent) = view.telemetry.context_percent {
        actions = actions.child(label(format!("{percent:.0}% context"), 11.5, p.text_muted));
    }
    if view.session.is_streaming {
        actions = actions
            .child(
                control("composer-steer-btn", "Steer", p)
                    .fill(p.accent_soft)
                    .child(Icon::Steer.render(13., p.accent))
                    .child(label("Steer", 12., p.accent))
                    .on_press(PrimeDesktopView::submit_steer),
            )
            .child(
                control("composer-queue-btn", "Queue follow-up", p)
                    .border(1., p.border_subtle)
                    .child(label("Queue", 12., p.text_secondary))
                    .on_press(PrimeDesktopView::submit_follow_up),
            )
            .child(
                icon_control("composer-stop-btn", "Stop response", Icon::Stop, p)
                    .fill(p.danger_soft)
                    .on_press(|s: &mut PrimeDesktopView| {
                        s.bridge.send(BridgeRequest::AbortSession {
                            active_session_id: s.session.active_session_id.clone(),
                        });
                    }),
            );
    } else {
        let enabled = !draft.trim().is_empty() || !view.staged_images.is_empty();
        actions = actions.child(
            control("composer-send-btn", "Send message", p)
                .size(34., 34.)
                .padding(0.)
                .radius(11.)
                .help("Send message")
                .fill(if enabled { p.accent } else { p.bg_user_bubble })
                .disabled(!enabled)
                .child(Icon::Send.render(
                    16.,
                    if enabled {
                        p.text_on_accent
                    } else {
                        p.text_muted
                    },
                ))
                .on_press(PrimeDesktopView::submit_composer),
        );
    }
    card = card
        .child(auto_grow(
            input,
            if view.session.entries.is_empty() {
                3
            } else {
                2
            },
            7,
        ))
        .child(
            row()
                .w_full()
                .center_y()
                .justify_between()
                .flex_wrap()
                .gap(5.)
                .child(tools)
                .child(actions),
        );
    section.child(card)
}

fn submit(s: &mut PrimeDesktopView) {
    if !s.complete_slash() {
        s.submit_composer();
    }
}

fn composer_key(s: &mut PrimeDesktopView, event: &KeyEvent, edge_lines: (bool, bool)) -> bool {
    if event.mods.shift || event.mods.alt || s.composer_input.preedit.is_some() {
        return false;
    }
    // Explicitly keep plain Return as Send, including inside code/list-looking prompts.
    if dismiss(s, event) {
        return true;
    }
    if event.key == Key::Enter {
        submit(s);
        return true;
    }
    if event.mods.cmd || event.mods.ctrl {
        return false;
    }
    let matches = slash_matches(&s.composer_input.text);
    if !matches.is_empty() {
        let selected = s.slash_selected_idx.min(matches.len() - 1);
        match event.key {
            Key::Up => s.slash_selected_idx = selected.saturating_sub(1),
            Key::Down => s.slash_selected_idx = (selected + 1).min(matches.len() - 1),
            Key::Tab => return s.complete_slash(),
            _ => return false,
        }
        return true;
    }
    let edit = &s.composer_input;
    if edit.anchor == edit.focus
        && ((event.key == Key::Up && (edit.focus == 0 || edge_lines.0))
            || (event.key == Key::Down && (edit.focus == edit.text.len() || edge_lines.1)))
    {
        return s.composer_history(event.key == Key::Up);
    }
    false
}

fn queued(s: &PrimeDesktopView, p: Palette) -> El {
    let queue = &s.session.queued;
    let count = queue.steering.len() + queue.follow_ups.len();
    let first = queue
        .steering
        .first()
        .or_else(|| queue.follow_ups.first())
        .or(queue.preparing.as_ref());
    row()
        .key("queued-messages")
        .w_full()
        .center_y()
        .flex_wrap()
        .gap(6.)
        .px(8.)
        .py(5.)
        .radius(8.)
        .fill(p.warning_soft)
        .child(label(format!("Queued ({count})"), 12., p.warning))
        .child(
            label(first.map_or("", String::as_str), 12., p.text_secondary)
                .grow(1.)
                .min_w(0.)
                .truncate(),
        )
        .child(
            control("queue-send-now-btn", "Interrupt and send queued message", p)
                .child(label("Interrupt & send", 12., p.accent))
                .on_press(|s: &mut PrimeDesktopView| {
                    s.bridge.send(BridgeRequest::AbortAndSendQueued {
                        active_session_id: s.session.active_session_id.clone(),
                    });
                }),
        )
        .child(
            control("queue-clear-btn", "Clear queued messages", p)
                .child(label("Clear", 12., p.text_secondary))
                .on_press(|s: &mut PrimeDesktopView| {
                    s.bridge.send(BridgeRequest::ClearQueue {
                        active_session_id: s.session.active_session_id.clone(),
                    });
                }),
        )
}
