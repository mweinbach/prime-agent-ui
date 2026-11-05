//! Overlay modals: Searchable Model Picker (`⌘M`), Quick Switcher / Command Palette (`⌘K`),
//! and Provider Credentials & Desktop Settings (`⌘,`).

mod motion;

use gpui::{
    AnyElement, App, Context, Focusable, FontWeight, SharedString, Window, div, prelude::*, px,
};

use crate::assets::Icon;
use crate::bridge::BridgeRequest;
use crate::state::harness_store::ProviderAuthItem;
use crate::state::{ActiveModal, format_tokens_compact, truncate_chars};
use crate::theme::{Palette, font_mono, font_sans, font_serif};
use crate::ui::text_input::{Down, Up};
use crate::ui::tool_cards::pill;
use crate::ui::workspace::PrimeDesktopView;

pub(super) fn confirm_action(
    message: &str,
    action: &'static str,
    window: &mut gpui::Window,
    cx: &mut Context<PrimeDesktopView>,
    confirmed: impl FnOnce(&mut PrimeDesktopView, &mut Context<PrimeDesktopView>) + 'static,
) {
    let answer = window.prompt(
        gpui::PromptLevel::Warning,
        message,
        None,
        &[
            gpui::PromptButton::cancel("Cancel"),
            gpui::PromptButton::new(action),
        ],
        cx,
    );
    cx.spawn(async move |this, cx| {
        if matches!(answer.await, Ok(1)) {
            let _ = this.update(cx, confirmed);
        }
    })
    .detach();
}

#[must_use]
pub fn render_active_modal(
    modal: ActiveModal,
    view: &PrimeDesktopView,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let (entrance_id, contents) = match modal {
        ActiveModal::ModelPicker => (
            "model-picker-entrance",
            render_model_picker_modal(view, palette, cx),
        ),
        ActiveModal::QuickSwitcher => (
            "quick-switcher-entrance",
            render_quick_switcher_modal(view, palette, cx),
        ),
        ActiveModal::SettingsAuth => (
            "settings-entrance",
            render_settings_auth_modal(view, palette, cx),
        ),
    };
    let backdrop = div()
        .id("modal-backdrop-scrim")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .p(px(24.0))
        .on_click(cx.listener(|this, _, window, cx| {
            this.close_modal(window, cx);
        }));
    let sheet = div()
        .id("modal-card-surface")
        .min_w_0()
        .w_full()
        .max_w(px(610.0))
        .max_h_full()
        .flex()
        .flex_col()
        .on_click(|_, _, cx| {
            cx.stop_propagation();
        })
        .child(contents);
    motion::ModalEntrance {
        entrance_id,
        backdrop,
        sheet,
        scrim: palette.bg_scrim,
        reduce_motion: view.prefs.reduce_motion,
    }
    .into_any_element()
}

