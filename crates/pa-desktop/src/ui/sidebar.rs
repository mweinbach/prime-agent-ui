//! Project folders and their conversations. The flat outline keeps large CLI histories virtualized.

use gpui::{
    AnyElement, AnyView, App, Context, FontWeight, SharedString, Window, div, prelude::*, px,
    uniform_list,
};

use crate::assets::Icon;
use crate::bridge::BridgeRequest;
use crate::state::projects::{ProjectChat, ProjectGroup};
use crate::state::{ActiveSessionRow, SavedSessionRow};
use crate::theme::{Palette, ThemeMode, font_sans, font_serif};
use crate::ui::modals::confirm_action;
use crate::ui::workspace::PrimeDesktopView;

enum OutlineRow {
    Header(usize),
    Chat(ProjectChat),
}

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

#[must_use]
#[allow(clippy::too_many_lines)]
pub fn render_sidebar(
    view: &PrimeDesktopView,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let query = view.sidebar_search_input.read(cx).text();
    let groups = view.project_groups(query);
    let mut outline = Vec::new();
    for (i, group) in groups.iter().enumerate() {
        outline.push(OutlineRow::Header(i));
        if !group.collapsed {
            outline.extend(group.chats.iter().copied().map(OutlineRow::Chat));
        }
    }
    let empty = outline.is_empty();

    div()
        .w(px(256.0))
        .h_full()
        .flex_none()
        .flex()
        .flex_col()
        .bg(palette.bg_sidebar)
        .border_r_1()
        .border_color(palette.border_subtle)
        // Top titlebar zone clearing native macOS traffic lights (x: 16..84)
        .child(
            div()
                .h(px(52.0))
                .flex_none()
                .pl(px(if cfg!(target_os = "macos") {
                    84.0
                } else {
                    18.0
                }))
                .pr(px(12.0))
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .font_family(font_serif())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(15.5))
                        .text_color(palette.text_primary)
                        .child("Prime Agent"),
                )
                .child(
                    div()
                        .id("sidebar-collapse-btn")
                        .p(px(5.0))
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|s| s.bg(palette.bg_hover))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toggle_sidebar(cx);
                        }))
                        .child(Icon::Sidebar.render(15.0, palette.text_secondary)),
                ),
        )
        .child(
            div()
                .mx(px(12.0))
                .my(px(8.0))
                .px(px(9.0))
                .py(px(5.0))
                .rounded_lg()
                .border_1()
                .border_color(palette.border_subtle)
                .bg(palette.bg_canvas)
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(Icon::Search.render(13.0, palette.text_muted))
                .child(div().flex_1().child(view.sidebar_search_input.clone())),
        )
        .child(
            div()
                .px(px(16.0))
                .pt(px(10.0))
                .pb(px(6.0))
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(12.0))
                        .text_color(palette.text_secondary)
                        .child("Projects"),
                )
                .child(
                    div()
                        .id("add-project")
                        .debug_selector(|| "add-project".into())
                        .rounded_md()
                        .px(px(6.0))
                        .py(px(4.0))
                        .cursor_pointer()
                        .text_size(px(11.5))
                        .text_color(palette.accent)
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .hover(|s| s.bg(palette.bg_hover))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.prompt_choose_project_dir(window, cx);
                        }))
                        .child(Icon::Plus.render(12.0, palette.accent))
                        .child("Add project"),
                ),
        )
        .child(
            div()
                .px(px(7.0))
                .min_h_0()
                .flex_1()
                .flex()
                .flex_col()
                .overflow_hidden()
                .when(empty, |d| {
                    d.child(
                        div()
                            .px(px(10.0))
                            .py(px(12.0))
                            .text_size(px(12.0))
                            .text_color(palette.text_muted)
                            .child("No matching projects or chats."),
                    )
                })
                .child(
                    uniform_list(
                        "project-outline",
                        outline.len(),
                        cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                            range
                                .filter_map(|i| match outline.get(i)? {
                                    OutlineRow::Header(index) => {
                                        let group = groups.get(*index)?;
                                        Some(render_project(
                                            group,
                                            group.folder() == Some(view.current_project()),
                                            palette,
                                            cx,
                                        ))
                                    }
                                    OutlineRow::Chat(ProjectChat::Live(index)) => {
                                        let row = view.active_sessions.get(*index)?;
                                        Some(render_active_agent_row(
                                            row,
                                            row.active_session_id == view.session.active_session_id,
                                            palette,
                                            cx,
                                        ))
                                    }
                                    OutlineRow::Chat(ProjectChat::Saved(index)) => {
                                        let row = view.saved_sessions.get(*index)?;
                                        Some(render_saved_row(
                                            row,
                                            view.session.session_id == row.id
                                                || view.session.session_file.as_deref()
                                                    == Some(row.path.as_str()),
                                            palette,
                                            cx,
                                        ))
                                    }
                                })
                                .collect()
                        }),
                    )
                    .flex_1(),
                )
                .when_some(view.saved_next_cursor.clone(), |d, cursor| {
                    d.child(
                        div()
                            .id("load-older-chats")
                            .debug_selector(|| "load-older-chats".into())
                            .rounded_md()
                            .px(px(10.0))
                            .py(px(6.0))
                            .text_size(px(12.0))
                            .text_color(palette.text_secondary)
                            .cursor_pointer()
                            .hover(|s| s.bg(palette.bg_hover))
                            .on_click(cx.listener(move |this, _, _, _| {
                                this.bridge.send(BridgeRequest::LoadOlderSessions {
                                    cursor: cursor.clone(),
                                });
                            }))
                            .child("Load older chats"),
                    )
                }),
        )
        .child(
            div()
                .flex_none()
                .px(px(12.0))
                .py(px(12.0))
                .border_t_1()
                .border_color(palette.border_subtle)
                .flex()
                .items_center()
                .justify_between()
                .gap(px(6.0))
                .child(
                    div()
                        .id("sidebar-settings")
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(7.0))
                        .px(px(6.0))
                        .py(px(5.0))
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|s| s.bg(palette.bg_hover))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_settings_modal(window, cx)),
                        )
                        .child(
                            div()
                                .size(px(6.0))
                                .rounded_full()
                                .bg(if view.daemon_connected {
                                    palette.success
                                } else {
                                    palette.text_muted
                                }),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(12.0))
                                .text_color(palette.text_secondary)
                                .child(if view.daemon_connected {
                                    view.prefs.daemon_target.label()
                                } else {
                                    "Not connected"
                                }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(2.0))
                        .child(
                            div()
                                .id("sidebar-refresh-btn")
                                .p(px(6.0))
                                .rounded_md()
                                .cursor_pointer()
                                .hover(|s| s.bg(palette.bg_hover))
                                .on_click(cx.listener(|this, _, _, cx| this.refresh_all(cx)))
                                .child(Icon::Refresh.render(14.0, palette.text_muted)),
                        )
                        .child(
                            div()
                                .id("open-settings-auth-btn")
                                .p(px(6.0))
                                .rounded_md()
                                .cursor_pointer()
                                .hover(|s| s.bg(palette.bg_hover))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_settings_modal(window, cx);
                                }))
                                .child(Icon::Sliders.render(14.0, palette.text_secondary)),
                        )
                        .child(
                            div()
                                .id("toggle-theme-btn")
                                .p(px(6.0))
                                .rounded_md()
                                .cursor_pointer()
                                .hover(|s| s.bg(palette.bg_hover))
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_theme(cx)))
                                .child(
                                    match palette.mode {
                                        ThemeMode::Dark => Icon::Sun,
                                        ThemeMode::Light => Icon::Moon,
                                    }
                                    .render(14.0, palette.text_secondary),
                                ),
                        ),
                ),
        )
        .into_any_element()
}

