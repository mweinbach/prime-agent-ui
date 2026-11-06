//! A single set of commands for Rune keyboard events and the native macOS menus.
use rune::{App, Key, KeyEvent, MenuItem, Tx};

use crate::state::ActiveModal;
use crate::ui::workspace::PrimeDesktopView as Workspace;

struct Binding {
    menu: Option<&'static str>,
    title: &'static str,
    key: char,
    shift: bool,
    run: fn(&mut Workspace, &mut Tx),
}

impl Binding {
    const fn new(
        menu: Option<&'static str>,
        title: &'static str,
        key: char,
        run: fn(&mut Workspace, &mut Tx),
    ) -> Self {
        Self {
            menu,
            title,
            key,
            shift: false,
            run,
        }
    }

    const fn shifted(mut self) -> Self {
        self.shift = true;
        self
    }
}

const SESSION: Option<&str> = Some("Session");
const VIEW: Option<&str> = Some("View");

const BINDINGS: &[Binding] = &[
    Binding::new(None, "Settings & Providers…", ',', |s, _| {
        s.open_settings_modal();
    }),
    Binding::new(SESSION, "New Chat in Project", 'n', |s, _| {
        s.start_fresh_session();
    }),
    Binding::new(SESSION, "Add Project…", 'o', |s, _| {
        s.prompt_choose_project_dir();
    })
    .shifted(),
    Binding::new(SESSION, "Quick Switcher…", 'k', |s, _| {
        s.open_modal(ActiveModal::QuickSwitcher);
    }),
    Binding::new(SESSION, "Select Model…", 'm', |s, _| {
        s.open_modal(ActiveModal::ModelPicker);
    }),
    Binding::new(SESSION, "Choose Reasoning Effort…", 'e', |s, _| {
        s.open_reasoning_picker();
    })
    .shifted(),
    Binding::new(SESSION, "Side Question (/btw)", '/', |s, _| {
        s.toggle_side_question();
    }),
    Binding::new(SESSION, "Compact Context", 'k', |s, _| s.trigger_compact()).shifted(),
    Binding::new(SESSION, "Refine Continual Harness", 'r', |s, _| {
        s.trigger_refine();
    })
    .shifted(),
    Binding::new(VIEW, "Toggle Sidebar", 'b', |s, _| s.toggle_sidebar()),
    Binding::new(VIEW, "Toggle Harness Inspector", 'i', |s, _| {
        s.toggle_inspector();
    }),
    Binding::new(VIEW, "Open Continual Harness Studio", 'h', |s, _| {
        s.open_harness_tab();
    })
    .shifted(),
    Binding::new(VIEW, "Cycle Transcript Detail", 'o', |s, _| {
        s.cycle_detail_level();
    }),
    Binding::new(VIEW, "Toggle Light / Dark Theme", 'l', |s, _| {
        s.toggle_theme();
    })
    .shifted(),
];

pub(super) fn handle_key(state: &mut Workspace, event: &KeyEvent, tx: &mut Tx) -> bool {
    if event.key == Key::Escape && event.mods == rune::Mods::default() {
        return state.dismiss_overlay();
    }
    // Rune maps the platform's primary modifier to cmd on every OS. Leave text
    // editing, selection, IME, and AltGr combinations to the focused control.
    let Key::Command(key) = &event.key else {
        return false;
    };
    if event.mods.alt
        || (cfg!(target_os = "macos") && event.mods.ctrl)
        || (!cfg!(target_os = "macos") && event.mods.logo)
    {
        return false;
    }
    if *key == 'q' && !event.mods.shift {
        tx.quit();
        return true;
    }
    if let Some(binding) = BINDINGS
        .iter()
        .find(|b| b.key == *key && b.shift == event.mods.shift)
    {
        if state.selected_attachment_preview().is_none() {
            (binding.run)(state, tx);
        }
        return true;
    }
    false
}

pub(super) fn install_menus(mut app: App<Workspace>) -> App<Workspace> {
    if cfg!(target_os = "macos") || rune::is_headless() {
        for binding in BINDINGS {
            let shift = if binding.shift { "shift+" } else { "" };
            let chord = format!("cmd+{shift}{}", binding.key);
            let mut item = MenuItem::new(binding.title, binding.run)
                .key_equivalent(chord.parse().expect("valid desktop keyboard shortcut"))
                .keys_first();
            if let Some(menu) = binding.menu {
                item = item.menu(menu);
            }
            app = app.menu_item(item);
        }
    }
    app
}
