//! Transcript text uses the same shaped glyphs for painting, selection and hit testing.

use std::{ops::Range, rc::Rc, time::Duration};

use gpui::{
    App, Bounds, ClipboardItem, CursorStyle, DispatchPhase, Element, ElementId, Entity,
    FocusHandle, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, RenderOnce,
    SharedString, StyledText, Task, TextAlign, TextLayout, TextRun, WeakEntity, Window,
    WrappedLineLayout, actions, div, fill, point, prelude::*, px, size,
};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::{
    assets::Icon,
    theme::{Palette, font_sans},
    ui::workspace::PrimeDesktopView,
};

actions!(transcript, [CopySelection]);

#[derive(IntoElement)]
pub(super) struct SelectableText {
    id: SharedString,
    text: SharedString,
    runs: Option<Vec<TextRun>>,
    links: Vec<(Range<usize>, String)>,
    selection_bg: Hsla,
}

impl SelectableText {
    pub(super) fn new(
        id: impl Into<SharedString>,
        text: impl Into<SharedString>,
        palette: Palette,
    ) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            runs: None,
            links: Vec::new(),
            selection_bg: palette.selection_bg,
        }
    }

    pub(super) fn with_runs(mut self, runs: Vec<TextRun>) -> Self {
        self.runs = Some(runs);
        self
    }

    pub(super) fn with_links(mut self, links: Vec<(Range<usize>, String)>) -> Self {
        self.links = links
            .into_iter()
            .filter(|(_, url)| {
                ["https://", "http://", "mailto:"].iter().any(|scheme| {
                    url.get(..scheme.len())
                        .is_some_and(|p| p.eq_ignore_ascii_case(scheme))
                }) && !url.contains(char::is_control)
            })
            .collect();
        self
    }
}

/// The workspace holds just the focused block's weak handle. Removing a turn or
/// switching sessions therefore cannot leave a clipboard owner behind.
#[derive(Default)]
pub(super) struct ActiveSelection(Option<WeakEntity<Selection>>);

impl ActiveSelection {
    pub(super) fn copy(&self, window: &Window, cx: &mut App) {
        if let Some(owner) = self.0.as_ref().and_then(WeakEntity::upgrade) {
            let state = owner.read(cx);
            if state.focus.is_focused(window)
                && let Some(value) = state
                    .text
                    .get(state.range.clone())
                    .filter(|s| !s.is_empty())
            {
                cx.write_to_clipboard(ClipboardItem::new_string(value.to_owned()));
            }
        }
    }
}

struct Selection {
    text: SharedString,
    focus: FocusHandle,
    range: Range<usize>,
    drag: Option<Drag>,
}

struct Drag {
    origin: Point<Pixels>,
    anchor: Range<usize>,
    unit: Unit,
    moved: bool,
    link: Option<usize>,
}

#[derive(Clone, Copy)]
enum Unit {
    Caret,
    Word,
    Line,
}

impl Unit {
    fn range(self, text: &str, index: usize) -> Range<usize> {
        let i = text.floor_char_boundary(index.min(text.len()));
        match self {
            Self::Caret => i..i,
            Self::Word => text
                .split_word_bound_indices()
                .find(|(start, word)| i < start + word.len())
                .map_or(i..i, |(start, word)| start..start + word.len()),
            Self::Line => {
                let start = line_start(text, i);
                let end = text[i..].find('\n').map_or(text.len(), |n| i + n + 1);
                start..end
            }
        }
    }
}

fn line_start(text: &str, i: usize) -> usize {
    text[..i].rfind('\n').map_or(0, |n| n + 1)
}

impl RenderOnce for SelectableText {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let selection = window.use_keyed_state(self.id.clone(), cx, |_, cx| Selection {
            text: SharedString::default(),
            focus: cx.focus_handle(),
            range: 0..0,
            drag: None,
        });
        let mut glyphs = StyledText::new(self.text.clone());
        let active = selection.read(cx);
        let focus = active.focus.clone();
        if let Some(mut runs) = self.runs {
            if focus.is_focused(window)
                && !active.range.is_empty()
                && self.text.get(active.range.clone()).is_some()
                && runs.iter().any(|run| run.background_color.is_some())
            {
                runs = selection_runs(runs, &active.range);
            }
            glyphs = glyphs.with_runs(runs);
        }
        let text = self.text;
        if focus.is_focused(window) {
            selection.update(cx, |state, _| state.text.clone_from(&text));
        }
        div()
            .id(self.id.clone())
            .debug_selector(|| self.id.to_string())
            .min_w_0()
            .track_focus(&focus)
            .key_context("TranscriptSelection")
            .child(SelectionGlyphs {
                glyphs,
                events: TextEvents {
                    selection,
                    text,
                    links: Rc::new(self.links),
                },
                selection_bg: self.selection_bg,
            })
    }
}