fn render_project(
    group: &ProjectGroup,
    current: bool,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let key = group.key();
    let toggle = key.to_owned();
    let folder = group.folder();
    let temporary = group.is_temporary();
    let ink = if current {
        palette.text_primary
    } else {
        palette.text_secondary
    };
    let heading = div()
        .id(SharedString::from(format!("toggle-{key}")))
        .debug_selector(|| format!("project-toggle-{key}"))
        .h_full()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(6.0))
        .pl(px(6.0))
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_project(&toggle, cx)))
        .when_some(folder, |d, cwd| {
            d.tooltip(project_tooltip(cwd.to_owned().into(), palette))
        })
        .when(temporary, |d| {
            d.tooltip(project_tooltip(
                "Saved chats from temporary folders.".into(),
                palette,
            ))
        })
        .child(
            if group.collapsed {
                Icon::ChevronRight
            } else {
                Icon::ChevronDown
            }
            .render(10.0, palette.text_muted),
        )
        .child(if temporary { Icon::Clock } else { Icon::Folder }.render(14.0, ink))
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(12.5))
                .text_color(ink)
                .child(group.label.clone()),
        )
        .when(temporary && group.collapsed, |d| {
            d.child(
                div()
                    .ml_auto()
                    .pr(px(8.0))
                    .flex_none()
                    .text_size(px(11.5))
                    .text_color(palette.text_secondary)
                    .child(group.chats.len().to_string()),
            )
        });
    div()
        .id(SharedString::from(format!("group-{key}")))
        .group("project-row")
        .h(px(34.0))
        .w_full()
        .flex()
        .items_center()
        .gap(px(2.0))
        .rounded_md()
        .when(current, |d| d.bg(palette.bg_selected))
        .hover(|s| s.bg(palette.bg_hover))
        .child(heading)
        .when_some(folder, |d, cwd| {
            let cwd = cwd.to_owned();
            d.when(group.added, |d| {
                let unpin = cwd.clone();
                d.child(
                    div()
                        .id(SharedString::from(format!("unpin-{cwd}")))
                        .invisible()
                        .group_hover("project-row", gpui::Styled::visible)
                        .flex_none()
                        .p(px(4.0))
                        .cursor_pointer()
                        .rounded_sm()
                        .hover(|s| s.bg(palette.bg_canvas))
                        .on_click(cx.listener(move |_, _, window, cx| {
                            let path = unpin.clone();
                            confirm_action(
                                &format!("Remove {path} from added projects? Its files and chats will stay where they are."),
                                "Remove project",
                                window,
                                cx,
                                move |view, cx| view.unpin_project(&path, cx),
                            );
                        }))
                        .child(Icon::Close.render(10.0, palette.text_muted)),
                )
            })
            .child(
                div()
                    .id(SharedString::from(format!("new-chat-{cwd}")))
                    .debug_selector(|| format!("project-new-chat-{cwd}"))
                    .flex_none()
                    .mr(px(3.0))
                    .p(px(5.0))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(palette.bg_canvas))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.new_project_chat(&cwd, window, cx);
                    }))
                    .child(Icon::Plus.render(13.0, ink)),
            )
        })
        .into_any_element()
}

