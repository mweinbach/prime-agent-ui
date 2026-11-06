//! Previews use the immutable bytes staged with a draft. No file reads or image
//! decoding run during layout.

use std::path::Path;

use rune::{Cx, El, ScrollSpec, SurfaceFit, TextStyle, Transition, col, el, row, text};

use crate::assets::Icon;
use crate::state::{AttachmentPreview, StagedAttachment};
use crate::theme::Palette;
use crate::ui::selectable_text::{control, icon_control, label};
use crate::ui::workspace::PrimeDesktopView;

fn file_type(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| !ext.is_empty() && ext.len() <= 8)
        .map_or_else(|| "File".into(), str::to_ascii_uppercase)
}

fn byte_size(bytes: usize) -> String {
    if bytes < 1024 {
        return if bytes == 1 {
            "1 byte".into()
        } else {
            format!("{bytes} bytes")
        };
    }
    let (unit, suffix) = if bytes < 1024 * 1024 {
        (1024, "KiB")
    } else {
        (1024 * 1024, "MiB")
    };
    let fraction = bytes % unit * 10 / unit;
    if fraction == 0 {
        format!("{} {suffix}", bytes / unit)
    } else {
        format!("{}.{fraction} {suffix}", bytes / unit)
    }
}

fn attachment_card(attachment: &StagedAttachment, p: Palette) -> El {
    let id = attachment.id;
    let name = attachment.filename.as_str();
    let tile = el()
        .size(42., 42.)
        .center()
        .radius(5.)
        .clip_rounded(5.)
        .fill(p.bg_code);
    let (thumbnail, subtitle) = match &attachment.preview {
        AttachmentPreview::Image(image) => (
            tile.pixels_fit(&image.thumbnail, SurfaceFit::Fill)
                .a11y_hidden(),
            format!(
                "{} × {} · {}",
                image.width,
                image.height,
                byte_size(attachment.byte_len)
            ),
        ),
        AttachmentPreview::Text { excerpt, .. } => {
            let sample = excerpt
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map_or_else(
                    || byte_size(attachment.byte_len),
                    |line| crate::state::truncate_chars(line, 100),
                );
            (
                tile.child(
                    text(file_type(name))
                        .font(TextStyle::new(10., 550.).mono())
                        .color(p.text_secondary),
                )
                .a11y_hidden(),
                sample,
            )
        }
    };
    let preview = control(
        &format!("preview-attachment-{id}"),
        &format!("Preview {name}"),
        p,
    )
    .preserve_focus_on_press(false)
    .min_w(0.)
    .grow(1.)
    .justify_start()
    .h(52.)
    .gap(8.)
    .px(5.)
    .py(5.)
    .help(name)
    .child(thumbnail)
    .child(
        col()
            .min_w(0.)
            .grow(1.)
            .gap(3.)
            .child(
                label(name, 12.5, p.text_primary)
                    .font(TextStyle::new(12.5, 550.))
                    .truncate_middle(),
            )
            .child(label(subtitle, 11.5, p.text_secondary).truncate()),
    )
    .on_press(move |s: &mut PrimeDesktopView| s.open_attachment_preview(id));
    let remove = icon_control(
        &format!("remove-attachment-{id}"),
        &format!("Remove {name}"),
        Icon::Close,
        p,
    )
    .size(24., 24.)
    .on_press(move |s: &mut PrimeDesktopView| s.remove_attachment(id));
    row()
        .key(format!("attachment-{id}"))
        .w(218.)
        .h(54.)
        .pr(4.)
        .radius(8.)
        .center_y()
        .border(1., p.border_subtle)
        .fill(p.bg_elevated)
        .child(preview)
        .child(remove)
}

pub(super) fn staged(view: &PrimeDesktopView, p: Palette) -> El {
    let mut strip = col().key("staged-attachments").w_full().min_w(0.).gap(4.);
    if !view.staged_attachments.is_empty() {
        // Keep the typing area reachable even when many files are attached.
        strip = strip.child(
            row()
                .key("attachment-strip")
                .w_full()
                .min_w(0.)
                .scroll_x()
                .child(
                    row().items_center().gap(6.).children(
                        view.staged_attachments
                            .iter()
                            .map(|item| attachment_card(item, p)),
                    ),
                ),
        );
    }
    if view.attachments_loading() {
        strip = strip.child(label("Preparing attachments…", 11.5, p.text_secondary).role("status"));
    }
    strip
}

