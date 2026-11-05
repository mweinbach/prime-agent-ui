//! `CommonMark` and GFM transcript blocks, rendered with native GPUI text runs.

mod document;
mod streaming;
#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::ops::Range;

use document::{Block, Inline, Kind};
use gpui::{
    AnyElement, FontStyle, FontWeight, Hsla, IntoElement, SharedString, StrikethroughStyle,
    TextAlign, TextRun, UnderlineStyle, div, font, prelude::*, px,
};
use pulldown_cmark::{Alignment, BlockQuoteKind, HeadingLevel};

use super::selectable_text::{SelectableText, copy_button};
use super::syntax;
use super::tool_cards::{horizontal_scroll, limited_lines};
use crate::assets::Icon;
use crate::theme::{Palette, font_display, font_mono, font_sans};

/// Render a complete Markdown document.
#[must_use]
pub fn render_markdown(id_prefix: &str, markdown: &str, palette: Palette) -> AnyElement {
    render_markdown_streaming(id_prefix, markdown, false, palette)
}

/// Render a streaming prefix without exposing incomplete fence or link delimiters.
#[must_use]
pub fn render_markdown_streaming(
    id_prefix: &str,
    markdown: &str,
    streaming: bool,
    palette: Palette,
) -> AnyElement {
    let source = if streaming {
        streaming::normalize(markdown)
    } else {
        Cow::Borrowed(markdown)
    };
    Renderer {
        prefix: id_prefix,
        serial: 0,
        palette,
    }
    .block(document::parse(&source), &TypeStyle::body(palette))
}

struct Renderer<'a> {
    prefix: &'a str,
    serial: usize,
    palette: Palette,
}

#[derive(Clone)]
struct TypeStyle {
    family: SharedString,
    weight: FontWeight,
    ink: Hsla,
}

impl TypeStyle {
    fn body(palette: Palette) -> Self {
        Self {
            family: font_sans(),
            weight: FontWeight::NORMAL,
            ink: palette.text_primary,
        }
    }
}

type Links = Vec<(Range<usize>, String)>;

fn inline_runs(inline: &Inline, base: &TypeStyle, palette: Palette) -> (Vec<TextRun>, Links) {
    let mut links: Links = Vec::new();
    let runs = inline
        .spans
        .iter()
        .map(|(range, style)| {
            let mut face = font(if style.code {
                font_mono()
            } else {
                base.family.clone()
            });
            face.weight = if style.bold {
                FontWeight::BOLD
            } else if style.code && base.weight < FontWeight::MEDIUM {
                FontWeight::MEDIUM
            } else {
                base.weight
            };
            if style.italic {
                face.style = FontStyle::Italic;
            }
            if let Some(url) = &style.link {
                if let Some((previous, previous_url)) = links.last_mut()
                    && previous.end == range.start
                    && previous_url.as_str() == url.as_ref()
                {
                    previous.end = range.end;
                } else {
                    links.push((range.clone(), url.to_string()));
                }
            }
            let ink = if style.link.is_some() {
                palette.info
            } else if style.code || style.checkbox {
                palette.accent
            } else {
                base.ink
            };
            TextRun {
                len: range.len(),
                font: face,
                color: ink,
                background_color: style.code.then_some(palette.bg_code),
                underline: style.link.as_ref().map(|_| UnderlineStyle {
                    color: Some(ink),
                    thickness: px(1.0),
                    wavy: false,
                }),
                strikethrough: style.strike.then_some(StrikethroughStyle {
                    thickness: px(1.0),
                    color: Some(ink),
                }),
            }
        })
        .collect();
    (runs, links)
}

