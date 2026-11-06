//! Connection controls use provider capabilities and display only public challenge information.

use std::time::Duration;

use rune::{Cursor, El, Key, ScrollSpec, col, el, row};

use crate::assets::Icon;
use crate::provider_connect::{ConnectMethod, ConnectStatus, ConnectionPane};
use crate::state::harness_store::ProviderAuthItem;
use crate::theme::Palette;
use crate::ui::panel_controls::{button, control, icon_button, label};
use crate::ui::text_input::field;
use crate::ui::workspace::PrimeDesktopView;

impl PrimeDesktopView {
    pub(super) fn provider_connect_sheet(&self, p: Palette) -> El {
        let pane = self.provider_connection.as_ref();
        let title = pane.map_or_else(
            || "Connect provider".to_owned(),
            |active| format!("Connect {}", active.display_name),
        );
        let heading = row()
            .w_full()
            .h(52.0)
            .px(16.0)
            .gap(8.0)
            .center_y()
            .child(
                label(&title, 17.0, 600.0, p.text_primary)
                    .grow(1.0)
                    .min_w(0.0)
                    .truncate(),
            )
            .child(
                icon_button(
                    "provider-connect-cancel",
                    "Cancel connection",
                    Icon::Close,
                    p,
                )
                .on_press(Self::cancel_provider_connection),
            );
        let contents = if let Some(active) = pane {
            self.connection_steps(active, p)
        } else {
            self.connection_providers(p)
        };
        col()
            .key("provider-connect-content")
            .label(title)
            .min_h(0.0)
            .max_h(480.0)
            .shrink()
            .child(heading)
            .child(el().h(1.0).fill(p.border_subtle))
            .child(contents)
    }

    fn filtered_providers(&self) -> Vec<&ProviderAuthItem> {
        let query = self.connection_input.text.trim().to_lowercase();
        self.provider_auth
            .iter()
            .filter(|item| {
                if !item.can_connect && !item.can_edit_api_key {
                    return false;
                }
                if query.is_empty() {
                    return true;
                }
                let name = format!("{} {}", item.display_name, item.provider_id).to_lowercase();
                query.split_whitespace().all(|term| name.contains(term))
            })
            .collect()
    }

    fn connection_providers(&self, p: Palette) -> El {
        let providers = self.filtered_providers();
        let count = providers.len();
        let selected = self.modal_selected_idx.min(count.saturating_sub(1));
        let keys: Vec<_> = providers
            .iter()
            .map(|item| format!("provider-option-{}", item.provider_id))
            .collect();
        let mut search = field(
            "connect-provider-search",
            &self.connection_input,
            "Search providers",
            p,
            |s| &mut s.connection_input,
        )
        .single_line()
        .grow(1.0)
        .min_w(0.0)
        .on_submit(|s: &mut Self, _| {
            let filtered = s.filtered_providers();
            if let Some(item) =
                filtered.get(s.modal_selected_idx.min(filtered.len().saturating_sub(1)))
            {
                let id = item.provider_id.clone();
                s.open_provider_connect(&id);
            }
        })
        .on_input_change(|s: &mut Self, tx| {
            s.modal_selected_idx = 0;
            tx.scroll_to_y("connect-provider-list", 0.0);
        });
        if let Some(active) = keys.get(selected) {
            search = search.active_descendant(active.clone());
        }
        search = search.on_input_key(move |s: &mut Self, event, tx| {
            match event.key {
                Key::Down => {
                    s.modal_selected_idx = (s.modal_selected_idx + 1).min(count.saturating_sub(1));
                }
                Key::Up => {
                    s.modal_selected_idx = s
                        .modal_selected_idx
                        .saturating_sub(1)
                        .min(count.saturating_sub(1));
                }
                Key::Escape if event.mods == rune::Mods::default() => {
                    s.cancel_provider_connection();
                    return true;
                }
                _ => return false,
            }
            if let Some(key) = keys.get(s.modal_selected_idx) {
                tx.scroll_into_view(key.clone());
            }
            true
        });
        let field = row()
            .h(32.0)
            .gap(6.0)
            .px(8.0)
            .center_y()
            .radius(6.0)
            .fill(p.bg_canvas)
            .border(1.0, p.border_subtle)
            .child(Icon::Search.render(14.0, p.text_muted))
            .child(search);
        let list = if providers.is_empty() {
            let message = if self.provider_auth.is_empty() {
                "Loading providers…"
            } else if self.connection_input.text.trim().is_empty() {
                "No editable providers were found in the CLI data folder."
            } else {
                "No matching providers."
            };
            label(message, 13.0, 400.0, p.text_secondary)
                .padding(14.0)
                .wrap()
        } else {
            let rows = providers.iter().enumerate().map(|(index, item)| {
                let shared_name = providers.iter().any(|other| {
                    other.provider_id != item.provider_id && other.display_name == item.display_name
                });
                provider_option(
                    item,
                    shared_name,
                    index == selected && self.connection_input.focused,
                    p,
                )
            });
            let height = (count as f32 * 40.0 + 8.0).min(330.0);
            col()
                .key("connect-provider-list")
                .h(height)
                .min_h(0.0)
                .shrink()
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(col().px(6.0).py(4.0).children(rows))
        };
        col()
            .min_h(0.0)
            .shrink()
            .child(col().px(12.0).py(8.0).child(field))
            .child(list)
    }

