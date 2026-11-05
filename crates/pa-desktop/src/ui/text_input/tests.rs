use super::*;

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

struct TestField(Entity<TextInput>, Pixels);

impl Render for TestField {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(self.1).child(self.0.clone())
    }
}

fn mount(
    app: &mut gpui::TestAppContext,
    kind: InputKind,
    width: Pixels,
) -> (Entity<TextInput>, &mut gpui::VisualTestContext) {
    app.update(bind_text_input_keys);
    let (view, cx) = app.add_window_view(|window, cx| {
        let input = TextInput::new(cx, kind, "");
        window.focus(&input.read(cx).focus_handle);
        TestField(input, width)
    });
    let input = cx.read(|cx| view.read(cx).0.clone());
    (input, cx)
}

fn draw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear());
}

#[test]
fn measured_wrap_rows_drive_height_vertical_selection_and_history() {
    let mut app = gpui::TestAppContext::single();
    let (input, cx) = mount(&mut app, InputKind::Composer, px(38.0));
    input.update(cx, |input, cx| {
        input.record_history("old");
        input.set_text("aaaaaa\n🍊🍊\ncccc", cx);
    });
    draw(cx);
    input.update_in(cx, |input, window, cx| {
        let rows = &input.visual_lines;
        // The test font has double-width emoji. Four letters or two emoji fit.
        assert_eq!(
            rows.iter()
                .map(|line| line.shaped.text.as_str())
                .collect::<Vec<_>>(),
            ["aaaa", "aa", "🍊🍊", "cccc"]
        );
        let bounds = input.last_bounds.unwrap();
        assert_eq!(bounds.size.height, input.last_line_height * 4);
        let middle = input.bounds_for_range(4..6, bounds, window, cx).unwrap();
        assert_eq!(middle.top(), bounds.top() + input.last_line_height);

        input.move_to(18, cx); // Two letters into the last row.
        input.up(&Up, window, cx);
        assert_eq!(input.cursor_offset(), 11); // One double-width emoji.
        input.move_vertically(Direction::Above, true, window, cx);
        assert_eq!(input.selected_range, 6..11);
        assert_eq!(
            input.preferred_x,
            Some(input.visual_lines[0].shaped.x_for_index(2))
        );
        input.up(&Up, window, cx);
        assert_eq!(input.cursor_offset(), 2);
        input.up(&Up, window, cx);
        assert_eq!(input.text(), "old");
        input.down(&Down, window, cx);
        assert_eq!(input.text(), "aaaaaa\n🍊🍊\ncccc");
    });
    app.quit();
}

#[test]
fn masked_caret_hit_testing_and_ime_bounds_share_the_scrolled_origin() {
    let mut app = gpui::TestAppContext::single();
    let (input, cx) = mount(&mut app, InputKind::Secret, px(41.0));
    input.update(cx, |input, cx| input.set_text("漢🍊0123456界", cx));
    draw(cx);
    input.update_in(cx, |input, window, cx| {
        let bounds = input.last_bounds.unwrap();
        assert!(input.scroll_x > px(0.0));
        let len = input.content.encode_utf16().count();
        let last = input
            .bounds_for_range(len - 1..len, bounds, window, cx)
            .unwrap();
        assert!(last.right() > last.left() && last.right() <= bounds.right());
        assert_eq!(
            input.index_for_mouse_position(point(last.left(), last.center().y)),
            input.content.len() - '界'.len_utf8()
        );
        assert_eq!(
            input.index_for_mouse_position(point(last.right(), last.center().y)),
            input.content.len()
        );
        input.move_to(0, cx);
    });
    draw(cx);
    assert_eq!(cx.read(|cx| input.read(cx).scroll_x), px(0.0));
    app.quit();
}

