//! Command and model pickers, provider settings, and destructive-action confirmations.

use std::fmt::Write as _;
use std::rc::Rc;

use rune::{Cx, El, GrowthSpec, Key, ScrollSpec, Transition, col, el, row};

#[cfg(test)]
mod tests;

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
                ActiveModal::QuickSwitcher => ("quick-switcher", self.command_picker(p)),
                ActiveModal::ModelPicker => ("model-picker", self.model_picker(p)),
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
            .transition(Transition::none())
            .instant_changes()
            .radius(16.0)
            .clip_rounded(16.0)
            .border(1.0, p.border_strong)
            .fill(p.bg_elevated)
            .shadow(8.0, 30.0, p.bg_scrim);
        // Fade only the scrim. A sheet with a Rune presence transition ignores
        // pointer presses until its entrance is under way.
        let mut scrim = el().absolute(0.0, 0.0).w_full().h_full().fill(p.bg_scrim);
        scrim = scrim.transition(if self.prefs.reduce_motion || cx.is_reduced_motion() {
            Transition::none()
        } else {
            Transition::opacity()
        });
        if let Some(toast) = self.notification(p) {
            card = card.child(toast.pin(None, None, Some(12.0), Some(12.0)));
        }
        Some(
            col()
                .key("modal-backdrop-scrim")
                .transition(Transition::none())
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .padding(22.0)
                .center()
                .child(scrim)
                .child(card),
        )
    }

    /// Real catalog indices, in the same order for rendering and keyboard submission.
    fn matching_models(&self) -> Vec<usize> {
        if !self.model_picker_ready() {
            return Vec::new();
        }
        let query = &self.modal_search_input.text;
        let mut choices: Vec<_> = self
            .catalog_models
            .iter()
            .enumerate()
            .filter(|(_, model)| {
                (!self.model_picker_configured_only || model.configured)
                    && model.matches_query(query)
            })
            .map(|(index, _)| index)
            .collect();
        if let Some(current) = self.current_catalog_model() {
            // Start on the actual model when present. Keep its provider together
            // and preserve the daemon catalog order within the other groups.
            choices.sort_by_key(|&i| {
                let model = &self.catalog_models[i];
                if model.provider == current.provider {
                    u8::from(model.id != current.id)
                } else {
                    2
                }
            });
        }
        choices
    }

    fn model_picker(&self, p: Palette) -> El {
        let session = self.session.active_session_id.clone();
        let choices: Rc<Vec<CatalogChoice>> = Rc::new(
            self.matching_models()
                .into_iter()
                .map(|index| {
                    let model = &self.catalog_models[index];
                    CatalogChoice {
                        index,
                        provider: model.provider.clone(),
                        id: model.id.clone(),
                    }
                })
                .collect(),
        );
        let navigation = Rc::new(PickerNavigation {
            keys: choices.iter().map(CatalogChoice::key).collect(),
            model_session: Some(session.clone()),
        });
        let selected = navigation.selected(self);
        let choices_for_submit = Rc::clone(&choices);
        let expected = session.clone();
        let search = self.picker_search(
            "Search models or providers",
            p,
            navigation,
            move |s, index| {
                if let Some(choice) = choices_for_submit.get(index) {
                    s.select_model_for(&expected, &choice.provider, &choice.id);
                }
            },
        );
        let filter = self.model_filter(p);
        let title = row()
            .center_y()
            .gap(8.0)
            .child(label("Choose model", 17.0, 600.0, p.text_primary).grow(1.0))
            .child(filter)
            .child(
                icon_button("close-picker", "Close model picker", Icon::Close, p)
                    .on_press(Self::close_modal),
            );
        let mut items = Vec::new();
        let mut previous = None;
        for (rank, choice) in choices.iter().enumerate() {
            if previous != Some(choice.provider.as_str()) {
                items.push(ModelListRow::Provider(rank));
                previous = Some(choice.provider.as_str());
            }
            items.push(ModelListRow::Choice(rank));
        }

        let height = (100.0
            + items
                .iter()
                .map(|item| match item {
                    ModelListRow::Provider(_) => 28.0,
                    ModelListRow::Choice(_) => 34.0,
                })
                .sum::<f32>())
        .clamp(164.0, 452.0);
        let result = if items.is_empty() {
            let message = if !self.daemon_connected {
                "Connect to the daemon to load models."
            } else if self.model_setting_pending() {
                "Applying selection…"
            } else if !self.model_picker_ready() || !self.model_catalog_loaded() {
                "Loading models…"
            } else if !self.modal_search_input.text.trim().is_empty() {
                "No matching models."
            } else if self.model_picker_configured_only {
                "No models from configured providers."
            } else {
                "The daemon returned no models."
            };
            picker_empty(message, p)
        } else {
            let rows = Rc::new(items);
            let keys = Rc::clone(&rows);
            let choices_for_keys = Rc::clone(&choices);
            let list = col()
                .key("catalog-rows")
                .growth(GrowthSpec::default())
                .virtual_rows(
                    rows.len(),
                    34.0,
                    move |_: &Self, i| {
                        match keys[i] {
                            ModelListRow::Provider(rank) => {
                                format!("provider-{}", choices_for_keys[rank].provider)
                            }
                            ModelListRow::Choice(rank) => choices_for_keys[rank].key(),
                        }
                        .into()
                    },
                    move |s: &Self, i| {
                        let rank = match rows[i] {
                            ModelListRow::Provider(rank) | ModelListRow::Choice(rank) => rank,
                        };
                        // A retained builder can run after a daemon refresh but before this
                        // snapshot gets replaced. Never reinterpret its old catalog indices.
                        let Some(index) = choices[rank].resolve(&s.catalog_models) else {
                            return el();
                        };
                        match rows[i] {
                            ModelListRow::Provider(_) => {
                                provider_heading(&s.catalog_models[index], &s.catalog_models, p)
                            }
                            ModelListRow::Choice(_) => {
                                s.model_option(index, Some(rank) == selected, &session, p)
                            }
                        }
                    },
                );
            picker_viewport()
                .role("listbox")
                .label("Models")
                .child(list)
        };
        col()
            .h(height)
            .min_h(0.0)
            .shrink()
            .label("Choose model")
            .child(col().padding(13.0).gap(8.0).child(title).child(search))
            .child(el().h(1.0).fill(p.border_subtle))
            .child(result)
    }

    fn model_filter(&self, p: Palette) -> El {
        let mut options = row()
            .role("group")
            .label("Model providers")
            .gap(2.0)
            .padding(2.0)
            .radius(7.0)
            .fill(p.bg_code);
        for (configured, title, key) in [
            (false, "All", "show-all-models"),
            (true, "Configured", "toggle-configured-models"),
        ] {
            let chosen = configured == self.model_picker_configured_only;
            let mut option = control(key, format!("{title} providers"), p)
                .role("radio")
                .checked(chosen)
                .h(23.0)
                .px(8.0)
                .radius(5.0)
                .center()
                .child(label(
                    title,
                    11.5,
                    if chosen { 550.0 } else { 400.0 },
                    if chosen {
                        p.text_primary
                    } else {
                        p.text_secondary
                    },
                ));
            if chosen {
                option = option.fill(p.bg_elevated).border(1.0, p.border_subtle);
            }
            options = options.child(option.on_press_tx(move |s: &mut Self, tx| {
                s.model_picker_configured_only = configured;
                s.modal_model_key = None;
                s.modal_selected_idx = 0;
                s.focus_input(|s| &mut s.modal_search_input);
                tx.scroll_to_y("picker-results", 0.0);
            }));
        }
        options
    }

    fn model_option(
        &self,
        index: usize,
        highlighted: bool,
        expected_session: &str,
        p: Palette,
    ) -> El {
        let model = &self.catalog_models[index];
        let current = self.session.model_provider.as_deref() == Some(model.provider.as_str())
            && self.session.model_id.as_deref() == Some(model.id.as_str());
        let mut details = model.selector();
        if model.context_window > 0 {
            let _ = write!(
                details,
                " · {} context",
                format_tokens_compact(model.context_window)
            );
        }
        if model.reasoning {
            details.push_str(" · reasoning");
        }
        if model.supports_images {
            details.push_str(" · images");
        }
        let mut option = result_row(model_key(model), &model.name, highlighted, p)
            .role("option")
            .focus_selected(current)
            .tab_index(-1)
            .help(details)
            .child(
                label(
                    &model.name,
                    13.0,
                    if current { 550.0 } else { 400.0 },
                    p.text_primary,
                )
                .min_w(0.0)
                .grow(1.0)
                .truncate(),
            );
        // The catalog is sorted by name within a provider. Distinguish versions
        // that share a display name, while keeping ordinary rows to a single label.
        let same_name = |other: &&CatalogModel| {
            other.provider == model.provider && other.name == model.name && other.id != model.id
        };
        if self
            .catalog_models
            .get(index.wrapping_sub(1))
            .iter()
            .chain(self.catalog_models.get(index + 1).iter())
            .any(same_name)
        {
            option = option.child(
                label(&model.id, 11.0, 400.0, p.text_secondary)
                    .max_w(210.0)
                    .truncate(),
            );
        }
        if current {
            option = option.description("Current model").child(
                Icon::Check
                    .render(14.0, p.text_primary)
                    .key("current-model-check"),
            );
        }
        let (session, provider, id) = (
            expected_session.to_owned(),
            model.provider.clone(),
            model.id.clone(),
        );
        option.on_press(move |s: &mut Self| {
            s.select_model_for(&session, &provider, &id);
        })
    }

    fn command_picker(&self, p: Palette) -> El {
        let commands = Rc::new(switch_rows(self));
        let count = commands.len();
        let navigation = Rc::new(PickerNavigation {
            keys: commands
                .iter()
                .enumerate()
                .map(|(i, _)| format!("picker-row-{i}"))
                .collect(),
            model_session: None,
        });
        let selected = navigation.selected(self);
        let submitted = Rc::clone(&commands);
        let search = self.picker_search(
            "Search commands, projects, or chats",
            p,
            navigation,
            move |s, index| {
                if let Some(command) = submitted.get(index) {
                    command.target.activate(s);
                }
            },
        );
        let rows = commands.iter().enumerate().map(|(i, command)| {
            let commands = Rc::clone(&commands);
            result_row(
                format!("picker-row-{i}"),
                &command.title,
                Some(i) == selected,
                p,
            )
            .tab_index(-1)
            .child(
                label(&command.title, 13.0, 500.0, p.text_primary)
                    .key(&command.id)
                    .grow(1.0)
                    .min_w(0.0)
                    .truncate(),
            )
            .child(
                label(&command.hint, 11.0, 400.0, p.text_muted)
                    .max_w(170.0)
                    .truncate(),
            )
            .on_press(move |s: &mut Self| commands[i].target.activate(s))
        });
        let results = if count == 0 {
            picker_empty("No matching commands, projects, or chats.", p)
        } else {
            picker_viewport().child(col().children(rows))
        };
        col()
            .h((70.0 + 34.0 * count as f32).clamp(105.0, 448.0))
            .min_h(0.0)
            .shrink()
            .label("Quick switcher")
            .child(
                row()
                    .center_y()
                    .px(12.0)
                    .py(8.0)
                    .gap(6.0)
                    .child(search.grow(1.0))
                    .child(
                        icon_button("close-picker", "Close quick switcher", Icon::Close, p)
                            .on_press(Self::close_modal),
                    ),
            )
            .child(el().h(1.0).fill(p.border_subtle))
            .child(results)
    }

    fn picker_search(
        &self,
        placeholder: &str,
        p: Palette,
        navigation: Rc<PickerNavigation>,
        submit: impl Fn(&mut Self, usize) + 'static,
    ) -> El {
        let submitted = Rc::clone(&navigation);
        let mut input = field(
            "modal-search",
            &self.modal_search_input,
            placeholder,
            p,
            |s| &mut s.modal_search_input,
        )
        .single_line()
        .grow(1.0)
        .min_w(0.0)
        .on_submit(move |s: &mut Self, _| {
            if let Some(index) = submitted.selected(s) {
                submit(s, index);
            }
        })
        .on_input_change(|s: &mut Self, tx| {
            s.modal_model_key = None;
            s.modal_selected_idx = 0;
            tx.scroll_to_y("picker-results", 0.0);
        });
        if let Some(index) = navigation.selected(self) {
            input = input.active_descendant(navigation.keys[index].clone());
        }
        input = input.on_input_key(move |s: &mut Self, event, tx| {
            if event.key == Key::Escape && event.mods == rune::Mods::default() {
                return s.dismiss_overlay();
            }
            let selected = navigation.selected(s);
            let last = navigation.keys.len().saturating_sub(1);
            let index = match event.key {
                Key::Up => selected.map_or(0, |i| i.saturating_sub(1)),
                Key::Down => selected.map_or(0, |i| (i + 1).min(last)),
                _ => return false,
            };
            if let Some(key) = navigation.keys.get(index) {
                s.modal_selected_idx = index;
                if let Some(session) = &navigation.model_session {
                    s.modal_model_key = Some((session.clone(), key.clone()));
                }
                tx.scroll_into_view(key.clone());
            }
            true
        });
        row()
            .h(32.0)
            .center_y()
            .px(8.0)
            .gap(7.0)
            .radius(6.0)
            .fill(p.bg_canvas)
            .border(
                1.0,
                if self.modal_search_input.focused {
                    p.border_strong
                } else {
                    p.border_subtle
                },
            )
            .child(Icon::Search.render(14.0, p.text_muted))
            .child(input.no_focus_ring())
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

struct PickerNavigation {
    keys: Vec<String>,
    model_session: Option<String>,
}

impl PickerNavigation {
    fn selected(&self, view: &PrimeDesktopView) -> Option<usize> {
        if let Some(session) = &self.model_session {
            if session != &view.session.active_session_id {
                return None;
            }
            if let Some((chosen_session, key)) = &view.modal_model_key {
                if chosen_session != session {
                    return None;
                }
                return self.keys.iter().position(|item| item == key);
            }
        }
        (!self.keys.is_empty()).then(|| view.modal_selected_idx.min(self.keys.len() - 1))
    }
}

struct CatalogChoice {
    index: usize,
    provider: String,
    id: String,
}
impl CatalogChoice {
    fn key(&self) -> String {
        format!("model-option-{}/{}", self.provider, self.id)
    }
    fn resolve(&self, catalog: &[CatalogModel]) -> Option<usize> {
        let matches = |m: &CatalogModel| m.provider == self.provider && m.id == self.id;
        if catalog.get(self.index).is_some_and(matches) {
            Some(self.index)
        } else {
            catalog.iter().position(matches)
        }
    }
}
#[derive(Clone, Copy)]
enum ModelListRow {
    Provider(usize),
    Choice(usize),
}

fn model_key(model: &CatalogModel) -> String {
    format!("model-option-{}", model.selector())
}

fn provider_heading(model: &CatalogModel, catalog: &[CatalogModel], p: Palette) -> El {
    let mut header = row().h(28.0).center_y().gap(8.0).px(8.0).child(
        label(model.provider_display(), 11.5, 600.0, p.text_secondary)
            .grow(1.0)
            .min_w(0.0)
            .truncate(),
    );
    // Different Google backends can share a display name and advertise the same model.
    if catalog.iter().any(|other| {
        other.provider != model.provider && other.provider_display() == model.provider_display()
    }) {
        header = header.child(label(&model.provider, 10.5, 400.0, p.text_muted).truncate());
    }
    if !model.configured {
        header = header.child(label("Unconfigured", 10.5, 400.0, p.text_muted));
    }
    header.help(&model.provider)
}

fn picker_viewport() -> El {
    col()
        .key("picker-results")
        .grow(1.0)
        .min_h(0.0)
        .px(7.0)
        .py(5.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
}

fn picker_empty(message: &str, p: Palette) -> El {
    label(message, 13.0, 400.0, p.text_secondary)
        .padding(15.0)
        .wrap()
}

fn result_row(key: String, title: &str, highlighted: bool, p: Palette) -> El {
    let item = control(key, title, p).h(34.0).px(9.0).gap(7.0).center_y();
    if highlighted {
        item.fill(p.bg_hover).border(1.0, p.border_subtle)
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
