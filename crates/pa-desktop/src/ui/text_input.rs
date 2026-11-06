//! Native Rune editors share the app's font, selection color and focus ring.

use rune::{El, Key, KeyEvent, Mods, TextEdit, TextStyle, text_input};

use crate::theme::Palette;
use crate::ui::workspace::PrimeDesktopView;

pub(super) const LINE_HEIGHT: f32 = 21.0;

pub(super) fn field(
    key: &str,
    edit: &TextEdit,
    placeholder: &str,
    palette: Palette,
    access: fn(&mut PrimeDesktopView) -> &mut TextEdit,
) -> El {
    text_input(edit, TextStyle::new(14., 400.).lh(LINE_HEIGHT))
        .key(key)
        .label(placeholder)
        .placeholder(placeholder, palette.text_muted)
        .caret(palette.accent, palette.selection_bg)
        .focus_ring(1., 2., palette.accent)
        .color(palette.text_primary)
        .min_w(0.)
        .w_full()
        .on_edit(access)
        .on_input_key(|s: &mut PrimeDesktopView, ev, _| dismiss(s, ev))
}

pub(super) fn dismiss(s: &mut PrimeDesktopView, ev: &KeyEvent) -> bool {
    ev.key == Key::Escape && ev.mods == Mods::default() && s.dismiss_overlay()
}
