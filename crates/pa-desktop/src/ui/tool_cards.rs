//! Expandable tool calls, diffs and console output in the session timeline.

use pa_types::ai::is_python_tool_name;
use rune::{Color, El, TextStyle, col, el, row};

use super::selectable_text::{control, copy_button, label};
use super::syntax::{code_text, strip_ansi};
use super::workspace::PrimeDesktopView;
use crate::assets::Icon;
use crate::state::{
    transcript::{DiffView, ToolCard},
    truncate_chars,
};
use crate::theme::Palette;

pub(super) fn pill(value: &str, bg: Color, fg: Color) -> El {
    row()
        .radius(6.)
        .px(6.)
        .py(2.)
        .fill(bg)
        .child(label(value, 11.5, fg))
}

pub(super) fn disclosure(key: &str, title: &str, open: bool, p: Palette) -> El {
    let id = key.to_owned();
    control(&format!("{key}-hdr"), title, p)
        .expanded(open)
        .child(
            if open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            }
            .render(12., p.text_muted),
        )
        .child(label(title, 12.5, p.text_secondary).min_w(0.).truncate())
        .on_press(move |s: &mut PrimeDesktopView| s.session.set_expanded(&id, !open))
}

pub(super) fn render_tool_card(view: &PrimeDesktopView, card: &ToolCard, p: Palette) -> El {
    let detail = view.prefs.detail_level;
    let key = format!("tool-{}", card.id);
    let open = view
        .session
        .is_expanded(&key, detail.expand_tool_output() || card.is_error);
    let (kind, lang, icon) = match card.name.as_str() {
        name if is_python_tool_name(name) => ("Python", Some("python"), Icon::Cpu),
        "bash" => ("Shell", Some("sh"), Icon::Terminal),
        "edit" => ("Edit", None, Icon::EditPen),
        "attach_image" => ("Image", None, Icon::Image),
        other => (other, None, Icon::Zap),
    };
    let first = card
        .code_or_cmd
        .lines()
        .find(|s| !s.trim().is_empty())
        .unwrap_or_default();
    let title = if first.is_empty() {
        kind.into()
    } else {
        format!("{kind}  {}", truncate_chars(first.trim(), 80))
    };
    let (status, ink) = if card.running {
        ("Running…".into(), p.warning)
    } else if card.is_error
        || card
            .python
            .as_ref()
            .is_some_and(|py| py.error_name.is_some())
    {
        ("Error".into(), p.danger)
    } else if let Some(ms) = card.python.as_ref().and_then(|py| py.duration_ms) {
        (
            if ms >= 1000. {
                format!("{:.2}s", ms / 1000.)
            } else {
                format!("{ms:.0}ms")
            },
            p.success,
        )
    } else if card.settled {
        ("Done".into(), p.success)
    } else {
        ("Streaming…".into(), p.accent)
    };

    let copy = card
        .edit_diff
        .as_ref()
        .map_or(card.code_or_cmd.as_str(), |d| d.unified.as_str());
    let id = key.clone();
    let mut toggle = control(&format!("{key}-hdr"), &title, p)
        .grow(1.)
        .min_w(0.)
        .justify_start()
        .gap(9.)
        .px(2.)
        .py(2.)
        .expanded(open)
        .on_press(move |s: &mut PrimeDesktopView| s.session.set_expanded(&id, !open))
        .child(
            row()
                .size(28., 28.)
                .center()
                .radius(8.)
                .fill(p.bg_sidebar)
                .child(icon.render(15., p.text_secondary)),
        )
        .child(label(kind, 13., p.text_primary).font(TextStyle::new(13., 600.)))
        .child(
            if open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            }
            .render(11., p.text_muted),
        );
    if !open && !first.is_empty() {
        toggle = toggle.child(
            label(first.trim(), 12., p.text_secondary)
                .min_w(0.)
                .shrink()
                .truncate(),
        );
    }
    let status_chip = row()
        .h(23.)
        .gap(5.)
        .px(7.)
        .center_y()
        .radius(7.)
        .fill(ink.alpha(0.10))
        .child(el().size(5., 5.).radius(3.).fill(ink))
        .child(label(status, 11.5, ink));
    let mut header = row()
        .w_full()
        .center_y()
        .gap(7.)
        .child(toggle)
        .child(status_chip);
    if !open && !copy.is_empty() {
        header = header.child(copy_button(&format!("{key}-copy"), copy, p));
    }
    let mut body = col()
        .key(&key)
        .w_full()
        .gap(8.)
        .padding(10.)
        .radius(13.)
        .fill(p.bg_elevated)
        .border(
            1.,
            if card.is_error {
                p.danger
            } else {
                p.border_strong
            },
        )
        .child(header);
    if open && !card.code_or_cmd.is_empty() {
        body = body.child(console(
            view,
            &format!("{key}-src"),
            if card.name == "bash" {
                "Command"
            } else {
                "Source"
            },
            &card.code_or_cmd,
            lang,
            36,
            p,
        ));
    }
    if open || detail.expand_diffs() {
        for (i, diff) in card
            .edit_diff
            .iter()
            .chain(card.python.iter().flat_map(|py| &py.diffs))
            .enumerate()
        {
            body = body.child(diff_card(view, &format!("{key}-diff-{i}"), diff, p));
        }
    }
    if !open {
        return body;
    }
    if card.image_count > 0 {
        body = body.child(label(
            format!("{} image(s) attached", card.image_count),
            12.,
            p.info,
        ));
    }
    if let Some(py) = &card.python {
        if let Some(cmd) = &py.bash_first {
            body = body.child(
                label(
                    format!("Shell ({})  $ {cmd}", py.bash_count),
                    12.,
                    p.warning,
                )
                .wrap_anywhere(),
            );
        }
        for receipt in &py.sent_mail {
            body = body.child(label(
                format!("Message to {} ({})", receipt.target, receipt.status),
                12.,
                p.info,
            ));
        }
        let trace = if py.traceback.is_empty() {
            py.error_name
                .as_ref()
                .map(|name| format!("{name}: {}", py.error_value.as_deref().unwrap_or_default()))
                .unwrap_or_default()
        } else {
            py.traceback.join("\n")
        };
        for (name, value) in [
            ("stdout", py.stdout.as_str()),
            ("result", py.return_value.as_deref().unwrap_or_default()),
            (
                "background",
                py.background_output.as_deref().unwrap_or_default(),
            ),
            ("stderr", &py.stderr),
            ("traceback", &trace),
        ] {
            if !value.is_empty() {
                body = body.child(console(
                    view,
                    &format!("{key}-{name}"),
                    name,
                    value,
                    None,
                    if detail.expand_tool_output() { 120 } else { 18 },
                    p,
                ));
            }
        }
    } else if !card.text_output.is_empty() {
        body = body.child(console(
            view,
            &format!("{key}-output"),
            "Output",
            &card.text_output,
            None,
            if detail.expand_tool_output() || card.is_error {
                120
            } else {
                18
            },
            p,
        ));
    }
    body
}

