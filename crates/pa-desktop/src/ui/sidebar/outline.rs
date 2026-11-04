//! Flatten the project outline once, then build elements only for visible rows.

use gpui::{AnyElement, Context, FontWeight, SharedString, div, prelude::*, px, uniform_list};

use super::rows::{chat_row, render_active_agent_row, render_project, render_saved_row};
use super::{ROW_HEIGHT, project_tooltip};
use crate::assets::Icon;
use crate::state::projects::{ProjectChat, ProjectGroup};
use crate::theme::Palette;
use crate::ui::workspace::PrimeDesktopView;

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

fn history_row(
    count: usize,
    open: bool,
    searching: bool,
    p: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    div()
        .id("sidebar-history")
        .debug_selector(|| "sidebar-history".into())
        .h(px(ROW_HEIGHT))
        .w_full()
        .px(px(8.0))
        .flex()
        .items_center()
        .gap(px(7.0))
        .border_t_1()
        .border_color(p.border_subtle)
        .when(!searching, |d| {
            d.cursor_pointer()
                .hover(|s| s.bg(p.bg_hover))
                .on_click(cx.listener(|view, _, _, cx| {
                    view.sidebar_history_open = !view.sidebar_history_open;
                    cx.notify();
                }))
        })
        .child(Icon::History.render(14.0, p.text_muted))
        .child(
            div()
                .flex_1()
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.text_secondary)
                .child("Folder history"),
        )
        .child(
            div()
                .text_size(px(11.0))
                .text_color(p.text_muted)
                .child(count.to_string()),
        )
        .when(!searching, |d| {
            d.child(
                if open {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                }
                .render(12.0, p.text_muted),
            )
        })
        .into_any_element()
}

fn draft_row(
    cwd: &str,
    project: &ProjectGroup,
    view: &PrimeDesktopView,
    p: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let selected = view.session.active_session_id.is_empty() && view.working_cwd == cwd;
    let blank =
        selected && view.composer_input.read(cx).text().is_empty() && view.staged_images.is_empty();
    let name = if blank { "New chat" } else { "Unsent draft" };
    let suffix = project
        .folder()
        .and_then(|root| std::path::Path::new(cwd).strip_prefix(root).ok())
        .filter(|s| !s.as_os_str().is_empty());
    let label = suffix.map_or_else(|| name.into(), |p| format!("{name} · {}", p.display()));
    let cwd = cwd.to_owned();
    chat_row(
        format!("draft-{cwd}").into(),
        Icon::EditPen.render(12.0, p.text_muted),
        label.into(),
        selected,
        p,
    )
    .debug_selector(|| format!("sidebar-draft-{cwd}"))
    .tooltip(project_tooltip(format!("{name} in {cwd}").into(), p))
    .on_click(cx.listener(move |view, _, window, cx| view.new_project_chat(&cwd, window, cx)))
    .into_any_element()
}

fn more_row(group: &ProjectGroup, p: Palette, cx: &mut Context<PrimeDesktopView>) -> AnyElement {
    let cwd = group.key().to_owned();
    div()
        .id(SharedString::from(format!("more-{cwd}")))
        .debug_selector(|| format!("project-more-{cwd}"))
        .h(px(ROW_HEIGHT))
        .w_full()
        .pl(px(43.0))
        .flex()
        .items_center()
        .gap(px(4.0))
        .rounded_md()
        .cursor_pointer()
        .hover(|s| s.bg(p.bg_hover))
        .text_size(px(11.5))
        .text_color(p.text_secondary)
        .on_click(cx.listener(move |view, _, _, cx| view.focus_sidebar_project(&cwd, cx)))
        .child(format!("View all {} chats", group.chats.len()))
        .child(Icon::ChevronRight.render(11.0, p.text_muted))
        .into_any_element()
}

pub(super) fn render_outline(
    view: &PrimeDesktopView,
    p: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let query = view.sidebar_search_input.read(cx).text();
    let searching = !query.trim().is_empty();
    let groups = view.project_groups(query);
    let rows = outline_rows(&groups, view, searching);
    let empty = rows.is_empty();
    let scoped = view.sidebar_project.is_some();
    // Each project has a fresh viewport when entered from deep in the overview.
    let list_id = SharedString::from(format!(
        "outline-{}-{searching}",
        view.sidebar_project.as_deref().unwrap_or("")
    ));
    div()
        .px(px(7.0))
        .pb(px(6.0))
        .min_h_0()
        .flex_1()
        .flex()
        .flex_col()
        .overflow_hidden()
        .when(empty, |d| {
            d.child(
                div()
                    .debug_selector(|| "sidebar-empty".into())
                    .px(px(9.0))
                    .py(px(12.0))
                    .text_size(px(12.0))
                    .text_color(p.text_muted)
                    .child(match (scoped, searching) {
                        (true, false) => "No chats in this project yet.",
                        (true, true) => "No matching chats in this project.",
                        _ => "No matching projects or chats.",
                    }),
            )
        })
        .child(
            uniform_list(
                list_id,
                rows.len(),
                cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                    range
                        .filter_map(|i| {
                            Some(match *rows.get(i)? {
                                OutlineRow::Folder(i) => render_project(
                                    &groups[i],
                                    groups[i].folder() == Some(view.current_project()),
                                    searching,
                                    p,
                                    cx,
                                ),
                                OutlineRow::Draft { group, index } => draft_row(
                                    &groups[group].draft_cwds[index],
                                    &groups[group],
                                    view,
                                    p,
                                    cx,
                                ),
                                OutlineRow::More(i) => more_row(&groups[i], p, cx),
                                OutlineRow::History(count) => {
                                    history_row(count, view.sidebar_history_open, searching, p, cx)
                                }
                                OutlineRow::Chat(chat @ ProjectChat::Live(index)) => {
                                    render_active_agent_row(
                                        view.active_sessions.get(index)?,
                                        is_selected(view, chat),
                                        p,
                                        cx,
                                    )
                                }
                                OutlineRow::Chat(chat @ ProjectChat::Saved(index)) => {
                                    render_saved_row(
                                        view.saved_sessions.get(index)?,
                                        is_selected(view, chat),
                                        p,
                                        cx,
                                    )
                                }
                            })
                        })
                        .collect()
                }),
            )
            .flex_1(),
        )
        .into_any_element()
}
