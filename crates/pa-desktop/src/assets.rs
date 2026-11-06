//! The Google Sans typefaces and monochrome vector icons used by Rune.

use rune::{Color, El};

pub(crate) const EMBEDDED_FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/GoogleSans-Regular.ttf"),
    include_bytes!("../assets/fonts/GoogleSans-Medium.ttf"),
    include_bytes!("../assets/fonts/GoogleSans-SemiBold.ttf"),
    include_bytes!("../assets/fonts/GoogleSans-Bold.ttf"),
    include_bytes!("../assets/fonts/GoogleSans-Italic.ttf"),
    include_bytes!("../assets/fonts/GoogleSans-BoldItalic.ttf"),
    include_bytes!("../assets/fonts/GoogleSansCode-Regular.ttf"),
    include_bytes!("../assets/fonts/GoogleSansCode-Medium.ttf"),
    include_bytes!("../assets/fonts/GoogleSansCode-SemiBold.ttf"),
    include_bytes!("../assets/fonts/GoogleSansCode-Bold.ttf"),
    include_bytes!("../assets/fonts/GoogleSansCode-Italic.ttf"),
    include_bytes!("../assets/fonts/GoogleSansFlex-Regular.ttf"),
    include_bytes!("../assets/fonts/GoogleSansFlex-Medium.ttf"),
    include_bytes!("../assets/fonts/GoogleSansFlex-SemiBold.ttf"),
    include_bytes!("../assets/fonts/GoogleSansFlex-Bold.ttf"),
];

/// Named vector icons embedded directly into the executable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    Prime,
    Sidebar,
    Plus,
    Send,
    Stop,
    Steer,
    Folder,
    FolderOpen,
    FolderPlus,
    Compose,
    History,
    Pin,
    PinOff,
    Search,
    Sparkles,
    Cpu,
    Terminal,
    GitBranch,
    Layers,
    Sliders,
    Sun,
    Moon,
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    Copy,
    Check,
    Trash,
    EditPen,
    Image,
    Clock,
    Target,
    Zap,
    MessageSquare,
    Refresh,
    Close,
    ArrowDown,
    Shield,
}

impl Icon {
    pub const ALL: [Self; 38] = [
        Self::Prime,
        Self::Sidebar,
        Self::Plus,
        Self::Send,
        Self::Stop,
        Self::Steer,
        Self::Folder,
        Self::FolderOpen,
        Self::FolderPlus,
        Self::Compose,
        Self::History,
        Self::Pin,
        Self::PinOff,
        Self::Search,
        Self::Sparkles,
        Self::Cpu,
        Self::Terminal,
        Self::GitBranch,
        Self::Layers,
        Self::Sliders,
        Self::Sun,
        Self::Moon,
        Self::ChevronLeft,
        Self::ChevronRight,
        Self::ChevronDown,
        Self::Copy,
        Self::Check,
        Self::Trash,
        Self::EditPen,
        Self::Image,
        Self::Clock,
        Self::Target,
        Self::Zap,
        Self::MessageSquare,
        Self::Refresh,
        Self::Close,
        Self::ArrowDown,
        Self::Shield,
    ];

    /// Render the icon through Rune’s scale-aware glyph atlas.
    #[must_use]
    pub fn render(self, size: f32, color: Color) -> El {
        rune::icon(self.glyph(), size).color(color)
    }

