//! Project and chat rows. Navigation and destructive controls have separate hit targets.

use super::ROW_HEIGHT;
use crate::assets::Icon;
use crate::bridge::BridgeRequest;
use crate::state::{ActiveSessionRow, SavedSessionRow, projects::ProjectGroup};
use crate::theme::Palette;
use crate::ui::panel_controls::{control, icon_button, label};
use crate::ui::workspace::PrimeDesktopView;
use rune::{Cursor, El, col, el, row};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub(super) fn project_pin(cwd: &str, pinned: bool, p: Palette) -> El {
    let path = cwd.to_owned();
    let (icon, title) = if pinned {
        (Icon::PinOff, "Unpin project")
    } else {
        (Icon::Pin, "Pin project")
    };
    icon_button(format!("project-pin-{path}"), title, icon, p)
        .size(22.0, 24.0)
        .on_press(move |s: &mut PrimeDesktopView| {
            if pinned {
                let path = path.clone();
                s.confirm(
                    format!("Unpin {path}? Its files and chats will stay where they are."),
                    "Unpin project",
                    move |s| s.unpin_project(&path),
                );
            } else {
                s.pin_project(&path);
            }
        })
}

pub(super) fn project(
    group: &ProjectGroup,
    current: bool,
    searching: bool,
    focused: Option<&str>,
    p: Palette,
) -> El {
    let key = group.key().to_owned();
    let mut heading = row()
        .hover_group()
        .h(ROW_HEIGHT)
        .center_y()
        .gap(2.0)
        .px(2.0);
    if !searching {
        let icon = if group.collapsed {
            Icon::ChevronRight
        } else {
            Icon::ChevronDown
        };
        let hint = if group.collapsed {
            "Expand project"
        } else {
            "Collapse project"
        };
        let key = key.clone();
        heading = heading.child(
            icon_button(format!("project-toggle-{key}"), hint, icon, p)
                .size(18.0, 24.0)
                .on_press(move |s: &mut PrimeDesktopView| s.toggle_project(&key)),
        );
    }
    let folder_icon = if group.is_temporary() {
        Icon::History
    } else if group.collapsed {
        Icon::Folder
    } else {
        Icon::FolderOpen
    };
    let mut title = row()
        .key(format!("project-focus-{key}"))
        .grow(1.0)
        .min_w(0.0)
        .h(26.0)
        .center_y()
        .gap(5.0)
        .radius(5.0)
        .child(folder_icon.render(14.0, p.text_secondary))
        .child(
            label(
                &group.label,
                13.0,
                if current { 600.0 } else { 500.0 },
                p.text_primary,
            )
            .grow(1.0)
            .min_w(0.0)
            .truncate(),
        );
    if let Some(path) = group.folder() {
        let path = path.to_owned();
        title = title
            .role("button")
            .label(&group.label)
            .help(&path)
            .focusable(true)
            .cursor(Cursor::Pointer)
            .hover(p.bg_hover)
            .on_press(move |s: &mut PrimeDesktopView| s.focus_sidebar_project(&path));
    } else if !searching {
        title = title
            .role("button")
            .label(&group.label)
            .focusable(true)
            .cursor(Cursor::Pointer)
            .on_press(move |s: &mut PrimeDesktopView| s.toggle_project(&key));
    }
    heading = heading.child(title);
    if let Some(path) = group.folder() {
        let path = path.to_owned();
        let pin_key = format!("project-pin-{path}");
        let new_key = format!("project-new-chat-{path}");
        heading = heading
            .child(row_action(
                project_pin(&path, group.added, p),
                focused,
                &pin_key,
            ))
            .child(
                row_action(
                    icon_button(
                        new_key.clone(),
                        format!("New chat in {}", group.label),
                        Icon::Compose,
                        p,
                    ),
                    focused,
                    &new_key,
                )
                .size(22.0, 24.0)
                .on_press(move |s: &mut PrimeDesktopView| s.new_project_chat(&path)),
            );
    } else if group.collapsed {
        heading = heading.child(label(
            group.chats.len().to_string(),
            11.0,
            400.0,
            p.text_muted,
        ));
    }
    heading
}

