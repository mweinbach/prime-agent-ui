//! Overlay modals: Searchable Model Picker (`⌘M`), Quick Switcher / Command Palette (`⌘K`),
//! and Provider Credentials & Desktop Settings (`⌘,`).

use gpui::{AnyElement, Context, Focusable, FontWeight, SharedString, div, prelude::*, px};

use crate::assets::Icon;
use crate::bridge::BridgeRequest;
use crate::state::harness_store::ProviderAuthItem;
use crate::state::{ActiveModal, format_tokens_compact, truncate_chars};
use crate::theme::{Palette, font_mono, font_sans, font_serif};
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
    let card = match modal {
        ActiveModal::ModelPicker => render_model_picker_modal(view, palette, cx),
        ActiveModal::QuickSwitcher => render_quick_switcher_modal(view, palette, cx),
        ActiveModal::SettingsAuth => render_settings_auth_modal(view, palette, cx),
    };

    div()
        .id("modal-backdrop-scrim")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .bg(palette.bg_scrim)
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .p(px(24.0))
        .on_click(cx.listener(|this, _, window, cx| {
            this.close_modal(window, cx);
        }))
        .child(
            div()
                .id("modal-card-surface")
                .w_full()
                .max_w(px(610.0))
                .max_h_full()
                .flex()
                .flex_col()
                .on_click(|_, _, cx| {
                    cx.stop_propagation();
                })
                .child(card),
        )
        .into_any_element()
}

#[allow(clippy::too_many_lines)]
fn render_model_picker_modal(
    view: &PrimeDesktopView,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let query = view.modal_search_input.read(cx).text().to_string();
    let filtered: Vec<_> = view
        .catalog_models
        .iter()
        .filter(|m| (!view.model_picker_configured_only || m.configured) && m.matches_query(&query))
        .take(90)
        .collect();

    div()
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
                        .child(div().flex_1().child(view.modal_search_input.clone())),
                ),
        )
        // Model Rows
        .child(
            div()
                .id("model-picker-scroll")
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
                .children(filtered.into_iter().map(|m| {
                    let is_current = view.session.model_provider.as_deref()
                        == Some(m.provider.as_str())
                        && view.session.model_id.as_deref() == Some(m.id.as_str());
                    let prov = m.provider.clone();
                    let mid = m.id.clone();

                    div()
                        .id(SharedString::from(format!("pick-m-{}/{}", m.provider, m.id)))
                        .px(px(12.0))
                        .py(px(8.5))
                        .rounded_xl()
                        .cursor_pointer()
                        .bg(if is_current {
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
                                .overflow_hidden()
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
                                                .child(m.name.clone()),
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
                                        .child(format!("{} · {}", m.provider_display(), m.id)),
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

fn render_quick_switcher_modal(
    view: &PrimeDesktopView,
    palette: Palette,
    cx: &mut Context<PrimeDesktopView>,
) -> AnyElement {
    let q = view
        .modal_search_input
        .read(cx)
        .text()
        .trim()
        .to_ascii_lowercase();

    let actions: [(&str, &str, &str); 6] = [
        ("act-new", "New chat in this project", "⌘N"),
        ("act-model", "Switch model", "⌘M"),
        ("act-folder", "Add project…", "⇧⌘O"),
        ("act-harness", "Open harness inspector", "⇧⌘H"),
        ("act-compact", "Compact conversation context", "⇧⌘K"),
        ("act-theme", "Switch light or dark appearance", "⇧⌘L"),
    ];

    let matching_actions: Vec<_> = actions
        .into_iter()
        .filter(|(_, label, _)| q.is_empty() || label.to_ascii_lowercase().contains(&q))
        .collect();

    let groups = view.project_groups(&q);
    let project_rows = groups
        .iter()
        .filter_map(|g| {
            let cwd = g.folder()?;
            (q.is_empty() && g.added
                || !q.is_empty()
                    && (cwd.to_lowercase().contains(&q) || g.label.to_lowercase().contains(&q)))
            .then_some((g, cwd.to_owned()))
        })
        .take(8);
    let chats = groups
        .iter()
        .flat_map(|group| group.chats.iter().map(move |chat| (chat, &group.label)))
        .take(20);

    div()
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
                .child(div().flex_1().child(view.modal_search_input.clone())),
        )
        .child(
            div()
                .id("quick-switcher-scroll")
                .flex_1()
                .overflow_y_scroll()
                .p(px(8.0))
                .flex()
                .flex_col()
                .gap(px(3.0))
                .children(matching_actions.into_iter().map(|(id, label, shortcut)| {
                    div()
                        .id(SharedString::from(id))
                        .px(px(12.0))
                        .py(px(8.0))
                        .rounded_lg()
                        .cursor_pointer()
                        .hover(|s| s.bg(palette.bg_hover))
                        .flex()
                        .items_center()
                        .justify_between()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.execute_palette_action(id, window, cx);
                        }))
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(palette.text_primary)
                                .child(label),
                        )
                        .child(
                            div()
                                .font_family(font_mono())
                                .text_size(px(11.0))
                                .text_color(palette.text_muted)
                                .child(shortcut),
                        )
                }))
                .children(project_rows.map(|(project, cwd)| {
                    div()
                        .id(SharedString::from(format!("switch-project-{cwd}")))
                        .px(px(12.0))
                        .py(px(8.0))
                        .cursor_pointer()
                        .rounded_lg()
                        .hover(|s| s.bg(palette.bg_hover))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.close_modal(window, cx);
                            this.new_project_chat(&cwd, window, cx);
                        }))
                        .child(Icon::Folder.render(14.0, palette.accent))
                        .child(project.label.clone())
                }))
                .children(chats.map(|(chat, project)| {
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
                    div()
                        .id(SharedString::from(format!("switch-chat-{chat:?}")))
                        .px(px(12.0))
                        .py(px(8.0))
                        .rounded_lg()
                        .cursor_pointer()
                        .hover(|s| s.bg(palette.bg_hover))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(6.0))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_session(request.clone(), cx);
                            this.close_modal(window, cx);
                        }))
                        .child(
                            div()
                                .truncate()
                                .text_size(px(13.0))
                                .text_color(palette.text_primary)
                                .child(truncate_chars(title, 46)),
                        )
                        .child(
                            div()
                                .truncate()
                                .max_w(px(150.0))
                                .text_size(px(11.0))
                                .text_color(palette.text_muted)
                                .child(project.clone()),
                        )
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
                                        .child(view.daemon_socket_display.clone()),
                                ),
                        )
                        .when(view.prefs.cli_data_dir.is_none(), |card| {
                            card.child(
                                div()
                                    .id("settings-switch-daemon-btn")
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
