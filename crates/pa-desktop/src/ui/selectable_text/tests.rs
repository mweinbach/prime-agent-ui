use gpui::{Context, Focusable as _, Modifiers, Render, TestAppContext, VisualTestContext};

use super::*;
use crate::{
    state::transcript::{AssistantBlock, TranscriptEntry},
    ui::{
        text_input::bind_text_input_keys,
        workspace::{LaunchOptions, register_workspace_bindings_and_menus},
    },
};

const SAMPLE: &str = "link café\nsecond line";
const URL: &str = "https://example.test/";
const MARKDOWN: &str = "[link](https://example.test/) café  \n[second](file:///x) line";

struct Transcript;

impl Render for Transcript {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn clipboard(cx: &VisualTestContext) -> String {
    cx.read_from_clipboard()
        .and_then(|item| item.text())
        .expect("copied text")
}

fn click_count(cx: &mut VisualTestContext, pos: Point<Pixels>, count: usize) {
    cx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: pos,
        click_count: count,
        ..MouseDownEvent::default()
    });
    cx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: pos,
        click_count: count,
        ..MouseUpEvent::default()
    });
}

#[test]
fn drag_and_multi_click_copy_without_activating_links() {
    let mut app = TestAppContext::single();
    app.dispatcher.allow_parking();
    let profile = tempfile::tempdir().unwrap();
    app.update(|cx| {
        register_workspace_bindings_and_menus(cx);
        bind_text_input_keys(cx);
    });
    let (view, cx) = app.add_window_view(|window, cx| {
        let options = LaunchOptions {
            daemon_socket: Some(profile.path().join("no-listener")),
            ..LaunchOptions::default()
        };
        PrimeDesktopView::new(options, profile.path().to_owned(), window, cx)
    });
    cx.executor()
        .block_test(cx.condition(&view, |view, _| view.toast.is_some()));
    view.update_in(cx, |view, _, cx| {
        view.toast = None;
        view.prefs.inspector_open = false;
        view.prefs.sidebar_open = false;
        view.session.entries.push(TranscriptEntry::Assistant {
            key: "turn".into(),
            blocks: vec![AssistantBlock::Text(MARKDOWN.to_owned())],
            error: None,
            model_label: None,
            streaming: false,
        });
        cx.notify();
    });
    cx.simulate_resize(size(px(960.), px(640.)));
    cx.update(|window, cx| window.draw(cx).clear());
    let bounds = cx.debug_bounds("turn-md-0-md-1").unwrap();
    let first = bounds.origin + point(px(1.), px(1.));
    let after_last = bounds.bottom_right() + point(px(2.), px(2.));
    cx.simulate_mouse_down(first, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(after_last, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(after_last, MouseButton::Left, Modifiers::none());
    cx.simulate_keystrokes("secondary-c");
    assert_eq!(clipboard(cx), SAMPLE);
    assert_eq!(cx.opened_url(), None);

    click_count(cx, first, 2);
    cx.dispatch_action(CopySelection);
    assert_eq!(clipboard(cx), "link");
    assert_eq!(cx.opened_url(), None);
    click_count(cx, first, 3);
    cx.dispatch_action(CopySelection);
    assert_eq!(clipboard(cx), "link café\n");

    let second = bounds.bottom_left() + point(px(1.), px(-2.));
    cx.simulate_click(second, Modifiers::none());
    assert_eq!(cx.opened_url(), None);
    cx.simulate_click(first, Modifiers::none());
    assert_eq!(cx.opened_url().as_deref(), Some(URL));
    let button = cx.debug_bounds("turn-copy").unwrap().center();
    cx.simulate_click(button, Modifiers::none());
    assert_eq!(clipboard(cx), MARKDOWN);
    cx.update(|window, cx| window.draw(cx).clear());
    assert!(cx.debug_bounds("turn-copy-copied").is_some());
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.update(|window, cx| window.draw(cx).clear());
    assert!(cx.debug_bounds("turn-copy").is_some());
    view.update_in(cx, |view, window, cx| {
        view.composer_input
            .update(cx, |input, cx| input.set_text("draft", cx));
        window.focus(&view.composer_input.focus_handle(cx));
    });
    cx.simulate_keystrokes("secondary-a secondary-c");
    assert_eq!(clipboard(cx), "draft");
    app.quit();
}

#[test]
fn soft_wraps_alignment_and_selected_newlines_share_glyph_geometry() {
    let mut app = TestAppContext::single();
    let (_, cx) = app.add_window_view(|_, _| Transcript);
    let text = "wide wide wide wide wide\n\n雪";
    let styled = StyledText::new(text);
    let layout = styled.layout().clone();
    cx.draw(point(px(0.), px(0.)), size(px(62.), px(300.)), |_, _| {
        div()
            .w(px(62.))
            .text_align(gpui::TextAlign::Right)
            .child(styled)
    });
    let mut quads = Vec::new();
    range_rects(&layout, text, 0..text.len(), TextAlign::Right, |r| {
        quads.push(r);
    });
    assert!(quads.len() > text.lines().count());
    assert_eq!(quads[quads.len() - 2].size.width, px(5.));
    assert!(
        quads
            .windows(2)
            .all(|pair| pair[0].bottom() == pair[1].top())
    );
    let last = quads.last().unwrap();
    let left = last.origin + point(px(0.1), px(1.));
    assert_eq!(
        hit_test(&layout, text, TextAlign::Right, left).glyph,
        text.rfind('雪')
    );
    app.quit();
}
