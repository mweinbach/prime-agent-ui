//! Native Rune desktop application and agentic harness studio for Prime Agent.
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
mod shortcuts;
pub mod state;
pub mod theme;
pub mod ui;

use rune::{App, AppSpec, Chrome};

use crate::assets::EMBEDDED_FONTS;
use crate::theme::{font_mono, font_sans};
use crate::ui::workspace::{LaunchOptions, PrimeDesktopView};

fn open_workspace(options: LaunchOptions) -> PrimeDesktopView {
    let agent_dir = pa_types::platform::agent_dir().unwrap_or_else(|| ".prime/agent".into());
    PrimeDesktopView::new(options, agent_dir)
}

/// Launch the Rune desktop window or run Rune's headless tools on the same view.
///
/// # Panics
///
/// Panics if the platform cannot initialize the desktop or its I/O runtime.
pub fn run_desktop_app(options: LaunchOptions) {
    let spec = AppSpec {
        fonts: EMBEDDED_FONTS,
        families: (font_sans(), font_mono()),
        min_size: Some((660.0, 540.0)),
        setup: Some(PrimeDesktopView::set_waker),
        poll: Some(PrimeDesktopView::poll),
        on_key: Some(shortcuts::handle_key),
        ..AppSpec::new(
            "Prime Agent",
            (1360.0, 860.0),
            || open_workspace(LaunchOptions::default()),
            PrimeDesktopView::view,
        )
    };
    rune::set_app_id("ai.primeintellect.prime-agent.desktop");
    let app = App::new(spec)
        .init(move || open_workspace(options))
        .chrome(Chrome {
            full_size_content: cfg!(target_os = "macos"),
            traffic_lights: cfg!(target_os = "macos").then_some((16.0, 15.0)),
            ..Chrome::default()
        })
        .standard_menus();
    shortcuts::install_menus(app).run();
}