impl PrimeDesktopView {
    pub(in crate::ui) fn attachment_preview(&self, cx: &Cx<Self>, p: Palette) -> Option<El> {
        let attachment = self.selected_attachment_preview()?;
        let id = attachment.id;
        let name = attachment.filename.as_str();
        // Reserve enough room for the close button and file controls at small window sizes.
        let [_, _, window_width, window_height] =
            cx.layout("workspace").unwrap_or([0., 0., 960., 660.]);
        let width = (window_width - 32.).clamp(1., 784.);
        let body_limit = (window_height - 168.).clamp(28., 550.);
        let media = match &attachment.preview {
            AttachmentPreview::Image(image) => {
                let ratio = image.height.max(1) as f32 / image.width.max(1) as f32;
                let height = ((width - 2.) * ratio).clamp(28., body_limit);
                el().key("attachment-preview-image")
                    .role("img")
                    .label(name)
                    .w_full()
                    .h(height)
                    .fill(p.bg_code)
                    .pixels_fit(
                        self.expanded_attachment_pixels(id)
                            .unwrap_or(&image.thumbnail),
                        SurfaceFit::Fit,
                    )
            }
            AttachmentPreview::Text { excerpt, truncated } => {
                let mut body = col()
                    .key("attachment-preview-document")
                    .w_full()
                    .min_w(0.)
                    .gap(8.);
                let scroll = col()
                    .key("attachment-preview-text-scroll")
                    .role("document")
                    .label(name)
                    .focusable(true)
                    .focus_ring(1., 0., p.text_secondary)
                    .shrink()
                    .w_full()
                    .max_h(body_limit)
                    .min_h(28.)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .fill(p.bg_code)
                    .child(if excerpt.is_empty() {
                        label("Empty file", 13., p.text_secondary).padding(16.)
                    } else {
                        text(excerpt)
                            .key("attachment-preview-text")
                            .font(TextStyle::new(13., 400.).mono().lh(19.))
                            .color(p.text_primary)
                            .wrap_anywhere()
                            .selectable(true)
                            .selection_color(p.selection_bg)
                            .padding(16.)
                    });
                body = body.child(scroll);
                if *truncated {
                    body = body.child(
                        label(
                            format!("Preview limited to the first {}.", byte_size(excerpt.len())),
                            12.,
                            p.text_secondary,
                        )
                        .px(16.),
                    );
                }
                body
            }
        };
        let media = if let Some(error) = self.selected_attachment_preview_error() {
            col().gap(6.).child(media).child(
                label(error, 12., p.text_secondary)
                    .role("status")
                    .px(16.)
                    .wrap(),
            )
        } else {
            media
        };
        let metadata = match &attachment.preview {
            AttachmentPreview::Image(image) => format!(
                "{} · {} × {} · {}",
                attachment
                    .image_mime()
                    .and_then(|mime| mime.strip_prefix("image/"))
                    .map_or_else(|| "Image".into(), str::to_ascii_uppercase),
                image.width,
                image.height,
                byte_size(attachment.byte_len)
            ),
            AttachmentPreview::Text { .. } => {
                format!("{} · {}", file_type(name), byte_size(attachment.byte_len))
            }
        };
        let heading = row()
            .w_full()
            .center_y()
            .gap(12.)
            .px(16.)
            .py(12.)
            .child(
                col()
                    .grow(1.)
                    .min_w(0.)
                    .gap(2.)
                    .child(
                        label(name, 16., p.text_primary)
                            .font(TextStyle::new(16., 600.))
                            .truncate_middle()
                            .help(name),
                    )
                    .child(label(metadata, 12., p.text_secondary)),
            )
            .child(
                icon_control("attachment-preview-close", "Close preview", Icon::Close, p)
                    .on_press(Self::close_attachment_preview),
            );
        let footer = row().justify_end().px(11.).py(6.).child(
            control("attachment-preview-remove", "Remove attachment", p)
                .child(Icon::Trash.render(13., p.text_secondary))
                .child(label("Remove attachment", 12., p.text_secondary))
                .on_press(move |s: &mut Self| s.remove_attachment(id)),
        );
        let dialog = col()
            .key("attachment-preview")
            .transition(Transition::none())
            .instant_changes()
            .role("dialog")
            .label(format!("Preview {name}"))
            .modal(true, Self::close_attachment_preview)
            .focus_return(format!("preview-attachment-{id}"))
            .hit()
            .w(width)
            .radius(14.)
            .clip_rounded(14.)
            .fill(p.bg_elevated)
            .border(1., p.border_strong)
            .shadow(6., 24., p.bg_scrim)
            .child(heading)
            .child(media)
            .child(footer);
        Some(
            col()
                .key("attachment-preview-layer")
                .transition(Transition::none())
                .pin(Some(0.), Some(0.), Some(0.), Some(0.))
                .center()
                .child(
                    el().key("attachment-preview-backdrop")
                        .absolute(0., 0.)
                        .w_full()
                        .h_full()
                        .fill(p.bg_scrim)
                        .on_press(Self::close_attachment_preview),
                )
                .child(dialog),
        )
    }
}