// StyledText paints inline backgrounds immediately before glyphs. Leave the selection
// quads visible beneath selected inline code while preserving its font and foreground.
fn selection_runs(runs: Vec<TextRun>, selected: &Range<usize>) -> Vec<TextRun> {
    let mut result = Vec::with_capacity(runs.len() + 2);
    let mut start = 0;
    for run in runs {
        let end = start + run.len;
        let a = selected.start.clamp(start, end);
        let b = selected.end.clamp(start, end);
        if run.background_color.is_some() && a < b {
            if a > start {
                result.push(TextRun {
                    len: a - start,
                    ..run.clone()
                });
            }
            result.push(TextRun {
                len: b - a,
                background_color: None,
                ..run.clone()
            });
            if end > b {
                result.push(TextRun {
                    len: end - b,
                    ..run
                });
            }
        } else {
            result.push(run);
        }
        start = end;
    }
    result
}

struct SelectionGlyphs {
    glyphs: StyledText,
    events: TextEvents,
    selection_bg: Hsla,
}

#[derive(Clone)]
struct TextEvents {
    selection: Entity<Selection>,
    text: SharedString,
    links: Rc<Vec<(Range<usize>, String)>>,
}

// GPUI's TextLayout::index_for_position assumes left alignment, while StyledText paints
// each wrapped row using the inherited text alignment (including aligned table cells).
fn aligned_left(bounds: Bounds<Pixels>, width: Pixels, alignment: TextAlign) -> Pixels {
    let spare = bounds.size.width - width;
    bounds.left()
        + match alignment {
            TextAlign::Left => px(0.),
            TextAlign::Center => spare / 2.,
            TextAlign::Right => spare,
        }
}

fn row_boundary(line: &WrappedLineLayout, row: usize) -> usize {
    line.wrap_boundaries.get(row).map_or(line.len(), |b| {
        line.runs()[b.run_ix].glyphs[b.glyph_ix].index
    })
}

#[derive(Clone, Copy)]
struct Hit {
    caret: usize,
    glyph: Option<usize>,
}

