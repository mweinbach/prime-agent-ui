//! `prime-agent-ui` binary entrypoint.

use std::path::PathBuf;

use pa_desktop::run_desktop_app;
use pa_desktop::state::ActiveModal;
use pa_desktop::theme::{InspectorTab, ThemeMode};
use pa_desktop::ui::workspace::LaunchOptions;

fn main() {
    let mut options = LaunchOptions::default();
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--daemon-socket" => {
                if let Some(sock) = args.next() {
                    options.daemon_socket = Some(PathBuf::from(sock));
                }
            }
            "--shared-daemon" => {
                options.shared_daemon = true;
            }
            "--cwd" => {
                if let Some(dir) = args.next() {
                    options.initial_cwd = Some(PathBuf::from(dir));
                }
            }
            "--light" => {
                options.theme_override = Some(ThemeMode::Light);
            }
            "--dark" => {
                options.theme_override = Some(ThemeMode::Dark);
            }
            "--tab" => {
                if let Some(tab) = args.next() {
                    options.initial_tab = match tab.as_str() {
                        "agents" => Some(InspectorTab::Agents),
                        "harness" => Some(InspectorTab::Harness),
                        "runtime" => Some(InspectorTab::Runtime),
                        "system" => Some(InspectorTab::System),
                        _ => None,
                    };
                }
            }
            "--modal" => {
                if let Some(m) = args.next() {
                    options.initial_modal = match m.as_str() {
                        "models" => Some(ActiveModal::ModelPicker),
                        "switcher" => Some(ActiveModal::QuickSwitcher),
                        "settings" => Some(ActiveModal::SettingsAuth),
                        _ => None,
                    };
                }
            }
            "--version" | "-V" => {
                println!("prime-agent-ui {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--help" | "-h" => {
                println!(
                    "Prime Agent Desktop (Rune)\n\n\
                     Usage: prime-agent-ui [OPTIONS]\n\n\
                     Options:\n  \
                       --cwd <DIR>            Start in the given project directory\n  \
                       --daemon-socket <PATH> Attach to an existing daemon socket\n  \
                       --shared-daemon        Attach to the shared CLI daemon socket\n  \
                       --light / --dark       Override initial theme\n  \
                       --tab <NAME>           Open inspector tab (agents|harness|runtime|system)\n  \
                       --modal <NAME>         Open modal (models|switcher|settings)\n  \
                       --snap <FILE>          Render a PNG headlessly with Rune\n  \
                       --tree <FILE>          Write the headless element tree\n  \
                       --script <FILE>        Replay scripted Rune input\n  \
                       --size <WxH>           Set the Rune viewport size in points\n  \
                       -V, --version          Print version\n  \
                       -h, --help             Print help"
                );
                return;
            }
            _ => {}
        }
    }

    run_desktop_app(options);
}