#[allow(clippy::too_many_lines)]
fn render_model_picker_modal(
    view: &PrimeDesktopView,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let filtered = matching_models(view, cx);
    let count = filtered.len();

    picker_surface(count, cx)
        .w_full()
        .max_h(px(520.0))
        .rounded_2xl()
        .border_1()
        .border_color(palette.border_strong)
        .bg(palette.bg_elevated)
        .shadow_lg()
        .overflow_hidden()
        .flex()
        .flex_col()
        // Header + Search
        .child(
            div()
                .p(px(14.0))
                .border_b_1()
                .border_color(palette.border_subtle)
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .font_family(font_serif())
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(px(17.0))
                                .text_color(palette.text_primary)
                                .child("Choose a model"),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .child(
                                    div()
                                        .id("toggle-configured-models")
                                        .px(px(8.0))
                                        .py(px(3.5))
                                        .rounded_md()
                                        .cursor_pointer()
                                        .bg(if view.model_picker_configured_only {
                                            palette.accent_soft
                                        } else {
                                            palette.bg_code
                                        })
                                        .font_family(font_mono())
                                        .text_size(px(11.0))
                                        .text_color(if view.model_picker_configured_only {
                                            palette.accent
                                        } else {
                                            palette.text_secondary
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.model_picker_configured_only =
                                                !this.model_picker_configured_only;
                                            this.modal_selected_idx = 0;
                                            this.modal_scroll.scroll_to_item(0);
                                            cx.notify();
                                        }))
                                        .child(if view.model_picker_configured_only {
                                            "Configured providers"
                                        } else {
                                            "All catalog models"
                                        }),
                                )
                                .child(
                                    div()
                                        .id("close-model-modal")
                                        .p(px(4.0))
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.close_modal(window, cx);
                                        }))
                                        .child(Icon::Close.render(14.0, palette.text_muted)),
                                ),
                        ),
                )
                .child(
                    div()
                        .px(px(10.0))
                        .py(px(6.5))
                        .rounded_lg()
                        .border_1()
                        .border_color(palette.accent)
                        .bg(palette.bg_code)
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(Icon::Search.render(14.0, palette.accent))
                        .child(div().flex_1().min_w_0().child(view.modal_search_input.clone())),
                ),
        )
        // Model Rows
        .child(
            div()
                .id("model-picker-scroll")
                .track_scroll(&view.modal_scroll)
                .flex_1()
                .overflow_y_scroll()
                .p(px(8.0))
                .flex()
                .flex_col()
                .gap(px(3.0))
                .when(filtered.is_empty(), |d| {
                    d.child(
                        div()
                            .p(px(20.0))
                            .text_size(px(13.0))
                            .text_color(palette.text_muted)
                            .child("No matching models. Try another search or change the provider filter."),
                    )
                })
                .children(filtered.into_iter().enumerate().map(|(index, m)| {
                    let is_current = view.session.model_provider.as_deref()
                        == Some(m.provider.as_str())
                        && view.session.model_id.as_deref() == Some(m.id.as_str());
                    let prov = m.provider.clone();
                    let mid = m.id.clone();

                    div()
                        .id(SharedString::from(format!("pick-m-{}/{}", m.provider, m.id))).flex_none().min_w_0()
                        .px(px(12.0))
                        .py(px(8.5))
                        .rounded_xl()
                        .cursor_pointer()
                        .bg(if index == view.modal_selected_idx.min(count.saturating_sub(1)) {
                            palette.bg_selected
                        } else {
                            gpui::transparent_black()
                        })
                        .hover(|s| s.bg(palette.bg_hover))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(10.0))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_model(&prov, &mid, window, cx);
                        }))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .min_w_0().flex_1().overflow_hidden()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(7.0))
                                        .child(
                                            div()
                                                .font_family(font_sans())
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_size(px(13.0))
                                                .text_color(palette.text_primary)
                                                .min_w_0().truncate().child(m.name.clone()),
                                        )
                                        .when(is_current, |d| {
                                            d.child(Icon::Check.render(13.0, palette.accent))
                                        }),
                                )
                                .child(
                                    div()
                                        .font_family(font_mono())
                                        .text_size(px(11.0))
                                        .text_color(palette.text_muted)
                                        .min_w_0().truncate().child(format!("{} · {}", m.provider_display(), m.id)),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.0))
                                .flex_none()
                                .when(m.reasoning, |d| {
                                    d.child(pill("reasoning", palette.accent_soft, palette.accent))
                                })
                                .when(m.supports_images, |d| {
                                    d.child(pill("vision", palette.info_soft, palette.info))
                                })
                                .child(pill(
                                    &format_tokens_compact(m.context_window),
                                    palette.bg_code,
                                    palette.text_secondary,
                                )),
                        )
                })),
        )
        .into_any_element()
}

fn matching_models<'a>(
    view: &'a PrimeDesktopView,
    cx: &App,
) -> Vec<&'a crate::state::models::CatalogModel> {
    let query = view.modal_search_input.read(cx).text();
    view.catalog_models
        .iter()
        .filter(|m| (!view.model_picker_configured_only || m.configured) && m.matches_query(query))
        .take(90)
        .collect()
}