fn hit_test(layout: &TextLayout, text: &str, alignment: TextAlign, position: Point<Pixels>) -> Hit {
    let bounds = layout.bounds();
    let lh = layout.line_height();
    // Probe just inside the row. At an exact hard-line boundary GPUI otherwise maps
    // the position to the preceding logical line.
    let y = position.y + px(0.001);
    let estimate = layout
        .index_for_position(point(bounds.left(), y))
        .unwrap_or_else(|i| i);
    let estimate = text.floor_char_boundary(estimate.min(text.len()));
    let start = line_start(text, estimate);
    if y < bounds.top() || y >= bounds.bottom() || lh <= px(0.) {
        return Hit {
            caret: if y < bounds.top() { 0 } else { text.len() },
            glyph: None,
        };
    }
    let Some((line, origin)) = layout
        .line_layout_for_index(start)
        .zip(layout.position_for_index(start))
    else {
        return Hit {
            caret: estimate,
            glyph: None,
        };
    };
    let row = row_for_height(y - origin.y, lh).min(line.wrap_boundaries.len());
    let row_start = row.checked_sub(1).map_or(0, |i| row_boundary(&line, i));
    let row_end = row_boundary(&line, row);
    let x0 = line.unwrapped_layout.x_for_index(row_start);
    let x1 = line.unwrapped_layout.x_for_index(row_end);
    let x = position.x - aligned_left(bounds, x1 - x0, alignment) + x0;
    let glyph = if x >= x0 && x < x1 {
        line.unwrapped_layout.index_for_x(x).map(|i| start + i)
    } else {
        None
    };
    let caret = if x <= x0 {
        row_start
    } else if x >= x1 {
        row_end
    } else {
        line.unwrapped_layout
            .closest_index_for_x(x)
            .clamp(row_start.min(row_end), row_start.max(row_end))
    };
    Hit {
        caret: start + caret,
        glyph,
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn row_for_height(height: Pixels, lh: Pixels) -> usize {
    (height / lh).floor().max(0.) as usize
}

fn range_rects(
    layout: &TextLayout,
    text: &str,
    selection: Range<usize>,
    align: TextAlign,
    mut visit: impl FnMut(Bounds<Pixels>),
) {
    if selection.is_empty() || text.get(selection.clone()).is_none() {
        return;
    }
    let bounds = layout.bounds();
    let lh = layout.line_height();
    let mut start = line_start(text, selection.start);
    let Some(origin) = layout.position_for_index(start) else {
        return;
    };
    let mut y = origin.y;
    while start < selection.end {
        let Some(line) = layout.line_layout_for_index(start) else {
            break;
        };
        let mut row_start = 0;
        for row in 0..=line.wrap_boundaries.len() {
            let row_end = row_boundary(&line, row);
            // Visual boundaries can have descending byte indices across RTL runs.
            let lo = row_start.min(row_end);
            let hi = row_start.max(row_end);
            let a = selection.start.saturating_sub(start).clamp(lo, hi);
            let b = selection.end.saturating_sub(start).clamp(lo, hi);
            let newline = row == line.wrap_boundaries.len() && selection.end > start + row_end;
            if a < b || newline {
                let x = |index| line.unwrapped_layout.x_for_index(index);
                let left = aligned_left(bounds, x(row_end) - x(row_start), align);
                let width = x(b) - x(a) + if newline { px(5.) } else { px(0.) };
                visit(Bounds::new(
                    point(left + x(a) - x(row_start), y),
                    size(width, lh),
                ));
            }
            y += lh;
            row_start = row_end;
        }
        start += line.len() + 1;
    }
}

impl TextEvents {
    fn install(&self, layout: &TextLayout, align: TextAlign, hitbox: &Hitbox, window: &mut Window) {
        let down = self.clone();
        let shaped = layout.clone();
        let hovered = hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if event.button != MouseButton::Left {
                return;
            }
            if !hovered.is_hovered(window) {
                if phase == DispatchPhase::Capture
                    && down.selection.read(cx).focus.is_focused(window)
                {
                    down.selection.update(cx, |state, cx| {
                        state.range = 0..0;
                        state.drag = None;
                        cx.notify();
                    });
                }
                return;
            }
            if phase == DispatchPhase::Bubble {
                let hit = hit_test(&shaped, &down.text, align, event.position);
                let unit = match event.click_count {
                    0 | 1 => Unit::Caret,
                    2 => Unit::Word,
                    _ => Unit::Line,
                };
                let index = match unit {
                    Unit::Caret => hit.caret,
                    _ => hit.glyph.unwrap_or(hit.caret),
                };
                let anchor = unit.range(&down.text, index);
                let link = hit
                    .glyph
                    .and_then(|ix| down.links.iter().position(|(r, _)| r.contains(&ix)));
                if let Some(view) = window.root::<PrimeDesktopView>().flatten() {
                    view.update(cx, |view, _| {
                        view.active_selection.0 = Some(down.selection.downgrade());
                    });
                }
                down.selection.update(cx, |state, cx| {
                    window.focus(&state.focus);
                    state.text.clone_from(&down.text);
                    state.range = anchor.clone();
                    state.drag = Some(Drag {
                        origin: event.position,
                        anchor,
                        unit,
                        moved: false,
                        link,
                    });
                    cx.notify();
                });
                cx.stop_propagation();
            }
        });

        let moving = self.clone();
        let shaped = layout.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
            if phase == DispatchPhase::Capture
                && event.dragging()
                && moving.selection.read(cx).drag.is_some()
            {
                let hit = hit_test(&shaped, &moving.text, align, event.position);
                moving.selection.update(cx, |state, cx| {
                    state.extend(&moving.text, hit, event.position);
                    cx.notify();
                });
            }
        });

        let up = self.clone();
        let shaped = layout.clone();
        let hovered = hitbox.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase == DispatchPhase::Capture
                && event.button == MouseButton::Left
                && up.selection.read(cx).drag.is_some()
            {
                let hit = hit_test(&shaped, &up.text, align, event.position);
                up.selection.update(cx, |state, cx| {
                    let glyph = hit.glyph;
                    state.extend(&up.text, hit, event.position);
                    if let Some(drag) = state.drag.take()
                        && !drag.moved
                        && state.range.is_empty()
                        && hovered.is_hovered(window)
                        && let Some((range, url)) = drag.link.and_then(|link| up.links.get(link))
                        && glyph.is_some_and(|index| range.contains(&index))
                    {
                        cx.open_url(url);
                    }
                    cx.notify();
                });
                cx.stop_propagation();
            }
        });
    }
}

