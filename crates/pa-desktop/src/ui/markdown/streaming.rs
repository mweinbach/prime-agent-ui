//! Complete only the final live block. Let `CommonMark` identify its boundaries:
//! a backtick inside a fenced example, table, or previous paragraph must never
//! change the interpretation of the rest of the document.
use std::{borrow::Cow, ops::Range};

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};

pub(super) fn normalize(source: &str) -> Cow<'_, str> {
    if source.ends_with('\n') || !source.contains(['`', '~', '[']) {
        return Cow::Borrowed(source);
    }
    let mut inline_start = None;
    let mut complete_spans: Vec<Range<usize>> = Vec::new();
    for (event, range) in
        Parser::new_ext(source, Options::ENABLE_TABLES | Options::ENABLE_GFM).into_offset_iter()
    {
        // Never interpret punctuation in an already parsed code span, HTML tag,
        // or link as a new delimiter (including a complete reference link).
        if inline_start.is_some()
            && matches!(
                event,
                Event::Code(_)
                    | Event::InlineHtml(_)
                    | Event::Start(Tag::Link { .. } | Tag::Image { .. })
            )
            && complete_spans
                .last()
                .is_none_or(|last| last.end <= range.start)
        {
            complete_spans.push(range.clone());
        }
        if range.end != source.len() {
            continue;
        }
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))) => {
                let fence = &source[range.clone()];
                let Some((header, last)) = fence.rsplit_once('\n') else {
                    // The language itself may still be arriving.
                    return Cow::Borrowed(&source[..range.start]);
                };
                // CommonMark already closes an unfinished block at EOF. Hide the
                // transient one/two backticks while its closing fence is arriving.
                let marker = header.as_bytes()[0];
                let width = header.bytes().take_while(|b| *b == marker).count();
                let last = last.trim_start_matches(' ');
                if !last.is_empty() && last.len() < width && last.bytes().all(|b| b == marker) {
                    return Cow::Borrowed(&source[..source.len() - last.len()]);
                }
                return Cow::Borrowed(source);
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Indented) | Tag::HtmlBlock) => {
                return Cow::Borrowed(source);
            }
            Event::Start(Tag::Paragraph | Tag::Item | Tag::Heading { .. } | Tag::TableCell) => {
                inline_start = Some(range.start);
                complete_spans.clear();
            }
            _ => {}
        }
    }
    inline_start.map_or(Cow::Borrowed(source), |start| {
        normalize_inline(source, start, &complete_spans)
    })
}

fn normalize_inline<'a>(source: &'a str, start: usize, spans: &[Range<usize>]) -> Cow<'a, str> {
    let bytes = source.as_bytes();
    let mut i = start;
    let mut code: Option<(usize, usize)> = None;
    let mut brackets = 0usize;
    let mut destination: Option<(usize, usize)> = None;
    let mut label_end = None;
    let mut protected = spans.iter().peekable();
    while i < bytes.len() {
        if let Some(span) = protected.next_if(|span| i >= span.start) {
            i = i.max(span.end);
            continue;
        }
        let ch = bytes[i];
        if ch == b'\\' && code.is_none() {
            i += 2;
            continue;
        }
        if let Some((_, depth)) = destination.as_mut() {
            match ch {
                b'(' => *depth += 1,
                b')' => {
                    *depth -= 1;
                    if *depth == 0 {
                        destination = None;
                    }
                }
                _ => {}
            }
        } else if ch == b'`' {
            let width = bytes[i..].iter().take_while(|b| **b == b'`').count();
            match code {
                None => code = Some((i, width)),
                Some((_, open)) if open == width => code = None,
                _ => {}
            }
            i += width;
            continue;
        } else if code.is_none() {
            match ch {
                b'[' => brackets += 1,
                b']' if brackets > 0 => {
                    brackets -= 1;
                    label_end = Some(i + 1);
                }
                b'(' if label_end == Some(i) => destination = Some((i, 1)),
                _ => {}
            }
        }
        i += 1;
    }

    if let Some((open, _)) = destination {
        let mut normalized = source[..open].to_owned();
        normalized.push_str("()");
        return Cow::Owned(normalized);
    }
    if code.is_none() && brackets == 0 && label_end != Some(source.len()) {
        return Cow::Borrowed(source);
    }
    let mut normalized = source.to_owned();
    if let Some((open, width)) = code {
        if open + width == bytes.len() {
            normalized.truncate(open);
        } else {
            // A shorter backtick run can legitimately be part of the span.
            if normalized.ends_with('`') {
                normalized.push(' ');
            }
            normalized.extend(std::iter::repeat_n('`', width));
        }
    }
    if brackets > 0 {
        normalized.extend(std::iter::repeat_n(']', brackets));
        normalized.push_str("()");
    } else if label_end == Some(source.len()) {
        normalized.push_str("()");
    }
    Cow::Owned(normalized)
}
