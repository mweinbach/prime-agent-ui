//! Shared controls for conversation content. Rune handles text selection and copying.

use rune::{Color, Cursor, El, TextStyle, row, text};

use crate::assets::Icon;
use crate::theme::Palette;
use crate::ui::workspace::PrimeDesktopView;

pub(super) fn label(content: impl Into<String>, size: f32, color: Color) -> El {
    text(content).font(TextStyle::new(size, 450.)).color(color)
}

pub(super) fn control(key: &str, title: &str, p: Palette) -> El {
    row()
        .key(key)
        .role("button")
        .label(title)
        .focusable(true)
        .cursor(Cursor::Pointer)
        .preserve_focus_on_press(true)
        .focus_ring(1.5, 1., p.text_secondary)
        .hover(p.bg_hover)
        .press(p.bg_selected)
        .center()
        .gap(5.)
        .px(7.)
        .py(5.)
        .radius(6.)
}

pub(super) fn icon_control(key: &str, title: &str, icon: Icon, p: Palette) -> El {
    control(key, title, p)
        .size(28., 28.)
        .padding(0.)
        .help(title)
        .child(icon.render(16., p.text_secondary))
}

pub(super) fn copy_button(key: &str, value: &str, p: Palette) -> El {
    let value = value.to_string();
    icon_control(key, "Copy", Icon::Copy, p)
        .on_press_tx(move |_: &mut PrimeDesktopView, tx| tx.copy(value.clone()))
}
