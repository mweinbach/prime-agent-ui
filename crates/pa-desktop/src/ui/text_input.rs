//! Native GPUI text input supporting both single-line fields and multi-line wrapped editors.

use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, KeyBinding, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, ShapedLine, SharedString, Style,
    TextRun, UTF16Selection, UnderlineStyle, Window, actions, div, fill, point, prelude::*, px,
    relative, size,
};
use unicode_segmentation::UnicodeSegmentation;

actions!(
    pa_text_input,
    [
        Backspace,
        Delete,
        DeleteWordBack,
        Left,
        Right,
        Up,
        Down,
        WordLeft,
        WordRight,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        EnterKey,
        ShiftEnterKey,
        Paste,
        Cut,
        Copy,
        ShowCharacterPalette,
    ]
);

/// Register global key bindings for [`TextInput`] contexts once at app startup.
pub fn bind_text_input_keys(cx: &mut App) {
    let ctx = Some("PaTextInput");
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("alt-backspace", DeleteWordBack, ctx),
        KeyBinding::new("left", Left, ctx),
        KeyBinding::new("right", Right, ctx),
        KeyBinding::new("up", Up, ctx),
        KeyBinding::new("down", Down, ctx),
        KeyBinding::new("alt-left", WordLeft, ctx),
        KeyBinding::new("alt-right", WordRight, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new("cmd-a", SelectAll, ctx),
        KeyBinding::new("cmd-left", Home, ctx),
        KeyBinding::new("cmd-right", End, ctx),
        KeyBinding::new("home", Home, ctx),
        KeyBinding::new("end", End, ctx),
        KeyBinding::new("enter", EnterKey, ctx),
        KeyBinding::new("shift-enter", ShiftEnterKey, ctx),
        KeyBinding::new("cmd-enter", EnterKey, ctx),
        KeyBinding::new("cmd-v", Paste, ctx),
        KeyBinding::new("cmd-c", Copy, ctx),
        KeyBinding::new("cmd-x", Cut, ctx),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, ctx),
    ]);
}

/// Visual/behavioral mode of a [`TextInput`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    /// Single-line field; `Enter` emits a submit event.
    SingleLine,
    /// Multi-line prompt composer; `Enter` submits, `Shift+Enter` inserts newline.
    Composer,
    /// Multi-line document editor; `Enter` inserts newline.
    MultilineEditor,
    /// Masked secret field (`••••`).
    Secret,
}

pub(crate) struct TextSubmitted;
pub(crate) struct TextChanged;

impl EventEmitter<TextSubmitted> for TextInput {}
impl EventEmitter<TextChanged> for TextInput {}

#[derive(Clone)]
struct VisualLine {
    /// Byte range in `TextInput::content` covered by this visual line (excluding trailing `\n`).
    byte_range: Range<usize>,
    shaped: ShapedLine,
}

/// Interactive GPUI text input entity.
pub struct TextInput {
    focus_handle: FocusHandle,
    kind: InputKind,
    content: String,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    visual_lines: Vec<VisualLine>,
    last_bounds: Option<Bounds<Pixels>>,
    last_line_height: Pixels,
    is_selecting: bool,
    history: Vec<String>,
    history_cursor: Option<usize>,
    draft_before_history: String,
    text_color: Hsla,
    placeholder_color: Hsla,
    accent_color: Hsla,
    selection_color: Hsla,
    font_size_px: f32,
    min_lines: usize,
    max_lines: usize,
}

