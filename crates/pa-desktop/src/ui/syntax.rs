//! Syntax coloring for tool code, diffs and terminal output.

use crate::theme::{Palette, ThemeMode};
use rune::{El, TextStyle, rich};
use rune_kit::syntax::{self, Syntax, SyntaxTheme};

pub(super) fn code_text(source: &str, language: Option<&str>, p: Palette) -> El {
    let theme = SyntaxTheme::neutral(p.mode == ThemeMode::Dark)
        .with(Syntax::Keyword, p.accent)
        .with(Syntax::Inserted, p.success)
        .with(Syntax::Deleted, p.danger)
        .with(Syntax::Comment, p.text_muted);
    let spans = theme.spans(&syntax::highlight(language, source), 0);
    rich(source, spans)
        .font(TextStyle::new(12.5, 400.).mono().lh(18.))
        .color(p.text_primary)
        .selectable(true)
        .selection_color(p.selection_bg)
}

pub(super) fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {}
            '\x1b' => match chars.peek() {
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '\x07' || c == '\x1b' && chars.next_if_eq(&'\\').is_some() {
                            break;
                        }
                    }
                }
                _ => {}
            },
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::strip_ansi;

    #[test]
    fn cleans_terminal_colors_and_links_without_losing_text() {
        assert_eq!(
            strip_ansi("\x1b[31mred\x1b[0m\r\n\x1b]8;;https://a\x07猫\x1b]8;;\x1b\\ end"),
            "red\n猫 end"
        );
    }
}
