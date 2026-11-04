//! Fast `pulldown-cmark` Markdown -> GPUI element renderer with syntax-tinted code blocks.

use std::ops::Range;

use gpui::{
    AnyElement, ClipboardItem, FontStyle, FontWeight, HighlightStyle, IntoElement, SharedString,
    StyledText, UnderlineStyle, div, prelude::*, px,
};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::assets::Icon;
use crate::theme::{Palette, font_mono, font_sans, font_serif};

type Highlights = Vec<(Range<usize>, HighlightStyle)>;
type ListItemRow = (usize, String, String, Highlights);

// StyledText takes non-overlapping byte ranges. Markdown emphasis, code, and links can nest.
fn inline_highlights(
    text: &str,
    spans: Highlights,
) -> impl Iterator<Item = (Range<usize>, HighlightStyle)> + use<> {
    let len = text.trim_end().len();
    gpui::combine_highlights(
        spans
            .into_iter()
            .map(move |(r, style)| (r.start.min(len)..r.end.min(len), style)),
        [],
    )
}

/// Render Markdown prose & code blocks into a vertical stack of GPUI elements.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn render_markdown(id_prefix: &str, markdown: &str, palette: Palette) -> AnyElement {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(markdown, opts);
    let mut children: Vec<AnyElement> = Vec::new();

    let mut inline_text = String::new();
    let mut highlights: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    let mut bold_start: Option<usize> = None;
    let mut italic_start: Option<usize> = None;
    let mut link_start: Option<usize> = None;

    let mut heading_level: Option<HeadingLevel> = None;
    let mut in_blockquote = false;
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_buf = String::new();

    let mut list_stack: Vec<Option<u64>> = Vec::new();
    let mut list_items: Vec<ListItemRow> = Vec::new();
    let mut in_item = false;

    let mut in_table = false;
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut cell_buf = String::new();
    let mut in_cell = false;

    let mut block_idx = 0usize;

    let flush_paragraph = |text: &mut String,
                           spans: &mut Vec<(Range<usize>, HighlightStyle)>,
                           quote: bool,
                           out: &mut Vec<AnyElement>| {
        let trimmed_len = text.trim_end().len();
        if trimmed_len == 0 {
            text.clear();
            spans.clear();
            return;
        }
        text.truncate(trimmed_len);
        let runs = inline_highlights(text, std::mem::take(spans));
        let body = StyledText::new(SharedString::from(std::mem::take(text))).with_highlights(runs);
        let elem = if quote {
            div()
                .pl(px(12.0))
                .py(px(4.0))
                .border_l_2()
                .border_color(palette.accent)
                .bg(palette.bg_hover)
                .rounded_r_md()
                .text_size(px(14.0))
                .line_height(px(22.0))
                .text_color(palette.text_secondary)
                .font_family(font_sans())
                .child(body)
                .into_any_element()
        } else {
            div()
                .text_size(px(14.5))
                .line_height(px(23.0))
                .text_color(palette.text_primary)
                .font_family(font_sans())
                .child(body)
                .into_any_element()
        };
        out.push(elem);
    };

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    flush_paragraph(
                        &mut inline_text,
                        &mut highlights,
                        in_blockquote,
                        &mut children,
                    );
                    heading_level = Some(level);
                }
                Tag::BlockQuote(_) => {
                    flush_paragraph(
                        &mut inline_text,
                        &mut highlights,
                        in_blockquote,
                        &mut children,
                    );
                    in_blockquote = true;
                }
                Tag::CodeBlock(kind) => {
                    flush_paragraph(
                        &mut inline_text,
                        &mut highlights,
                        in_blockquote,
                        &mut children,
                    );
                    in_code_block = true;
                    code_buf.clear();
                    code_lang = match kind {
                        CodeBlockKind::Fenced(info) => info
                            .split_whitespace()
                            .next()
                            .unwrap_or_default()
                            .to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                }
                Tag::List(start) => {
                    if list_stack.is_empty() {
                        flush_paragraph(
                            &mut inline_text,
                            &mut highlights,
                            in_blockquote,
                            &mut children,
                        );
                    } else if in_item && !inline_text.trim().is_empty() {
                        let depth = list_stack.len().saturating_sub(1);
                        let marker = match list_stack.last_mut() {
                            Some(Some(num)) => {
                                let m = format!("{num}.");
                                *num += 1;
                                m
                            }
                            _ => "•".to_string(),
                        };
                        list_items.push((
                            depth,
                            marker,
                            std::mem::take(&mut inline_text),
                            std::mem::take(&mut highlights),
                        ));
                        inline_text.clear();
                    }
                    list_stack.push(start);
                }
                Tag::Item => {
                    in_item = true;
                    inline_text.clear();
                    highlights.clear();
                }
                Tag::Table(_) => {
                    flush_paragraph(
                        &mut inline_text,
                        &mut highlights,
                        in_blockquote,
                        &mut children,
                    );
                    in_table = true;
                    table_rows.clear();
                }
                Tag::TableHead | Tag::TableRow => {
                    current_row.clear();
                }
                Tag::TableCell => {
                    in_cell = true;
                    cell_buf.clear();
                }
                Tag::Strong => {
                    bold_start = Some(inline_text.len());
                }
                Tag::Emphasis => {
                    italic_start = Some(inline_text.len());
                }
                Tag::Link { .. } => {
                    link_start = Some(inline_text.len());
                }
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Heading(_) => {
                    let level = heading_level.take().unwrap_or(HeadingLevel::H3);
                    let txt = std::mem::take(&mut inline_text);
                    highlights.clear();
                    if !txt.trim().is_empty() {
                        let (sz, lh, serif) = match level {
                            HeadingLevel::H1 => (21.0, 29.0, true),
                            HeadingLevel::H2 => (18.0, 26.0, true),
                            HeadingLevel::H3 => (15.5, 23.0, false),
                            _ => (14.5, 22.0, false),
                        };
                        children.push(
                            div()
                                .pt(px(6.0))
                                .text_size(px(sz))
                                .line_height(px(lh))
                                .font_weight(FontWeight::SEMIBOLD)
                                .font_family(if serif { font_serif() } else { font_sans() })
                                .text_color(palette.text_primary)
                                .child(txt.trim().to_string())
                                .into_any_element(),
                        );
                    }
                }
                TagEnd::Paragraph => {
                    if !in_item && !in_table {
                        flush_paragraph(
                            &mut inline_text,
                            &mut highlights,
                            in_blockquote,
                            &mut children,
                        );
                    }
                }
                TagEnd::BlockQuote(_) => {
                    flush_paragraph(&mut inline_text, &mut highlights, true, &mut children);
                    in_blockquote = false;
                }
                TagEnd::CodeBlock => {
                    in_code_block = false;
                    block_idx += 1;
                    let key = format!("{id_prefix}-cb-{block_idx}");
                    children.push(render_code_block(
                        &key,
                        &code_lang,
                        code_buf.trim_end_matches('\n'),
                        palette,
                    ));
                    code_buf.clear();
                    code_lang.clear();
                }
                TagEnd::Item => {
                    if !inline_text.trim().is_empty() {
                        let depth = list_stack.len().saturating_sub(1);
                        let marker = match list_stack.last_mut() {
                            Some(Some(num)) => {
                                let m = format!("{num}.");
                                *num += 1;
                                m
                            }
                            _ => "•".to_string(),
                        };
                        list_items.push((
                            depth,
                            marker,
                            std::mem::take(&mut inline_text),
                            std::mem::take(&mut highlights),
                        ));
                    }
                    inline_text.clear();
                    highlights.clear();
                    in_item = false;
                }
                TagEnd::List(_) => {
                    list_stack.pop();
                    if list_stack.is_empty() && !list_items.is_empty() {
                        let mut list_div = div().flex().flex_col().gap(px(4.0));
                        for (depth, marker, text, spans) in list_items.drain(..) {
                            let runs = inline_highlights(&text, spans);
                            list_div = list_div.child(
                                div()
                                    .flex()
                                    .gap(px(5.0))
                                    .pl(px((depth as f32) * 16.0 + 4.0))
                                    .text_size(px(14.5))
                                    .line_height(px(22.0))
                                    .text_color(palette.text_primary)
                                    .font_family(font_sans())
                                    .child(marker)
                                    .child(
                                        StyledText::new(SharedString::from(
                                            text.trim_end().to_owned(),
                                        ))
                                        .with_highlights(runs),
                                    ),
                            );
                        }
                        children.push(list_div.into_any_element());
                    }
                }
                TagEnd::TableCell => {
                    in_cell = false;
                    current_row.push(cell_buf.trim().to_string());
                    cell_buf.clear();
                }
                TagEnd::TableHead | TagEnd::TableRow => {
                    if !current_row.is_empty() {
                        table_rows.push(std::mem::take(&mut current_row));
                    }
                }
                TagEnd::Table => {
                    in_table = false;
                    if !table_rows.is_empty() {
                        children.push(render_table(&table_rows, palette));
                        table_rows.clear();
                    }
                }
                TagEnd::Strong => {
                    if let Some(start) = bold_start.take()
                        && start < inline_text.len()
                    {
                        highlights.push((
                            start..inline_text.len(),
                            HighlightStyle {
                                font_weight: Some(FontWeight::SEMIBOLD),
                                ..Default::default()
                            },
                        ));
                    }
                }
                TagEnd::Emphasis => {
                    if let Some(start) = italic_start.take()
                        && start < inline_text.len()
                    {
                        highlights.push((
                            start..inline_text.len(),
                            HighlightStyle {
                                font_style: Some(FontStyle::Italic),
                                ..Default::default()
                            },
                        ));
                    }
                }
                TagEnd::Link => {
                    if let Some(start) = link_start.take()
                        && start < inline_text.len()
                    {
                        highlights.push((
                            start..inline_text.len(),
                            HighlightStyle {
                                color: Some(palette.info),
                                underline: Some(UnderlineStyle {
                                    color: Some(palette.info),
                                    thickness: px(1.0),
                                    wavy: false,
                                }),
                                ..Default::default()
                            },
                        ));
                    }
                }
                _ => {}
            },
            Event::Text(text) => {
                if in_code_block {
                    code_buf.push_str(&text);
                } else if in_cell {
                    cell_buf.push_str(&text);
                } else {
                    inline_text.push_str(&text);
                }
            }
            Event::Code(code) => {
                if in_cell {
                    cell_buf.push('`');
                    cell_buf.push_str(&code);
                    cell_buf.push('`');
                } else {
                    let start = inline_text.len();
                    inline_text.push_str(&code);
                    let end = inline_text.len();
                    highlights.push((
                        start..end,
                        HighlightStyle {
                            color: Some(palette.accent),
                            background_color: Some(palette.bg_code),
                            font_weight: Some(FontWeight::MEDIUM),
                            ..Default::default()
                        },
                    ));
                }
            }
            Event::SoftBreak => {
                if in_code_block {
                    code_buf.push('\n');
                } else if !in_cell {
                    inline_text.push(' ');
                }
            }
            Event::HardBreak => {
                if in_code_block {
                    code_buf.push('\n');
                } else if !in_cell {
                    inline_text.push('\n');
                }
            }
            Event::Rule => {
                flush_paragraph(
                    &mut inline_text,
                    &mut highlights,
                    in_blockquote,
                    &mut children,
                );
                children.push(
                    div()
                        .w_full()
                        .h(px(1.0))
                        .my(px(6.0))
                        .bg(palette.border_subtle)
                        .into_any_element(),
                );
            }
            _ => {}
        }
    }

    flush_paragraph(
        &mut inline_text,
        &mut highlights,
        in_blockquote,
        &mut children,
    );

    div()
        .flex()
        .flex_col()
        .gap(px(9.0))
        .w_full()
        .children(children)
        .into_any_element()
}

