//! Project and chat navigation. Browsing a folder never changes the open conversation.

mod outline;
mod rows;
#[cfg(test)]
mod tests;

use gpui::{
    AnyElement, AnyView, App, Context, Div, Focusable, FontWeight, SharedString, Stateful, Window,
    div, prelude::*, px,
};

use crate::assets::Icon;
use crate::bridge::BridgeRequest;
use crate::state::projects::folder_label;
use crate::theme::{Palette, ThemeMode, font_sans};
use crate::ui::text_input::TextInput;
use crate::ui::workspace::PrimeDesktopView;

const ROW_HEIGHT: f32 = 30.0;
pub(super) const SIDEBAR_WIDTH: f32 = 264.0;
pub(super) const GLOBAL_SEARCH: &str = "Search projects and chats";

struct ProjectTooltip {
    text: SharedString,
    palette: Palette,
}

impl Render for ProjectTooltip {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let max_width = (window.viewport_size().width - px(16.0)).clamp(px(1.0), px(480.0));
        div()
            .debug_selector(|| "project-tooltip".to_owned())
            .max_w(max_width)
            .px(px(9.0))
            .py(px(6.0))
            .rounded_md()
            .border_1()
            .border_color(self.palette.border_strong)
            .bg(self.palette.bg_elevated)
            .font_family(font_sans())
            .text_size(px(12.0))
            .text_color(self.palette.text_primary)
            .child(self.text.clone())
    }
}

pub(super) fn project_tooltip(
    text: SharedString,
    palette: Palette,
) -> impl Fn(&mut Window, &mut App) -> AnyView {
    move |_, cx| {
        cx.new(|_| ProjectTooltip {
            text: text.clone(),
            palette,
        })
        .into()
    }
}

impl PrimeDesktopView {
    pub(crate) fn focus_sidebar_project(&mut self, cwd: &str, cx: &mut Context<Self>) {
        self.set_sidebar_scope(Some(cwd.to_owned()), cx);
    }

    pub(crate) fn reveal_current_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.current_project().is_empty() {
            return;
        }
        if !self.prefs.sidebar_open {
            self.toggle_sidebar(window, cx);
        }
        let folder = self.current_project().to_owned();
        self.focus_sidebar_project(&folder, cx);
    }

    pub(crate) fn show_all_projects(&mut self, cx: &mut Context<Self>) {
        self.set_sidebar_scope(None, cx);
    }

    fn set_sidebar_scope(&mut self, project: Option<String>, cx: &mut Context<Self>) {
        self.sidebar_search_input.update(cx, |input, cx| {
            input.set_placeholder(if project.is_some() {
                "Search this project"
            } else {
                GLOBAL_SEARCH
            });
            input.clear(cx);
        });
        self.sidebar_project = project;
        cx.notify();
    }
}

fn icon_button(
    id: &'static str,
    icon: Icon,
    label: impl Into<SharedString>,
    palette: Palette,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(|| id.to_owned())
        .size(px(28.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .cursor_pointer()
        .hover(|s| s.bg(palette.bg_hover))
        .active(|s| s.bg(palette.bg_selected))
        .tooltip(project_tooltip(label.into(), palette))
        .child(icon.render(15.0, palette.text_secondary))
}

fn titlebar(scoped: bool, p: Palette, cx: &mut Context<PrimeDesktopView>) -> impl IntoElement {
    div()
        .h(px(48.0))
        .flex_none()
        .pl(px(if cfg!(target_os = "macos") {
            84.0
        } else {
            14.0
        }))
        .pr(px(9.0))
        .flex()
        .items_center()
        .justify_between()
        .child(if scoped {
            div()
                .id("sidebar-all-projects")
                .debug_selector(|| "sidebar-all-projects".into())
                .h(px(28.0))
                .px(px(4.0))
                .flex()
                .items_center()
                .gap(px(3.0))
                .rounded_md()
                .cursor_pointer()
                .hover(|s| s.bg(p.bg_hover))
                .on_click(cx.listener(|view, _, _, cx| view.show_all_projects(cx)))
                .text_size(px(12.0))
                .text_color(p.text_secondary)
                .child(Icon::ChevronLeft.render(13.0, p.text_muted))
                .child("All projects")
                .into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.text_secondary)
                .child(Icon::Prime.render(18.0, p.accent))
                .child("Prime Agent")
                .into_any_element()
        })
        .child(
            icon_button("sidebar-collapse-btn", Icon::Sidebar, "Hide sidebar", p)
                .on_click(cx.listener(|view, _, window, cx| view.toggle_sidebar(window, cx))),
        )
}

fn project_toolbar(
    view: &PrimeDesktopView,
    p: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> impl IntoElement {
    let scope = view.sidebar_project.as_deref();
    let compose_folder = folder_label(scope.unwrap_or_else(|| view.current_project()));
    div()
        .flex_none()
        .px(px(12.0))
        .pb(px(6.0))
        .h(px(38.0))
        .flex()
        .items_center()
        .gap(px(2.0))
        .child(
            div()
                .id("sidebar-scope")
                .min_w_0()
                .flex_1()
                .pl(px(2.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .when_some(scope, |d, cwd| {
                    d.tooltip(project_tooltip(cwd.to_owned().into(), p))
                        .child(Icon::FolderOpen.render(16.0, p.text_secondary))
                })
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(16.0))
                        .child(scope.map_or_else(|| "Projects".to_owned(), folder_label)),
                ),
        )
        .child(if let Some(cwd) = scope {
            rows::project_pin(
                cwd,
                view.prefs
                    .project_folders
                    .iter()
                    .any(|folder| folder == cwd),
                p,
                cx,
            )
            .size(px(28.0))
        } else {
            icon_button("add-project", Icon::FolderPlus, "Add project", p).on_click(
                cx.listener(|view, _, window, cx| view.prompt_choose_project_dir(window, cx)),
            )
        })
        .child(
            icon_button(
                "sidebar-compose",
                Icon::Compose,
                format!("New chat in {compose_folder}"),
                p,
            )
            .on_click(cx.listener(|view, _, window, cx| view.start_fresh_session(window, cx))),
        )
}

fn search_field(
    view: &PrimeDesktopView,
    window: &Window,
    p: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> impl IntoElement {
    let input = &view.sidebar_search_input;
    div()
        .id("sidebar-search")
        .debug_selector(|| "sidebar-search".into())
        .flex_none()
        .mx(px(12.0))
        .mb(px(9.0))
        .h(px(30.0))
        .pl(px(8.0))
        .pr(px(2.0))
        .flex()
        .items_center()
        .gap(px(7.0))
        .rounded_md()
        .border_1()
        .border_color(p.border_subtle)
        .bg(p.bg_canvas)
        .when(input.focus_handle(cx).is_focused(window), |d| {
            d.border_color(p.accent)
        })
        .on_click(cx.listener(|view, _, window, cx| {
            window.focus(&view.sidebar_search_input.focus_handle(cx));
        }))
        .child(Icon::Search.render(14.0, p.text_muted))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .child(input.clone()),
        )
        .when(!input.read(cx).text().is_empty(), |d| {
            d.child(
                icon_button("sidebar-search-clear", Icon::Close, "Clear search", p)
                    .size(px(24.0))
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.sidebar_search_input.update(cx, TextInput::clear);
                        window.focus(&view.sidebar_search_input.focus_handle(cx));
                    })),
            )
        })
}

