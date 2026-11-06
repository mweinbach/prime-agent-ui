//! Three panes on large windows; the same panels become drawers at narrow widths.
use rune::{Color, Cx, Edge, El, Transition, col, el, row};

use super::PrimeDesktopView;
use crate::theme::Palette;
use crate::ui::panel_controls::{control, label};

pub(super) const SIDEBAR_BREAKPOINT: f32 = 760.0;
pub(super) const INSPECTOR_BREAKPOINT: f32 = 1000.0;

impl PrimeDesktopView {
    pub fn view(&self, cx: &mut Cx<Self>) -> El {
        cx.reduced_motion(self.prefs.reduce_motion || cx.system_reduced_motion());
        let p = Palette::for_mode(self.prefs.theme);
        let show_left = self.prefs.sidebar_open;
        let show_right = self.prefs.inspector_open;
        let dock_left = self.viewport_width >= SIDEBAR_BREAKPOINT;
        let dock_right = self.viewport_width >= INSPECTOR_BREAKPOINT;
        let mut panes = row().key("panes").w_full().h_full();
        if dock_left && show_left {
            panes = panes.child(
                self.sidebar(cx)
                    .transition(Transition::move_edge(Edge::Leading)),
            );
        }
        panes = panes.child(
            col()
                .key("center-pane")
                .grow(1.0)
                .min_w(0.0)
                .h_full()
                .child(self.stage(cx)),
        );
        if dock_right && show_right {
            panes = panes.child(
                self.inspector(cx)
                    .transition(Transition::move_edge(Edge::Trailing)),
            );
        }

        let mut root = col()
            .key("workspace")
            .w_full()
            .h_full()
            .fill(p.bg_canvas)
            .on_layout(|s: &mut Self, [_, _, width, _]| s.viewport_width = width)
            .child(panes);
        let left_drawer = show_left && !dock_left;
        let right_drawer = show_right && !dock_right;
        if left_drawer || right_drawer {
            root = root.child(
                el().key("drawer-scrim")
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .fill(Color {
                        a: 0.28,
                        ..Color::default()
                    })
                    .on_press(|s: &mut Self| {
                        s.dismiss_overlay();
                    }),
            );
            // Both can be open when resizing a previously wide window. Place the
            // inspector in front, matching Escape's first drawer to dismiss.
            if left_drawer {
                root = root.child(
                    col()
                        .key("sidebar-drawer")
                        .pin(Some(0.0), Some(0.0), None, Some(0.0))
                        .hit()
                        .child(
                            self.sidebar(cx)
                                .transition(Transition::move_edge(Edge::Leading)),
                        ),
                );
            }
            if right_drawer {
                root = root.child(
                    col()
                        .key("inspector-drawer")
                        .pin(None, Some(0.0), Some(0.0), Some(0.0))
                        .hit()
                        .child(
                            self.inspector(cx)
                                .transition(Transition::move_edge(Edge::Trailing)),
                        ),
                );
            }
        }
        if let Some(modal) = self.active_modal(cx) {
            root = root.child(modal);
        }
        if let Some((message, error)) = &self.toast {
            let color = if *error { p.danger } else { p.text_primary };
            root = root.child(
                control("toast-banner", "Dismiss notification", p)
                    .pin(None, None, Some(22.0), Some(22.0))
                    .max_w((self.viewport_width - 44.0).clamp(0.0, 420.0))
                    .px(14.0)
                    .py(9.0)
                    .radius(12.0)
                    .fill(p.bg_elevated)
                    .border(1.0, if *error { p.danger } else { p.border_strong })
                    .on_press(|s: &mut Self| s.toast = None)
                    .child(label(message, 12.5, 500.0, color).wrap()),
            );
        }
        root
    }
}
