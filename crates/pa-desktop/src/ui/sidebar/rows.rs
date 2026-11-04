use gpui::{
    AnyElement, Context, Div, ElementId, FontWeight, SharedString, Stateful, div, prelude::*, px,
};

use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::{ROW_HEIGHT, project_tooltip};
use crate::assets::Icon;
use crate::bridge::BridgeRequest;
use crate::state::projects::ProjectGroup;
use crate::state::{ActiveSessionRow, SavedSessionRow};
use crate::theme::Palette;
use crate::ui::modals::confirm_action;
use crate::ui::workspace::PrimeDesktopView;

fn row_button(
    id: impl Into<ElementId>,
    tooltip: impl Into<SharedString>,
    palette: Palette,
) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(24.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .cursor_pointer()
        .tooltip(project_tooltip(tooltip.into(), palette))
}

fn project_heading(
    group: &ProjectGroup,
    current: bool,
    searching: bool,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> Stateful<Div> {
    let icon = if group.is_temporary() {
        Icon::History
    } else if group.collapsed {
        Icon::Folder
    } else {
        Icon::FolderOpen
    };
    let label = div()
        .id("project-heading")
        .h_full()
        .min_w_0()
        .flex_1()
        .flex()
        .items_center()
        .pl(px(8.0))
        .gap(px(7.0))
        .child(icon.render(
            15.0,
            if current {
                palette.accent
            } else {
                palette.text_muted
            },
        ))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.text_secondary)
                .child(group.label.clone()),
        );
    if let Some(cwd) = group.folder() {
        let cwd = cwd.to_owned();
        label
            .cursor_pointer()
            .debug_selector(|| format!("project-focus-{cwd}"))
            .tooltip(project_tooltip(
                format!("Show chats in {cwd}").into(),
                palette,
            ))
            .on_click(cx.listener(move |view, _, _, cx| {
                cx.stop_propagation();
                view.focus_sidebar_project(&cwd, cx);
            }))
    } else {
        let toggle = group.key().to_owned();
        let hint = if group.is_temporary() {
            "Saved chats from temporary folders"
        } else {
            "Chats without a working folder"
        };
        label
            .tooltip(project_tooltip(hint.into(), palette))
            .when(!searching, |d| {
                d.cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        cx.stop_propagation();
                        view.toggle_project(&toggle, cx);
                    }))
            })
    }
}

