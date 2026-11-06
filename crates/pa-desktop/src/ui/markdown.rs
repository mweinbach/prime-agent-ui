//! Rune Kit parses, lays out and caches rich documents and streaming responses.

use rune::{El, TextStyle};
use rune_kit::markdown::{Markdown, MarkdownStyle};

use crate::theme::{Palette, ThemeMode};
use crate::ui::workspace::PrimeDesktopView;

pub(super) fn render_markdown(key: &str, content: &str, streaming: bool, p: Palette) -> El {
    let style = MarkdownStyle::neutral(p.mode == ThemeMode::Dark)
        .body_font(TextStyle::new(15., 400.).lh(23.))
        .code_block_font(TextStyle::new(12.5, 400.).mono().lh(18.))
        .text(p.text_primary)
        .muted(p.text_secondary)
        .link(p.info)
        .selection(p.selection_bg)
        .rule(p.border_subtle)
        .quote_rule(p.border_strong)
        .inline_code_bg(p.bg_code)
        .code_block(p.bg_code, p.border_subtle)
        .diff(p.success_soft, p.danger_soft)
        .diff_text(p.success, p.danger);
    Markdown::new(content)
        .streaming(streaming)
        .selectable(true)
        .copy_source(true)
        .scroll_wide(true)
        .tasks_read_only()
        .on_link_tx(|_: &mut PrimeDesktopView, url, tx| {
            let _ = tx.open_url(url);
        })
        .render(&style)
        .key(key)
        .w_full()
        .min_w(0.)
}