/// Render a recessed monospace code block with language badge, Copy button, and syntax tinting.
#[must_use]
pub fn render_code_block(id: &str, lang: &str, code: &str, palette: Palette) -> AnyElement {
    let copy_payload = code.to_string();
    let label = if lang.is_empty() { "code" } else { lang };

    let lines: Vec<AnyElement> = code
        .lines()
        .take(160)
        .map(|line| render_tinted_code_line(line, lang, palette))
        .collect();
    let total_lines = code.lines().count();

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
                .px(px(12.0))
                .py(px(5.0))
                .border_b_1()
                .border_color(palette.border_subtle)
                .bg(palette.bg_elevated)
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_family(font_mono())
                        .text_color(palette.text_muted)
                        .child(label.to_string()),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("{id}-copy")))
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .px(px(7.0))
                        .py(px(2.0))
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(|s| s.bg(palette.bg_hover))
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy_payload.clone()));
                        })
                        .child(Icon::Copy.render(12.0, palette.text_secondary))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(palette.text_secondary)
                                .child("Copy"),
                        ),
                ),
        )
        .child(
            div()
                .id(SharedString::from(format!("{id}-scroll")))
                .p(px(12.0))
                .overflow_x_scroll()
                .font_family(font_mono())
                .text_size(px(12.5))
                .line_height(px(19.0))
                .flex()
                .flex_col()
                .children(lines)
                .when(total_lines > 160, |d| {
                    d.child(
                        div()
                            .pt(px(6.0))
                            .text_size(px(11.5))
                            .text_color(palette.text_muted)
                            .child(format!(
                                "… {} more lines (click Copy for full block)",
                                total_lines - 160
                            )),
                    )
                }),
        )
        .into_any_element()
}