impl TextInput {
    #[must_use]
    pub fn new(
        cx: &mut App,
        kind: InputKind,
        placeholder: impl Into<SharedString>,
    ) -> Entity<Self> {
        cx.new(|cx| Self {
            focus_handle: cx.focus_handle(),
            kind,
            content: String::new(),
            placeholder: placeholder.into(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            visual_lines: Vec::new(),
            last_bounds: None,
            last_line_height: px(21.0),
            is_selecting: false,
            history: Vec::new(),
            history_cursor: None,
            draft_before_history: String::new(),
            text_color: gpui::white(),
            placeholder_color: gpui::opaque_grey(0.5, 0.5),
            accent_color: gpui::rgb(0x00D9_7757).into(),
            selection_color: gpui::rgba(0xD977_5742).into(),
            font_size_px: 14.0,
            min_lines: match kind {
                InputKind::Composer => 2,
                InputKind::MultilineEditor => 4,
                InputKind::SingleLine | InputKind::Secret => 1,
            },
            max_lines: match kind {
                InputKind::Composer => 10,
                InputKind::MultilineEditor => 12,
                InputKind::SingleLine | InputKind::Secret => 1,
            },
        })
    }

    pub fn set_colors(
        &mut self,
        text_color: Hsla,
        placeholder_color: Hsla,
        accent_color: Hsla,
        selection_color: Hsla,
    ) {
        self.text_color = text_color;
        self.placeholder_color = placeholder_color;
        self.accent_color = accent_color;
        self.selection_color = selection_color;
    }

    pub fn set_font_size(&mut self, px_size: f32) {
        self.font_size_px = px_size;
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>) {
        self.placeholder = placeholder.into();
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.content
    }

    pub fn set_text(&mut self, value: impl Into<String>, cx: &mut Context<Self>) {
        self.content = value.into();
        let end = self.content.len();
        self.selected_range = end..end;
        self.selection_reversed = false;
        self.marked_range = None;
        self.history_cursor = None;
        self.notify_text_changed(cx);
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.set_text(String::new(), cx);
    }

    pub fn record_history(&mut self, entry: &str) {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            return;
        }
        if self.history.last().is_none_or(|last| last != trimmed) {
            self.history.push(trimmed.to_string());
            if self.history.len() > 80 {
                self.history.remove(0);
            }
        }
        self.history_cursor = None;
    }

