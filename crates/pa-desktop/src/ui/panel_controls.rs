//! Shared controls for navigation and workspace dialogs.

use rune::{Color, Cursor, El, TextStyle, el, text};

use crate::assets::Icon;
use crate::theme::Palette;

pub(super) fn label(value: impl Into<String>, size: f32, weight: f32, color: Color) -> El {
    text(value.into())
        .font(TextStyle::new(size, weight))
        .color(color)
}

pub(super) fn control(key: impl Into<String>, title: impl Into<String>, p: Palette) -> El {
    let title = title.into();
    el().key(key)
        .role("button")
        .label(title.clone())
        .help(title)
        .focusable(true)
        .cursor(Cursor::Pointer)
        .hover(p.bg_hover)
        .press(p.bg_selected)
        .focus_ring(1.5, 0.0, p.accent)
        .radius(6.0)
}

pub(super) fn icon_button(
    key: impl Into<String>,
    title: impl Into<String>,
    icon: Icon,
    p: Palette,
) -> El {
    control(key, title, p)
        .size(26.0, 26.0)
        .center()
        .child(icon.render(14.0, p.text_secondary))
}

pub(super) fn button(key: impl Into<String>, title: &str, p: Palette) -> El {
    control(key, title, p).h(28.0).px(8.0).center().child(label(
        title,
        12.0,
        500.0,
        p.text_secondary,
    ))
}