    fn glyph(self) -> &'static rune::Icon {
        match self {
            Self::Prime => &rune::Icon {
                name: "prime",
                view: 30.0,
                body: r#"<path fill="currentColor" d="M9 5h7l-6 20H3zm11 0h7l-6 20h-7z"/>"#,
            },
            Self::Sidebar => &rune::Icon {
                name: "sidebar",
                view: 24.0,
                body: concat!(
                    r#"<rect fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" width="18" height="18" x="3" y="3" rx="3"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M9 3v18"/>"#,
                ),
            },
            Self::Plus => &rune::Icon {
                name: "plus",
                view: 24.0,
                body: concat!(
                    r#"<path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="M12 5v14"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="M5 12h14"/>"#,
                ),
            },
            Self::Send => &rune::Icon {
                name: "send",
                view: 24.0,
                body: concat!(
                    r#"<path fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round" d="M12 19V5"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round" d="m5 12 7-7 7 7"/>"#,
                ),
            },
            Self::Stop => &rune::Icon {
                name: "stop",
                view: 24.0,
                body: r#"<rect fill="currentColor" x="6" y="6" width="12" height="12" rx="2.5"/>"#,
            },
            Self::Steer => &rune::Icon {
                name: "steer",
                view: 24.0,
                body: concat!(
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" d="m15 14 5-5-5-5"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" d="M4 20v-7a4 4 0 0 1 4-4h12"/>"#,
                ),
            },
            Self::Folder => &rune::Icon {
                name: "folder",
                view: 24.0,
                body: r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M3 18V6a2 2 0 0 1 2-2h4l3 3h7a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z"/>"#,
            },
            Self::FolderOpen => &rune::Icon {
                name: "folder-open",
                view: 24.0,
                body: r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M3 18V6a2 2 0 0 1 2-2h4l3 3h6a2 2 0 0 1 2 2v2 M3 18l2.7-6.2A1.4 1.4 0 0 1 7 11h15l-3.2 7.7A2 2 0 0 1 17 20H5a2 2 0 0 1-2-2Z"/>"#,
            },
            Self::FolderPlus => &rune::Icon {
                name: "folder-plus",
                view: 24.0,
                body: r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M3 18V6a2 2 0 0 1 2-2h4l3 3h7a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z M12 10.5v6 M9 13.5h6"/>"#,
            },
            Self::Compose => &rune::Icon {
                name: "compose",
                view: 24.0,
                body: r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M12 4H6a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-6 M10 14l1-4 7.5-7.5 3 3L14 13Z"/>"#,
            },
            Self::History => &rune::Icon {
                name: "history",
                view: 24.0,
                body: r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M3.5 4.5v5h5 M4 9a8.5 8.5 0 1 1 .4 7 M12 7.5v5l3.5 2"/>"#,
            },
            Self::Pin => &rune::Icon {
                name: "pin",
                view: 24.0,
                body: r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M8 3.5h8 M9 3.5v6.8L6 14v1.5h12V14l-3-3.7V3.5 M12 15.5V21"/>"#,
            },
            Self::PinOff => &rune::Icon {
                name: "pin-off",
                view: 24.0,
                body: r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M9 3.5h7 M15 3.5v6.8l2 2.4 M9 10.3 6 14v1.5h9.5 M12 15.5V21 M3 3l18 18"/>"#,
            },
            Self::Search => &rune::Icon {
                name: "search",
                view: 24.0,
                body: concat!(
                    r#"<circle fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" cx="11" cy="11" r="8"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="m21 21-4.3-4.3"/>"#,
                ),
            },
            Self::Sparkles => &rune::Icon {
                name: "sparkles",
                view: 24.0,
                body: concat!(
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M20 3v4"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M22 5h-4"/>"#,
                ),
            },
            Self::Cpu => &rune::Icon {
                name: "cpu",
                view: 24.0,
                body: concat!(
                    r#"<rect fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" width="16" height="16" x="4" y="4" rx="2"/>"#,
                    r#"<rect fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" width="6" height="6" x="9" y="9" rx="1"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M15 2v2"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M15 20v2"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M2 15h2"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M2 9h2"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M20 15h2"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M20 9h2"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M9 2v2"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M9 20v2"/>"#,
                ),
            },
            Self::Terminal => &rune::Icon {
                name: "terminal",
                view: 24.0,
                body: concat!(
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M4 17L10 11 4 5"/>"#,
                    r#"<line fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" x1="12" x2="20" y1="19" y2="19"/>"#,
                ),
            },
            Self::GitBranch => &rune::Icon {
                name: "git-branch",
                view: 24.0,
                body: concat!(
                    r#"<line fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" x1="6" x2="6" y1="3" y2="15"/>"#,
                    r#"<circle fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" cx="18" cy="6" r="3"/>"#,
                    r#"<circle fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" cx="6" cy="18" r="3"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M18 9a9 9 0 0 1-9 9"/>"#,
                ),
            },
            Self::Layers => &rune::Icon {
                name: "layers",
                view: 24.0,
                body: concat!(
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="m12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83Z"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="m22 17.65-9.17 4.16a2 2 0 0 1-1.66 0L2 17.65"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="m22 12.65-9.17 4.16a2 2 0 0 1-1.66 0L2 12.65"/>"#,
                ),
            },
            Self::Sliders => &rune::Icon {
                name: "sliders",
                view: 24.0,
                body: concat!(
                    r#"<line fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" x1="4" x2="4" y1="21" y2="14"/>"#,
                    r#"<line fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" x1="4" x2="4" y1="10" y2="3"/>"#,
                    r#"<line fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" x1="12" x2="12" y1="21" y2="12"/>"#,
                    r#"<line fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" x1="12" x2="12" y1="8" y2="3"/>"#,
                    r#"<line fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" x1="20" x2="20" y1="21" y2="16"/>"#,
                    r#"<line fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" x1="20" x2="20" y1="12" y2="3"/>"#,
                    r#"<line fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" x1="2" x2="6" y1="14" y2="14"/>"#,
                    r#"<line fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" x1="10" x2="14" y1="8" y2="8"/>"#,
                    r#"<line fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" x1="18" x2="22" y1="16" y2="16"/>"#,
                ),
            },
            Self::Sun => &rune::Icon {
                name: "sun",
                view: 24.0,
                body: concat!(
                    r#"<circle fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" cx="12" cy="12" r="4"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M12 2v2"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M12 20v2"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="m4.93 4.93 1.41 1.41"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="m17.66 17.66 1.41 1.41"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M2 12h2"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M20 12h2"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="m6.34 17.66-1.41 1.41"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="m19.07 4.93-1.41 1.41"/>"#,
                ),
            },
            Self::Moon => &rune::Icon {
                name: "moon",
                view: 24.0,
                body: r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>"#,
            },
            Self::ChevronLeft => &rune::Icon {
                name: "chevron-left",
                view: 24.0,
                body: r#"<path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="m15 18-6-6 6-6"/>"#,
            },
            Self::ChevronRight => &rune::Icon {
                name: "chevron-right",
                view: 24.0,
                body: r#"<path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="m9 18 6-6-6-6"/>"#,
            },
            Self::ChevronDown => &rune::Icon {
                name: "chevron-down",
                view: 24.0,
                body: r#"<path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="m6 9 6 6 6-6"/>"#,
            },
            Self::Copy => &rune::Icon {
                name: "copy",
                view: 24.0,
                body: concat!(
                    r#"<rect fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" width="14" height="14" x="8" y="8" rx="2"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>"#,
                ),
            },
            Self::Check => &rune::Icon {
                name: "check",
                view: 24.0,
                body: r#"<path fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" d="M20 6 9 17l-5-5"/>"#,
            },
            Self::Trash => &rune::Icon {
                name: "trash",
                view: 24.0,
                body: concat!(
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M3 6h18"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>"#,
                ),
            },
            Self::EditPen => &rune::Icon {
                name: "edit-pen",
                view: 24.0,
                body: concat!(
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M12 20h9"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M16.376 3.622a1 1 0 0 1 3.002 3.002L7.368 18.635a2 2 0 0 1-.855.506l-2.872.838a.5.5 0 0 1-.62-.62l.838-2.872a2 2 0 0 1 .506-.854z"/>"#,
                ),
            },
            Self::Image => &rune::Icon {
                name: "image",
                view: 24.0,
                body: concat!(
                    r#"<rect fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" width="18" height="18" x="3" y="3" rx="2"/>"#,
                    r#"<circle fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" cx="9" cy="9" r="2"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>"#,
                ),
            },
            Self::Clock => &rune::Icon {
                name: "clock",
                view: 24.0,
                body: concat!(
                    r#"<circle fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" cx="12" cy="12" r="10"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M12 6L12 12 16 14"/>"#,
                ),
            },
            Self::Target => &rune::Icon {
                name: "target",
                view: 24.0,
                body: concat!(
                    r#"<circle fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" cx="12" cy="12" r="10"/>"#,
                    r#"<circle fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" cx="12" cy="12" r="6"/>"#,
                    r#"<circle fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" cx="12" cy="12" r="2"/>"#,
                ),
            },
            Self::Zap => &rune::Icon {
                name: "zap",
                view: 24.0,
                body: r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M4 14a1 1 0 0 1-.78-1.63l9.9-10.2a.5.5 0 0 1 .86.46l-1.92 6.02A1 1 0 0 0 13 10h7a1 1 0 0 1 .78 1.63l-9.9 10.2a.5.5 0 0 1-.86-.46l1.92-6.02A1 1 0 0 0 11 14z"/>"#,
            },
            Self::MessageSquare => &rune::Icon {
                name: "message-square",
                view: 24.0,
                body: r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>"#,
            },
            Self::Refresh => &rune::Icon {
                name: "refresh",
                view: 24.0,
                body: concat!(
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M21 3v5h-5"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M8 16H3v5"/>"#,
                ),
            },
            Self::Close => &rune::Icon {
                name: "close",
                view: 24.0,
                body: concat!(
                    r#"<path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="M18 6 6 18"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="m6 6 12 12"/>"#,
                ),
            },
            Self::ArrowDown => &rune::Icon {
                name: "arrow-down",
                view: 24.0,
                body: concat!(
                    r#"<path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="M12 5v14"/>"#,
                    r#"<path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="m19 12-7 7-7-7"/>"#,
                ),
            },
            Self::Shield => &rune::Icon {
                name: "shield",
                view: 24.0,
                body: r#"<path fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/>"#,
            },
        }
    }
}

#[cfg(test)]
mod tests;
