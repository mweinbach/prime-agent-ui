//! Command and model pickers, provider settings, and destructive-action confirmations.

use rune::{Cx, El, Key, ScrollSpec, Transition, col, el, row};

use crate::assets::Icon;
use crate::bridge::BridgeRequest;
use crate::state::{
    ActiveModal, format_tokens_compact, harness_store::ProviderAuthItem, models::CatalogModel,
    truncate_chars,
};
use crate::theme::{DaemonTarget, Palette};
use crate::ui::panel_controls::{button, control, icon_button, label};
use crate::ui::text_input::field;
use crate::ui::workspace::PrimeDesktopView;

/// Keep the exact target captured at confirmation time (selections can change on daemon events).
pub(super) struct Confirmation {
    message: String,
    action: String,
    confirmed: Box<dyn FnOnce(&mut PrimeDesktopView)>,
}

impl PrimeDesktopView {
    pub(super) fn confirm(
        &mut self,
        message: impl Into<String>,
        action: impl Into<String>,
        confirmed: impl FnOnce(&mut Self) + 'static,
    ) {
        self.confirm = Some(Confirmation {
            message: message.into(),
            action: action.into(),
            confirmed: Box::new(confirmed),
        });
    }

    pub(super) fn active_modal(&self, cx: &mut Cx<Self>) -> Option<El> {
        let p = Palette::for_mode(self.prefs.theme);
        let (key, mut card) = if let Some(confirm) = &self.confirm {
            let buttons = row()
                .justify_end()
                .gap(8.0)
                .child(
                    button("cancel-confirm", "Cancel", p).on_press(|s: &mut Self| s.confirm = None),
                )
                .child(
                    button("accept-confirm", &confirm.action, p)
                        .fill(p.danger_soft)
                        .on_press(|s: &mut Self| {
                            if let Some(confirm) = s.confirm.take() {
                                (confirm.confirmed)(s);
                            }
                        }),
                );
            (
                "confirm-card",
                col()
                    .w_full()
                    .max_w(420.0)
                    .gap(16.0)
                    .padding(18.0)
                    .child(label(&confirm.message, 14.0, 500.0, p.text_primary).wrap())
                    .child(buttons)
                    .modal(false, |s: &mut Self| s.confirm = None),
            )
        } else {
            let modal = self.active_modal?;
            let (key, content) = match modal {
                ActiveModal::QuickSwitcher => ("quick-switcher", self.picker(false, p)),
                ActiveModal::ModelPicker => ("model-picker", self.picker(true, p)),
                ActiveModal::SettingsAuth => ("settings", self.settings(p)),
            };
            (
                key,
                content.w_full().max_w(610.0).modal(true, Self::close_modal),
            )
        };
        card = card
            .key(key)
            .role("dialog")
            .radius(16.0)
            .clip_rounded(16.0)
            .border(1.0, p.border_strong)
            .fill(p.bg_elevated)
            .shadow(8.0, 30.0, p.bg_scrim);
        // Fade only the scrim. A sheet with a Rune presence transition ignores
        // pointer presses until its entrance is under way.
        let mut scrim = el().absolute(0.0, 0.0).w_full().h_full().fill(p.bg_scrim);
        if !self.prefs.reduce_motion && !cx.is_reduced_motion() {
            scrim = scrim.transition(Transition::opacity());
        }
        if let Some(toast) = self.notification(p) {
            card = card.child(toast.pin(None, None, Some(12.0), Some(12.0)));
        }
        Some(
            col()
                .key("modal-backdrop-scrim")
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .padding(22.0)
                .center()
                .child(scrim)
                .child(card),
        )
    }

    fn matching_models(&self) -> Vec<&CatalogModel> {
        let query = &self.modal_search_input.text;
        self.catalog_models
            .iter()
            .filter(|m| {
                (!self.model_picker_configured_only || m.configured) && m.matches_query(query)
            })
            .take(90)
            .collect()
    }

