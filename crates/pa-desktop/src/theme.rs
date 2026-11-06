//! Visual design tokens, typography, and persisted desktop preferences.
//!
//! Warm surfaces for conversation, with restrained color for state and actions.

use std::path::{Path, PathBuf};

use rune::Color;
use rune::color::srgb_decode;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// Desktop color scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    /// Charcoal surfaces.
    Dark,
    /// Light paper surfaces.
    #[default]
    Light,
}

impl ThemeMode {
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }
}

/// Which Prime Agent supervisor socket the desktop app attaches to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonTarget {
    /// Dedicated supervisor (`desktop-daemon.sock`) running the workspace/bundled
    /// `prime-agent` binary with authorized Codex and Gemini catalog support.
    #[default]
    DesktopHarness,
    /// Shared CLI supervisor (`daemon.sock`) so terminal sessions can be
    /// observed and steered live from the desktop app.
    SharedCli,
}

impl DaemonTarget {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::DesktopHarness => "Desktop daemon",
            Self::SharedCli => "CLI daemon",
        }
    }

    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::DesktopHarness => Self::SharedCli,
            Self::SharedCli => Self::DesktopHarness,
        }
    }
}

/// Transcript verbosity tier (mirrors Prime Agent's `overview` -> `details` -> `all`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DetailLevel {
    /// Clean prose-first reading: thinking collapsed, tool outputs compact, diffs collapsed.
    #[default]
    Overview,
    /// Thinking & edit diffs expanded, long stdout compact.
    Details,
    /// Every thinking block, REPL output, shell log, and diff expanded.
    All,
}

impl DetailLevel {
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Overview => Self::Details,
            Self::Details => Self::All,
            Self::All => Self::Overview,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Details => "Details",
            Self::All => "All",
        }
    }

    #[must_use]
    pub const fn show_thinking(self) -> bool {
        !matches!(self, Self::Overview)
    }

    #[must_use]
    pub const fn expand_diffs(self) -> bool {
        !matches!(self, Self::Overview)
    }

    #[must_use]
    pub const fn expand_tool_output(self) -> bool {
        matches!(self, Self::All)
    }
}

/// Right-hand Agentic Harness inspector tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InspectorTab {
    #[default]
    Agents,
    Harness,
    Runtime,
    System,
}

impl InspectorTab {
    pub const ALL: [Self; 4] = [Self::Agents, Self::Harness, Self::Runtime, Self::System];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Agents => "Agents",
            Self::Harness => "Harness",
            Self::Runtime => "Runtime",
            Self::System => "System",
        }
    }
}

/// Lightweight desktop-only UI preferences saved under `~/.prime/agent/desktop-ui.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DesktopPreferences {
    pub theme: ThemeMode,
    #[serde(default)]
    pub reduce_motion: bool,
    pub sidebar_open: bool,
    pub inspector_open: bool,
    pub inspector_tab: InspectorTab,
    pub detail_level: DetailLevel,
    pub daemon_target: DaemonTarget,
    #[serde(alias = "recentCwds")]
    pub project_folders: Vec<String>,
    pub last_project: Option<String>,
    pub project_collapsed: std::collections::BTreeMap<String, bool>,
    pub cli_data_dir: Option<PathBuf>,
}

impl Default for DesktopPreferences {
    fn default() -> Self {
        Self {
            theme: ThemeMode::default(),
            reduce_motion: false,
            sidebar_open: true,
            inspector_open: false,
            inspector_tab: InspectorTab::Agents,
            detail_level: DetailLevel::Overview,
            daemon_target: DaemonTarget::DesktopHarness,
            project_folders: Vec::new(),
            last_project: None,
            project_collapsed: std::collections::BTreeMap::new(),
            cli_data_dir: None,
        }
    }
}

impl DesktopPreferences {
    #[must_use]
    pub(crate) fn load(agent_dir: &Path) -> Self {
        let path = agent_dir.join("desktop-ui.json");
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub(crate) fn save(&self, agent_dir: &Path) -> std::io::Result<()> {
        let path = agent_dir.join("desktop-ui.json");
        std::fs::create_dir_all(agent_dir)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)
    }
}