fn render_active_agent_row(
    row: &ActiveSessionRow,
    selected: bool,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let sid = row.active_session_id.clone();
    let kill_sid = row.active_session_id.clone();
    let kill_title = format!(
        "Stop “{}”? Its worker and any running turn will stop.",
        row.name
    );
    let dot_color = if row.is_streaming {
        palette.warning
    } else {
        palette.success
    };

    div()
        .id(SharedString::from(format!(
            "active-row-{}",
            row.active_session_id
        )))
        .debug_selector(|| "live-session".to_owned())
        .group("chat-row")
        .when(!row.cwd.is_empty(), |d| {
            d.tooltip(project_tooltip(row.cwd.clone().into(), palette))
        })
        .h(px(34.0))
        .pl(px(28.0))
        .pr(px(7.0))
        .py(px(6.0))
        .rounded_lg()
        .cursor_pointer()
        .bg(if selected {
            palette.bg_selected
        } else {
            gpui::transparent_black()
        })
        .hover(|s| {
            if selected {
                s.bg(palette.bg_selected)
            } else {
                s.bg(palette.bg_hover)
            }
        })
        .flex()
        .items_center()
        .justify_between()
        .gap(px(6.0))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.open_session(
                BridgeRequest::AttachSession {
                    active_session_id: sid.clone(),
                },
                cx,
            );
            cx.notify();
        }))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(7.0))
                .overflow_hidden()
                .child(
                    div()
                        .w(px(7.0))
                        .h(px(7.0))
                        .flex_none()
                        .rounded_full()
                        .bg(dot_color),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.5))
                        .text_color(palette.text_primary)
                        .child(row.name.clone()),
                ),
        )
        .child(
            div()
                .id(SharedString::from(format!(
                    "kill-active-{}",
                    row.active_session_id
                )))
                .invisible()
                .group_hover("chat-row", gpui::Styled::visible)
                .p(px(5.0))
                .rounded_sm()
                .hover(|s| s.bg(palette.danger_soft))
                .on_click(cx.listener(move |_this, _, window, cx| {
                    cx.stop_propagation();
                    let sid = kill_sid.clone();
                    confirm_action(&kill_title, "Stop session", window, cx, move |this, _| {
                        this.bridge.send(BridgeRequest::KillSession {
                            active_session_id: sid,
                        });
                    });
                }))
                .child(Icon::Close.render(11.0, palette.text_muted)),
        )
        .into_any_element()
}