#[cfg(test)]
mod tests {
    use rune::{Env, Key, KeyEvent, Mods, Runtime};
    use tempfile::TempDir;

    use crate::state::StagedAttachment;
    use crate::ui::workspace::{LaunchOptions, PrimeDesktopView};

    fn click(app: &mut Runtime<PrimeDesktopView>, key: &str) {
        app.frame(1. / 60.);
        let element = app
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
            .unwrap();
        let [x, y, w, h] = element.visible;
        let (x, y) = (x + w / 2., y + h / 2.);
        app.pointer_down(x, y);
        app.pointer_up(x, y);
        app.frame(0.);
    }

    fn press(app: &mut Runtime<PrimeDesktopView>, key: Key, mods: Mods) {
        let ev = KeyEvent { key, mods };
        app.key(ev.clone());
        app.key_up(ev);
        app.frame(0.);
    }

    #[test]
    fn staged_file_preview_is_read_only_and_removal_has_its_own_target() {
        let profile = TempDir::new().unwrap();
        let mut view = PrimeDesktopView::new(
            LaunchOptions {
                initial_cwd: Some(profile.path().into()),
                daemon_socket: Some(profile.path().join("isolated.sock")),
                ..LaunchOptions::default()
            },
            profile.path().into(),
        );
        view.daemon_connected = true;
        view.prefs.sidebar_open = false;
        view.prefs.inspector_open = false;
        view.populate_draft("Keep this draft.");
        let path = profile.path().join("notes.md");
        let source = "First snapshot\n".repeat(420);
        std::fs::write(&path, &source).unwrap();
        let first = StagedAttachment::read(&path, source.len()).unwrap();
        let first_id = first.id;
        std::fs::write(&path, "A different file").unwrap();
        let second = StagedAttachment::read(&path, 1024).unwrap();
        let second_id = second.id;

        let mut app = Runtime::new(
            view,
            PrimeDesktopView::view,
            (540., 460.),
            Env::new(false, 1.),
        );
        app.on_key = Some(crate::shortcuts::handle_key);
        app.frame(1. / 60.);
        // File preparation completes after the composer is first laid out. New
        // cards must be clickable in their presented position immediately.
        app.state.staged_attachments = vec![first, second];
        app.invalidate();
        app.frame(1. / 60.);
        click(&mut app, &format!("preview-attachment-{first_id}"));
        assert_eq!(
            app.state.selected_attachment_preview().map(|item| item.id),
            Some(first_id)
        );
        let content = app
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("attachment-preview-text"))
            .unwrap()
            .text
            .unwrap();
        assert!(
            source.starts_with(&content)
                && content.starts_with("First snapshot")
                && content.len() < source.len()
        );
        assert!(app.elements().iter().any(|e| {
            e.text
                .as_deref()
                .is_some_and(|s| s.starts_with("Preview limited to"))
        }));
        let [x, y, w, h] = app.presented("attachment-preview").unwrap().0;
        assert!(x >= 0. && y >= 0. && x + w <= 541. && y + h <= 461.);
        press(&mut app, Key::Escape, Mods::default());
        assert!(app.state.selected_attachment_preview().is_none());
        click(&mut app, &format!("remove-attachment-{second_id}"));
        assert_eq!(app.state.staged_attachments.len(), 1);
        assert_eq!(app.state.staged_attachments[0].id, first_id);
        assert!(app.state.selected_attachment_preview().is_none());
        click(&mut app, &format!("preview-attachment-{first_id}"));
        click(&mut app, "attachment-preview-text-scroll");
        press(
            &mut app,
            Key::Enter,
            Mods {
                cmd: true,
                ..Mods::default()
            },
        );
        assert_eq!(app.state.composer_input.text, "Keep this draft.");
        assert_eq!(app.state.staged_attachments.len(), 1);
        click(&mut app, "attachment-preview-remove");
        assert!(
            app.state.selected_attachment_preview().is_none()
                && app.state.staged_attachments.is_empty()
        );
    }
}