fn footer(
    view: &PrimeDesktopView,
    p: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> impl IntoElement {
    let daemon = if view.daemon_connected {
        view.prefs.daemon_target.label()
    } else {
        "Not connected"
    };
    let (theme_icon, theme_label) = match p.mode {
        ThemeMode::Dark => (Icon::Sun, "Use light theme"),
        ThemeMode::Light => (Icon::Moon, "Use dark theme"),
    };
    div()
        .h(px(44.0))
        .px(px(10.0))
        .flex_none()
        .border_t_1()
        .border_color(p.border_subtle)
        .flex()
        .items_center()
        .gap(px(2.0))
        .child(
            div()
                .id("sidebar-settings")
                .h(px(28.0))
                .flex_1()
                .min_w_0()
                .px(px(5.0))
                .flex()
                .items_center()
                .gap(px(7.0))
                .rounded_md()
                .cursor_pointer()
                .hover(|s| s.bg(p.bg_hover))
                .tooltip(project_tooltip("Connection settings".into(), p))
                .on_click(cx.listener(|view, _, window, cx| view.open_settings_modal(window, cx)))
                .child(div().flex_none().size(px(6.0)).rounded_full().bg(
                    if view.daemon_connected {
                        p.success
                    } else {
                        p.text_muted
                    },
                ))
                .child(
                    div()
                        .truncate()
                        .text_size(px(11.5))
                        .text_color(p.text_secondary)
                        .child(daemon),
                ),
        )
        .child(
            icon_button("sidebar-refresh-btn", Icon::Refresh, "Refresh", p)
                .on_click(cx.listener(|view, _, _, cx| view.refresh_all(cx))),
        )
        .child(
            icon_button("toggle-theme-btn", theme_icon, theme_label, p)
                .on_click(cx.listener(|view, _, _, cx| view.toggle_theme(cx))),
        )
        .child(
            icon_button("open-settings-auth-btn", Icon::Sliders, "Settings", p)
                .on_click(cx.listener(|view, _, window, cx| view.open_settings_modal(window, cx))),
        )
}

#[must_use]
pub fn render_sidebar(
    view: &PrimeDesktopView,
    window: &Window,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    div()
        .w(px(SIDEBAR_WIDTH))
        .max_w_full()
        .h_full()
        .flex_none()
        .flex()
        .flex_col()
        .bg(palette.bg_sidebar)
        .border_r_1()
        .border_color(palette.border_subtle)
        .child(titlebar(view.sidebar_project.is_some(), palette, cx))
        .child(project_toolbar(view, palette, cx))
        .child(search_field(view, window, palette, cx))
        .child(outline::render_outline(view, palette, cx))
        .when_some(view.saved_next_cursor.clone(), |d, cursor| {
            d.child(
                div()
                    .id("load-older-chats")
                    .debug_selector(|| "load-older-chats".into())
                    .mx(px(8.0))
                    .px(px(8.0))
                    .h(px(30.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .rounded_md()
                    .text_size(px(12.0))
                    .text_color(palette.text_secondary)
                    .cursor_pointer()
                    .hover(|s| s.bg(palette.bg_hover))
                    .on_click(cx.listener(move |view, _, _, _| {
                        view.bridge.send(BridgeRequest::LoadOlderSessions {
                            cursor: cursor.clone(),
                        });
                    }))
                    .child(Icon::History.render(13.0, palette.text_muted))
                    .child("Load older chats"),
            )
        })
        .child(footer(view, palette, cx))
        .into_any_element()
}