    fn connection_steps(&self, pane: &ConnectionPane, p: Palette) -> El {
        let mut body = col().padding(16.0).gap(12.0).min_w(0.0);
        if let Some(error) = &pane.error {
            body = body.child(
                label(error, 13.0, 400.0, p.danger)
                    .key("provider-connect-error")
                    .wrap_anywhere(),
            );
        }
        if matches!(pane.status, ConnectStatus::Choose | ConnectStatus::Failed) {
            if pane.provider_id == "anthropic" && pane.can_browser {
                // The CLI discloses this when a Claude subscription is connected.
                body = body.child(label(
                    "Subscription sign-in uses your Claude plan and identifies Prime Agent as Claude Code. Anthropic may restrict your account for this use.",
                    13.0, 400.0, p.text_secondary,
                ).wrap());
            }
            let mut methods = row().center_y().gap(8.0).flex_wrap();
            if pane.can_browser {
                methods = methods.child(
                    primary("connect-use-browser", "Sign in with browser", p).on_press(
                        |s: &mut Self| s.start_provider_connection(ConnectMethod::Browser),
                    ),
                );
            }
            if pane.can_api_key {
                let method = if pane.can_browser {
                    button("connect-use-key", "Use API key", p)
                } else {
                    primary("connect-use-key", "Use API key", p)
                };
                methods = methods.child(method.on_press(|s: &mut Self| {
                    s.start_provider_connection(ConnectMethod::ApiKey);
                }));
            }
            return body.child(methods);
        }
        if !pane.progress.is_empty() {
            body = body.child(
                label(&pane.progress, 13.0, 400.0, p.text_secondary)
                    .key("provider-connect-progress")
                    .wrap(),
            );
        }
        if let Some(browser) = &pane.browser {
            if let Some(instructions) = browser
                .instructions
                .as_deref()
                .filter(|text| !text.is_empty())
            {
                body = body.child(label(instructions, 13.0, 400.0, p.text_primary).wrap_anywhere());
            }
            if let Some(code) = &browser.code {
                let copied = code.clone();
                let mut code_row = row().center_y().gap(8.0);
                if !browser
                    .instructions
                    .as_deref()
                    .is_some_and(|instructions| instructions.contains(code.as_str()))
                {
                    code_row = code_row.child(
                        label(code, 16.0, 600.0, p.text_primary)
                            .key("provider-device-code")
                            .selectable(true),
                    );
                }
                body = body.child(code_row.child(
                    button("provider-copy-code", "Copy code", p).on_press_tx(
                        move |_: &mut Self, tx| {
                            tx.copy_secret(copied.clone(), Duration::from_secs(60));
                        },
                    ),
                ));
            }
            body = body.child(
                row().child(
                    primary("provider-open-browser", "Open browser", p)
                        .on_press_tx(Self::open_connection_browser),
                ),
            );
        }
        if let Some(prompt) = &pane.prompt {
            let placeholder = prompt.placeholder.as_deref().unwrap_or("");
            let mut input = field(
                "provider-connect-input",
                &self.connection_input,
                placeholder,
                p,
                |s| &mut s.connection_input,
            )
            .label(&prompt.label)
            .single_line()
            .min_w(0.0)
            .grow(1.0)
            .on_submit(|s: &mut Self, _| s.submit_connection_input())
            .on_input_key(|s: &mut Self, event, _| {
                if event.key == Key::Escape && event.mods == rune::Mods::default() {
                    s.cancel_provider_connection();
                    return true;
                }
                false
            });
            if prompt.secret {
                input = input.secure();
            }
            let enabled = prompt.allow_empty || !self.connection_input.text.trim().is_empty();
            let submit = if enabled {
                primary("provider-connect-submit", "Continue", p)
                    .on_press(Self::submit_connection_input)
            } else {
                row()
                    .key("provider-connect-submit")
                    .role("button")
                    .label("Continue")
                    .disabled(true)
                    .h(30.0)
                    .px(10.0)
                    .radius(6.0)
                    .center()
                    .fill(p.bg_code)
                    .child(label("Continue", 12.5, 500.0, p.text_muted))
            };
            body = body
                .child(
                    col()
                        .gap(6.0)
                        .child(label(&prompt.label, 12.5, 500.0, p.text_primary).wrap())
                        .child(
                            row()
                                .h(35.0)
                                .px(8.0)
                                .center_y()
                                .radius(6.0)
                                .border(1.0, p.border_strong)
                                .fill(p.bg_canvas)
                                .child(input),
                        ),
                )
                .child(row().justify_end().child(submit));
        }
        if let Some(choice) = &pane.choice {
            let entries = choice.options.iter().enumerate().map(|(index, title)| {
                control(format!("provider-team-{index}"), title, p)
                    .h(34.0)
                    .px(8.0)
                    .center_y()
                    .gap(6.0)
                    .child(
                        label(title, 13.0, 500.0, p.text_primary)
                            .min_w(0.0)
                            .grow(1.0)
                            .truncate(),
                    )
                    .child(Icon::ChevronRight.render(12.0, p.text_muted))
                    .focus_selected(index == choice.selected)
                    .on_press(move |s: &mut Self| s.select_connection_choice(index))
            });
            body = body.child(
                col()
                    .gap(5.0)
                    .child(label(&choice.label, 12.5, 500.0, p.text_primary).wrap())
                    .child(
                        col()
                            .key("provider-team-list")
                            .h((choice.options.len() as f32 * 34.0).clamp(34.0, 200.0))
                            .scroll(ScrollSpec {
                                stick_bottom: false,
                            })
                            .child(col().role("list").focus_group().children(entries)),
                    ),
            );
        }
        if pane.browser.is_none()
            && pane.prompt.is_none()
            && pane.choice.is_none()
            && pane.progress.is_empty()
        {
            body = body.child(label(
                if matches!(pane.status, ConnectStatus::Saving) {
                    "Saving credentials…"
                } else {
                    "Connecting…"
                },
                13.0,
                400.0,
                p.text_secondary,
            ));
        }
        col()
            .key("provider-connect-steps")
            .min_h(0.0)
            .max_h(398.0)
            .shrink()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(body)
    }
}

