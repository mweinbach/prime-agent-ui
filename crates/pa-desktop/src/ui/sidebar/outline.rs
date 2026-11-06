//! Flatten groups once per commit and build only rows inside the Rune scroll viewport.

use super::{ROW_HEIGHT, rows};
use crate::assets::Icon;
use crate::state::projects::{ProjectChat, ProjectGroup};
use crate::theme::Palette;
use crate::ui::panel_controls::{control, label};
use crate::ui::workspace::PrimeDesktopView;
use rune::{El, GrowthSpec, ScrollSpec, col, row};
use std::rc::Rc;

const PREVIEW_CHATS: usize = 5;

#[derive(Clone, Copy)]
enum OutlineRow {
    Folder(usize),
    Chat(ProjectChat),
    Draft { group: usize, index: usize },
    More(usize),
    History(usize),
}

fn is_selected(view: &PrimeDesktopView, chat: ProjectChat) -> bool {
    match chat {
        ProjectChat::Live(i) => view
            .active_sessions
            .get(i)
            .is_some_and(|row| row.active_session_id == view.session.active_session_id),
        ProjectChat::Saved(i) => view.saved_sessions.get(i).is_some_and(|row| {
            (!row.id.is_empty() && row.id == view.session.session_id)
                || Some(row.path.as_str()) == view.session.session_file.as_deref()
        }),
    }
}

fn append_folder(
    rows: &mut Vec<OutlineRow>,
    group: &ProjectGroup,
    i: usize,
    view: &PrimeDesktopView,
    searching: bool,
) {
    let scoped = view.sidebar_project.is_some();
    if !scoped {
        rows.push(OutlineRow::Folder(i));
        if group.collapsed {
            return;
        }
    }
    rows.extend((0..group.draft_cwds.len()).map(|index| OutlineRow::Draft { group: i, index }));
    if scoped || searching || group.folder().is_none() || group.chats.len() <= PREVIEW_CHATS {
        rows.extend(group.chats.iter().copied().map(OutlineRow::Chat));
    } else {
        let (head, tail) = group.chats.split_at(PREVIEW_CHATS);
        rows.extend(head.iter().copied().map(OutlineRow::Chat));
        // Keep the open chat visible even when it is older than the preview.
        if let Some(&selected) = tail.iter().find(|&&chat| is_selected(view, chat)) {
            *rows.last_mut().unwrap() = OutlineRow::Chat(selected);
        }
        rows.push(OutlineRow::More(i));
    }
}

fn outline_rows(
    groups: &[ProjectGroup],
    view: &PrimeDesktopView,
    searching: bool,
) -> Vec<OutlineRow> {
    let mut rows = Vec::new();
    if let Some(scope) = view.sidebar_project.as_deref() {
        if let Some((i, project)) = groups
            .iter()
            .enumerate()
            .find(|(_, g)| g.folder() == Some(scope))
        {
            append_folder(&mut rows, project, i, view, searching);
        }
        return rows;
    }
    let current = view.current_project();
    let mut history_count = 0;
    for (i, group) in groups.iter().enumerate() {
        if group.is_project(current) {
            append_folder(&mut rows, group, i, view, searching);
        } else {
            history_count += 1;
        }
    }
    if history_count > 0 {
        rows.push(OutlineRow::History(history_count));
        if searching || view.sidebar_history_open {
            for (i, group) in groups
                .iter()
                .enumerate()
                .filter(|(_, g)| !g.is_project(current))
            {
                append_folder(&mut rows, group, i, view, searching);
            }
        }
    }
    rows
}

impl OutlineRow {
    fn key(self, groups: &[ProjectGroup], view: &PrimeDesktopView) -> String {
        match self {
            Self::Folder(i) => format!("folder-{}", groups[i].key()),
            Self::Draft { group, index } => format!("draft-{}", groups[group].draft_cwds[index]),
            Self::History(_) => "history-row".into(),
            Self::More(i) => format!("more-{}", groups[i].key()),
            Self::Chat(ProjectChat::Live(i)) => {
                format!("live-{}", view.active_sessions[i].active_session_id)
            }
            Self::Chat(ProjectChat::Saved(i)) => format!("saved-{}", view.saved_sessions[i].path),
        }
    }