fn diff_card(view: &PrimeDesktopView, key: &str, diff: &DiffView, p: Palette) -> El {
    let title = format!(
        "{}    +{}  −{}",
        diff.path, diff.added_lines, diff.removed_lines
    );
    console(view, key, &title, &diff.unified, Some("diff"), 36, p)
}

pub(super) fn console(
    view: &PrimeDesktopView,
    key: &str,
    title: &str,
    value: &str,
    language: Option<&str>,
    limit: usize,
    p: Palette,
) -> El {
    let clean = strip_ansi(value);
    let clipped = clean
        .match_indices('\n')
        .nth(limit - 1)
        .map(|(end, _)| end)
        .filter(|end| end + 1 < clean.len());
    let expanded = view.session.is_expanded(key, false);
    let shown = if let Some(end) = clipped.filter(|_| !expanded) {
        &clean[..end]
    } else {
        &clean
    };
    let mut body = col()
        .w_full()
        .min_w(0.)
        .radius(8.)
        .hover_group()
        .fill(p.bg_code)
        .px(12.)
        .py(8.)
        .gap(3.)
        .child(
            row()
                .justify_between()
                .center_y()
                .child(
                    label(title, 12., p.text_secondary)
                        .font(TextStyle::new(12., 600.))
                        .min_w(0.)
                        .shrink(),
                )
                .child(copy_button(&format!("{key}-copy"), &clean, p).group_opacity(0.55, 1.)),
        )
        .child(
            col()
                .key(format!("{key}-scroll"))
                .scroll_x()
                .child(code_text(shown, language, p).key(format!("{key}-text"))),
        );
    if clipped.is_some() {
        let caption = if expanded {
            "Show fewer lines".into()
        } else {
            format!("Show all {} lines", clean.lines().count())
        };
        body = body.child(disclosure(key, &caption, expanded, p).self_start());
    }
    body
}

#[cfg(test)]
mod tests {
    use super::console;
    use crate::theme::{Palette, ThemeMode};
    use crate::ui::workspace::{LaunchOptions, PrimeDesktopView};
    use rune::{Env, Runtime, clipboard, col};
    use tempfile::TempDir;

    #[test]
    fn collapsed_console_copies_all_lines_and_can_expand() {
        const OUTPUT: &str = "\x1b[32mfirst\x1b[0m\n\nthird\nlast";
        let profile = TempDir::new().unwrap();
        let options = LaunchOptions {
            initial_cwd: Some(profile.path().into()),
            daemon_socket: Some(profile.path().join("isolated.sock")),
            ..LaunchOptions::default()
        };
        let view = PrimeDesktopView::new(options, profile.path().into());
        let mut app = Runtime::new(
            view,
            |view, _| {
                col().w_full().child(console(
                    view,
                    "test",
                    "Output",
                    OUTPUT,
                    None,
                    2,
                    Palette::for_mode(ThemeMode::Light),
                ))
            },
            (280., 240.),
            Env::new(false, 1.),
        );
        let board = clipboard::memory();
        app.use_clipboard(&board);
        app.frame(0.);
        let output = |app: &Runtime<PrimeDesktopView>| {
            app.elements()
                .into_iter()
                .find(|e| e.key.as_deref() == Some("test-text"))
                .unwrap()
                .text
                .unwrap()
        };
        assert_eq!(output(&app), "first\n");
        for key in ["test-copy", "test-hdr"] {
            let (x, y) = app.find(key).unwrap();
            app.pointer_down(x, y);
            app.pointer_up(x, y);
            app.frame(0.);
        }
        assert_eq!(board.text().unwrap().as_str(), "first\n\nthird\nlast");
        assert_eq!(output(&app), "first\n\nthird\nlast");
    }
}