fn row_action(button: El, focused: Option<&str>, key: &str) -> El {
    let rest = if focused == Some(key) { 1.0 } else { 0.0 };
    button.group_opacity(rest, 1.0)
}

pub(super) fn chat(key: String, title: &str, icon: El, selected: bool, p: Palette) -> El {
    let mut item = control(key, title, p)
        .grow(1.0)
        .min_w(0.0)
        .h(ROW_HEIGHT)
        .pl(32.0)
        .center_y()
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .center_y()
                .gap(6.0)
                .child(col().w(14.0).center().child(icon))
                .child(
                    label(
                        title,
                        12.0,
                        if selected { 500.0 } else { 400.0 },
                        if selected {
                            p.text_primary
                        } else {
                            p.text_secondary
                        },
                    )
                    .grow(1.0)
                    .min_w(0.0)
                    .truncate(),
                ),
        );
    if selected {
        item = item.fill(p.text_primary.alpha(0.07));
    }
    item
}

pub(super) fn live_chat(
    agent: &ActiveSessionRow,
    selected: bool,
    focused: Option<&str>,
    p: Palette,
) -> El {
    let sid = agent.active_session_id.clone();
    let id = sid.clone();
    let name = agent.name.clone();
    let stop_key = format!("kill-active-{id}");
    let marker = if agent.is_streaming {
        Icon::Zap.render(12.0, p.warning)
    } else {
        el().size(7.0, 7.0).radius(4.0).fill(p.success)
    };
    let status = if agent.is_streaming {
        "Working"
    } else {
        "Live · Idle"
    };
    row()
        .hover_group()
        .h(ROW_HEIGHT)
        .center_y()
        .child(
            chat(format!("live-session-{sid}"), &name, marker, selected, p)
                .help(format!("{}\n{status} · {}", name, agent.cwd))
                .on_press(move |s: &mut PrimeDesktopView| {
                    s.close_sidebar_overlay();
                    s.open_session(BridgeRequest::AttachSession {
                        active_session_id: sid.clone(),
                    });
                }),
        )
        .child(
            row_action(
                icon_button(&stop_key, "Stop session", Icon::Stop, p),
                focused,
                &stop_key,
            )
            .size(22.0, 24.0)
            .on_press(move |s: &mut PrimeDesktopView| {
                let sid = id.clone();
                s.confirm(
                    format!("Stop “{name}”? Its worker and any running turn will stop."),
                    "Stop session",
                    move |s| {
                        s.bridge.send(BridgeRequest::KillSession {
                            active_session_id: sid,
                        });
                    },
                );
            }),
        )
}

pub(super) fn saved_chat(
    agent: &SavedSessionRow,
    selected: bool,
    focused: Option<&str>,
    p: Palette,
) -> El {
    let path = agent.path.clone();
    let cwd = (!agent.cwd.is_empty()).then(|| agent.cwd.clone());
    let del = path.clone();
    let delete_key = format!("del-saved-{del}");
    let title = agent.name.clone();
    let mut item = row().hover_group().h(ROW_HEIGHT).center_y().child(
        chat(
            format!("saved-chat-{path}"),
            &title,
            Icon::History.render(12.0, p.text_muted),
            selected,
            p,
        )
        .help(format!("{}\nSaved chat · {}", title, agent.cwd))
        .on_press(move |s: &mut PrimeDesktopView| {
            s.close_sidebar_overlay();
            s.open_session(BridgeRequest::OpenSavedSession {
                session_path: path.clone(),
                cwd: cwd.clone(),
            });
        }),
    );
    if let Some(age) = modified_age(&agent.modified, OffsetDateTime::now_utc()) {
        item = item.child(label(age, 10.0, 400.0, p.text_muted));
    }
    item.child(
        row_action(
            icon_button(&delete_key, "Delete chat", Icon::Trash, p),
            focused,
            &delete_key,
        )
        .size(22.0, 24.0)
        .on_press(move |s: &mut PrimeDesktopView| {
            let del = del.clone();
            s.confirm(
                format!("Permanently delete “{title}” from saved chats?"),
                "Delete chat",
                move |s| {
                    s.bridge
                        .send(BridgeRequest::DeleteSavedSession { session_path: del });
                },
            );
        }),
    )
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
