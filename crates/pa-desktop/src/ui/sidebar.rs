//! Project and chat navigation. Browsing a folder leaves the current conversation open.

mod outline;
mod rows;
#[cfg(test)]
mod tests;

use rune::{Cx, El, col, el, row};

use crate::assets::Icon;
use crate::bridge::BridgeRequest;
use crate::state::projects::folder_label;
use crate::theme::{Palette, ThemeMode};
use crate::ui::panel_controls::{button, control, icon_button, label};
use crate::ui::text_input::field;
use crate::ui::workspace::PrimeDesktopView;

pub(super) const SIDEBAR_WIDTH: f32 = 264.0;
pub(super) const GLOBAL_SEARCH: &str = "Search projects and chats";
const ROW_HEIGHT: f32 = 30.0;

impl PrimeDesktopView {
    pub(crate) fn focus_sidebar_project(&mut self, cwd: &str) {
        self.sidebar_project = Some(cwd.to_owned());
        self.sidebar_search_input.replace_text("");
    }

    pub(crate) fn reveal_current_project(&mut self) {
        let cwd = self.current_project().to_owned();
        if !cwd.is_empty() {
            if !self.prefs.sidebar_open {
                self.toggle_sidebar();
            }
            self.focus_sidebar_project(&cwd);
        }
    }

    pub(crate) fn show_all_projects(&mut self) {
        self.sidebar_project = None;
        self.sidebar_search_input.replace_text("");
    }

    pub(super) fn sidebar(&self, cx: &mut Cx<Self>) -> El {
        let p = Palette::for_mode(self.prefs.theme);
        let scope = self.sidebar_project.as_deref();
        let top = row()
            .h(48.0)
            .center_y()
            .justify_between()
            .pl(cx.titlebar_inset().max(16.0))
            .pr(12.0)
            .window_drag()
            .child(
                row()
                    .center_y()
                    .gap(7.0)
                    .child(Icon::Prime.render(19.0, p.accent))
                    .child(label("Prime Agent", 14.0, 600.0, p.text_primary)),
            )
            .child(
                icon_button("sidebar-collapse-btn", "Hide sidebar", Icon::Sidebar, p)
                    .on_press(Self::toggle_sidebar),
            );
        let shortcut = if cfg!(target_os = "macos") {
            "⌘N"
        } else {
            "Ctrl+N"
        };
        let compose = control("sidebar-compose", "New chat", p)
            .h(36.0)
            .w_full()
            .radius(9.0)
            .px(11.0)
            .center_y()
            .gap(9.0)
            .fill(p.accent_soft)
            .press(p.bg_selected)
            .child(Icon::Compose.render(15.0, p.accent))
            .child(label("New chat", 13.5, 600.0, p.text_primary).grow(1.0))
            .child(label(shortcut, 10.5, 500.0, p.text_secondary))
            .on_press(Self::start_fresh_session);
        let mut toolbar = row().h(35.0).center_y().pl(15.0).pr(12.0).gap(5.0);
        if scope.is_some() {
            toolbar = toolbar.child(
                icon_button("sidebar-all-projects", "All projects", Icon::ChevronLeft, p)
                    .size(22.0, 24.0)
                    .on_press(Self::show_all_projects),
            );
        }
        toolbar = toolbar
            .child(
                label(
                    scope.map_or_else(|| "Projects".into(), folder_label),
                    12.0,
                    600.0,
                    p.text_secondary,
                )
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
            )
            .child(if let Some(cwd) = scope {
                rows::project_pin(
                    cwd,
                    self.prefs
                        .project_folders
                        .iter()
                        .any(|folder| folder == cwd),
                    p,
                )
            } else {
                icon_button("add-project", "Add project", Icon::FolderPlus, p)
                    .size(23.0, 24.0)
                    .on_press(Self::prompt_choose_project_dir)
            });
        let mut search = row()
            .h(30.0)
            .px(7.0)
            .gap(5.0)
            .center_y()
            .radius(6.0)
            .fill(p.bg_canvas)
            .border(1.0, p.border_subtle)
            .child(Icon::Search.render(14.0, p.text_muted))
            .child(
                field(
                    "sidebar-search",
                    &self.sidebar_search_input,
                    if scope.is_some() {
                        "Search this project"
                    } else {
                        GLOBAL_SEARCH
                    },
                    p,
                    |s| &mut s.sidebar_search_input,
                )
                .single_line()
                .min_w(0.0)
                .grow(1.0),
            );
        if !self.sidebar_search_input.text.is_empty() {
            search = search.child(
                icon_button("sidebar-search-clear", "Clear search", Icon::Close, p)
                    .size(20.0, 20.0)
                    .on_press(|s: &mut Self| {
                        s.sidebar_search_input.replace_text("");
                        s.focus_input(|s| &mut s.sidebar_search_input);
                    }),
            );
        }

        let mut sidebar = col()
            .key("sidebar")
            .w(SIDEBAR_WIDTH)
            .h_full()
            .min_h(0.0)
            .fill(p.bg_sidebar)
            .child(top)
            .child(col().px(12.0).gap(7.0).pb(7.0).child(compose).child(search))
            .child(toolbar)
            .child(outline::render(self, p));
        if let Some(cursor) = &self.saved_next_cursor {
            let cursor = cursor.clone();
            sidebar = sidebar.child(button("load-older-chats", "Load older chats", p).on_press(
                move |s: &mut Self| {
                    s.bridge.send(BridgeRequest::LoadOlderSessions {
                        cursor: cursor.clone(),
                    });
                },
            ));
        }
        let status = if self.daemon_connected {
            self.prefs.daemon_target.label()
        } else {
            "Not connected"
        };
        let (icon, hint) = match p.mode {
            ThemeMode::Dark => (Icon::Sun, "Use light theme"),
            ThemeMode::Light => (Icon::Moon, "Use dark theme"),
        };
        let footer = row()
            .h(44.0)
            .px(10.0)
            .center_y()
            .gap(3.0)
            .child(
                control("open-settings-auth-btn", "Connection settings", p)
                    .h(28.0)
                    .grow(1.0)
                    .min_w(0.0)
                    .px(5.0)
                    .center_y()
                    .gap(7.0)
                    .child(Icon::Sliders.render(14.0, p.text_secondary))
                    .child(label(status, 11.5, 500.0, p.text_secondary).truncate())
                    .child(
                        el().size(5.0, 5.0)
                            .radius(3.0)
                            .fill(if self.daemon_connected {
                                p.success
                            } else {
                                p.text_muted
                            }),
                    )
                    .on_press(Self::open_settings_modal),
            )
            .child(
                icon_button("sidebar-refresh-btn", "Refresh", Icon::Refresh, p)
                    .on_press(Self::refresh_all),
            )
            .child(icon_button("toggle-theme-btn", hint, icon, p).on_press(Self::toggle_theme));
        sidebar
            .child(el().h(1.0).fill(p.border_subtle))
            .child(footer)
    }
}