#[test]
fn wheel_and_multiclick_use_the_visible_logical_line() {
    let mut app = gpui::TestAppContext::single();
    let (input, cx) = mount(&mut app, InputKind::MultilineEditor, px(300.0));
    input.update(cx, |input, cx| {
        input.set_text("one two\n".repeat(18), cx);
        input.move_to(0, cx);
    });
    draw(cx);
    input.update_in(cx, |input, window, cx| {
        input.on_scroll_wheel(
            &ScrollWheelEvent {
                delta: gpui::ScrollDelta::Lines(point(0., -3.)),
                ..Default::default()
            },
            window,
            cx,
        );
    });
    draw(cx);
    input.update_in(cx, |input, window, cx| {
        let bounds = input.last_bounds.unwrap();
        let start = 3 * "one two\n".len();
        assert_eq!(input.index_for_mouse_position(bounds.origin), start);
        let position = point(bounds.left() + px(42.0), bounds.top() + px(5.0));
        let mut click = MouseDownEvent {
            button: MouseButton::Left,
            position,
            click_count: 2,
            modifiers: gpui::Modifiers::default(),
            first_mouse: false,
        };
        input.on_mouse_down(&click, window, cx);
        assert_eq!(input.selected_range, start + 4..start + 7);
        click.click_count = 3;
        input.on_mouse_down(&click, window, cx);
        assert_eq!(input.selected_range, start..start + 7);
    });
    app.quit();
}

#[test]
fn pasted_line_endings_and_undo_redo_restore_unicode_selections() {
    let mut app = gpui::TestAppContext::single();
    let (input, cx) = mount(&mut app, InputKind::MultilineEditor, px(180.0));
    input.update_in(cx, |input, window, cx| {
        input.set_text("漢🍊", cx);
        input.select_to(3, cx);
        cx.write_to_clipboard(ClipboardItem::new_string("a\r\nb\rc".into()));
        input.paste(&Paste, window, cx);
        assert_eq!(input.text(), "漢a\nb\nc");
        input.undo(&Undo, window, cx);
        assert_eq!(input.text(), "漢🍊");
        assert_eq!(input.selected_range, 3..7);
        assert!(input.selection_reversed);
        input.redo(&Redo, window, cx);
        assert_eq!(input.text(), "漢a\nb\nc");
        input.undo(&Undo, window, cx);
        input.replace_text_in_range(None, "x", window, cx);
        input.redo(&Redo, window, cx);
        assert_eq!(input.text(), "漢x");
    });
    app.quit();
}

#[test]
fn editor_shortcuts_keep_enter_and_submit_distinct() {
    let mut app = gpui::TestAppContext::single();
    let (input, cx) = mount(&mut app, InputKind::MultilineEditor, px(160.0));
    let submissions = std::rc::Rc::new(std::cell::Cell::new(0));
    let count = submissions.clone();
    cx.update(|_, cx| {
        cx.subscribe(&input, move |_, _: &TextSubmitted, _| {
            count.set(count.get() + 1);
        })
        .detach();
    });
    cx.simulate_keystrokes("enter secondary-enter");
    assert_eq!(submissions.get(), 1);
    assert_eq!(cx.read(|cx| input.read(cx).text().to_string()), "\n");
    cx.simulate_keystrokes("secondary-z");
    assert!(cx.read(|cx| input.read(cx).text().is_empty()));
    cx.simulate_keystrokes("secondary-shift-z");
    assert_eq!(cx.read(|cx| input.read(cx).text().to_string()), "\n");
    app.quit();
}

#[test]
fn narrow_wraps_keep_combining_marks_and_joined_emoji_intact() {
    let mut app = gpui::TestAppContext::single();
    let (input, cx) = mount(&mut app, InputKind::MultilineEditor, px(9.0));
    let text = "a\u{3000}e\u{301}👩🏽‍💻z";
    input.update(cx, |input, cx| input.set_text(text, cx));
    draw(cx);
    cx.read(|cx| {
        let input = input.read(cx);
        assert_eq!(
            input
                .visual_lines
                .iter()
                .map(|row| row.shaped.text.as_str())
                .collect::<Vec<_>>(),
            text.graphemes(true).collect::<Vec<_>>()
        );
        for row in &input.visual_lines {
            let hit = input.offset_for_x(row, row.shaped.width / 2.0);
            assert!(hit == row.byte_range.start || hit == row.byte_range.end);
        }
    });
    app.quit();
}