impl Renderer<'_> {
    fn id(&mut self) -> String {
        self.serial += 1;
        format!("{}-md-{}", self.prefix, self.serial)
    }

    fn block(&mut self, block: Block, style: &TypeStyle) -> AnyElement {
        let palette = self.palette;
        match block {
            Block::Text(inline) => {
                let (runs, links) = inline_runs(&inline, style, palette);
                SelectableText::new(self.id(), inline.text, palette)
                    .with_runs(runs)
                    .with_links(links)
                    .into_any_element()
            }
            Block::Rule => div()
                .w_full()
                .h(px(1.))
                .my(px(6.))
                .bg(palette.border_subtle)
                .into_any_element(),
            Block::Code(lang, text) => render_code_block(&self.id(), &lang, &text, palette),
            Block::Group(kind, children) => match kind {
                Kind::Root => div()
                    .flex()
                    .flex_col()
                    .gap(px(9.))
                    .w_full()
                    .min_w_0()
                    .text_size(px(14.5))
                    .line_height(px(23.))
                    .children(children.into_iter().map(|b| self.block(b, style)))
                    .into_any_element(),
                Kind::Heading(level) => {
                    let (size, height, family) = match level {
                        HeadingLevel::H1 => (22., 30., font_display()),
                        HeadingLevel::H2 => (19., 27., font_display()),
                        HeadingLevel::H3 => (16.5, 24., font_display()),
                        HeadingLevel::H4 => (15., 22., font_sans()),
                        HeadingLevel::H5 => (14., 21., font_sans()),
                        HeadingLevel::H6 => (13., 20., font_sans()),
                    };
                    let heading = TypeStyle {
                        family,
                        weight: FontWeight::SEMIBOLD,
                        ink: style.ink,
                    };
                    div()
                        .min_w_0()
                        .pt(px(5.))
                        .text_size(px(size))
                        .line_height(px(height))
                        .children(children.into_iter().map(|b| self.block(b, &heading)))
                        .into_any_element()
                }
                Kind::Quote(alert) => self.quote(alert, children, style),
                Kind::List(start) => {
                    let rows = children.into_iter().enumerate().map(|(i, child)| {
                        let marker = start.map_or_else(
                            || "•".to_owned(),
                            |n| format!("{}.", n.saturating_add(i as u64)),
                        );
                        div()
                            .flex()
                            .w_full()
                            .min_w_0()
                            .gap(px(7.))
                            .child(
                                div()
                                    .flex_none()
                                    .text_color(palette.text_secondary)
                                    .child(marker),
                            )
                            .child(div().flex_1().min_w_0().child(self.block(child, style)))
                    });
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(5.))
                        .min_w_0()
                        .children(rows)
                        .into_any_element()
                }
                Kind::Table(align) => self.table(&align, children, style),
                Kind::Item | Kind::Paragraph | Kind::Cell | Kind::Row => div()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .min_w_0()
                    .children(children.into_iter().map(|b| self.block(b, style)))
                    .into_any_element(),
                Kind::Code(_) => unreachable!("code frames finish as leaf blocks"),
            },
        }
    }

    fn quote(
        &mut self,
        kind: Option<BlockQuoteKind>,
        children: Vec<Block>,
        style: &TypeStyle,
    ) -> AnyElement {
        let p = self.palette;
        let (ink, icon, title) = match kind {
            None => (p.border_strong, Icon::MessageSquare, ""),
            Some(BlockQuoteKind::Note) => (p.info, Icon::MessageSquare, "Note"),
            Some(BlockQuoteKind::Tip) => (p.success, Icon::Sparkles, "Tip"),
            Some(BlockQuoteKind::Important) => (p.accent, Icon::Zap, "Important"),
            Some(BlockQuoteKind::Warning) => (p.warning, Icon::Shield, "Warning"),
            Some(BlockQuoteKind::Caution) => (p.danger, Icon::Shield, "Caution"),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .min_w_0()
            .pl(px(12.))
            .py(px(5.))
            .border_l_2()
            .border_color(ink)
            .bg(p.bg_hover)
            .rounded_r_md()
            .when(kind.is_some(), |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ink)
                        .child(icon.render(14., ink))
                        .child(title),
                )
            })
            .children(children.into_iter().map(|b| self.block(b, style)))
            .into_any_element()
    }

    fn table(&mut self, align: &[Alignment], rows: Vec<Block>, style: &TypeStyle) -> AnyElement {
        let p = self.palette;
        let id = self.id();
        let mut table = div()
            .flex()
            .flex_col()
            .w_full()
            .min_w(px(108. * align.len() as f32))
            .rounded_md()
            .border_1()
            .border_color(p.border_subtle);
        for (index, row) in rows.into_iter().enumerate() {
            let Block::Group(Kind::Row, cells) = row else {
                continue;
            };
            let mut cells = cells.into_iter();
            let header = index == 0;
            let face = TypeStyle {
                weight: if header {
                    FontWeight::SEMIBOLD
                } else {
                    style.weight
                },
                ink: if header {
                    p.text_primary
                } else {
                    p.text_secondary
                },
                family: style.family.clone(),
            };
            let row = div()
                .flex()
                .w_full()
                .px(px(10.))
                .py(px(6.))
                .gap(px(12.))
                .when(header, |d| d.bg(p.bg_elevated))
                .when(!header, |d| d.border_t_1().border_color(p.border_subtle))
                .children(align.iter().map(|alignment| {
                    let align = match alignment {
                        Alignment::None | Alignment::Left => TextAlign::Left,
                        Alignment::Center => TextAlign::Center,
                        Alignment::Right => TextAlign::Right,
                    };
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(13.))
                        .text_align(align)
                        .children(cells.next().map(|b| self.block(b, &face)))
                }));
            table = table.child(row);
        }
        div()
            .id(SharedString::from(id))
            .w_full()
            .min_w_0()
            .overflow_x_scroll()
            .child(table)
            .into_any_element()
    }
}

/// Render a selectable code pane. Expansion and copy always operate on the same source.
#[must_use]
pub fn render_code_block(id: &str, lang: &str, code: &str, palette: Palette) -> AnyElement {
    let text: SharedString = code.to_owned().into();
    let language: SharedString = lang.to_owned().into();
    let content_id = format!("{id}-source");
    let scroll_id: SharedString = format!("{id}-scroll").into();
    div()
        .w_full()
        .min_w_0()
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
                .px(px(12.))
                .py(px(5.))
                .border_b_1()
                .border_color(palette.border_subtle)
                .bg(palette.bg_elevated)
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.5))
                        .font_family(font_mono())
                        .text_color(palette.text_secondary)
                        .child(if lang.is_empty() {
                            "code".into()
                        } else {
                            language.clone()
                        }),
                )
                .child(copy_button(format!("{id}-copy"), text.clone(), palette)),
        )
        .child(limited_lines(
            format!("{id}-lines"),
            text,
            160,
            palette,
            move |visible| {
                let runs = syntax::highlight(&language, visible, palette);
                horizontal_scroll(
                    scroll_id,
                    SelectableText::new(content_id, visible.to_owned(), palette).with_runs(runs),
                )
                .p(px(12.))
                .text_size(px(12.5))
                .line_height(px(19.))
                .font_family(font_mono())
                .into_any_element()
            },
        ))
        .into_any_element()
}
