//! Native GPUI desktop application and agentic harness studio for Prime Agent.
// UI layout coordinates and byte sizes convert across bounded f32/usize/u64 boundaries;
// GPUI render trees are flat builders by design, and Copy theme `Palette` structs are passed
// by value across pure element functions just like GPUI `Hsla`/`Style` values.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::struct_excessive_bools,
    clippy::too_many_lines,
    clippy::large_types_passed_by_value
)]

pub mod assets;
pub mod bridge;
pub mod state;
pub mod theme;
pub mod ui;

use gpui::{
    App, Application, Bounds, TitlebarOptions, WindowBackgroundAppearance, WindowBounds,
    WindowOptions, point, prelude::*, px, size,
};

use crate::assets::{EmbeddedAssets, register_embedded_fonts};
use crate::ui::text_input::bind_text_input_keys;
use crate::ui::workspace::{
    LaunchOptions, PrimeDesktopView, register_workspace_bindings_and_menus,
};

/// Launch the GPUI desktop window and enter the native platform runloop.
///
/// # Panics
///
/// Panics only if the native window server rejects opening the primary application window.
pub fn run_desktop_app(options: LaunchOptions) {
    Application::new()
        .with_assets(EmbeddedAssets)
        .run(move |cx: &mut App| {
            register_embedded_fonts(cx);
            bind_text_input_keys(cx);
            register_workspace_bindings_and_menus(cx);

            let bounds = Bounds::centered(None, size(px(1360.0), px(860.0)), cx);
            let agent_dir =
                pa_types::platform::agent_dir().unwrap_or_else(|| ".prime/agent".into());
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(660.0), px(540.0))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Prime Agent".into()),
                        appears_transparent: cfg!(target_os = "macos"),
                        traffic_light_position: cfg!(target_os = "macos")
                            .then(|| point(px(16.0), px(15.0))),
                    }),
                    window_background: WindowBackgroundAppearance::Opaque,
                    focus: true,
                    show: true,
                    ..Default::default()
                },
                move |window, cx| {
                    cx.new(|cx| PrimeDesktopView::new(options, agent_dir, window, cx))
                },
            )
            .expect("open Prime Agent GPUI window");

            cx.activate(true);
        });
}