/// Complete semantic color palette for a single frame render.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub mode: ThemeMode,
    /// Primary stage background.
    pub bg_canvas: Color,
    /// Left sidebar rail background.
    pub bg_sidebar: Color,
    /// Right inspector drawer background.
    pub bg_inspector: Color,
    /// Elevated card / composer surface.
    pub bg_elevated: Color,
    /// Subtle recessed code / terminal block background.
    pub bg_code: Color,
    /// User message warm bubble background.
    pub bg_user_bubble: Color,
    /// Hover tint on interactive rows/buttons.
    pub bg_hover: Color,
    /// Selected row tint in sidebar/lists.
    pub bg_selected: Color,
    /// Modal backdrop scrim.
    pub bg_scrim: Color,

    /// Subtle hairline divider.
    pub border_subtle: Color,
    /// Emphasized border (composer focus, active cards).
    pub border_strong: Color,

    /// Primary body & heading ink.
    pub text_primary: Color,
    /// Secondary label & metadata ink.
    pub text_secondary: Color,
    /// Muted placeholder / timestamp ink.
    pub text_muted: Color,
    /// Crisp ink over `accent` buttons.
    pub text_on_accent: Color,

    /// Terracotta accent for primary actions.
    pub accent: Color,
    /// Hovered primary accent.
    pub accent_hover: Color,
    /// Soft translucent terracotta wash for badges/pills.
    pub accent_soft: Color,
    /// Highlight behind selected text in the focused control.
    pub selection_bg: Color,
    /// Preserve the selection context when focus moves elsewhere.
    pub selection_bg_inactive: Color,

    /// Sage green (healthy daemon, completed/live agent, added diff lines).
    pub success: Color,
    pub success_soft: Color,
    /// Ochre amber (thinking/working pulse, compaction, warnings).
    pub warning: Color,
    pub warning_soft: Color,
    /// Soft rose (errors, abort, removed diff lines).
    pub danger: Color,
    pub danger_soft: Color,
    /// Slate blue (RLM subagent links, depth tags, python kernel accent).
    pub info: Color,
    pub info_soft: Color,
}

impl Palette {
    #[must_use]
    pub fn for_mode(mode: ThemeMode) -> Self {
        static DARK: LazyLock<Palette> = LazyLock::new(|| Palette::build(ThemeMode::Dark));
        static LIGHT: LazyLock<Palette> = LazyLock::new(|| Palette::build(ThemeMode::Light));
        match mode {
            ThemeMode::Dark => *DARK,
            ThemeMode::Light => *LIGHT,
        }
    }

