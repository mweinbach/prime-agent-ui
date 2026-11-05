//! A block owns its inline content. In particular, tight list items can contain
//! text directly, before and after nested block containers.
use std::ops::Range;

use gpui::SharedString;
use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag,
    TagEnd,
};

#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub(super) struct InlineStyle {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub checkbox: bool,
    pub link: Option<SharedString>,
}

#[derive(Default, Debug)]
pub(super) struct Inline {
    pub text: String,
    pub spans: Vec<(Range<usize>, InlineStyle)>,
}

impl Inline {
    fn append(&mut self, value: &str, style: &InlineStyle) {
        if value.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(value);
        if let Some((range, last_style)) = self.spans.last_mut()
            && last_style == style
        {
            range.end = self.text.len();
        } else {
            self.spans.push((start..self.text.len(), style.clone()));
        }
    }
}

#[derive(Debug)]
pub(super) enum Kind {
    Root,
    Paragraph,
    Heading(HeadingLevel),
    Quote(Option<BlockQuoteKind>),
    List(Option<u64>),
    Item,
    Table(Vec<Alignment>),
    Row,
    Cell,
    Code(String),
}

#[derive(Debug)]
pub(super) enum Block {
    Group(Kind, Vec<Self>),
    Text(Inline),
    Code(String, String),
    Rule,
}

struct Frame {
    kind: Kind,
    children: Vec<Block>,
    inline: Inline,
}

impl Frame {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            children: Vec::new(),
            inline: Inline::default(),
        }
    }

    fn flush(&mut self) {
        if !self.inline.text.is_empty() {
            self.children
                .push(Block::Text(std::mem::take(&mut self.inline)));
        }
    }

    fn finish(mut self) -> Block {
        if let Kind::Code(lang) = self.kind {
            // CommonMark appends a final LF, including to unterminated fences.
            if self.inline.text.ends_with('\n') {
                self.inline.text.pop();
            }
            return Block::Code(lang, self.inline.text);
        }
        self.flush();
        Block::Group(self.kind, self.children)
    }
}

fn block_kind(tag: &Tag<'_>) -> Option<Kind> {
    Some(match tag {
        Tag::Paragraph | Tag::HtmlBlock => Kind::Paragraph,
        Tag::Heading { level, .. } => Kind::Heading(*level),
        Tag::BlockQuote(kind) => Kind::Quote(*kind),
        Tag::List(start) => Kind::List(*start),
        Tag::Item => Kind::Item,
        Tag::Table(align) => Kind::Table(align.clone()),
        Tag::TableHead | Tag::TableRow => Kind::Row,
        Tag::TableCell => Kind::Cell,
        Tag::CodeBlock(code) => Kind::Code(match code {
            CodeBlockKind::Fenced(info) => {
                info.split_whitespace().next().unwrap_or_default().into()
            }
            CodeBlockKind::Indented => String::new(),
        }),
        _ => return None,
    })
}

pub(super) fn parse(markdown: &str) -> Block {
    let options = Options::ENABLE_GFM
        | Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH;
    let mut frames = vec![Frame::new(Kind::Root)];
    let mut styles = vec![InlineStyle::default()];
    // Keep the rendered tree shallow even for untrusted, pathologically nested output.
    let mut flattened_depth = 0usize;
    for event in Parser::new_ext(markdown, options) {
        let frame = frames.last_mut().expect("document root");
        let style = styles.last().expect("base style");
        match event {
            Event::Start(tag) => {
                if let Some(kind) = block_kind(&tag) {
                    frame.flush();
                    if frames.len() < 64 {
                        frames.push(Frame::new(kind));
                    } else {
                        flattened_depth += 1;
                    }
                } else {
                    let mut next = style.clone();
                    match tag {
                        Tag::Emphasis => next.italic = true,
                        Tag::Strong => next.bold = true,
                        Tag::Strikethrough => next.strike = true,
                        Tag::Link {
                            dest_url,
                            link_type,
                            ..
                        } => {
                            next.link = Some(if link_type == LinkType::Email {
                                format!("mailto:{dest_url}").into()
                            } else {
                                dest_url.to_string().into()
                            });
                        }
                        _ => {}
                    }
                    styles.push(next);
                }
            }
            Event::End(end) => match end {
                TagEnd::Paragraph
                | TagEnd::HtmlBlock
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote(_)
                | TagEnd::List(_)
                | TagEnd::Item
                | TagEnd::Table
                | TagEnd::TableRow
                | TagEnd::TableHead
                | TagEnd::TableCell
                | TagEnd::CodeBlock => {
                    if flattened_depth > 0 {
                        flattened_depth -= 1;
                    } else {
                        let block = frames.pop().expect("balanced block").finish();
                        frames
                            .last_mut()
                            .expect("parent block")
                            .children
                            .push(block);
                    }
                }
                _ => {
                    styles.pop();
                }
            },
            Event::Text(value) => frame.inline.append(&value, style),
            Event::Code(value) => frame.inline.append(
                &value,
                &InlineStyle {
                    code: true,
                    ..style.clone()
                },
            ),
            Event::Html(value) | Event::InlineHtml(value) => {
                let trimmed = value.trim();
                let br = ["<br>", "<br/>", "<br />"]
                    .iter()
                    .any(|br| trimmed.eq_ignore_ascii_case(br));
                if br {
                    frame.inline.append(
                        if matches!(frame.kind, Kind::Cell) {
                            " "
                        } else {
                            "\n"
                        },
                        style,
                    );
                } else {
                    frame.inline.append(&value, style);
                }
            }
            Event::HardBreak if !matches!(frame.kind, Kind::Cell) => {
                frame.inline.append("\n", style);
            }
            Event::HardBreak | Event::SoftBreak => frame.inline.append(" ", style),
            Event::TaskListMarker(checked) => frame.inline.append(
                if checked { "☑ " } else { "☐ " },
                &InlineStyle {
                    checkbox: true,
                    ..style.clone()
                },
            ),
            Event::Rule => {
                frame.flush();
                frame.children.push(Block::Rule);
            }
            // These events require parser extensions which we do not enable.
            Event::InlineMath(_) | Event::DisplayMath(_) | Event::FootnoteReference(_) => {}
        }
    }
    frames.pop().expect("document root").finish()
}