pub(super) fn project_pin(
    cwd: &str,
    pinned: bool,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> Stateful<Div> {
    let cwd = cwd.to_owned();
    let (action, hint, icon) = if pinned {
        ("unpin", "Unpin project", Icon::PinOff)
    } else {
        ("pin", "Pin project", Icon::Pin)
    };
    row_button(SharedString::from(format!("{action}-{cwd}")), hint, palette)
        .debug_selector(|| format!("project-pin-{cwd}"))
        .hover(|s| s.bg(palette.bg_hover))
        .child(icon.render(13.0, palette.text_secondary))
        .on_click(cx.listener(move |view, _, window, cx| {
            cx.stop_propagation();
            if pinned {
                let path = cwd.clone();
                confirm_action(
                    &format!("Unpin {path}? Its files and chats will stay where they are."),
                    "Unpin project",
                    window,
                    cx,
                    move |view, cx| view.unpin_project(&path, cx),
                );
            } else {
                view.pin_project(&cwd, cx);
            }
        }))
}

pub(super) fn render_project(
    group: &ProjectGroup,
    current: bool,
    searching: bool,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let key = group.key();
    let toggle = key.to_owned();
    let (chevron, hint) = if group.collapsed {
        (Icon::ChevronRight, "Expand project")
    } else {
        (Icon::ChevronDown, "Collapse project")
    };
    let folder = group.folder();
    let pin_slot = div()
        .relative()
        .min_w(px(24.0))
        .h(px(24.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .when(group.collapsed, |slot| {
            slot.child(
                div()
                    .px(px(3.0))
                    .text_size(px(10.5))
                    .text_color(palette.text_muted)
                    .when(folder.is_some(), |n| {
                        n.group_hover("project-row", gpui::Styled::invisible)
                    })
                    .child((group.chats.len() + group.draft_cwds.len()).to_string()),
            )
        })
        .when_some(folder, |slot, cwd| {
            slot.child(
                project_pin(cwd, group.added, palette, cx)
                    .absolute()
                    .invisible()
                    .group_hover("project-row", gpui::Styled::visible),
            )
        });
    div()
        .id(SharedString::from(format!("group-{key}")))
        .group("project-row")
        .h(px(ROW_HEIGHT))
        .w_full()
        .pr(px(3.0))
        .flex()
        .items_center()
        .rounded_md()
        .hover(|s| s.bg(palette.bg_hover))
        .child(project_heading(group, current, searching, palette, cx))
        .child(pin_slot)
        .when(!searching, |d| {
            d.child(
                row_button(SharedString::from(format!("toggle-{key}")), hint, palette)
                    .debug_selector(|| format!("project-toggle-{key}"))
                    .hover(|s| s.bg(palette.bg_hover))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        cx.stop_propagation();
                        view.toggle_project(&toggle, cx);
                    }))
                    .child(chevron.render(12.0, palette.text_muted)),
            )
        })
        .when_some(folder, |row, cwd| {
            let cwd = cwd.to_owned();
            row.child(
                row_button(
                    SharedString::from(format!("new-chat-{cwd}")),
                    format!("New chat in {}", group.label),
                    palette,
                )
                .debug_selector(|| format!("project-new-chat-{cwd}"))
                .hover(|s| s.bg(palette.bg_hover))
                .on_click(cx.listener(move |view, _, window, cx| {
                    cx.stop_propagation();
                    view.new_project_chat(&cwd, window, cx);
                }))
                .child(Icon::Compose.render(14.0, palette.text_secondary)),
            )
        })
        .into_any_element()
}