    fn picker(&self, models: bool, p: Palette) -> El {
        let mut rows = Vec::new();
        if models {
            let matching = self.matching_models();
            let selected = self
                .modal_selected_idx
                .min(matching.len().saturating_sub(1));
            for (index, model) in matching.into_iter().enumerate() {
                let provider = model.provider.clone();
                let id = model.id.clone();
                let is_current = self.session.model_provider.as_deref() == Some(&provider)
                    && self.session.model_id.as_deref() == Some(&id);
                let mut badges = Vec::new();
                if model.reasoning {
                    badges.push("reasoning".into());
                }
                if model.supports_images {
                    badges.push("vision".into());
                }
                badges.push(format_tokens_compact(model.context_window));
                let title = row()
                    .gap(5.0)
                    .center_y()
                    .child(label(&model.name, 13.0, 500.0, p.text_primary).truncate());
                let title = if is_current {
                    title.child(Icon::Check.render(13.0, p.accent))
                } else {
                    title
                };
                let content = row()
                    .grow(1.0)
                    .min_w(0.0)
                    .center_y()
                    .gap(6.0)
                    .child(
                        col().grow(1.0).min_w(0.0).gap(2.0).child(title).child(
                            label(
                                format!("{} · {id}", model.provider_display()),
                                11.0,
                                400.0,
                                p.text_muted,
                            )
                            .truncate(),
                        ),
                    )
                    .children(badges.into_iter().map(|b| {
                        label(b, 10.0, 400.0, p.text_secondary)
                            .px(5.0)
                            .py(3.0)
                            .radius(5.0)
                            .fill(p.bg_code)
                    }));
                rows.push(
                    result_row(index, &model.name, selected, p)
                        .child(content)
                        .on_press(move |s: &mut Self| s.select_model(&provider, &id)),
                );
            }
        } else {
            let commands = switch_rows(self);
            let selected = self
                .modal_selected_idx
                .min(commands.len().saturating_sub(1));
            for (index, command) in commands.into_iter().enumerate() {
                let title = command.title.clone();
                let content = row()
                    .grow(1.0)
                    .min_w(0.0)
                    .center_y()
                    .gap(7.0)
                    .child(
                        label(title, 13.0, 500.0, p.text_primary)
                            .grow(1.0)
                            .min_w(0.0)
                            .truncate(),
                    )
                    .child(
                        label(command.hint, 11.0, 400.0, p.text_muted)
                            .max_w(170.0)
                            .truncate(),
                    );
                rows.push(
                    result_row(index, &command.title, selected, p)
                        .h(36.0)
                        .child(content.key(command.id))
                        .on_press(move |s: &mut Self| command.target.activate(s)),
                );
            }
        }
        let count = rows.len();
        let placeholder = if models {
            "Search models or providers…"
        } else {
            "Search commands, projects, or chats…"
        };
        let search = field(
            "modal-search",
            &self.modal_search_input,
            placeholder,
            p,
            |s| &mut s.modal_search_input,
        )
        .single_line()
        .min_w(0.0)
        .grow(1.0)
        .on_submit(|s: &mut Self, _| s.submit_modal_selection())
        .on_input_change(|s: &mut Self, tx| {
            s.modal_selected_idx = 0;
            tx.scroll_to_y("picker-results", 0.0);
        })
        .on_input_key(move |s: &mut Self, event, tx| {
            match event.key {
                Key::Up => {
                    s.modal_selected_idx = s
                        .modal_selected_idx
                        .saturating_sub(1)
                        .min(count.saturating_sub(1));
                }
                Key::Down => {
                    s.modal_selected_idx = (s.modal_selected_idx + 1).min(count.saturating_sub(1));
                }
                Key::Escape if event.mods == rune::Mods::default() => return s.dismiss_overlay(),
                _ => return false,
            }
            if count > 0 {
                tx.scroll_into_view(format!("picker-row-{}", s.modal_selected_idx));
            }
            true
        });
        if rows.is_empty() {
            rows.push(
                label(
                    if models {
                        "No matching models. Try another search or change the provider filter."
                    } else {
                        "No matching commands, projects, or chats."
                    },
                    13.0,
                    400.0,
                    p.text_muted,
                )
                .padding(12.0)
                .wrap(),
            );
        }
        let mut search_bar = row()
            .h(34.0)
            .center_y()
            .gap(7.0)
            .child(Icon::Search.render(14.0, p.text_muted))
            .child(search);
        let close =
            icon_button("close-picker", "Close", Icon::Close, p).on_press(Self::close_modal);
        let top = if models {
            let filter = if self.model_picker_configured_only {
                "Configured providers"
            } else {
                "All catalog models"
            };
            let filter = button("toggle-configured-models", filter, p)
                .fill(if self.model_picker_configured_only {
                    p.accent_soft
                } else {
                    p.bg_code
                })
                .on_press_tx(|s: &mut Self, tx| {
                    s.model_picker_configured_only = !s.model_picker_configured_only;
                    s.modal_selected_idx = 0;
                    tx.scroll_to_y("picker-results", 0.0);
                });
            let header = row()
                .h(32.0)
                .gap(6.0)
                .center_y()
                .child(
                    label("Choose a model", 17.0, 600.0, p.text_primary)
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(filter)
                .child(close);
            search_bar = search_bar
                .px(8.0)
                .radius(8.0)
                .border(1.0, p.accent)
                .fill(p.bg_code);
            col().padding(14.0).gap(8.0).child(header).child(search_bar)
        } else {
            col().px(14.0).py(6.0).child(search_bar.child(close))
        };
        let height = if models {
            480.0
        } else {
            (64.0 + 38.0 * rows.len() as f32).min(480.0)
        };
        col()
            .h(height)
            .min_h(0.0)
            .shrink()
            .child(top)
            .child(el().h(1.0).fill(p.border_subtle))
            .child(
                col()
                    .key("picker-results")
                    .min_h(0.0)
                    .grow(1.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(col().padding(8.0).gap(2.0).children(rows)),
            )
    }

    fn settings(&self, p: Palette) -> El {
        let header = row()
            .h(54.0)
            .px(18.0)
            .center_y()
            .gap(8.0)
            .child(Icon::Shield.render(16.0, p.accent))
            .child(label("Settings", 19.0, 600.0, p.text_primary).grow(1.0))
            .child(
                icon_button("close-settings", "Close settings", Icon::Close, p)
                    .on_press(Self::close_modal),
            );
        let reduced = self.prefs.reduce_motion;
        let track = el()
            .key("motion-switch-track")
            .size(32.0, 19.0)
            .radius(10.0)
            .fill(if reduced { p.accent } else { p.border_strong })
            .child(
                el().key("motion-switch-thumb")
                    .size(13.0, 13.0)
                    .radius(7.0)
                    .fill(if reduced {
                        p.text_on_accent
                    } else {
                        p.text_secondary
                    })
                    .absolute(if reduced { 16.0 } else { 3.0 }, 3.0),
            );
        let motion = control("reduce-motion", "Reduce motion", p)
            .role("switch")
            .checked(reduced)
            .h(37.0)
            .px(10.0)
            .center_y()
            .child(label("Reduce motion", 13.0, 500.0, p.text_primary).grow(1.0))
            .child(track)
            .on_press(Self::toggle_reduce_motion);
        let mut locations = row().gap(8.0).child(
            button("choose-cli-folder", "Choose folder…", p).on_press(Self::prompt_choose_cli_dir),
        );
        if self.prefs.cli_data_dir.is_some() {
            locations = locations.child(
                button("reset-cli-folder", "Use default", p)
                    .on_press(|s: &mut Self| s.use_cli_folder(None)),
            );
        }
        let data = settings_group(p)
            .child(label("Prime Agent CLI data", 13.0, 600.0, p.text_primary))
            .child(
                label(
                    "Chats, saved credentials and settings are shared with the CLI in this folder.",
                    12.0,
                    400.0,
                    p.text_secondary,
                )
                .wrap(),
            )
            .child(
                label(
                    self.agent_dir.display().to_string(),
                    11.0,
                    400.0,
                    p.text_muted,
                )
                .wrap_anywhere(),
            )
            .child(locations);
        let mut connection = settings_group(p)
            .child(label(
                self.prefs.daemon_target.label(),
                13.0,
                600.0,
                p.text_primary,
            ))
            .child(label(&self.daemon_socket_display, 11.0, 400.0, p.text_muted).wrap_anywhere());
        if self.prefs.cli_data_dir.is_none() {
            let title = match self.prefs.daemon_target {
                DaemonTarget::DesktopHarness => "Attach to CLI daemon",
                DaemonTarget::SharedCli => "Use desktop daemon",
            };
            connection = connection.child(
                button("settings-switch-daemon-btn", title, p).on_press(Self::switch_daemon_target),
            );
        }
        let content = col()
            .padding(16.0)
            .gap(12.0)
            .child(motion)
            .child(data)
            .child(connection)
            .child(label("Provider credentials", 14.0, 600.0, p.text_primary))
            .children(
                self.provider_auth
                    .iter()
                    .map(|item| self.credential_row(item, p)),
            );
        col()
            .h(600.0)
            .min_h(0.0)
            .shrink()
            .child(header)
            .child(el().h(1.0).fill(p.border_subtle))
            .child(
                col()
                    .key("settings-scroll")
                    .min_h(0.0)
                    .grow(1.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(content),
            )
    }

    fn credential_row(&self, item: &ProviderAuthItem, p: Palette) -> El {
        let selected = item.can_edit_api_key && self.selected_auth_provider == item.provider_id;
        let mut summary = row()
            .key(format!("auth-prov-{}", item.provider_id))
            .center_y()
            .gap(6.0)
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(label(&item.display_name, 13.0, 500.0, p.text_primary))
                    .child(label(&item.hint, 11.0, 400.0, p.text_secondary).wrap()),
            )
            .child(label(
                &item.auth_kind,
                11.0,
                500.0,
                if item.configured {
                    p.success
                } else {
                    p.text_muted
                },
            ));
        if item.can_edit_api_key {
            let pid = item.provider_id.clone();
            summary = summary
                .role("button")
                .label(&item.display_name)
                .focusable(true)
                .cursor(rune::Cursor::Pointer)
                .hover(p.bg_hover)
                .on_press(move |s: &mut Self| {
                    if s.selected_auth_provider != pid {
                        s.api_key_input.replace_text("");
                    }
                    s.selected_auth_provider.clone_from(&pid);
                    s.focus_input(|s| &mut s.api_key_input);
                });
        }
        let mut card = settings_group(p).child(summary);
        if selected {
            let has_key = !self.api_key_input.text.trim().is_empty();
            let input = field("api-key", &self.api_key_input, "API key", p, |s| {
                &mut s.api_key_input
            })
            .single_line()
            .grow(1.0)
            .min_w(0.0)
            .on_submit(|s: &mut Self, _| {
                if !s.api_key_input.text.trim().is_empty() && !s.auth_write_pending {
                    s.save_selected_provider_key();
                }
            });
            let mut editor = row()
                .h(32.0)
                .gap(6.0)
                .px(7.0)
                .center_y()
                .radius(6.0)
                .border(1.0, p.border_subtle)
                .child(input);
            if self.auth_write_pending {
                editor = editor.child(label("Updating…", 12.0, 400.0, p.text_muted));
            } else if has_key {
                editor = editor.child(
                    button("save-provider-key-btn", "Save key", p)
                        .on_press(Self::save_selected_provider_key),
                );
            } else if item.has_stored_api_key {
                editor = editor.child(
                    button("remove-provider-key-btn", "Remove key…", p).on_press(|s: &mut Self| {
                        let provider = s.selected_auth_provider.clone();
                        let dir = s.agent_dir.clone();
                        s.confirm(
                            format!("Remove the saved API key for {provider}?"),
                            "Remove key",
                            move |s| {
                                if s.selected_auth_provider == provider
                                    && s.agent_dir == dir
                                    && s.api_key_input.text.trim().is_empty()
                                    && !s.auth_write_pending
                                {
                                    s.save_selected_provider_key();
                                }
                            },
                        );
                    }),
                );
            }
            card = card.border(1.0, p.accent).child(editor);
        }
        card
    }
}

fn settings_group(p: Palette) -> El {
    col()
        .padding(12.0)
        .gap(7.0)
        .radius(10.0)
        .border(1.0, p.border_subtle)
        .fill(p.bg_canvas)
}

fn result_row(index: usize, title: &str, selected: usize, p: Palette) -> El {
    let item = control(format!("picker-row-{index}"), title, p)
        .h(43.0)
        .px(8.0)
        .center_y();
    if index == selected {
        item.fill(p.bg_selected)
    } else {
        item
    }
}
enum SwitchTarget {
    Command(&'static str),
    Project(String),
    Chat(BridgeRequest),
}

struct SwitchRow {
    id: String,
    title: String,
    hint: String,
    target: SwitchTarget,
}

impl SwitchTarget {
    fn activate(&self, view: &mut PrimeDesktopView) {
        match self {
            Self::Command(id) => view.execute_palette_action(id),
            Self::Project(cwd) => {
                view.close_modal();
                if !view.prefs.sidebar_open {
                    view.toggle_sidebar();
                }
                view.focus_sidebar_project(cwd);
                view.focus_input(|s| &mut s.sidebar_search_input);
            }
            Self::Chat(request) => {
                view.open_session(request.clone());
                view.close_modal();
            }
        }
    }
}

impl PrimeDesktopView {
    pub(super) fn submit_modal_selection(&mut self) {
        match self.active_modal {
            Some(ActiveModal::ModelPicker) => {
                let models = self.matching_models();
                if let Some(model) =
                    models.get(self.modal_selected_idx.min(models.len().saturating_sub(1)))
                {
                    let (provider, id) = (model.provider.clone(), model.id.clone());
                    self.select_model(&provider, &id);
                }
            }
            Some(ActiveModal::QuickSwitcher) => {
                let rows = switch_rows(self);
                if let Some(row) =
                    rows.get(self.modal_selected_idx.min(rows.len().saturating_sub(1)))
                {
                    row.target.activate(self);
                }
            }
            Some(ActiveModal::SettingsAuth) | None => {}
        }
    }
}

fn switch_rows(view: &PrimeDesktopView) -> Vec<SwitchRow> {
    let q = view.modal_search_input.text.trim().to_ascii_lowercase();
    let actions = [
        ("act-new", "New chat in this project", "⌘N", "Ctrl+N"),
        ("act-model", "Switch model", "⌘M", "Ctrl+M"),
        ("act-folder", "Add project…", "⇧⌘O", "Ctrl+Shift+O"),
        (
            "act-harness",
            "Open harness inspector",
            "⇧⌘H",
            "Ctrl+Shift+H",
        ),
        (
            "act-compact",
            "Compact conversation context",
            "⇧⌘K",
            "Ctrl+Shift+K",
        ),
        (
            "act-theme",
            "Switch light or dark appearance",
            "⇧⌘L",
            "Ctrl+Shift+L",
        ),
    ];

    let mut rows = actions
        .into_iter()
        .filter(|(_, label, _, _)| label.to_ascii_lowercase().contains(&q))
        .map(|(id, title, mac, other)| SwitchRow {
            id: id.to_owned(),
            title: title.to_owned(),
            hint: (if cfg!(target_os = "macos") {
                mac
            } else {
                other
            })
            .to_owned(),
            target: SwitchTarget::Command(id),
        })
        .collect::<Vec<_>>();
    let groups = view.project_groups(&q);
    rows.extend(
        groups
            .iter()
            .filter_map(|group| {
                let cwd = group.folder()?;
                (if q.is_empty() {
                    group.added
                } else {
                    cwd.to_lowercase().contains(&q) || group.label.to_lowercase().contains(&q)
                })
                .then(|| SwitchRow {
                    id: format!("switch-project-{cwd}"),
                    title: group.label.clone(),
                    hint: String::new(),
                    target: SwitchTarget::Project(cwd.to_owned()),
                })
            })
            .take(8),
    );
    rows.extend(
        groups
            .iter()
            .flat_map(|group| group.chats.iter().map(move |chat| (chat, &group.label)))
            .take(20)
            .map(|(chat, project)| {
                let (title, request) = match *chat {
                    crate::state::projects::ProjectChat::Live(i) => {
                        let row = &view.active_sessions[i];
                        (
                            &row.name,
                            BridgeRequest::AttachSession {
                                active_session_id: row.active_session_id.clone(),
                            },
                        )
                    }
                    crate::state::projects::ProjectChat::Saved(i) => {
                        let row = &view.saved_sessions[i];
                        (
                            &row.name,
                            BridgeRequest::OpenSavedSession {
                                session_path: row.path.clone(),
                                cwd: (!row.cwd.is_empty()).then(|| row.cwd.clone()),
                            },
                        )
                    }
                };
                SwitchRow {
                    id: format!("switch-chat-{chat:?}"),
                    title: truncate_chars(title, 46),
                    hint: project.clone(),
                    target: SwitchTarget::Chat(request),
                }
            }),
    );
    rows
}