fn render_saved_row(
    row: &SavedSessionRow,
    selected: bool,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let open_path = row.path.clone();
    let open_cwd = (!row.cwd.is_empty()).then(|| row.cwd.clone());
    let del_path = row.path.clone();
    let delete_title = format!("Permanently delete “{}” from saved chats?", row.name);

    div()
        .id(SharedString::from(format!("saved-row-{}", row.path)))
        .debug_selector(|| "saved-chat".to_owned())
        .group("chat-row")
        .when(!row.cwd.is_empty(), |d| {
            d.tooltip(project_tooltip(row.cwd.clone().into(), palette))
        })
        .h(px(34.0))
        .pl(px(41.0))
        .pr(px(7.0))
        .py(px(6.0))
        .rounded_lg()
        .cursor_pointer()
        .bg(if selected {
            palette.bg_selected
        } else {
            gpui::transparent_black()
        })
        .hover(|s| {
            if selected {
                s.bg(palette.bg_selected)
            } else {
                s.bg(palette.bg_hover)
            }
        })
        .flex()
        .items_center()
        .justify_between()
        .gap(px(6.0))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.open_session(
                BridgeRequest::OpenSavedSession {
                    session_path: open_path.clone(),
                    cwd: open_cwd.clone(),
                },
                cx,
            );
            cx.notify();
        }))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(12.5))
                .text_color(palette.text_secondary)
                .child(row.name.clone()),
        )
        .child(
            div()
                .id(SharedString::from(format!("del-saved-{}", row.id)))
                .invisible()
                .group_hover("chat-row", gpui::Styled::visible)
                .p(px(5.0))
                .rounded_sm()
                .hover(|s| s.bg(palette.danger_soft))
                .on_click(cx.listener(move |_this, _, window, cx| {
                    cx.stop_propagation();
                    let path = del_path.clone();
                    confirm_action(&delete_title, "Delete chat", window, cx, move |this, _| {
                        this.bridge
                            .send(BridgeRequest::DeleteSavedSession { session_path: path });
                    });
                }))
                .child(Icon::Trash.render(11.5, palette.text_muted)),
        )
        .into_any_element()
}