fn render_tinted_code_line(line: &str, lang: &str, palette: Palette) -> AnyElement {
    let trimmed = line.trim_start();
    let color = if trimmed.starts_with('#') || trimmed.starts_with("//") {
        palette.text_muted
    } else if lang == "diff" && line.starts_with('+') {
        palette.success
    } else if lang == "diff" && line.starts_with('-') {
        palette.danger
    } else if [
        "def ",
        "async def ",
        "class ",
        "import ",
        "from ",
        "return ",
        "await ",
        "fn ",
        "pub ",
        "let ",
        "impl ",
        "struct ",
        "enum ",
        "use ",
    ]
    .iter()
    .any(|kw| trimmed.starts_with(kw))
    {
        palette.accent
    } else {
        palette.text_primary
    };

    div()
        .whitespace_nowrap()
        .text_color(color)
        .child(if line.is_empty() {
            " ".to_string()
        } else {
            line.to_string()
        })
        .into_any_element()
}

fn render_table(rows: &[Vec<String>], palette: Palette) -> AnyElement {
    let mut table = div()
        .w_full()
        .rounded_md()
        .border_1()
        .border_color(palette.border_subtle)
        .overflow_hidden()
        .flex()
        .flex_col();

    for (r_idx, row) in rows.iter().enumerate() {
        let is_header = r_idx == 0;
        let mut row_div = div()
            .flex()
            .w_full()
            .px(px(10.0))
            .py(px(6.0))
            .gap(px(12.0))
            .when(is_header, |d| d.bg(palette.bg_elevated))
            .when(r_idx > 0, |d| {
                d.border_t_1().border_color(palette.border_subtle)
            });
        for cell in row {
            row_div = row_div.child(
                div()
                    .flex_1()
                    .text_size(px(12.5))
                    .font_weight(if is_header {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(if is_header {
                        palette.text_primary
                    } else {
                        palette.text_secondary
                    })
                    .child(cell.clone()),
            );
        }
        table = table.child(row_div);
    }

    table.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_markdown_styles_partition_unicode_text_without_overlapping_runs() {
        let source = "鳥abc猫 ";
        let bold = HighlightStyle {
            font_weight: Some(FontWeight::SEMIBOLD),
            ..Default::default()
        };
        let italic = HighlightStyle {
            font_style: Some(FontStyle::Italic),
            ..Default::default()
        };
        let runs = inline_highlights(source, vec![(3..6, italic), (0..source.len(), bold)])
            .map(|(r, st)| (&source[r], st.font_style, st.font_weight))
            .collect::<Vec<_>>();
        assert_eq!(
            runs,
            [
                ("鳥", None, bold.font_weight),
                ("abc", italic.font_style, bold.font_weight),
                ("猫", None, bold.font_weight)
            ]
        );
    }
}
