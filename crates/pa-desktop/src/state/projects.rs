//! Sidebar projects are a view over session working directories, not a second session store.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::{ActiveSessionRow, SavedSessionRow, compact_path};
use crate::theme::DesktopPreferences;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProjectChat {
    Live(usize),
    Saved(usize),
}

pub(crate) struct ProjectGroup {
    folder: Option<String>,
    pub label: String,
    pub added: bool,
    pub collapsed: bool,
    pub chats: Vec<ProjectChat>,
}

impl ProjectGroup {
    pub(crate) fn key(&self) -> &str {
        // Session cwds are filesystem paths; NUL cannot occur in a real folder name.
        self.folder.as_deref().unwrap_or("\0temporary")
    }

    pub(crate) fn folder(&self) -> Option<&str> {
        self.folder.as_deref().filter(|cwd| !cwd.is_empty())
    }

    pub(crate) fn is_temporary(&self) -> bool {
        self.folder.is_none()
    }
}

pub(crate) fn folder_label(cwd: &str) -> String {
    if cwd.is_empty() {
        return "Unknown folder".into();
    }
    let path = Path::new(cwd);
    if pa_types::platform::home_dir().as_deref() == Some(path) {
        return "Home folder".into();
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(cwd)
        .to_owned()
}

// Match whole components. In particular `/var/folders/.../C` is a cache, not a temp directory.
fn is_temp_folder(path: &Path, temp_dir: &Path) -> bool {
    if temp_dir.is_absolute() && path.starts_with(temp_dir)
        || path.starts_with("/tmp")
        || path.starts_with("/private/tmp")
    {
        return true;
    }
    let path = path
        .strip_prefix("/var/folders")
        .or_else(|_| path.strip_prefix("/private/var/folders"));
    path.is_ok_and(|rest| {
        rest.components()
            .nth(2)
            .is_some_and(|part| part.as_os_str() == "T")
    })
}

// Prefer the immediate parent, adding ancestors only until folders with the same leaf differ.
fn qualify_duplicate_labels(groups: &mut [ProjectGroup]) {
    let mut locations = HashMap::<String, Vec<usize>>::new();
    for (i, group) in groups
        .iter()
        .enumerate()
        .filter(|(_, g)| g.folder().is_some())
    {
        locations.entry(group.label.clone()).or_default().push(i);
    }
    for indices in locations.values().filter(|indices| indices.len() > 1) {
        let parents: Vec<_> = indices
            .iter()
            .map(|i| {
                Path::new(groups[*i].folder().unwrap())
                    .parent()
                    .unwrap_or(Path::new(""))
                    .to_owned()
            })
            .collect();
        for (&i, parent) in indices.iter().zip(&parents) {
            // Strip progressively shorter prefixes (work/x/src -> x then work/x).
            let suffix = parent
                .ancestors()
                .skip(1)
                .filter_map(|ancestor| parent.strip_prefix(ancestor).ok())
                .find(|suffix| parents.iter().filter(|p| p.ends_with(suffix)).count() == 1)
                .unwrap_or(parent);
            let label = &mut groups[i].label;
            label.push_str(" · ");
            label.push_str(&compact_path(&suffix.to_string_lossy()));
        }
    }
}

impl DesktopPreferences {
    /// An added nested folder wins over its parent. Do not resolve symlinks or touch disk on paint.
    pub(crate) fn project_for<'a>(&'a self, cwd: &'a str) -> &'a str {
        let path = Path::new(cwd);
        self.project_folders
            .iter()
            .filter(|root| !root.is_empty() && path.starts_with(root))
            .max_by_key(|root| root.len())
            .map_or(cwd, String::as_str)
    }

    pub(crate) fn groups(
        &self,
        current: &str,
        live: &[ActiveSessionRow],
        saved: &[SavedSessionRow],
        query: &str,
    ) -> Vec<ProjectGroup> {
        let query = query.trim().to_lowercase();
        let searching = !query.is_empty();
        let mut groups = Vec::<ProjectGroup>::new();
        let mut positions = HashMap::<Option<String>, usize>::new();
        let temp_dir = std::env::temp_dir();
        let live_roots: HashSet<_> = live
            .iter()
            .filter(|row| row.rlm_depth == 0)
            .map(|row| self.project_for(&row.cwd))
            .collect();
        let mut insert = |cwd: &str, chat: Option<ProjectChat>| {
            let root = self.project_for(cwd);
            let added = self.project_folders.iter().any(|p| p == root);
            let temporary = matches!(chat, Some(ProjectChat::Saved(_)))
                && !added
                && root != current
                && !live_roots.contains(root)
                && is_temp_folder(Path::new(root), &temp_dir);
            let folder = (!temporary).then(|| root.to_owned());
            let ix = *positions.entry(folder.clone()).or_insert_with(|| {
                let mut group = ProjectGroup {
                    folder,
                    label: if temporary {
                        "Temporary chats".into()
                    } else {
                        folder_label(root)
                    },
                    added,
                    collapsed: false,
                    chats: Vec::new(),
                };
                group.collapsed = !searching
                    && self
                        .project_collapsed
                        .get(group.key())
                        .copied()
                        .unwrap_or(!added && root != current);
                groups.push(group);
                groups.len() - 1
            });
            if let Some(chat) = chat {
                groups[ix].chats.push(chat);
            }
        };
        for root in &self.project_folders {
            insert(root, None);
        }
        insert(current, None);
        let mut ids = HashMap::new();
        let mut files = HashMap::new();
        let mut visible_live = HashSet::new();
        for (index, row) in live
            .iter()
            .enumerate()
            .filter(|(_, row)| row.rlm_depth == 0)
        {
            if let Some(id) = &row.session_id {
                ids.insert(id.as_str(), index);
            }
            if let Some(file) = &row.session_file {
                files.insert(file.as_str(), index);
            }
            if !searching
                || [&row.cwd, &row.name]
                    .iter()
                    .any(|value| value.to_lowercase().contains(&query))
            {
                insert(&row.cwd, Some(ProjectChat::Live(index)));
                visible_live.insert(index);
            }
        }
        for (index, row) in saved
            .iter()
            .enumerate()
            .filter(|(_, row)| row.rlm_depth == 0)
        {
            if searching
                && ![&row.cwd, &row.name, &row.first_message]
                    .iter()
                    .any(|value| value.to_lowercase().contains(&query))
            {
                continue;
            }
            // Even a search that only matched persisted metadata must open the owning worker.
            if let Some(&i) = files
                .get(row.path.as_str())
                .or_else(|| ids.get(row.id.as_str()))
            {
                if visible_live.insert(i) {
                    insert(&live[i].cwd, Some(ProjectChat::Live(i)));
                }
            } else {
                insert(&row.cwd, Some(ProjectChat::Saved(index)));
            }
        }
        qualify_duplicate_labels(&mut groups);
        if searching {
            groups.retain(|g| {
                !g.chats.is_empty()
                    || g.label.to_lowercase().contains(&query)
                    || g.folder()
                        .is_some_and(|cwd| cwd.to_lowercase().contains(&query))
            });
        }
        if let Some(ix) = groups.iter().position(ProjectGroup::is_temporary) {
            let history = groups.remove(ix);
            groups.push(history);
        }
        groups
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn temporary_history_is_one_searchable_group_without_a_project_folder() {
        let saved = |i, cwd: &str| {
            SavedSessionRow::from_wire(&json!({"path":format!("/history/{i}.jsonl"), "id":format!("s{i}"), "cwd":cwd, "name":format!("Chat {i}")})).unwrap()
        };
        let keep = "/private/tmp/added";
        let prefs = DesktopPreferences {
            project_folders: vec![keep.into()],
            ..Default::default()
        };
        let rows = [
            saved(0, "/var/folders/y0/uid/T/.tmpOne/project"),
            saved(1, "/private/var/folders/y0/uid/T/.tmpTwo/project"),
            saved(2, keep),
            saved(3, "/var/folders/y0/uid/C/normal"),
            saved(4, "/work/live"),
        ];
        let groups = prefs.groups("/work/live", &[], &rows, "");
        let history = groups.last().unwrap();
        assert!(history.is_temporary() && history.collapsed && history.folder().is_none());
        assert_eq!(
            history.chats,
            [ProjectChat::Saved(0), ProjectChat::Saved(1)]
        );
        assert_eq!(
            groups
                .iter()
                .filter_map(ProjectGroup::folder)
                .collect::<Vec<_>>(),
            [keep, "/work/live", "/var/folders/y0/uid/C/normal"]
        );
        let groups = prefs.groups("/work/live", &[], &rows, ".tmpTwo");
        let [found] = groups.as_slice() else {
            panic!("expected one matching group")
        };
        assert!(found.is_temporary() && !found.collapsed);
        assert_eq!(found.chats, [ProjectChat::Saved(1)]);
    }

    #[test]
    fn duplicate_folder_labels_use_the_shortest_distinct_parent_suffix() {
        let paths = [
            "/work/alpha/backend/src",
            "/work/beta/backend/src",
            "/work/web/src",
        ];
        let prefs = DesktopPreferences {
            project_folders: paths.map(str::to_owned).into(),
            ..Default::default()
        };
        let groups = prefs.groups(paths[0], &[], &[], "");
        assert_eq!(
            groups.iter().map(|g| g.label.as_str()).collect::<Vec<_>>(),
            ["src · alpha/backend", "src · beta/backend", "src · web"]
        );
    }

    #[test]
    fn historical_matches_under_nested_roots_link_to_the_live_chat_once() {
        let prefs = DesktopPreferences {
            project_folders: vec!["/work".into(), "/work/api".into()],
            ..Default::default()
        };
        let live = ActiveSessionRow::from_wire(&json!({"id":"worker", "sessionId":"chat", "name":"Current title", "cwd":"/work/api/src"})).unwrap();
        let archived = SavedSessionRow::from_wire(&json!({"path":"/history/chat.jsonl", "id":"chat", "cwd":"/work/api/src", "firstMessage":"Earlier question"})).unwrap();
        let live = [live];
        let saved = [archived];
        for query in ["", "earlier"] {
            let groups = prefs.groups("/work", &live, &saved, query);
            let rows: Vec<_> = groups
                .iter()
                .filter(|g| !g.chats.is_empty())
                .map(|g| (g.folder().unwrap(), g.chats.as_slice()))
                .collect();
            assert_eq!(rows, [("/work/api", [ProjectChat::Live(0)].as_slice())]);
        }
        assert_eq!(prefs.project_for("/work/api-next"), "/work");
    }
}