// Arrow keys belong to the result list while the search field retains text entry and IME focus.
fn picker_surface(count: usize, cx: &Context<PrimeDesktopView>) -> gpui::Div {
    div()
        .capture_action(cx.listener(move |view, _: &Up, _, cx| {
            view.modal_selected_idx = view
                .modal_selected_idx
                .saturating_sub(1)
                .min(count.saturating_sub(1));
            view.modal_scroll.scroll_to_item(view.modal_selected_idx);
            cx.stop_propagation();
            cx.notify();
        }))
        .capture_action(cx.listener(move |view, _: &Down, _, cx| {
            view.modal_selected_idx = (view.modal_selected_idx + 1).min(count.saturating_sub(1));
            view.modal_scroll.scroll_to_item(view.modal_selected_idx);
            cx.stop_propagation();
            cx.notify();
        }))
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
    fn activate(
        &self,
        view: &mut PrimeDesktopView,
        window: &mut Window,
        cx: &mut Context<PrimeDesktopView>,
    ) {
        match self {
            Self::Command(id) => view.execute_palette_action(id, window, cx),
            Self::Project(cwd) => {
                view.close_modal(window, cx);
                if !view.prefs.sidebar_open {
                    view.toggle_sidebar(window, cx);
                }
                view.focus_sidebar_project(cwd, cx);
                window.focus(&view.sidebar_search_input.focus_handle(cx));
            }
            Self::Chat(request) => {
                view.open_session(request.clone(), cx);
                view.close_modal(window, cx);
            }
        }
    }
}

impl PrimeDesktopView {
    pub(super) fn submit_modal_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.active_modal {
            Some(ActiveModal::ModelPicker) => {
                let models = matching_models(self, cx);
                if let Some(model) =
                    models.get(self.modal_selected_idx.min(models.len().saturating_sub(1)))
                {
                    let (provider, id) = (model.provider.clone(), model.id.clone());
                    self.select_model(&provider, &id, window, cx);
                }
            }
            Some(ActiveModal::QuickSwitcher) => {
                let rows = switch_rows(self, cx);
                if let Some(row) =
                    rows.get(self.modal_selected_idx.min(rows.len().saturating_sub(1)))
                {
                    row.target.activate(self, window, cx);
                }
            }
            Some(ActiveModal::SettingsAuth) | None => {}
        }
    }
}

fn switch_rows(view: &PrimeDesktopView, cx: &App) -> Vec<SwitchRow> {
    let q = view
        .modal_search_input
        .read(cx)
        .text()
        .trim()
        .to_ascii_lowercase();
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

fn render_quick_switcher_modal(
    view: &PrimeDesktopView,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let rows = switch_rows(view, cx);
    let count = rows.len();
    picker_surface(count, cx)
        .w_full()
        .max_h(px(480.0))
        .rounded_2xl()
        .border_1()
        .border_color(palette.border_strong)
        .bg(palette.bg_elevated)
        .shadow_lg()
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(
            div()
                .p(px(12.0))
                .border_b_1()
                .border_color(palette.border_subtle)
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(Icon::Search.render(15.0, palette.accent))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(view.modal_search_input.clone()),
                ),
        )
        .child(
            div()
                .id("quick-switcher-scroll")
                .track_scroll(&view.modal_scroll)
                .flex_1()
                .overflow_y_scroll()
                .p(px(8.0))
                .flex()
                .flex_col()
                .gap(px(3.0))
                .when(rows.is_empty(), |d| {
                    d.child(
                        div()
                            .p(px(16.0))
                            .text_color(palette.text_secondary)
                            .child("No matching commands, projects, or chats."),
                    )
                })
                .children(rows.into_iter().enumerate().map(|(index, row)| {
                    let project = matches!(row.target, SwitchTarget::Project(_));
                    let command = matches!(row.target, SwitchTarget::Command(_));
                    div()
                        .id(SharedString::from(row.id))
                        .flex_none()
                        .min_w_0()
                        .px(px(12.0))
                        .py(px(8.0))
                        .rounded_lg()
                        .cursor_pointer()
                        .when(
                            index == view.modal_selected_idx.min(count.saturating_sub(1)),
                            |d| d.bg(palette.bg_selected),
                        )
                        .hover(|s| s.bg(palette.bg_hover))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .on_click(cx.listener(move |view, _, window, cx| {
                            row.target.activate(view, window, cx);
                        }))
                        .when(project, |d| {
                            d.child(Icon::Folder.render(14.0, palette.accent))
                        })
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .truncate()
                                .text_size(px(13.0))
                                .text_color(palette.text_primary)
                                .child(row.title),
                        )
                        .when(!row.hint.is_empty(), |d| {
                            d.child(
                                div()
                                    .min_w_0()
                                    .max_w(px(150.0))
                                    .truncate()
                                    .when(command, |d| d.font_family(font_mono()).flex_none())
                                    .text_size(px(11.0))
                                    .text_color(palette.text_muted)
                                    .child(row.hint),
                            )
                        })
                })),
        )
        .into_any_element()
}