pub(super) fn chat_row(
    id: SharedString,
    marker: impl IntoElement,
    title: SharedString,
    selected: bool,
    palette: Palette,
) -> Stateful<Div> {
    div()
        .id(id)
        .group("chat-row")
        .h(px(ROW_HEIGHT))
        .w_full()
        .pl(px(22.0))
        .pr(px(3.0))
        .rounded_md()
        .cursor_pointer()
        .when(selected, |row| row.bg(palette.bg_selected))
        .when(!selected, |row| row.hover(|s| s.bg(palette.bg_hover)))
        .flex()
        .items_center()
        .gap(px(7.0))
        .child(
            div()
                .w(px(14.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(marker),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .truncate()
                .text_size(px(12.5))
                .when(selected, |d| d.font_weight(FontWeight::MEDIUM))
                .text_color(if selected {
                    palette.text_primary
                } else {
                    palette.text_secondary
                })
                .child(title),
        )
}

fn chat_hint(title: &str, state: &str, cwd: &str) -> SharedString {
    if cwd.is_empty() {
        format!("{title}\n{state}").into()
    } else {
        format!("{title}\n{state} · {cwd}").into()
    }
}

fn chat_action(
    id: SharedString,
    hint: &'static str,
    icon: Icon,
    palette: Palette,
) -> Stateful<Div> {
    row_button(id, hint, palette)
        .invisible()
        .group_hover("chat-row", gpui::Styled::visible)
        .hover(|s| s.bg(palette.danger_soft))
        .child(icon.render(12.0, palette.text_muted))
}

pub(super) fn render_active_agent_row(
    row: &ActiveSessionRow,
    selected: bool,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let sid = row.active_session_id.clone();
    let kill_sid = sid.clone();
    let name = row.name.clone();
    let (marker, state) = if row.is_streaming {
        (
            Icon::Zap.render(12.0, palette.warning).into_any_element(),
            "Working",
        )
    } else {
        (
            div()
                .size(px(7.0))
                .rounded_full()
                .bg(palette.success)
                .into_any_element(),
            "Live · Idle",
        )
    };
    chat_row(
        format!("active-row-{sid}").into(),
        marker,
        row.name.clone().into(),
        selected,
        palette,
    )
    .debug_selector(|| "live-session".to_owned())
    .tooltip(project_tooltip(
        chat_hint(&row.name, state, &row.cwd),
        palette,
    ))
    .on_click(cx.listener(move |view, _, _, cx| {
        view.open_session(
            BridgeRequest::AttachSession {
                active_session_id: sid.clone(),
            },
            cx,
        );
        cx.notify();
    }))
    .child(
        chat_action(
            format!("kill-active-{kill_sid}").into(),
            "Stop session",
            Icon::Stop,
            palette,
        )
        .on_click(cx.listener(move |_, _, window, cx| {
            cx.stop_propagation();
            let sid = kill_sid.clone();
            confirm_action(
                &format!("Stop “{name}”? Its worker and any running turn will stop."),
                "Stop session",
                window,
                cx,
                move |view, _| {
                    view.bridge.send(BridgeRequest::KillSession {
                        active_session_id: sid,
                    });
                },
            );
        })),
    )
    .into_any_element()
}

fn modified_age(modified: &str, now: OffsetDateTime) -> Option<String> {
    let date = OffsetDateTime::parse(modified, &Rfc3339).ok()?;
    let age = now - date;
    Some(if age.whole_minutes() < 1 {
        "now".to_owned()
    } else if age.whole_hours() == 0 {
        format!("{}m", age.whole_minutes())
    } else if age.whole_days() == 0 {
        format!("{}h", age.whole_hours())
    } else if age.whole_days() < 30 {
        format!("{}d", age.whole_days())
    } else if age.whole_days() < 365 {
        format!("{}mo", age.whole_days() / 30)
    } else {
        format!("{}y", age.whole_days() / 365)
    })
}

pub(super) fn render_saved_row(
    row: &SavedSessionRow,
    selected: bool,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let open_path = row.path.clone();
    let open_cwd = (!row.cwd.is_empty()).then(|| row.cwd.clone());
    let del_path = open_path.clone();
    let name = row.name.clone();
    chat_row(
        format!("saved-row-{open_path}").into(),
        Icon::History.render(12.0, palette.text_muted),
        row.name.clone().into(),
        selected,
        palette,
    )
    .debug_selector(|| "saved-chat".to_owned())
    .tooltip(project_tooltip(
        chat_hint(&row.name, "Saved chat", &row.cwd),
        palette,
    ))
    .on_click(cx.listener(move |view, _, _, cx| {
        view.open_session(
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
            .debug_selector(|| format!("sidebar-saved-{}", row.path))
            .relative()
            .min_w(px(24.0))
            .h(px(24.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .when_some(
                modified_age(&row.modified, OffsetDateTime::now_utc()),
                |slot, age| {
                    slot.child(
                        div()
                            .px(px(3.0))
                            .text_size(px(10.5))
                            .text_color(palette.text_muted)
                            .group_hover("chat-row", gpui::Styled::invisible)
                            .child(age),
                    )
                },
            )
            .child(
                chat_action(
                    format!("del-saved-{}", row.id).into(),
                    "Delete chat",
                    Icon::Trash,
                    palette,
                )
                .on_click(cx.listener(move |_, _, window, cx| {
                    cx.stop_propagation();
                    let path = del_path.clone();
                    confirm_action(
                        &format!("Permanently delete “{name}” from saved chats?"),
                        "Delete chat",
                        window,
                        cx,
                        move |view, _| {
                            view.bridge
                                .send(BridgeRequest::DeleteSavedSession { session_path: path });
                        },
                    );
                }))
                .absolute(),
            ),
    )
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_chat_age_handles_offsets_clock_skew_and_missing_dates() {
        let now = OffsetDateTime::from_unix_timestamp(0).unwrap();
        for (stamp, expected) in [
            ("1970-01-01T03:00:00+01:00", Some("now")),
            ("1969-12-31T23:59:59Z", Some("now")),
            ("1970-01-01T01:00:00+02:00", Some("1h")),
            ("", None),
            ("unknown", None),
        ] {
            assert_eq!(modified_age(stamp, now).as_deref(), expected);
        }
    }
}
