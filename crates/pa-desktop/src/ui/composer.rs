//! Prompt editing, slash completion, staged attachments and live turn controls.

use rune::{
    Align, Cursor, Cx, El, Key, KeyEvent, ScrollSpec, Side, TextStyle, auto_grow, col, el, row,
};

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
        p.text_muted.alpha(0.62)
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

    let model = view.current_catalog_model();
    let model_name = model
        .map(|m| m.name.as_str())
        .filter(|n| !n.is_empty())
        .or(view.session.model_id.as_deref())
        .unwrap_or("Select model");
    let effort = view.session.thinking_level.as_str();
    let has_choices = model.is_some_and(|m| m.supported_levels.len() > 1);
    let reasoning = if model.is_some_and(|m| m.supported_levels.iter().any(|level| level == effort))
    {
        format!("{} reasoning", effort_name(effort))
    } else {
        "Reasoning".into()
    };
    let can_open = view.can_open_reasoning_picker();
    let show_reasoning =
        has_choices || (model.is_none() && (can_open || view.reasoning_picker_open));
    let mut tools = row()
        .min_w(0.)
        .items_center()
        .gap(3.)
        .flex_wrap()
        .child(
            icon_control("composer-attach-img-btn", "Attach images", Icon::Plus, p)
                .size(30., 30.)
                .on_press(PrimeDesktopView::prompt_attach_images),
        )
        .child(
            control("composer-model-pill", "Select model", p)
                .h(30.)
                .gap(5.)
                .px(6.)
                .help("Select model")
                .child(
                    label(model_name, 12.5, p.text_primary)
                        .font(TextStyle::new(12.5, 550.))
                        .max_w(200.)
                        .min_w(0.)
                        .truncate(),
                )
                .child(Icon::ChevronDown.render(11., p.text_muted))
                .on_press(|s: &mut PrimeDesktopView| s.open_modal(ActiveModal::ModelPicker)),
        );
    if show_reasoning {
        tools = tools.child(
            control("composer-effort-pill", "Reasoning effort", p)
                .h(30.)
                .gap(5.)
                .px(6.)
                .expanded(view.reasoning_picker_open)
                .help("Choose reasoning effort")
                .disabled(!can_open && !view.reasoning_picker_open)
                .preserve_focus_on_press(false)
                .child(label(reasoning, 12.5, p.text_secondary))
                .child(Icon::ChevronDown.render(11., p.text_muted))
                .on_press(PrimeDesktopView::open_reasoning_picker),
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
    if let Some(percent) = view
        .telemetry
        .context_percent
        .filter(|percent| *percent >= 1.)
    {
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

fn effort_name(level: &str) -> &str {
    match level {
        "off" => "Off",
        "minimal" => "Minimal",
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "Extra high",
        "max" => "Maximum",
        other => other,
    }
}

impl PrimeDesktopView {
    /// Mount at the window root so the menu can flip above a bottom-docked composer
    /// and float outside the welcome and transcript scroll viewports.
    pub(in crate::ui) fn reasoning_picker(&self, p: Palette) -> Option<El> {
        if !self.reasoning_picker_open {
            return None;
        }
        let mut options = col()
            .key("reasoning-options")
            .role("listbox")
            .label("Reasoning effort")
            .focus_group()
            .gap(1.)
            .min_h(0.)
            .max_h(248.)
            .scroll(ScrollSpec {
                stick_bottom: false,
            });
        if let Some(model) = self
            .current_catalog_model()
            .filter(|m| !m.supported_levels.is_empty())
        {
            for (index, level) in model.supported_levels.iter().enumerate() {
                let selected = self.session.thinking_level == *level;
                let (session, provider, model_id) = (
                    self.session.active_session_id.clone(),
                    model.provider.clone(),
                    model.id.clone(),
                );
                let id = level.clone();
                let mut option = row()
                    .key(format!("reasoning-level-{level}"))
                    .role("option")
                    .label(effort_name(level))
                    .focusable(true)
                    .focus_selected(selected)
                    .position_in_set(index + 1, model.supported_levels.len())
                    .h(30.)
                    .px(9.)
                    .gap(12.)
                    .radius(5.)
                    .items_center()
                    .cursor(Cursor::Pointer)
                    .hover(p.bg_hover)
                    .focus_highlight(p.bg_hover)
                    .child(label(effort_name(level), 13., p.text_primary).grow(1.))
                    .on_press(move |s: &mut Self| {
                        s.select_reasoning_level(&session, &provider, &model_id, &id);
                    });
                if selected {
                    option = option.child(Icon::Check.render(14., p.text_primary));
                }
                options = options.child(option);
            }
        } else {
            let message = if self.daemon_connected
                && (!self.model_catalog_loaded() || self.session.active_session_id.is_empty())
            {
                "Loading session settings…"
            } else {
                "No reasoning levels available for this model."
            };
            options = options.child(label(message, 12., p.text_secondary).wrap().padding(8.));
        }
        let surface = col()
            .key("reasoning-picker")
            .label("Reasoning effort")
            .hit()
            .w(178.)
            .padding(5.)
            .radius(9.)
            .fill(p.bg_elevated)
            .border(1., p.border_strong)
            .shadow(3., 12., rune::Color::BLACK.alpha(0.16))
            .focus_return("composer-effort-pill")
            .modal(true, Self::close_reasoning_picker)
            .anchored_fit("composer-effort-pill", Side::Top, Align::Start, 5.)
            .child(options);
        Some(
            el().key("reasoning-layer")
                .pin(Some(0.), Some(0.), Some(0.), Some(0.))
                .child(
                    el().key("reasoning-outside")
                        .role("none")
                        .w_full()
                        .h_full()
                        .on_press(Self::close_reasoning_picker),
                )
                .child(surface),
        )
    }
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

#[cfg(test)]
mod tests {
    use rune::{Env, Key, KeyEvent, Mods, Runtime};
    use tempfile::TempDir;

    use crate::state::models::parse_model_catalog;
    use crate::ui::workspace::{LaunchOptions, PrimeDesktopView};

    fn click(app: &mut Runtime<PrimeDesktopView>, key: &str) {
        let (x, y) = app.find(key).unwrap();
        app.pointer_down(x, y);
        app.pointer_up(x, y);
        app.frame(0.);
    }

    fn press(app: &mut Runtime<PrimeDesktopView>, key: Key) {
        let event = KeyEvent {
            key,
            mods: Mods::default(),
        };
        app.key(event.clone());
        app.key_up(event);
        app.frame(0.);
    }

    #[test]
    fn reasoning_menu_uses_model_levels_and_supports_direct_keyboard_selection() {
        let tmp = TempDir::new().unwrap();
        let mut view = PrimeDesktopView::new(
            LaunchOptions {
                initial_cwd: Some(tmp.path().into()),
                daemon_socket: Some(tmp.path().join("test.sock")),
                ..LaunchOptions::default()
            },
            tmp.path().into(),
        );
        view.daemon_connected = true;
        view.session.active_session_id = "agent".into();
        view.session.model_provider = Some("example".into());
        view.session.model_id = Some("sparse".into());
        view.session.thinking_level = "low".into();
        view.catalog_models = parse_model_catalog(&serde_json::json!({"models":[{
            "provider":"example", "id":"sparse", "reasoning":true,
            "thinkingLevelMap":{"off":null,"minimal":null,"medium":null,"high":null,"max":"maximum"}
        }]}))
        .0;
        let mut app = Runtime::new(
            view,
            PrimeDesktopView::view,
            (950., 720.),
            Env::new(false, 1.),
        );
        // A model that only supports Off has no setting to choose.
        app.state.catalog_models.push(
            parse_model_catalog(&serde_json::json!({"models":[
                {"provider":"example","id":"fixed","reasoning":false}
            ]}))
            .0
            .pop()
            .unwrap(),
        );
        app.state.session.model_id = Some("fixed".into());
        app.invalidate();
        app.frame(0.);
        assert!(app.find("composer-effort-pill").is_none());
        app.state.session.model_id = Some("sparse".into());
        app.invalidate();
        app.frame(0.);
        click(&mut app, "composer-effort-pill");
        let levels = app
            .elements()
            .into_iter()
            .filter(|e| {
                e.key
                    .as_deref()
                    .is_some_and(|k| k.starts_with("reasoning-level-"))
            })
            .map(|e| (e.key.unwrap(), e.item_selected))
            .collect::<Vec<_>>();
        assert_eq!(
            levels,
            [
                ("reasoning-level-low".into(), true),
                ("reasoning-level-max".into(), false)
            ]
        );
        press(&mut app, Key::Escape);
        assert!(!app.state.reasoning_picker_open);
        click(&mut app, "composer-effort-pill");
        // A click outside dismisses without sending a setting change.
        app.pointer_down(5., 5.);
        app.pointer_up(5., 5.);
        app.frame(0.);
        assert!(!app.state.reasoning_picker_open);
        click(&mut app, "composer-effort-pill");
        // Rune's pointer-opened listbox reveals the current highlight on the first
        // arrow. The following arrow moves it without committing a new setting.
        press(&mut app, Key::Down);
        assert!(
            app.elements()
                .iter()
                .any(|e| e.key.as_deref() == Some("reasoning-level-low") && e.focus_visible)
        );
        press(&mut app, Key::Down);
        assert!(
            app.elements()
                .iter()
                .any(|e| e.key.as_deref() == Some("reasoning-level-max") && e.focused)
        );
        press(&mut app, Key::Enter);
        assert!(!app.state.reasoning_picker_open);
    }
}
