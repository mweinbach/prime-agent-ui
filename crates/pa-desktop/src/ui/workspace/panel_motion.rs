//! Slide panels at their natural width so their labels stay still relative to the moving edge.
use std::time::{Duration, Instant};

use gpui::{AnyElement, Context, Pixels, Window, div, prelude::*, px};

use crate::theme::Palette;
use crate::ui::inspector::{INSPECTOR_WIDTH, render_inspector};
use crate::ui::sidebar::{SIDEBAR_WIDTH, render_sidebar};
use crate::ui::stage::render_stage;

use super::PrimeDesktopView;

#[cfg(test)]
mod tests;

pub(super) const SIDEBAR_BREAKPOINT: f32 = 760.0;
pub(super) const INSPECTOR_BREAKPOINT: f32 = 1000.0;

#[derive(Clone, Copy)]
enum Side {
    Left,
    Right,
}

#[derive(Default)]
struct Slide {
    open: bool,
    travel: Option<Travel>,
}

struct Travel {
    from: f32,
    start: Instant,
    duration: Duration,
}

impl Slide {
    fn position(&mut self, now: Instant) -> f32 {
        let end = f32::from(self.open);
        let Some(travel) = &self.travel else {
            return end;
        };
        let elapsed = now.saturating_duration_since(travel.start);
        if elapsed >= travel.duration {
            self.travel = None;
            return end;
        }
        let t = elapsed.as_secs_f32() / travel.duration.as_secs_f32();
        // Openings arrive briskly and slow near the edge. Closings ease away from rest.
        let eased = if self.open {
            1.0 - (1.0 - t).powi(3)
        } else {
            t * t * (3.0 - 2.0 * t)
        };
        travel.from + (end - travel.from) * eased
    }

    fn update(&mut self, open: bool, snap: bool, now: Instant) -> f32 {
        if snap {
            self.open = open;
            self.travel = None;
        } else if open != self.open {
            let from = self.position(now);
            self.open = open;
            let distance = (f32::from(open) - from).abs();
            self.travel = (distance > f32::EPSILON).then(|| Travel {
                from,
                start: now,
                duration: Duration::from_millis(if open { 190 } else { 150 }).mul_f32(distance),
            });
        }
        self.position(now)
    }
}

#[derive(Default)]
pub(super) struct PanelMotion {
    docking: Option<(bool, bool)>,
    sidebar: Slide,
    inspector: Slide,
    scrim: Slide,
}

#[derive(Clone, Copy)]
struct Panel {
    side: Side,
    width: Pixels,
    docked: bool,
    open: bool,
    fraction: f32,
}

impl Panel {
    fn mounted(self) -> bool {
        self.open || self.fraction > 0.0
    }

    fn render(
        self,
        view: &PrimeDesktopView,
        window: &Window,
        palette: Palette,
        cx: &mut Context<PrimeDesktopView>,
    ) -> AnyElement {
        let content = match self.side {
            Side::Left => render_sidebar(view, window, palette, cx),
            Side::Right => render_inspector(view, palette, cx),
        };
        // Block controls as soon as dismissal returns focus to the composer.
        let surface = div()
            .absolute()
            .top_0()
            .bottom_0()
            .w(self.width)
            .child(content)
            .when(!self.open, |panel| {
                panel.child(div().absolute().inset_0().occlude())
            });
        let visible_width = self.width * self.fraction;
        if self.docked {
            let id = match self.side {
                Side::Left => "sidebar-dock-clip",
                Side::Right => "inspector-dock-clip",
            };
            div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .relative()
                .h_full()
                .w(visible_width)
                .flex_none()
                .overflow_hidden()
                .child(match self.side {
                    Side::Left => surface.right_0(),
                    Side::Right => surface.left_0(),
                })
                .into_any_element()
        } else {
            let id = match self.side {
                Side::Left => "sidebar-drawer",
                Side::Right => "inspector-drawer",
            };
            let panel = div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .absolute()
                .top_0()
                .bottom_0()
                .w(self.width)
                .occlude()
                .shadow_lg()
                .child(surface.left_0());
            let inset = visible_width - self.width;
            match self.side {
                Side::Left => panel.left(inset),
                Side::Right => panel.right(inset),
            }
            .into_any_element()
        }
    }
}

impl PrimeDesktopView {
    pub(super) fn render_panes(
        &mut self,
        window: &mut Window,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let width = window.viewport_size().width;
        let docking = (
            width >= px(SIDEBAR_BREAKPOINT),
            width >= px(INSPECTOR_BREAKPOINT),
        );
        let motion = &mut self.panel_motion;
        let snap = self.prefs.reduce_motion || motion.docking != Some(docking);
        motion.docking = Some(docking);
        let now = cx.background_executor().now();
        let left = Panel {
            side: Side::Left,
            width: width.min(px(SIDEBAR_WIDTH)),
            docked: docking.0,
            open: self.prefs.sidebar_open,
            fraction: motion.sidebar.update(self.prefs.sidebar_open, snap, now),
        };
        let right = Panel {
            side: Side::Right,
            width: width.min(px(INSPECTOR_WIDTH)),
            docked: docking.1,
            open: self.prefs.inspector_open,
            fraction: motion
                .inspector
                .update(self.prefs.inspector_open, snap, now),
        };
        let overlay_open = (left.open && !left.docked) || (right.open && !right.docked);
        // One shared scrim stays steady when switching directly between drawers.
        let opacity = motion.scrim.update(overlay_open, snap, now);
        if motion.sidebar.travel.is_some()
            || motion.inspector.travel.is_some()
            || motion.scrim.travel.is_some()
        {
            window.request_animation_frame();
        }
        let sidebar_width = if left.docked {
            left.width * left.fraction
        } else {
            px(0.0)
        };
        let mut panes = div().relative().size_full().overflow_hidden().child(
            div()
                .size_full()
                .flex()
                .flex_row()
                .when(left.docked && left.mounted(), |d| {
                    d.child(left.render(self, window, palette, cx))
                })
                .child(render_stage(
                    self,
                    palette,
                    left.docked && left.open,
                    sidebar_width,
                    cx,
                ))
                .when(right.docked && right.mounted(), |d| {
                    d.child(right.render(self, window, palette, cx))
                }),
        );
        if overlay_open || opacity > 0.0 {
            panes = panes.child(
                div()
                    .id("drawer-scrim")
                    .debug_selector(|| "drawer-scrim".to_owned())
                    .absolute()
                    .inset_0()
                    .bg(palette.bg_scrim.opacity(opacity))
                    .when(overlay_open, |scrim| {
                        scrim
                            .occlude()
                            .on_click(cx.listener(move |view, _, window, cx| {
                                if !right.docked && view.prefs.inspector_open {
                                    view.toggle_inspector(window, cx);
                                } else if !left.docked && view.prefs.sidebar_open {
                                    view.toggle_sidebar(window, cx);
                                }
                            }))
                    }),
            );
        }
        // During a narrow-panel switch the incoming drawer owns any overlapping hit targets.
        let overlays = if left.open {
            [right, left]
        } else {
            [left, right]
        };
        for panel in overlays {
            if !panel.docked && panel.mounted() {
                panes = panes.child(panel.render(self, window, palette, cx));
            }
        }
        panes.into_any_element()
    }
}
