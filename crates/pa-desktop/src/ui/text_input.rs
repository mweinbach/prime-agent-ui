//! Native GPUI text input supporting both single-line fields and multi-line wrapped editors.

mod layout;

use layout::TextCanvas;
use std::{collections::VecDeque, ops::Range};

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Entity, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, Hsla, IntoElement, KeyBinding, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollWheelEvent, ShapedLine, SharedString,
    UTF16Selection, Window, actions, div, point, prelude::*, px,
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
        SelectUp,
        SelectDown,
        SelectWordLeft,
        SelectWordRight,
        SelectHome,
        SelectEnd,
        MoveToStart,
        MoveToEnd,
        DeleteWordForward,
        DeleteToLineStart,
        SubmitEditor,
        Undo,
        Redo,
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
    let macos = cfg!(target_os = "macos");
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new(
            if macos {
                "alt-backspace"
            } else {
                "ctrl-backspace"
            },
            DeleteWordBack,
            ctx,
        ),
        KeyBinding::new("left", Left, ctx),
        KeyBinding::new("right", Right, ctx),
        KeyBinding::new("up", Up, ctx),
        KeyBinding::new("down", Down, ctx),
        KeyBinding::new(if macos { "alt-left" } else { "ctrl-left" }, WordLeft, ctx),
        KeyBinding::new(
            if macos { "alt-right" } else { "ctrl-right" },
            WordRight,
            ctx,
        ),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new("secondary-a", SelectAll, ctx),
        KeyBinding::new("home", Home, ctx),
        KeyBinding::new("end", End, ctx),
        KeyBinding::new("enter", EnterKey, ctx),
        KeyBinding::new("shift-enter", ShiftEnterKey, ctx),
        KeyBinding::new("secondary-enter", SubmitEditor, ctx),
        KeyBinding::new("shift-up", SelectUp, ctx),
        KeyBinding::new("shift-down", SelectDown, ctx),
        KeyBinding::new(
            if macos {
                "alt-shift-left"
            } else {
                "ctrl-shift-left"
            },
            SelectWordLeft,
            ctx,
        ),
        KeyBinding::new(
            if macos {
                "alt-shift-right"
            } else {
                "ctrl-shift-right"
            },
            SelectWordRight,
            ctx,
        ),
        KeyBinding::new("shift-home", SelectHome, ctx),
        KeyBinding::new("shift-end", SelectEnd, ctx),
        KeyBinding::new(if macos { "cmd-up" } else { "ctrl-home" }, MoveToStart, ctx),
        KeyBinding::new(if macos { "cmd-down" } else { "ctrl-end" }, MoveToEnd, ctx),
        KeyBinding::new(
            if macos { "alt-delete" } else { "ctrl-delete" },
            DeleteWordForward,
            ctx,
        ),
        KeyBinding::new("secondary-z", Undo, ctx),
        KeyBinding::new("secondary-shift-z", Redo, ctx),
        KeyBinding::new("ctrl-y", Redo, ctx),
        KeyBinding::new("secondary-v", Paste, ctx),
        KeyBinding::new("secondary-c", Copy, ctx),
        KeyBinding::new("secondary-x", Cut, ctx),
    ]);
    if macos {
        cx.bind_keys([
            KeyBinding::new("cmd-backspace", DeleteToLineStart, ctx),
            KeyBinding::new("cmd-shift-left", SelectHome, ctx),
            KeyBinding::new("cmd-shift-right", SelectEnd, ctx),
            KeyBinding::new("cmd-left", Home, ctx),
            KeyBinding::new("cmd-right", End, ctx),
            KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, ctx),
        ]);
    }
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

#[derive(Clone)]
struct EditorSnapshot {
    content: String,
    selected_range: Range<usize>,
    selection_reversed: bool,
}

#[derive(Default)]
struct UndoHistory {
    past: VecDeque<EditorSnapshot>,
    future: Vec<EditorSnapshot>,
}

