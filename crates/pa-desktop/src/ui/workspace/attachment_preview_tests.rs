//! Drive the real attachment worker while controlling when its completions reach the UI.

use std::fs;
use std::time::{Duration, Instant};

use image::DynamicImage;
use tempfile::TempDir;

use super::{LaunchOptions, PrimeDesktopView};

fn wait_for(view: &mut PrimeDesktopView, ready: impl Fn(&PrimeDesktopView) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        view.drain_events();
        if ready(view) {
            return;
        }
        assert!(Instant::now() < deadline, "attachment worker timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn expanded_pixels_follow_only_the_current_attachment_and_draft() {
    let profile = TempDir::new().unwrap();
    let first = profile.path().join("landscape.png");
    let second = profile.path().join("portrait.png");
    DynamicImage::new_rgba8(320, 160).save(&first).unwrap();
    DynamicImage::new_rgba8(180, 300).save(&second).unwrap();
    let mut view = PrimeDesktopView::new(
        LaunchOptions {
            initial_cwd: Some(profile.path().to_owned()),
            daemon_socket: Some(profile.path().join("isolated.sock")),
            ..LaunchOptions::default()
        },
        profile.path().to_owned(),
    );
    wait_for(&mut view, |s| s.toast.is_some());
    view.attach_files(vec![first, second]);
    wait_for(&mut view, |s| s.staged_attachments.len() == 2);
    let landscape = view.staged_attachments[0].id;
    let portrait = view.staged_attachments[1].id;
    assert!(!view.image_decode_in_progress && view.expanded_attachment.is_none());

    // Switch before the first decode's completion can be delivered.
    view.open_attachment_preview(landscape);
    assert!(view.image_decode_in_progress);
    view.open_attachment_preview(portrait);
    assert!(view.expanded_attachment.is_none());
    wait_for(&mut view, |s| !s.image_decode_in_progress);
    assert!(view.expanded_attachment_pixels(landscape).is_none());
    assert_eq!(
        view.expanded_attachment_pixels(portrait).unwrap().size(),
        (180, 300)
    );

    // Opening a new image immediately releases the previous raster. Closing its
    // viewer while the worker is running must not resurrect the closed image.
    view.open_attachment_preview(landscape);
    assert!(view.expanded_attachment_pixels(portrait).is_none());
    view.close_attachment_preview();
    wait_for(&mut view, |s| !s.image_decode_in_progress);
    assert!(view.expanded_attachment.is_none() && view.selected_attachment_preview().is_none());

    view.open_attachment_preview(landscape);
    view.remove_attachment(landscape);
    wait_for(&mut view, |s| !s.image_decode_in_progress);
    assert!(view.expanded_attachment.is_none() && view.selected_attachment_preview().is_none());

    view.open_attachment_preview(portrait);
    let other = profile.path().join("other");
    fs::create_dir(&other).unwrap();
    view.new_project_chat(other.to_str().unwrap());
    wait_for(&mut view, |s| !s.image_decode_in_progress);
    assert!(view.expanded_attachment.is_none() && view.selected_attachment_preview().is_none());
    view.new_project_chat(profile.path().to_str().unwrap());
    assert_eq!(view.staged_attachments[0].id, portrait);
    assert!(!view.image_decode_in_progress && view.expanded_attachment.is_none());
}
