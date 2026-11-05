//! Measure and paint from the same shaped visual rows. Offsets stored on rows always
//! refer to the source string, including when the glyphs being painted are bullets.

use std::{cell::RefCell, ops::Range, rc::Rc};

use gpui::{
    App, AvailableSpace, Bounds, Element, ElementId, ElementInputHandler, Entity, GlobalElementId,
    Hsla, InspectorElementId, IntoElement, LayoutId, PaintQuad, Pixels, SharedString, Style,
    TextRun, UnderlineStyle, Window, fill, font, point, px, relative, size,
};

use unicode_segmentation::UnicodeSegmentation;

use super::{InputKind, TextInput, VisualLine};

pub(super) struct TextCanvas {
    pub(super) input: Entity<TextInput>,
}

#[derive(Default)]
pub(super) struct MeasuredRows {
    width: Option<Pixels>,
    rows: Vec<VisualLine>,
}

pub(super) struct CanvasPrepaint {
    visible: Vec<VisualLine>,
    selections: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
    scroll_x: Pixels,
    line_height: Pixels,
}

impl TextInput {
    pub(super) fn layout_rows(&self, width: Option<Pixels>, window: &Window) -> Vec<VisualLine> {
        let font = font(self.font_family.clone());
        let font_size = px(self.font_size_px);
        let text_system = window.text_system();
        if self.content.is_empty() || self.kind == InputKind::Secret {
            let (text, color): (SharedString, _) = if self.content.is_empty() {
                (self.placeholder.clone(), self.placeholder_color)
            } else {
                (
                    "•".repeat(self.content.chars().count()).into(),
                    self.text_color,
                )
            };
            let runs = build_runs_for_slice(&(0..text.len()), text.len(), None, &font, color);
            return vec![VisualLine {
                shaped: text_system.shape_line(text, font_size, &runs, None),
                byte_range: 0..self.content.len(),
            }];
        }

        let mut rows = Vec::new();
        let mut offset = 0;
        for logical in self.content.split('\n') {
            let mut start = 0;
            let mut push_row = |end: usize| {
                if end <= start && !logical.is_empty() {
                    return;
                }
                let byte_range = offset + start..offset + end;
                let runs = build_runs_for_slice(
                    &byte_range,
                    end - start,
                    self.marked_range.as_ref(),
                    &font,
                    self.text_color,
                );
                let shaped = text_system.shape_line(
                    logical[start..end].to_owned().into(),
                    font_size,
                    &runs,
                    None,
                );
                rows.push(VisualLine { byte_range, shaped });
                start = end;
            };
            if self.allows_newlines()
                && !logical.is_empty()
                && let Some(width) = width
            {
                let runs = build_runs_for_slice(
                    &(0..logical.len()),
                    logical.len(),
                    None,
                    &font,
                    self.text_color,
                );
                if let Ok(lines) = text_system.shape_text(
                    logical.to_owned().into(),
                    font_size,
                    &runs,
                    Some(width.max(px(1.0))),
                    None,
                ) && let Some(line) = lines.first()
                {
                    let mut clusters = logical.grapheme_indices(true).map(|(i, _)| i).peekable();
                    for boundary in line.wrap_boundaries() {
                        let index = line.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index;
                        while clusters.next_if(|i| *i < index).is_some() {}
                        if let Some(&end) = clusters.peek() {
                            push_row(end);
                        }
                    }
                }
            }
            push_row(logical.len());
            offset += logical.len() + 1;
        }
        rows
    }

    fn update_viewport(&mut self, bounds: Bounds<Pixels>, cursor_row: usize) {
        let limit = self.visual_lines.len().saturating_sub(self.max_lines);
        self.scroll_row_offset = self.scroll_row_offset.min(limit);
        if self.reveal_caret
            || self
                .last_bounds
                .is_none_or(|old| old.size.width != bounds.size.width)
        {
            self.scroll_row_offset = self
                .scroll_row_offset
                .clamp((cursor_row + 1).saturating_sub(self.max_lines), cursor_row);
        }
        self.reveal_caret = false;
        self.scroll_x = if self.allows_newlines() || self.content.is_empty() {
            px(0.0)
        } else {
            let line = &self.visual_lines[cursor_row];
            let caret_x = self.x_for_offset(line, self.cursor_offset());
            let width = (bounds.size.width - px(2.0)).max(px(0.0));
            self.scroll_x
                .min((line.shaped.width - width).max(px(0.0)))
                .clamp((caret_x - width).max(px(0.0)), caret_x)
        };
    }
}