#[derive(Clone, Copy)]
enum Direction {
    Above,
    Below,
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
    scroll_row_offset: usize,
    scroll_x: Pixels,
    wheel_remainder: Pixels,
    reveal_caret: bool,
    preferred_x: Option<Pixels>,
    caret_row_hint: Option<usize>,
    edits: UndoHistory,
    last_bounds: Option<Bounds<Pixels>>,
    last_line_height: Pixels,
    drag_anchor: Option<(Range<usize>, usize)>,
    history: Vec<String>,
    history_cursor: Option<usize>,
    draft_before_history: String,
    text_color: Hsla,
    placeholder_color: Hsla,
    accent_color: Hsla,
    selection_color: Hsla,
    selection_inactive_color: Hsla,
    font_family: SharedString,
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
            scroll_row_offset: 0,
            scroll_x: px(0.0),
            wheel_remainder: px(0.0),
            reveal_caret: true,
            preferred_x: None,
            caret_row_hint: None,
            edits: UndoHistory::default(),
            last_bounds: None,
            last_line_height: px(21.0),
            drag_anchor: None,
            history: Vec::new(),
            history_cursor: None,
            draft_before_history: String::new(),
            text_color: gpui::white(),
            placeholder_color: gpui::opaque_grey(0.5, 0.5),
            accent_color: gpui::rgb(0x00D9_7757).into(),
            selection_color: gpui::rgba(0xD977_5742).into(),
            selection_inactive_color: gpui::rgba(0xD977_5724).into(),
            font_family: crate::theme::font_sans(),
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
        selection_inactive_color: Hsla,
    ) {
        self.text_color = text_color;
        self.placeholder_color = placeholder_color;
        self.accent_color = accent_color;
        self.selection_color = selection_color;
        self.selection_inactive_color = selection_inactive_color;
    }

    pub fn set_font_family(&mut self, family: SharedString) {
        self.font_family = family;
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
        self.edits = UndoHistory::default();
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

    /// Painted canvas bounds and visual row widths from the last frame.
    #[cfg(test)]
    pub(crate) fn painted_rows(&self) -> (Option<Bounds<Pixels>>, Vec<Pixels>) {
        (
            self.last_bounds,
            self.visual_lines.iter().map(|l| l.shaped.width).collect(),
        )
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
        self.prepare_caret_move();
        let clamped = self.content.floor_char_boundary(offset);
        self.selected_range = clamped..clamped;
        self.selection_reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.prepare_caret_move();
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

    fn prepare_caret_move(&mut self) {
        self.reveal_caret = true;
        self.caret_row_hint = None;
        self.preferred_x = None;
        self.wheel_remainder = px(0.0);
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
        self.drag_anchor = None;
        self.prepare_caret_move();
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

    fn up(&mut self, _: &Up, window: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(Direction::Above, false, window, cx);
    }

    fn down(&mut self, _: &Down, window: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(Direction::Below, false, window, cx);
    }

    fn move_vertically(
        &mut self,
        direction: Direction,
        extend: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        // Several key events can arrive between paints, including just after an edit.
        if self.visual_lines.is_empty() {
            self.visual_lines = self.layout_rows(self.last_bounds.map(|b| b.size.width), window);
        }
        let cursor = self.cursor_offset();
        let row = self.row_for_offset(cursor);
        let target = match direction {
            Direction::Above => row.checked_sub(1),
            Direction::Below => (row + 1 < self.visual_lines.len()).then_some(row + 1),
        };
        if let Some(target) = target {
            let x = self
                .preferred_x
                .unwrap_or_else(|| self.x_for_offset(&self.visual_lines[row], cursor));
            let offset = self.offset_for_x(&self.visual_lines[target], x);
            if extend {
                self.select_to(offset, cx);
            } else {
                self.move_to(offset, cx);
            }
            self.caret_row_hint = Some(target);
            self.preferred_x = Some(x);
            return;
        }
        if !extend
            && self.kind == InputKind::Composer
            && (matches!(direction, Direction::Below) || self.scroll_row_offset == 0)
            && self.recall_history(direction, cx)
        {
            return;
        }
        let offset = match direction {
            Direction::Above => 0,
            Direction::Below => self.content.len(),
        };
        if extend {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx);
        }
    }

    fn recall_history(&mut self, direction: Direction, cx: &mut Context<Self>) -> bool {
        match (direction, self.history_cursor) {
            (Direction::Above, cursor) if !self.history.is_empty() => {
                if cursor.is_none() {
                    self.draft_before_history.clone_from(&self.content);
                }
                let index = cursor.map_or(self.history.len() - 1, |i| i.saturating_sub(1));
                self.history_cursor = Some(index);
                self.content.clone_from(&self.history[index]);
            }
            (Direction::Below, Some(i)) if i + 1 < self.history.len() => {
                self.history_cursor = Some(i + 1);
                self.content.clone_from(&self.history[i + 1]);
            }
            (Direction::Below, Some(_)) => {
                self.history_cursor = None;
                self.content.clone_from(&self.draft_before_history);
            }
            _ => return false,
        }
        self.selected_range = self.content.len()..self.content.len();
        self.selection_reversed = false;
        self.marked_range = None;
        self.edits = UndoHistory::default();
        self.notify_text_changed(cx);
        true
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.prepare_caret_move();
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

    fn delete_to(&mut self, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        let range = if self.selected_range.is_empty() {
            let cursor = self.cursor_offset();
            Some(self.range_to_utf16(&(cursor.min(offset)..cursor.max(offset))))
        } else {
            None
        };
        self.replace_text_in_range(range, "", window, cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_to(self.previous_boundary(self.cursor_offset()), window, cx);
    }

    fn delete_word_back(
        &mut self,
        _: &DeleteWordBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_to(
            self.previous_word_boundary(self.cursor_offset()),
            window,
            cx,
        );
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_to(self.next_boundary(self.cursor_offset()), window, cx);
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
            self.replace_text_in_range(None, &text, window, cx);
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
        let idx = self.index_for_mouse_position(event.position);
        if event.modifiers.shift {
            let anchor = if self.selection_reversed {
                self.selected_range.end
            } else {
                self.selected_range.start
            };
            self.drag_anchor = Some((anchor..anchor, 1));
            self.select_to(idx, cx);
        } else {
            self.prepare_caret_move();
            let range = self.click_range(idx, event.click_count);
            self.drag_anchor = Some((range.clone(), event.click_count));
            self.selected_range = range;
            self.selection_reversed = false;
            self.caret_row_hint = self.row_at_y(event.position.y);
            cx.notify();
        }
    }

    fn click_range(&self, idx: usize, click_count: usize) -> Range<usize> {
        match click_count {
            0 | 1 => idx..idx,
            2 => self
                .content
                .split_word_bound_indices()
                .find(|(start, part)| {
                    idx < start + part.len()
                        || idx == self.content.len() && idx == start + part.len()
                })
                .map_or(idx..idx, |(start, part)| start..start + part.len()),
            _ => self.current_line_start(idx)..self.current_line_end(idx),
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.drag_anchor = None;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((anchor, clicks)) = &self.drag_anchor {
            let range = self.click_range(self.index_for_mouse_position(event.position), *clicks);
            self.selection_reversed = range.start < anchor.start;
            self.selected_range = range.start.min(anchor.start)..range.end.max(anchor.end);
            self.prepare_caret_move();
            self.caret_row_hint = self.row_at_y(event.position.y);
            cx.notify();
        }
    }

    fn row_at_y(&self, y: Pixels) -> Option<usize> {
        let bounds = self.last_bounds?;
        let last = self.visual_lines.len().checked_sub(1)?;
        let rel_y = (y - bounds.top()).max(px(0.0));
        let visible_row =
            ((rel_y / self.last_line_height.max(px(1.0))).floor() as usize).min(self.max_lines - 1);
        Some((self.scroll_row_offset + visible_row).min(last))
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() || self.visual_lines.is_empty() {
            return 0;
        }
        let Some(bounds) = self.last_bounds else {
            return 0;
        };
        let first = self.scroll_row_offset.min(self.visual_lines.len() - 1);
        if position.y <= bounds.top() {
            return self.visual_lines[first].byte_range.start;
        }
        let line = &self.visual_lines[self.row_at_y(position.y).unwrap_or(first)];
        self.offset_for_x(line, position.x - bounds.left() + self.scroll_x)
    }

    fn offset_for_x(&self, line: &VisualLine, x: Pixels) -> usize {
        let idx = line.shaped.closest_index_for_x(x.max(px(0.0)));
        if self.kind == InputKind::Secret {
            self.content
                .char_indices()
                .nth(idx / '•'.len_utf8())
                .map_or(self.content.len(), |(byte, _)| byte)
        } else {
            let text = &self.content[line.byte_range.clone()];
            let idx = idx.min(text.len());
            let mut grapheme = unicode_segmentation::GraphemeCursor::new(idx, text.len(), true);
            let aligned = if grapheme.is_boundary(text, 0) == Ok(false) {
                let before = grapheme.prev_boundary(text, 0).ok().flatten().unwrap_or(0);
                let after = grapheme
                    .next_boundary(text, 0)
                    .ok()
                    .flatten()
                    .unwrap_or(text.len());
                if x - line.shaped.x_for_index(before) < line.shaped.x_for_index(after) - x {
                    before
                } else {
                    after
                }
            } else {
                idx
            };
            line.byte_range.start + aligned
        }
    }

    fn x_for_offset(&self, line: &VisualLine, offset: usize) -> Pixels {
        let offset = offset.clamp(line.byte_range.start, line.byte_range.end);
        let index = if self.kind == InputKind::Secret {
            self.content[..offset].chars().count() * '•'.len_utf8()
        } else {
            offset - line.byte_range.start
        };
        line.shaped.x_for_index(index)
    }

    fn row_for_offset(&self, offset: usize) -> usize {
        self.caret_row_hint
            .filter(|&row| {
                self.visual_lines.get(row).is_some_and(|line| {
                    (line.byte_range.start..=line.byte_range.end).contains(&offset)
                })
            })
            .unwrap_or_else(|| {
                self.visual_lines
                    .partition_point(|line| line.byte_range.start <= offset)
                    .saturating_sub(1)
            })
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let limit = self.visual_lines.len().saturating_sub(self.max_lines);
        let lh = self.last_line_height.max(px(1.0));
        let dy = event.delta.pixel_delta(lh).y;
        if (dy < px(0.0) && self.scroll_row_offset < limit)
            || (dy > px(0.0) && self.scroll_row_offset > 0)
        {
            self.wheel_remainder -= dy;
            let rows = (self.wheel_remainder / lh).trunc() as isize;
            self.wheel_remainder -= lh * rows as f32;
            self.scroll_row_offset = self
                .scroll_row_offset
                .saturating_add_signed(rows)
                .min(limit);
            self.reveal_caret = false;
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn snapshot(&self) -> EditorSnapshot {
        EditorSnapshot {
            content: self.content.clone(),
            selected_range: self.selected_range.clone(),
            selection_reversed: self.selection_reversed,
        }
    }

    fn record_edit(&mut self) {
        // Treat a marked IME composition (including its final commit) as one undo step.
        if self.marked_range.is_none() {
            self.edits.future.clear();
            self.edits.past.push_back(self.snapshot());
            while self.edits.past.len() > 64
                || (self.edits.past.len() > 1
                    && self
                        .edits
                        .past
                        .iter()
                        .map(|s| s.content.len())
                        .sum::<usize>()
                        > 4 * 1024 * 1024)
            {
                self.edits.past.pop_front();
            }
        }
    }

    fn restore_edit(&mut self, previous: EditorSnapshot, cx: &mut Context<Self>) {
        self.content = previous.content;
        self.selected_range = previous.selected_range;
        self.selection_reversed = previous.selection_reversed;
        self.marked_range = None;
        self.history_cursor = None;
        self.notify_text_changed(cx);
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(previous) = self.edits.past.pop_back() {
            self.edits.future.push(self.snapshot());
            self.restore_edit(previous, cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(next) = self.edits.future.pop() {
            self.edits.past.push_back(self.snapshot());
            self.restore_edit(next, cx);
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

        let normalized = new_text.replace("\r\n", "\n").replace('\r', "\n");
        let sanitized = if self.allows_newlines() {
            normalized
        } else {
            normalized.replace('\n', " ")
        };

        if !range.is_empty() || !sanitized.is_empty() {
            self.record_edit();
        }
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

        self.record_edit();
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
        let row = self.row_for_offset(range.start);
        if row < self.scroll_row_offset || row >= self.scroll_row_offset + self.max_lines {
            return None;
        }
        let line = self.visual_lines.get(row)?;
        let left = bounds.left() - self.scroll_x;
        let x0 = (left + self.x_for_offset(line, range.start)).clamp(bounds.left(), bounds.right());
        let x1 = (left + self.x_for_offset(line, range.end)).clamp(bounds.left(), bounds.right());
        let top = bounds.top() + self.last_line_height * (row - self.scroll_row_offset) as f32;
        Some(Bounds::from_corners(
            point(x0, top),
            point(x1, top + self.last_line_height),
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

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w_full()
            .min_w_0()
            // Stretch the canvas to the field. As a block child, its percentage width
            // could keep a max-content result and paint unwrapped past the clip.
            .flex()
            .flex_col()
            .overflow_hidden()
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
            .on_action(cx.listener(|this, _: &SelectUp, window, cx| {
                this.move_vertically(Direction::Above, true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectDown, window, cx| {
                this.move_vertically(Direction::Below, true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectWordLeft, _, cx| {
                this.select_to(this.previous_word_boundary(this.cursor_offset()), cx);
            }))
            .on_action(cx.listener(|this, _: &SelectWordRight, _, cx| {
                this.select_to(this.next_word_boundary(this.cursor_offset()), cx);
            }))
            .on_action(cx.listener(|this, _: &SelectHome, _, cx| {
                this.select_to(this.current_line_start(this.cursor_offset()), cx);
            }))
            .on_action(cx.listener(|this, _: &SelectEnd, _, cx| {
                this.select_to(this.current_line_end(this.cursor_offset()), cx);
            }))
            .on_action(cx.listener(|this, _: &MoveToStart, _, cx| this.move_to(0, cx)))
            .on_action(
                cx.listener(|this, _: &MoveToEnd, _, cx| this.move_to(this.content.len(), cx)),
            )
            .on_action(cx.listener(|this, _: &DeleteWordForward, window, cx| {
                this.delete_to(this.next_word_boundary(this.cursor_offset()), window, cx);
            }))
            .on_action(cx.listener(|this, _: &DeleteToLineStart, window, cx| {
                this.delete_to(this.current_line_start(this.cursor_offset()), window, cx);
            }))
            .on_action(cx.listener(|this, _: &SubmitEditor, _, cx| {
                if this.marked_range.is_none() {
                    cx.emit(TextSubmitted);
                }
            }))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
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
            .when(self.allows_newlines(), |field| {
                field.on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            })
            .font_family(self.font_family.clone())
            .child(TextCanvas { input: cx.entity() })
    }
}

#[cfg(test)]
mod tests;