#[allow(clippy::too_many_lines)]
fn render_settings_auth_modal(
    view: &PrimeDesktopView,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    div()
        .capture_key_down(cx.listener(PrimeDesktopView::tab_settings))
        .w_full()
        .max_h(px(520.0))
        .rounded_2xl()
        .border_1()
        .border_color(palette.border_strong)
        .bg(palette.bg_elevated)
        .shadow_lg()
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(
            div()
                .px(px(16.0))
                .py(px(13.0))
                .border_b_1()
                .border_color(palette.border_subtle)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(Icon::Shield.render(16.0, palette.accent))
                        .child(
                            div()
                                .font_family(font_serif())
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(px(17.0))
                                .text_color(palette.text_primary)
                                .child("Settings"),
                        ),
                )
                .child(
                    div()
                        .id("close-settings-modal")
                        .p(px(4.0))
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_modal(window, cx);
                        }))
                        .child(Icon::Close.render(14.0, palette.text_muted)),
                ),
        )
        .child(
            div()
                .id("settings-modal-scroll")
                .p(px(16.0))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(
                    div()
                        .id("reduce-motion")
                        .debug_selector(|| "reduce-motion".into())
                        .track_focus(&view.settings_motion_focus)
                        .cursor_pointer()
                        .flex_none()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.0))
                        .p(px(12.0))
                        .rounded_xl()
                        .border_1()
                        .border_color(palette.border_subtle)
                        .bg(palette.bg_canvas)
                        .hover(|s| s.bg(palette.bg_hover))
                        .focus(|s| s.border_color(palette.accent))
                        .on_click(cx.listener(|view, _, _, cx| view.toggle_reduce_motion(cx)))
                        .child(
                            div()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::MEDIUM)
                                .child("Reduce motion"),
                        )
                        .child(pill(
                            if view.prefs.reduce_motion {
                                "On"
                            } else {
                                "Off"
                            },
                            if view.prefs.reduce_motion {
                                palette.accent_soft
                            } else {
                                palette.bg_hover
                            },
                            if view.prefs.reduce_motion {
                                palette.accent
                            } else {
                                palette.text_secondary
                            },
                        )),
                )
                .child(view.cli_data_card(palette, cx))
                // Daemon Socket Info Card
                .child(
                    div()
                        .p(px(12.0))
                        .rounded_xl()
                        .border_1()
                        .border_color(palette.border_subtle)
                        .bg(palette.bg_canvas)
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(palette.text_primary)
                                        .child(view.prefs.daemon_target.label()),
                                )
                                .child(
                                    div()
                                        .font_family(font_mono())
                                        .text_size(px(11.0))
                                        .text_color(palette.text_muted)
                                        .min_w_0()
                                        .truncate()
                                        .child(view.daemon_socket_display.clone()),
                                ),
                        )
                        .when(view.prefs.cli_data_dir.is_none(), |card| {
                            card.child(
                                div()
                                    .id("settings-switch-daemon-btn")
                                    .flex_none()
                                    .whitespace_nowrap()
                                    .px(px(10.0))
                                    .py(px(5.0))
                                    .rounded_lg()
                                    .bg(palette.accent_soft)
                                    .cursor_pointer()
                                    .text_size(px(12.0))
                                    .text_color(palette.accent)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.switch_daemon_target(cx);
                                    }))
                                    .child(match view.prefs.daemon_target {
                                        crate::theme::DaemonTarget::DesktopHarness => {
                                            "Attach to CLI daemon"
                                        }
                                        crate::theme::DaemonTarget::SharedCli => {
                                            "Use desktop daemon"
                                        }
                                    }),
                            )
                        }),
                )
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(13.0))
                        .text_color(palette.text_primary)
                        .child("Provider credentials"),
                )
                .children(view.provider_auth.iter().map(|item| {
                    let pid = item.provider_id.clone();
                    let is_sel =
                        item.can_edit_api_key && view.selected_auth_provider == item.provider_id;
                    let row = div()
                        .id(SharedString::from(format!(
                            "auth-prov-{}",
                            item.provider_id
                        )))
                        .px(px(12.0))
                        .py(px(9.0))
                        .rounded_xl()
                        .border_1()
                        .border_color(if is_sel {
                            palette.accent
                        } else {
                            palette.border_subtle
                        })
                        .bg(palette.bg_canvas)
                        .flex()
                        .items_center()
                        .justify_between()
                        .when(item.can_edit_api_key, |row| {
                            row.cursor_pointer().on_click(cx.listener(
                                move |this, _, window, cx| {
                                    if this.selected_auth_provider != pid {
                                        this.api_key_input
                                            .update(cx, crate::ui::text_input::TextInput::clear);
                                    }
                                    this.selected_auth_provider.clone_from(&pid);
                                    window.focus(&this.api_key_input.focus_handle(cx));
                                    cx.notify();
                                },
                            ))
                        })
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(palette.text_primary)
                                        .child(item.display_name.clone()),
                                )
                                .child(
                                    div()
                                        .font_family(font_mono())
                                        .text_size(px(11.0))
                                        .text_color(palette.text_muted)
                                        .child(item.hint.clone()),
                                ),
                        )
                        .child(pill(
                            &item.auth_kind,
                            if item.configured {
                                palette.success_soft
                            } else {
                                palette.bg_hover
                            },
                            if item.configured {
                                palette.success
                            } else {
                                palette.text_muted
                            },
                        ));
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(row)
                        .when(is_sel, |group| {
                            group.child(render_api_key_editor(view, item, palette, cx))
                        })
                })),
        )
        .into_any_element()
}