    fn build(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self {
                mode,
                bg_canvas: rgb(0x001C_1B18),
                bg_sidebar: rgb(0x0015_1412),
                bg_inspector: rgb(0x0017_1613),
                bg_elevated: rgb(0x0025_2420),
                bg_code: rgb(0x0013_1210),
                bg_user_bubble: rgb(0x002B_2924),
                bg_hover: rgba(0xFFF7_E810),
                bg_selected: rgb(0x0030_2F2B),
                bg_scrim: rgba(0x0B0A_09AD),

                border_subtle: rgba(0xFFF6_E514),
                border_strong: rgba(0xFFF6_E529),

                text_primary: rgb(0x00F2_EFE9),
                text_secondary: rgb(0x00AD_A79C),
                text_muted: rgb(0x00A4_9E93),
                text_on_accent: rgb(0x0029_1F18),

                accent: rgb(0x00E0_A289),
                accent_hover: rgb(0x00F0_B298),
                accent_soft: rgba(0xD977_5726),
                selection_bg: rgba(0xE0A2_8948),
                selection_bg_inactive: rgba(0xE0A2_8924),

                success: rgb(0x006B_A87C),
                success_soft: rgba(0x6BA8_7C24),
                warning: rgb(0x00D9_A45B),
                warning_soft: rgba(0xD9A4_5B24),
                danger: rgb(0x00E1_7575),
                danger_soft: rgba(0xDC6B_6B24),
                info: rgb(0x0073_9EC2),
                info_soft: rgba(0x739E_C224),
            },
            ThemeMode::Light => Self {
                mode,
                bg_canvas: rgb(0x00FA_F9F6),
                bg_sidebar: rgb(0x00F1_F0EC),
                bg_inspector: rgb(0x00F6_F5F1),
                bg_elevated: rgb(0x00FF_FFFF),
                bg_code: rgb(0x00F2_F1EE),
                bg_user_bubble: rgb(0x00EF_ECE5),
                bg_hover: rgba(0x1F1D_190D),
                bg_selected: rgb(0x00E9_E8E3),
                bg_scrim: rgba(0x1C1A_1666),

                border_subtle: rgba(0x1F1D_1916),
                border_strong: rgba(0x1F1D_192E),

                text_primary: rgb(0x002B_2925),
                text_secondary: rgb(0x005E_5A52),
                text_muted: rgb(0x0068_645B),
                text_on_accent: rgb(0x00FF_FDF9),

                accent: rgb(0x009F_5037),
                accent_hover: rgb(0x0084_3F2A),
                accent_soft: rgba(0xC964_421E),
                selection_bg: rgba(0xC964_4236),
                selection_bg_inactive: rgba(0xC964_421C),

                success: rgb(0x002D_7341),
                success_soft: rgba(0x2F7D_471C),
                warning: rgb(0x008C_5B18),
                warning_soft: rgba(0xAD72_1E1C),
                danger: rgb(0x00B6_3838),
                danger_soft: rgba(0xBF3F_3F1C),
                info: rgb(0x0036_6994),
                info_soft: rgba(0x3669_941C),
            },
        }
    }
}

// Design tokens are authored in sRGB. Rune stores unpremultiplied, linear-light RGB.
fn rgb(value: u32) -> Color {
    rgba((value << 8) | 0xff)
}

fn rgba(value: u32) -> Color {
    let [r, g, b, a] = value.to_be_bytes().map(|byte| f32::from(byte) / 255.0);
    Color {
        r: srgb_decode(r),
        g: srgb_decode(g),
        b: srgb_decode(b),
        a,
    }
}

/// Display face for conversation headings.
#[must_use]
pub const fn font_display() -> &'static str {
    "Google Sans Flex"
}

/// Historical name for the conversation heading face.
#[must_use]
pub const fn font_serif() -> &'static str {
    font_display()
}

/// UI and conversation body font.
#[must_use]
pub const fn font_sans() -> &'static str {
    "Google Sans"
}

/// Crisp monospace font family for Python REPL cells, bash output, diffs, and IDs.
#[must_use]
pub const fn font_mono() -> &'static str {
    "Google Sans Code"
}

#[cfg(test)]
mod tests {
    use super::*;

    // Rune already stores linear RGB, so WCAG's luminance weights apply directly.
    fn contrast(fg: Color, bg: Color) -> f32 {
        let luminance = |c: Color| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
        let (a, b) = (luminance(fg), luminance(bg));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn labels_and_status_text_are_legible_on_each_workspace_surface() {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let p = Palette::for_mode(mode);
            let surfaces = [
                p.bg_canvas,
                p.bg_sidebar,
                p.bg_inspector,
                p.bg_elevated,
                p.bg_code,
                p.bg_user_bubble,
            ];
            for ink in [
                p.text_primary,
                p.text_secondary,
                p.text_muted,
                p.accent,
                p.success,
                p.warning,
                p.danger,
                p.info,
            ] {
                for surface in surfaces {
                    assert!(
                        contrast(ink, surface) >= 4.5,
                        "{mode:?} ink {ink:?} on {surface:?}"
                    );
                }
            }
            assert!(contrast(p.text_on_accent, p.accent) >= 4.5);
        }
    }
}
