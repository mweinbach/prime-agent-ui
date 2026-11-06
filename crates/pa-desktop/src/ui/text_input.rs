//! Native Rune editors share the app's font, selection color and focus ring.

use rune::{El, TextEdit, TextStyle, text_input};

use crate::theme::Palette;
use crate::ui::workspace::PrimeDesktopView;

pub(super) fn field(
    key: &str,
    edit: &TextEdit,
    placeholder: &str,
    palette: Palette,
    access: fn(&mut PrimeDesktopView) -> &mut TextEdit,
) -> El {
    text_input(edit, TextStyle::new(14., 400.).lh(21.))
        .key(key)
        .label(placeholder)
        .placeholder(placeholder, palette.text_muted)
        .caret(palette.accent, palette.selection_bg)
        .focus_ring(1., 2., palette.accent)
        .color(palette.text_primary)
        .min_w(0.)
        .w_full()
        .on_edit(access)
}