impl Selection {
    fn extend(&mut self, text: &str, hit: Hit, position: Point<Pixels>) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        let index = match drag.unit {
            Unit::Caret => hit.caret,
            Unit::Word | Unit::Line => hit.glyph.unwrap_or(hit.caret),
        };
        let extent = drag.unit.range(text, index);
        drag.moved |= extent != drag.anchor || (position - drag.origin).magnitude() > 4.;
        self.range = extent.start.min(drag.anchor.start)..extent.end.max(drag.anchor.end);
    }
}

impl IntoElement for SelectionGlyphs {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for SelectionGlyphs {
    type RequestLayoutState = ();
    type PrepaintState = (Hitbox, Vec<Hitbox>);

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.glyphs.request_layout(id, inspector, window, cx)
    }
    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.glyphs
            .prepaint(id, inspector, bounds, state, window, cx);
        let body = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let mut links = Vec::new();
        let align = window.text_style().text_align;
        for (range, _) in self.events.links.iter() {
            range_rects(
                self.glyphs.layout(),
                &self.events.text,
                range.clone(),
                align,
                |rect| {
                    links.push(window.insert_hitbox(rect, HitboxBehavior::Normal));
                },
            );
        }
        (body, links)
    }
    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        hitboxes: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let align = window.text_style().text_align;
        let selected = self.events.selection.read(cx);
        if selected.focus.is_focused(window) {
            range_rects(
                self.glyphs.layout(),
                &self.events.text,
                selected.range.clone(),
                align,
                |rect| {
                    window.paint_quad(fill(rect, self.selection_bg));
                },
            );
        }
        window.set_cursor_style(CursorStyle::IBeam, &hitboxes.0);
        for link in &hitboxes.1 {
            window.set_cursor_style(CursorStyle::PointingHand, link);
        }
        self.events
            .install(self.glyphs.layout(), align, &hitboxes.0, window);
        self.glyphs
            .paint(id, inspector, bounds, state, &mut (), window, cx);
    }
}

#[derive(IntoElement)]
struct CopyButton {
    id: SharedString,
    text: SharedString,
    palette: Palette,
}

pub(super) fn copy_button(
    id: impl Into<SharedString>,
    text: impl Into<SharedString>,
    palette: Palette,
) -> impl IntoElement {
    CopyButton {
        id: id.into(),
        text: text.into(),
        palette,
    }
}

impl RenderOnce for CopyButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let feedback = window.use_keyed_state(self.id.clone(), cx, |_, _| None::<Task<()>>);
        let copied = feedback.read(cx).is_some();
        let palette = self.palette;
        let (icon, label, ink) = if copied {
            (Icon::Check, "Copied", palette.success)
        } else {
            (Icon::Copy, "Copy", palette.text_secondary)
        };
        div()
            .id(self.id.clone())
            .debug_selector(|| {
                if copied {
                    format!("{}-copied", self.id)
                } else {
                    self.id.to_string()
                }
            })
            .flex()
            .flex_none()
            .whitespace_nowrap()
            .items_center()
            .gap(px(4.))
            .px(px(6.))
            .py(px(2.))
            .rounded(px(4.))
            .border_1()
            .border_color(palette.border_subtle)
            .min_w(px(58.))
            .justify_center()
            .font_family(font_sans())
            .text_size(px(11.))
            .text_color(ink)
            .cursor_pointer()
            .hover(move |s| s.bg(palette.bg_hover))
            .child(icon.render(12., ink))
            .child(label)
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                cx.write_to_clipboard(ClipboardItem::new_string(self.text.to_string()));
                feedback.update(cx, |task, cx| {
                    let delay = cx.background_executor().timer(Duration::from_millis(1500));
                    *task = Some(cx.spawn(async move |state, cx| {
                        delay.await;
                        let _ = state.update(cx, |task, cx| {
                            *task = None;
                            cx.notify();
                        });
                    }));
                    cx.notify();
                });
            })
    }
}

#[cfg(test)]
mod tests;
