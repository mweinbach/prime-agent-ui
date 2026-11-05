use super::{ActiveModal, DaemonTarget, LaunchOptions, PrimeDesktopView};
use crate::theme::Palette;
use gpui::{AnyElement, Context, FontWeight, PathPromptOptions, Window, div, prelude::*, px};
use std::path::PathBuf;

impl PrimeDesktopView {
    pub(in crate::ui) fn cli_data_card(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let choose = div()
            .id("choose-cli-folder")
            .flex_none()
            .cursor_pointer()
            .py(px(4.0))
            .text_color(palette.accent)
            .on_click(cx.listener(|_, _, window, cx| {
                let choice = cx.prompt_for_paths(PathPromptOptions {
                    files: false,
                    directories: true,
                    multiple: false,
                    prompt: Some("Use CLI data folder".into()),
                });
                cx.spawn_in(window, async move |this, cx| {
                    if let Ok(Ok(Some(paths))) = choice.await
                        && let Some(path) = paths.into_iter().next()
                    {
                        let _ = this.update_in(cx, |view, window, cx| {
                            view.use_cli_folder(Some(path), window, cx);
                        });
                    }
                })
                .detach();
            }))
            .child("Choose folder…");
        div()
            .p(px(12.0)).rounded_xl().border_1().border_color(palette.border_subtle).bg(palette.bg_canvas)
            .flex().flex_col().gap(px(5.0))
            .child(div().text_size(px(13.0)).font_weight(FontWeight::SEMIBOLD).child("Prime Agent CLI data"))
            .child(div().text_size(px(12.0)).text_color(palette.text_secondary).child("Chats, saved credentials and settings are shared with the CLI in this folder."))
            .child(div().min_w_0().truncate().text_size(px(11.0)).font_family(crate::theme::font_mono()).text_color(palette.text_muted).child(self.agent_dir.display().to_string()))
            .child(div().flex().items_center().gap(px(12.0)).text_size(px(12.0)).child(choose)
                .when(self.prefs.cli_data_dir.is_some(), |row| row.child(div()
                    .id("reset-cli-folder").cursor_pointer().text_color(palette.accent)
                    .on_click(cx.listener(|view, _, window, cx| view.use_cli_folder(None, window, cx)))
                    .child("Use default"))))
            .into_any_element()
    }

    fn use_cli_folder(
        &mut self,
        selected: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let folder = selected.as_ref().unwrap_or(&self.preferences_dir);
        if folder == &self.agent_dir {
            return;
        }
        if self.auth_write_pending
            || !self.composer_input.read(cx).text().is_empty()
            || !self.staged_images.is_empty()
            || self.drafts.values().any(|drafts| !drafts.is_empty())
            || !self.api_key_input.read(cx).text().is_empty()
            || self.harness_editor_open
        {
            self.show_toast(
                "Finish editing drafts and credentials before changing the CLI data folder.",
                true,
                cx,
            );
            return;
        }
        // Advise on a wrong folder rather than silently creating a second, empty CLI store there.
        let check = folder.clone();
        let reset = selected.is_none();
        let valid = cx.background_spawn(async move {
            reset
                || check.join("sessions").is_dir()
                || check.join("harness").is_dir()
                || ["settings.json", "auth.json", "models.json"]
                    .iter()
                    .any(|name| check.join(name).is_file())
        });
        cx.spawn_in(window, async move |this, cx| {
            let valid = valid.await;
            let _ = this.update_in(cx, |view, window, cx| {
                if !valid {
                    view.show_toast(
                        "Choose the CLI data folder that contains sessions, settings.json or auth.json.",
                        true,
                        cx,
                    );
                    return;
                }
                let mut prefs = view.prefs.clone();
                prefs.cli_data_dir = selected.filter(|dir| dir != &view.preferences_dir);
                prefs.daemon_target = DaemonTarget::DesktopHarness;
                if let Err(err) = prefs.save(&view.preferences_dir) {
                    view.show_toast(format!("Could not save the CLI data folder: {err}"), true, cx);
                    return;
                }
                let preferences_dir = view.preferences_dir.clone();
                let options = LaunchOptions { initial_cwd: Some(PathBuf::from(&view.working_cwd)), initial_modal: Some(ActiveModal::SettingsAuth), ..LaunchOptions::default() };
                window.replace_root(cx, |window, cx| Self::new(options, preferences_dir, window, cx));
            });
        }).detach();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::bridge::{CliData, socket_path_for_target};
    use gpui::TestAppContext;

    #[test]
    fn choosing_a_cli_folder_reloads_its_shared_keys_and_remembers_only_the_path() {
        let root = tempfile::tempdir().unwrap();
        let default = root.path().join("default");
        let selected = root.path().join("cli");
        std::fs::create_dir(&selected).unwrap();
        let credential =
            br#"{"import-fixture":{"type":"api_key","key":"never-copy-this-fixture"}}"#;
        std::fs::write(selected.join("auth.json"), credential).unwrap();
        let data = CliData::new(selected.clone());
        let socket = socket_path_for_target(DaemonTarget::DesktopHarness, None, &data);
        // Occupy just the socket for this unique temporary profile. A new companion must not be launched.
        std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let mut app = TestAppContext::single();
        let (old, cx) = app.add_window_view(|window, cx| {
            PrimeDesktopView::new(
                LaunchOptions {
                    daemon_socket: Some(root.path().join("no-listener")),
                    ..LaunchOptions::default()
                },
                default.clone(),
                window,
                cx,
            )
        });
        old.update_in(cx, |workspace, window, cx| {
            workspace.use_cli_folder(Some(selected.clone()), window, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let workspace = window.root::<PrimeDesktopView>().flatten().unwrap();
            let view = workspace.read(cx);
            assert_eq!(view.agent_dir, selected);
            assert!(
                view.provider_auth
                    .iter()
                    .any(|item| item.provider_id == "import-fixture" && item.has_stored_api_key)
            );
        });
        let prefs = std::fs::read_to_string(default.join("desktop-ui.json")).unwrap();
        assert_eq!(
            crate::theme::DesktopPreferences::load(&default)
                .cli_data_dir
                .as_deref(),
            Some(selected.as_path())
        );
        assert!(!prefs.contains("never-copy-this-fixture"));
        assert!(!default.join("auth.json").exists());
        assert_eq!(
            std::fs::read(selected.join("auth.json")).unwrap(),
            credential
        );
        app.quit();
        drop(listener);
        std::fs::remove_file(socket).unwrap();
    }
}