    fn allows_newlines(&self) -> bool {
        matches!(self.kind, InputKind::Composer | InputKind::MultilineEditor)
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let clamped = self.content.floor_char_boundary(offset);
        self.selected_range = clamped..clamped;
        self.selection_reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let clamped = self.content.floor_char_boundary(offset);
        if self.selection_reversed {
            self.selected_range.start = clamped;
        } else {
            self.selected_range.end = clamped;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }

    fn previous_word_boundary(&self, offset: usize) -> usize {
        self.content
            .unicode_word_indices()
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_word_boundary(&self, offset: usize) -> usize {
        self.content
            .unicode_word_indices()
            .find_map(|(idx, word)| {
                let end = idx + word.len();
                (end > offset).then_some(end)
            })
            .unwrap_or(self.content.len())
    }

    fn current_line_start(&self, offset: usize) -> usize {
        self.content[..offset.min(self.content.len())]
            .rfind('\n')
            .map_or(0, |pos| pos + 1)
    }

    fn current_line_end(&self, offset: usize) -> usize {
        let safe = offset.min(self.content.len());
        self.content[safe..]
            .find('\n')
            .map_or(self.content.len(), |rel| safe + rel)
    }

    fn notify_text_changed(&mut self, cx: &mut Context<Self>) {
        self.visual_lines.clear();
        cx.emit(TextChanged);
        cx.notify();
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.previous_word_boundary(self.cursor_offset()), cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.next_word_boundary(self.cursor_offset()), cx);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let cur = self.cursor_offset();
        let line_start = self.current_line_start(cur);
        if line_start > 0 {
            let col = cur - line_start;
            let prev_end = line_start - 1;
            let prev_start = self.current_line_start(prev_end);
            self.move_to((prev_start + col).min(prev_end), cx);
        } else if self.kind == InputKind::Composer && !self.history.is_empty() {
            let next_idx = match self.history_cursor {
                None => {
                    self.draft_before_history = self.content.clone();
                    self.history.len() - 1
                }
                Some(idx) => idx.saturating_sub(1),
            };
            self.history_cursor = Some(next_idx);
            self.content = self.history[next_idx].clone();
            let end = self.content.len();
            self.selected_range = end..end;
            self.selection_reversed = false;
            self.marked_range = None;
            self.notify_text_changed(cx);
        } else {
            self.move_to(0, cx);
        }
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let cur = self.cursor_offset();
        let line_end = self.current_line_end(cur);
        if line_end < self.content.len() {
            let line_start = self.current_line_start(cur);
            let col = cur - line_start;
            let next_start = line_end + 1;
            let next_end = self.current_line_end(next_start);
            self.move_to((next_start + col).min(next_end), cx);
        } else if let Some(idx) = self.history_cursor {
            if idx + 1 < self.history.len() {
                let next_idx = idx + 1;
                self.history_cursor = Some(next_idx);
                self.content = self.history[next_idx].clone();
            } else {
                self.history_cursor = None;
                self.content = self.draft_before_history.clone();
            }
            let end = self.content.len();
            self.selected_range = end..end;
            self.selection_reversed = false;
            self.marked_range = None;
            self.notify_text_changed(cx);
        } else {
            self.move_to(self.content.len(), cx);
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        cx.notify();
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.current_line_start(self.cursor_offset()), cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.current_line_end(self.cursor_offset()), cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete_word_back(
        &mut self,
        _: &DeleteWordBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_word_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn enter_key(&mut self, _: &EnterKey, window: &mut Window, cx: &mut Context<Self>) {
        if self.marked_range.is_some() {
            return;
        }
        if self.kind == InputKind::MultilineEditor {
            self.replace_text_in_range(None, "\n", window, cx);
        } else {
            // GPUI delivers this after the input update. The subscriber can read or clear it.
            cx.emit(TextSubmitted);
        }
    }

    fn shift_enter_key(&mut self, _: &ShiftEnterKey, window: &mut Window, cx: &mut Context<Self>) {
        if self.marked_range.is_some() {
            return;
        }
        if self.allows_newlines() {
            self.replace_text_in_range(None, "\n", window, cx);
        } else {
            self.enter_key(&EnterKey, window, cx);
        }
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let cleaned = if self.allows_newlines() {
                text.replace("\r\n", "\n")
            } else {
                text.replace(['\r', '\n'], " ")
            };
            self.replace_text_in_range(None, &cleaned, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() && self.kind != InputKind::Secret {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() && self.kind != InputKind::Secret {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    fn show_character_palette(
        _: &mut Self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        self.is_selecting = true;
        let idx = self.index_for_mouse_position(event.position);
        if event.modifiers.shift {
            self.select_to(idx, cx);
        } else {
            self.move_to(idx, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() || self.visual_lines.is_empty() {
            return 0;
        }
        let Some(bounds) = self.last_bounds else {
            return 0;
        };
        if position.y <= bounds.top() {
            return 0;
        }
        let lh = self.last_line_height.max(px(1.0));
        let rel_y = (position.y - bounds.top()).max(px(0.0));
        let row = ((rel_y / lh).floor() as usize).min(self.visual_lines.len().saturating_sub(1));
        let vline = &self.visual_lines[row];
        let local_x = (position.x - bounds.left()).max(px(0.0));
        let rel_idx = vline.shaped.closest_index_for_x(local_x);
        if self.kind == InputKind::Secret {
            // Each bullet is 3 UTF-8 bytes (`•`), map back to char index in `self.content`.
            let char_idx = rel_idx / "•".len();
            self.content
                .char_indices()
                .nth(char_idx)
                .map_or(self.content.len(), |(b, _)| b)
        } else {
            (vline.byte_range.start + rel_idx).min(vline.byte_range.end)
        }
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;
        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }
        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }
}

fn range_from_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    let byte_offset = |offset| {
        let mut consumed = 0;
        for (i, ch) in text.char_indices() {
            if consumed >= offset {
                return i;
            }
            consumed += ch.len_utf16();
        }
        text.len()
    };
    byte_offset(range.start)..byte_offset(range.end.max(range.start))
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        if self.kind == InputKind::Secret {
            return None;
        }
        let range = range_from_utf16(&self.content, &range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| range_from_utf16(&self.content, r))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());

        let sanitized = if self.allows_newlines() {
            new_text.to_string()
        } else {
            new_text.replace(['\r', '\n'], " ")
        };

        self.content.replace_range(range.clone(), &sanitized);
        let new_cursor = range.start + sanitized.len();
        self.selected_range = new_cursor..new_cursor;
        self.marked_range = None;
        self.selection_reversed = false;
        self.history_cursor = None;
        self.notify_text_changed(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| range_from_utf16(&self.content, r))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());

        self.content.replace_range(range.clone(), new_text);
        if new_text.is_empty() {
            self.marked_range = None;
        } else {
            self.marked_range = Some(range.start..range.start + new_text.len());
        }
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|r| range_from_utf16(new_text, r))
            .map_or_else(
                || range.start + new_text.len()..range.start + new_text.len(),
                |nr| nr.start + range.start..nr.end + range.start,
            );
        self.selection_reversed = false;
        self.history_cursor = None;
        self.notify_text_changed(cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = range_from_utf16(&self.content, &range_utf16);
        let first = self.visual_lines.first()?;
        let x0 = first
            .shaped
            .x_for_index(range.start.saturating_sub(first.byte_range.start));
        let x1 = first
            .shaped
            .x_for_index(range.end.saturating_sub(first.byte_range.start));
        Some(Bounds::from_corners(
            point(bounds.left() + x0, bounds.top()),
            point(bounds.left() + x1, bounds.top() + self.last_line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        pt: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let utf8 = self.index_for_mouse_position(pt);
        Some(self.offset_to_utf16(utf8))
    }
}

struct TextCanvas {
    input: Entity<TextInput>,
}

struct CanvasPrepaint {
    lines: Vec<VisualLine>,
    selections: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
    line_height: Pixels,
}

impl IntoElement for TextCanvas {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextCanvas {
    type RequestLayoutState = Pixels;
    type PrepaintState = CanvasPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let state = self.input.read(cx);
        let line_height = px((state.font_size_px * 1.48).round());
        let raw_line_count = if state.content.is_empty() {
            1
        } else {
            state.content.split('\n').count()
        };
        // Account for soft-wrapped long lines using last known width when available.
        let est_wrap_extra: usize = state
            .last_bounds
            .map(|b| f32::from(b.size.width).max(160.0))
            .map_or(0, |width_px| {
                let chars_per_row = ((width_px / (state.font_size_px * 0.56)) as usize).max(20);
                state
                    .content
                    .split('\n')
                    .map(|l| l.chars().count().saturating_sub(1) / chars_per_row)
                    .sum()
            });
        let visible_rows = (raw_line_count + est_wrap_extra)
            .max(state.min_lines)
            .min(state.max_lines);

        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = (line_height * visible_rows as f32).into();
        (window.request_layout(style, [], cx), line_height)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        line_height: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let state = self.input.read(cx);
        let font = window.text_style().font();
        let font_size = px(state.font_size_px);
        let lh = *line_height;

        if state.content.is_empty() {
            let run = TextRun {
                len: state.placeholder.len(),
                font,
                color: state.placeholder_color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let shaped =
                window
                    .text_system()
                    .shape_line(state.placeholder.clone(), font_size, &[run], None);
            let cursor = Some(fill(
                Bounds::new(bounds.origin, size(px(2.0), lh)),
                state.accent_color,
            ));
            return CanvasPrepaint {
                lines: vec![VisualLine {
                    byte_range: 0..0,
                    shaped,
                }],
                selections: Vec::new(),
                cursor,
                line_height: lh,
            };
        }

        let width_px = f32::from(bounds.size.width).max(120.0);
        let max_cols = if state.allows_newlines() {
            ((width_px / (state.font_size_px * 0.56)) as usize).max(18)
        } else {
            usize::MAX
        };

        let mut visual_lines: Vec<VisualLine> = Vec::new();
        if state.kind == InputKind::Secret {
            let masked: String = "•".repeat(state.content.chars().count());
            let run = TextRun {
                len: masked.len(),
                font,
                color: state.text_color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let shaped = window.text_system().shape_line(
                SharedString::from(masked),
                font_size,
                &[run],
                None,
            );
            visual_lines.push(VisualLine {
                byte_range: 0..state.content.len(),
                shaped,
            });
        } else {
            let mut offset = 0usize;
            for logical in state.content.split('\n') {
                if logical.is_empty() {
                    let shaped = window.text_system().shape_line(
                        SharedString::new_static(""),
                        font_size,
                        &[],
                        None,
                    );
                    visual_lines.push(VisualLine {
                        byte_range: offset..offset,
                        shaped,
                    });
                } else {
                    let segments = soft_wrap_segments(logical, max_cols);
                    for seg_range in segments {
                        let abs_range = (offset + seg_range.start)..(offset + seg_range.end);
                        let slice_str = &state.content[abs_range.clone()];
                        let runs = build_runs_for_slice(
                            &abs_range,
                            slice_str.len(),
                            state.marked_range.as_ref(),
                            &font,
                            state.text_color,
                        );
                        let shaped = window.text_system().shape_line(
                            SharedString::from(slice_str.to_string()),
                            font_size,
                            &runs,
                            None,
                        );
                        visual_lines.push(VisualLine {
                            byte_range: abs_range,
                            shaped,
                        });
                    }
                }
                offset += logical.len() + 1;
            }
        }

        // Scroll visible slice to keep cursor visible if lines exceed max_lines.
        let cursor = state.cursor_offset();
        let active_vline_idx = visual_lines
            .iter()
            .rposition(|vl| cursor >= vl.byte_range.start && cursor <= vl.byte_range.end)
            .unwrap_or(0);
        let max_rows = state.max_lines.max(1);
        let window_start = if active_vline_idx >= max_rows {
            active_vline_idx + 1 - max_rows
        } else {
            0
        };
        let window_end = (window_start + max_rows).min(visual_lines.len());
        let visible_slice = visual_lines[window_start..window_end].to_vec();

        let sel = &state.selected_range;
        let mut selections = Vec::new();
        let mut cursor_quad = None;

        if sel.is_empty() {
            if let Some((row_i, vl)) = visible_slice
                .iter()
                .enumerate()
                .rev()
                .find(|(_, vl)| cursor >= vl.byte_range.start && cursor <= vl.byte_range.end)
            {
                let local_byte = if state.kind == InputKind::Secret {
                    state.content[..cursor].chars().count() * "•".len()
                } else {
                    cursor - vl.byte_range.start
                };
                let cx_px = vl.shaped.x_for_index(local_byte);
                let top_y = bounds.top() + lh * (row_i as f32);
                cursor_quad = Some(fill(
                    Bounds::new(point(bounds.left() + cx_px, top_y), size(px(2.0), lh)),
                    state.accent_color,
                ));
            }
        } else {
            for (row_i, vl) in visible_slice.iter().enumerate() {
                let s_start = sel.start.max(vl.byte_range.start);
                let s_end = sel.end.min(vl.byte_range.end);
                if s_start < s_end {
                    let (l0, l1) = if state.kind == InputKind::Secret {
                        (
                            state.content[..s_start].chars().count() * "•".len(),
                            state.content[..s_end].chars().count() * "•".len(),
                        )
                    } else {
                        (s_start - vl.byte_range.start, s_end - vl.byte_range.start)
                    };
                    let x0 = vl.shaped.x_for_index(l0);
                    let x1 = vl.shaped.x_for_index(l1).max(x0 + px(4.0));
                    let top_y = bounds.top() + lh * (row_i as f32);
                    selections.push(fill(
                        Bounds::from_corners(
                            point(bounds.left() + x0, top_y),
                            point(bounds.left() + x1, top_y + lh),
                        ),
                        state.selection_color,
                    ));
                }
            }
        }

        CanvasPrepaint {
            lines: visible_slice,
            selections,
            cursor: cursor_quad,
            line_height: lh,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );

        for quad in prepaint.selections.drain(..) {
            window.paint_quad(quad);
        }

        let lh = prepaint.line_height;
        for (i, vl) in prepaint.lines.iter().enumerate() {
            let origin = point(bounds.left(), bounds.top() + lh * (i as f32));
            let _ = vl.shaped.paint(origin, lh, window, cx);
        }

        if focus_handle.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }

        let painted_lines = prepaint.lines.clone();
        self.input.update(cx, |input, _cx| {
            input.visual_lines = painted_lines;
            input.last_bounds = Some(bounds);
            input.last_line_height = lh;
        });
    }
}

fn soft_wrap_segments(line: &str, max_cols: usize) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < line.len() {
        let rest = &line[start..];
        let Some((hard_end, _)) = rest.grapheme_indices(true).nth(max_cols.max(1)) else {
            out.push(start..line.len());
            break;
        };
        let split = rest[..hard_end]
            .grapheme_indices(true)
            .rev()
            .find(|(_, g)| g.chars().all(char::is_whitespace))
            .filter(|(pos, _)| *pos > hard_end / 3)
            .map_or(hard_end, |(pos, g)| pos + g.len());
        out.push(start..start + split);
        start += split;
    }
    out
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

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w_full()
            .when(self.kind == InputKind::Composer, |input| {
                input.debug_selector(|| "composer-input".to_owned())
            })
            .key_context("PaTextInput")
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_back))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::enter_key))
            .on_action(cx.listener(Self::shift_enter_key))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::show_character_palette))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .font_family(crate::theme::font_sans())
            .child(TextCanvas { input: cx.entity() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_at_the_end_of_unicode_whitespace() {
        let text = "abc\u{3000}de\u{301}fgh";
        let wrapped = soft_wrap_segments(text, 5)
            .into_iter()
            .map(|r| &text[r])
            .collect::<Vec<_>>();
        assert_eq!(wrapped, ["abc\u{3000}", "de\u{301}fgh"]);
    }

    #[test]
    fn ime_selection_is_relative_to_the_inserted_mark() {
        // A three-byte prefix must not affect the UTF-16 selection inside the mark.
        let prefix = "漢";
        let inserted = "🍊子";
        let selected = range_from_utf16(inserted, &(2..3));
        let result = format!("{prefix}{inserted}!");
        assert_eq!(
            &result[prefix.len() + selected.start..prefix.len() + selected.end],
            "子"
        );
    }
}
