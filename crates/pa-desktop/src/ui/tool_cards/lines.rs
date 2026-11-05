//! Limit expensive text layout to the preview until the user expands a block.

use gpui::{
    AnyElement, App, Div, IntoElement, SharedString, Stateful, Window, div, prelude::*, px,
};

use crate::theme::Palette;

/// GPUI measures the immediate child to find the scroll range. Give that child its
/// intrinsic width, including when a single line is wider than the whole window.
pub(crate) fn horizontal_scroll(id: SharedString, content: impl IntoElement) -> Stateful<Div> {
    let mut scroll = div()
        .debug_selector(|| id.to_string())
        .id(id)
        .flex()
        .w_full()
        .min_w_0()
        .overflow_x_scroll()
        .whitespace_nowrap()
        .child(div().flex_none().child(content));
    scroll.style().restrict_scroll_to_axis = Some(true);
    scroll
}

pub(crate) fn limited_lines(
    id: impl Into<SharedString>,
    text: SharedString,
    collapsed_limit: usize,
    palette: Palette,
    render: impl FnOnce(&str) -> AnyElement + 'static,
) -> AnyElement {
    LinePreview {
        id: id.into(),
        text,
        collapsed_limit,
        palette,
        render,
    }
    .into_any_element()
}

#[derive(IntoElement)]
struct LinePreview<F: FnOnce(&str) -> AnyElement + 'static> {
    id: SharedString,
    text: SharedString,
    collapsed_limit: usize,
    palette: Palette,
    render: F,
}

// Return a UTF-8 boundary before the first hidden row without allocating all the lines.
fn preview_end(text: &str, limit: usize) -> Option<(usize, usize)> {
    let mut breaks = text.match_indices('\n');
    let (offset, _) = breaks.nth(limit.checked_sub(1)?)?;
    if offset + 1 == text.len() {
        return None;
    }
    Some((
        offset,
        limit + breaks.count() + usize::from(!text.ends_with('\n')),
    ))
}

impl<F: FnOnce(&str) -> AnyElement + 'static> RenderOnce for LinePreview<F> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some((end, total)) = preview_end(&self.text, self.collapsed_limit) else {
            return (self.render)(&self.text);
        };
        let expanded = window.use_keyed_state(self.id.clone(), cx, |_, _| false);
        let open = *expanded.read(cx);
        let palette = self.palette;
        div()
            .w_full()
            .min_w_0()
            .child((self.render)(if open {
                &self.text
            } else {
                &self.text[..end]
            }))
            .child(
                div()
                    .id(SharedString::from(format!("{}-toggle", self.id)))
                    .debug_selector({
                        let id = self.id;
                        move || format!("{id}-toggle")
                    })
                    .cursor_pointer()
                    .px(px(8.0))
                    .py(px(4.0))
                    .text_size(px(11.5))
                    .text_color(palette.accent)
                    .hover(|s| s.bg(palette.bg_hover))
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        expanded.update(cx, |open, cx| {
                            *open = !*open;
                            cx.notify();
                        });
                    })
                    .child(if open {
                        "Show less".into()
                    } else {
                        format!("Show all {total} lines")
                    }),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeMode;
    use gpui::{Context, Modifiers, TestAppContext};

    struct Console;

    impl Render for Console {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let output = format!("\x1b[31m{}\x1b[0m\n\nthird\nlast", "x".repeat(180));
            div()
                .w(px(220.0))
                .child(super::super::render_console_output(
                    "test",
                    "output",
                    &output,
                    2,
                    false,
                    Palette::for_mode(ThemeMode::Light),
                ))
        }
    }

    #[test]
    fn console_preview_scrolls_long_lines_and_copies_clean_full_output() {
        let mut app = TestAppContext::single();
        let (_, cx) = app.add_window_view(|_, _| Console);
        cx.update(|window, cx| window.draw(cx).clear());
        let scroll = cx.debug_bounds("test-output-scroll").unwrap();
        let preview = cx.debug_bounds("test-output-text").unwrap();
        assert!(preview.size.width > scroll.size.width);
        let toggle = cx.debug_bounds("test-output-lines-toggle").unwrap();
        cx.simulate_click(toggle.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear());
        let full = cx.debug_bounds("test-output-text").unwrap();
        assert!(full.size.height > preview.size.height);
        let copy = cx.debug_bounds("test-output-copy").unwrap();
        cx.simulate_click(copy.center(), Modifiers::default());
        cx.update(|_, cx| {
            let text = cx.read_from_clipboard().unwrap().text().unwrap();
            assert_eq!(text, format!("{}\n\nthird\nlast", "x".repeat(180)));
        });
        app.quit();
    }

    #[test]
    fn truncation_counts_empty_rows_and_treats_a_final_terminator_as_part_of_its_line() {
        assert_eq!(preview_end("界\n\nthree\n", 2), Some((4, 3)));
        assert_eq!(preview_end("界\n\nthree", 2), Some((4, 3)));
        assert_eq!(preview_end("界\n\n", 2), None);
        assert_eq!(preview_end("界\n", 1), None);
    }
}