    fn build(
        self,
        groups: &[ProjectGroup],
        view: &PrimeDesktopView,
        p: Palette,
        searching: bool,
    ) -> El {
        let focused = view.sidebar_focused_action.as_deref();
        match self {
            Self::Folder(i) => rows::project(
                &groups[i],
                groups[i].folder() == Some(view.current_project()),
                searching,
                focused,
                p,
            ),
            Self::Chat(chat @ ProjectChat::Live(i)) => rows::live_chat(
                &view.active_sessions[i],
                is_selected(view, chat),
                focused,
                p,
            ),
            Self::Chat(chat @ ProjectChat::Saved(i)) => {
                rows::saved_chat(&view.saved_sessions[i], is_selected(view, chat), focused, p)
            }
            Self::Draft { group, index } => {
                let group = &groups[group];
                let cwd = group.draft_cwds[index].clone();
                let selected = view.session.active_session_id.is_empty() && view.working_cwd == cwd;
                let name = "Unsent draft";
                let suffix = group
                    .folder()
                    .and_then(|root| std::path::Path::new(&cwd).strip_prefix(root).ok())
                    .filter(|suffix| !suffix.as_os_str().is_empty());
                let title = suffix.map_or_else(
                    || name.into(),
                    |path| format!("{name} · {}", path.display()),
                );
                rows::chat(
                    format!("sidebar-draft-{cwd}"),
                    &title,
                    Icon::EditPen.render(12.0, p.text_muted),
                    selected,
                    p,
                )
                .help(format!("{name} in {cwd}"))
                .on_press(move |s: &mut PrimeDesktopView| s.new_project_chat(&cwd))
            }
            Self::More(i) => {
                let cwd = groups[i].key().to_owned();
                let title = format!("View all {} chats", groups[i].chats.len());
                control(format!("project-more-{cwd}"), &title, p)
                    .h(ROW_HEIGHT)
                    .pl(42.0)
                    .center_y()
                    .child(label(title, 11.5, 400.0, p.text_secondary))
                    .on_press(move |s: &mut PrimeDesktopView| s.focus_sidebar_project(&cwd))
            }
            Self::History(n) => {
                let mut header = row()
                    .key("sidebar-history")
                    .role("button")
                    .label("Folder history")
                    .h(ROW_HEIGHT)
                    .px(8.0)
                    .center_y()
                    .gap(7.0)
                    .child(Icon::History.render(14.0, p.text_muted))
                    .child(label("Folder history", 12.0, 500.0, p.text_secondary).grow(1.0))
                    .child(label(n.to_string(), 11.0, 400.0, p.text_muted));
                if !searching {
                    let icon = if view.sidebar_history_open {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    };
                    header = header
                        .child(icon.render(12.0, p.text_muted))
                        .focusable(true)
                        .hover(p.bg_hover)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|s: &mut PrimeDesktopView| {
                            s.sidebar_history_open = !s.sidebar_history_open;
                        });
                }
                header
            }
        }
    }
}

pub(super) fn render(view: &PrimeDesktopView, p: Palette) -> El {
    let query = &view.sidebar_search_input.text;
    let searching = !query.trim().is_empty();
    let groups = view.project_groups(query);
    let rows = outline_rows(&groups, view, searching);
    let scope = view.sidebar_project.as_deref();
    let viewport = col()
        .key(format!("outline-{}-{searching}", scope.unwrap_or("")))
        .grow(1.0)
        .min_h(0.0)
        .px(7.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    if rows.is_empty() {
        let message = match (scope.is_some(), searching) {
            (true, false) => "No chats in this project yet.",
            (true, true) => "No matching chats in this project.",
            _ => "No matching projects or chats.",
        };
        return viewport.child(
            label(message, 12.0, 400.0, p.text_muted)
                .key("sidebar-empty")
                .padding(9.0)
                .wrap(),
        );
    }
    let count = rows.len();
    let data = Rc::new((groups, rows));
    let keys = Rc::clone(&data);
    viewport.child(
        col()
            .key("outline-rows")
            .growth(GrowthSpec::default())
            .virtual_rows(
                count,
                ROW_HEIGHT,
                move |v: &PrimeDesktopView, i| keys.1[i].key(&keys.0, v).into(),
                move |v: &PrimeDesktopView, i| {
                    col()
                        .h(ROW_HEIGHT)
                        .child(data.1[i].build(&data.0, v, p, searching))
                },
            ),
    )
}