impl IntoElement for TextCanvas {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextCanvas {
    type RequestLayoutState = Rc<RefCell<MeasuredRows>>;
    type PrepaintState = CanvasPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        let measured = Rc::new(RefCell::new(MeasuredRows::default()));
        let cache = measured.clone();
        let input = self.input.clone();
        let layout = window.request_measured_layout(style, move |known, available, window, cx| {
            let state = input.read(cx);
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(w) => Some(w),
                AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
            });
            let mut cache = cache.borrow_mut();
            if cache.rows.is_empty() || cache.width != width {
                cache.rows = state.layout_rows(width, window);
                cache.width = width;
            }
            let rows = cache.rows.len().clamp(state.min_lines, state.max_lines);
            let intrinsic_width = || {
                cache
                    .rows
                    .iter()
                    .map(|l| l.shaped.width)
                    .max()
                    .unwrap_or_default()
            };
            size(
                width.unwrap_or_else(intrinsic_width),
                px((state.font_size_px * 1.48).round()) * rows,
            )
        });
        (layout, measured)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        measured: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> CanvasPrepaint {
        self.input.update(cx, |state, _| {
            let mut cache = measured.borrow_mut();
            state.visual_lines = if cache.width == Some(bounds.size.width) && !cache.rows.is_empty()
            {
                std::mem::take(&mut cache.rows)
            } else {
                state.layout_rows(Some(bounds.size.width), window)
            };
            let offset = state.cursor_offset();
            let cursor_row = state.row_for_offset(offset);
            state.update_viewport(bounds, cursor_row);
            let first = state.scroll_row_offset;
            let end = (first + state.max_lines).min(state.visual_lines.len());
            let visible = &state.visual_lines[first..end];
            let lh = px((state.font_size_px * 1.48).round());
            let left = bounds.left() - state.scroll_x;
            let selection = &state.selected_range;
            let focused = state.focus_handle.is_focused(window);
            let color = if focused {
                state.selection_color
            } else {
                state.selection_inactive_color
            };
            let mut cursor = None;
            let mut selections = Vec::new();
            for (i, row) in visible.iter().enumerate() {
                let top = bounds.top() + lh * i;
                if selection.is_empty() {
                    if focused && cursor_row == first + i {
                        cursor = Some(fill(
                            Bounds::new(
                                point(left + state.x_for_offset(row, offset), top),
                                size(px(2.0), lh),
                            ),
                            state.accent_color,
                        ));
                    }
                } else {
                    let from = selection.start.max(row.byte_range.start);
                    let to = selection.end.min(row.byte_range.end);
                    let newline = selection.start <= row.byte_range.end
                        && row.byte_range.end < selection.end
                        && state.content.as_bytes().get(row.byte_range.end) == Some(&b'\n');
                    if from < to || newline {
                        let x0 = left + state.x_for_offset(row, from);
                        let x1 = left
                            + state.x_for_offset(row, to)
                            + if newline { px(6.0) } else { px(0.0) };
                        selections.push(fill(
                            Bounds::from_corners(
                                point(x0, top),
                                point(x1.max(x0 + px(1.0)), top + lh),
                            ),
                            color,
                        ));
                    }
                }
            }
            state.last_bounds = Some(bounds);
            state.last_line_height = lh;
            CanvasPrepaint {
                visible: visible.to_vec(),
                selections,
                cursor,
                scroll_x: state.scroll_x,
                line_height: lh,
            }
        })
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut CanvasPrepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        for quad in prepaint.selections.drain(..) {
            window.paint_quad(quad);
        }
        for (row, line) in prepaint.visible.iter().enumerate() {
            let origin = point(
                bounds.left() - prepaint.scroll_x,
                bounds.top() + prepaint.line_height * row,
            );
            let _ = line.shaped.paint(origin, prepaint.line_height, window, cx);
        }
        if let Some(caret) = prepaint.cursor.take() {
            window.paint_quad(caret);
        }
    }
}

fn build_runs_for_slice(
    abs_range: &Range<usize>,
    slice_len: usize,
    marked: Option<&Range<usize>>,
    font: &gpui::Font,
    color: Hsla,
) -> Vec<TextRun> {
    let base = TextRun {
        len: slice_len,
        font: font.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let Some(m) = marked else {
        return vec![base];
    };
    let m_start = m.start.max(abs_range.start);
    let m_end = m.end.min(abs_range.end);
    if m_start >= m_end {
        return vec![base];
    }
    let rel_start = m_start - abs_range.start;
    let rel_end = m_end - abs_range.start;
    let mut runs = Vec::new();
    if rel_start > 0 {
        runs.push(TextRun {
            len: rel_start,
            ..base.clone()
        });
    }
    runs.push(TextRun {
        len: rel_end - rel_start,
        underline: Some(UnderlineStyle {
            color: Some(color),
            thickness: px(1.0),
            wavy: false,
        }),
        ..base.clone()
    });
    if rel_end < slice_len {
        runs.push(TextRun {
            len: slice_len - rel_end,
            ..base
        });
    }
    runs
}