fn provider_option(
    item: &ProviderAuthItem,
    shared_name: bool,
    highlighted: bool,
    p: Palette,
) -> El {
    let pid = item.provider_id.clone();
    let kind = if item.configured {
        "Configured"
    } else if item.can_connect && item.can_edit_api_key {
        "Browser or API key"
    } else if item.can_connect {
        "Browser sign-in"
    } else {
        "API key"
    };
    let title = if shared_name {
        format!("{} ({})", item.display_name, item.provider_id)
    } else {
        item.display_name.clone()
    };
    let option = control(format!("provider-option-{pid}"), &title, p);
    let option = if highlighted {
        option.fill(p.bg_hover)
    } else {
        option
    };
    option
        .h(40.0)
        .px(9.0)
        .gap(7.0)
        .center_y()
        .child(
            label(title, 13.0, 500.0, p.text_primary)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
        .child(label(kind, 11.0, 400.0, p.text_secondary))
        .child(Icon::ChevronRight.render(13.0, p.text_muted))
        .on_press(move |s: &mut PrimeDesktopView| s.open_provider_connect(&pid))
}

fn primary(key: &str, title: &str, p: Palette) -> El {
    control(key, title, p)
        .h(30.0)
        .px(11.0)
        .center()
        .radius(6.0)
        .cursor(Cursor::Pointer)
        .fill(p.text_primary)
        .child(label(title, 12.5, 550.0, p.bg_canvas))
}
