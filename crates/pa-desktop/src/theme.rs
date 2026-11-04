//! Visual design tokens, typography, and persisted desktop preferences.
//!
//! Warm surfaces for conversation, with restrained color for state and actions.

use std::path::{Path, PathBuf};

use gpui::{Hsla, SharedString, rgb, rgba};
use serde::{Deserialize, Serialize};

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
    pub bg_canvas: Hsla,
    /// Left sidebar rail background.
    pub bg_sidebar: Hsla,
    /// Right inspector drawer background.
    pub bg_inspector: Hsla,
    /// Elevated card / composer surface.
    pub bg_elevated: Hsla,
    /// Subtle recessed code / terminal block background.
    pub bg_code: Hsla,
    /// User message warm bubble background.
    pub bg_user_bubble: Hsla,
    /// Hover tint on interactive rows/buttons.
    pub bg_hover: Hsla,
    /// Selected row tint in sidebar/lists.
    pub bg_selected: Hsla,
    /// Modal backdrop scrim.
    pub bg_scrim: Hsla,

    /// Subtle hairline divider.
    pub border_subtle: Hsla,
    /// Emphasized border (composer focus, active cards).
    pub border_strong: Hsla,

    /// Primary body & heading ink.
    pub text_primary: Hsla,
    /// Secondary label & metadata ink.
    pub text_secondary: Hsla,
    /// Muted placeholder / timestamp ink.
    pub text_muted: Hsla,
    /// Crisp ink over `accent` buttons.
    pub text_on_accent: Hsla,

    /// Terracotta accent for primary actions.
    pub accent: Hsla,
    /// Hovered primary accent.
    pub accent_hover: Hsla,
    /// Soft translucent terracotta wash for badges/pills.
    pub accent_soft: Hsla,

    /// Sage green (healthy daemon, completed/live agent, added diff lines).
    pub success: Hsla,
    pub success_soft: Hsla,
    /// Ochre amber (thinking/working pulse, compaction, warnings).
    pub warning: Hsla,
    pub warning_soft: Hsla,
    /// Soft rose (errors, abort, removed diff lines).
    pub danger: Hsla,
    pub danger_soft: Hsla,
    /// Slate blue (RLM subagent links, depth tags, python kernel accent).
    pub info: Hsla,
    pub info_soft: Hsla,
}

impl Palette {
    #[must_use]
    pub fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self {
                mode,
                bg_canvas: rgb(0x001C_1B18).into(),
                bg_sidebar: rgb(0x0015_1412).into(),
                bg_inspector: rgb(0x0017_1613).into(),
                bg_elevated: rgb(0x0025_2420).into(),
                bg_code: rgb(0x0013_1210).into(),
                bg_user_bubble: rgb(0x002B_2924).into(),
                bg_hover: rgba(0xFFF7_E810).into(),
                bg_selected: rgba(0xD977_5722).into(),
                bg_scrim: rgba(0x0B0A_09AD).into(),

                border_subtle: rgba(0xFFF6_E514).into(),
                border_strong: rgba(0xFFF6_E529).into(),

                text_primary: rgb(0x00F2_EFE9).into(),
                text_secondary: rgb(0x00AD_A79C).into(),
                text_muted: rgb(0x009A_958A).into(),
                text_on_accent: rgb(0x0029_1F18).into(),

                accent: rgb(0x00E0_A289).into(),
                accent_hover: rgb(0x00F0_B298).into(),
                accent_soft: rgba(0xD977_5726).into(),

                success: rgb(0x006B_A87C).into(),
                success_soft: rgba(0x6BA8_7C24).into(),
                warning: rgb(0x00D9_A45B).into(),
                warning_soft: rgba(0xD9A4_5B24).into(),
                danger: rgb(0x00DC_6B6B).into(),
                danger_soft: rgba(0xDC6B_6B24).into(),
                info: rgb(0x0073_9EC2).into(),
                info_soft: rgba(0x739E_C224).into(),
            },
            ThemeMode::Light => Self {
                mode,
                bg_canvas: rgb(0x00FA_F9F6).into(),
                bg_sidebar: rgb(0x00F1_F0EC).into(),
                bg_inspector: rgb(0x00F6_F5F1).into(),
                bg_elevated: rgb(0x00FF_FFFF).into(),
                bg_code: rgb(0x00F2_F1EE).into(),
                bg_user_bubble: rgb(0x00EF_ECE5).into(),
                bg_hover: rgba(0x1F1D_190D).into(),
                bg_selected: rgba(0xC964_421C).into(),
                bg_scrim: rgba(0x1C1A_1666).into(),

                border_subtle: rgba(0x1F1D_1916).into(),
                border_strong: rgba(0x1F1D_192E).into(),

                text_primary: rgb(0x002B_2925).into(),
                text_secondary: rgb(0x005E_5A52).into(),
                text_muted: rgb(0x006D_685E).into(),
                text_on_accent: rgb(0x00FF_FDF9).into(),

                accent: rgb(0x009F_5037).into(),
                accent_hover: rgb(0x0084_3F2A).into(),
                accent_soft: rgba(0xC964_421E).into(),

                success: rgb(0x002F_7D47).into(),
                success_soft: rgba(0x2F7D_471C).into(),
                warning: rgb(0x00AD_721E).into(),
                warning_soft: rgba(0xAD72_1E1C).into(),
                danger: rgb(0x00BF_3F3F).into(),
                danger_soft: rgba(0xBF3F_3F1C).into(),
                info: rgb(0x0036_6994).into(),
                info_soft: rgba(0x3669_941C).into(),
            },
        }
    }
}

/// Serif for conversation headings.
#[must_use]
pub fn font_serif() -> SharedString {
    SharedString::new_static("Georgia")
}

/// Native UI sans font family.
#[must_use]
pub fn font_sans() -> SharedString {
    SharedString::new_static(".SystemUIFont")
}

/// Crisp monospace font family for Python REPL cells, bash output, diffs, and IDs.
#[must_use]
pub fn font_mono() -> SharedString {
    SharedString::new_static("Menlo")
}