fn render_api_key_editor(
    view: &PrimeDesktopView,
    item: &ProviderAuthItem,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> gpui::Div {
    let has_new_key = !view.api_key_input.read(cx).text().trim().is_empty();
    div()
        .p(px(12.0))
        .rounded_xl()
        .border_1()
        .border_color(palette.border_subtle)
        .bg(palette.bg_canvas)
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(
            div()
                .flex_1()
                .px(px(9.0))
                .py(px(6.0))
                .rounded_lg()
                .bg(palette.bg_code)
                .child(view.api_key_input.clone()),
        )
        .when(view.auth_write_pending, |row| {
            row.child(pill("Updating…", palette.bg_hover, palette.text_muted))
        })
        .when(!view.auth_write_pending && has_new_key, |row| {
            row.child(
                div()
                    .id("save-provider-key-btn")
                    .px(px(12.0))
                    .py(px(6.0))
                    .rounded_lg()
                    .bg(palette.accent)
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(palette.text_on_accent)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.save_selected_provider_key(cx);
                    }))
                    .child("Save key"),
            )
        })
        .when(
            !view.auth_write_pending && item.has_stored_api_key && !has_new_key,
            |row| {
                row.child(
                    div()
                        .id("remove-provider-key-btn")
                        .px(px(12.0))
                        .py(px(6.0))
                        .rounded_lg()
                        .bg(palette.bg_hover)
                        .cursor_pointer()
                        .text_size(px(12.5))
                        .text_color(palette.text_secondary)
                        .on_click(cx.listener(|this, _, window, cx| {
                            let provider = this.selected_auth_provider.clone();
                            let agent_dir = this.agent_dir.clone();
                            confirm_action(
                                &format!("Remove the saved API key for {provider}?"),
                                "Remove key",
                                window,
                                cx,
                                move |this, cx| {
                                    if this.selected_auth_provider == provider
                                        && this.agent_dir == agent_dir
                                        && this.api_key_input.read(cx).text().trim().is_empty()
                                    {
                                        this.save_selected_provider_key(cx);
                                    }
                                },
                            );
                        }))
                        .child("Remove key…"),
                )
            },
        )
}
