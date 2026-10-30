use std::sync::Arc;

use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::commands::app::quit_app;
use crate::state::{Cmd, Shared};
use crate::{lang, mascot, screenshots, ui_windows};

fn menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let language = {
        let shared = app.state::<Arc<Shared>>();
        let setting = shared.config.lock().unwrap().str("language");
        lang::resolve(&setting)
    };
    let label = |text| lang::tr(text, language);
    let open = MenuItem::with_id(app, "open", label("Show"), true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", label("Pause / resume presence"), true, None::<&str>)?;
    let screenshot =
        MenuItem::with_id(app, "screenshot", label("Take screenshot"), true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", label("Quit"), true, None::<&str>)?;
    Menu::with_items(app, &[&open, &pause, &screenshot, &separator, &quit])
}

pub fn refresh(app: &AppHandle) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id("main"), menu(app)) {
        let _ = tray.set_menu(Some(menu));
    }
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let menu = menu(app)?;

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().expect("app icon"))
        .tooltip("Shiori RPC")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            let left_click = matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            );
            if left_click {
                ui_windows::show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

pub fn handle_menu(app: &AppHandle, event: MenuEvent) {
    let shared = Arc::clone(&app.state::<Arc<Shared>>());

    match event.id.as_ref() {
        "open" | "mascot-open" => ui_windows::show_main(app),
        "pause" => {
            let paused = shared.snapshot().paused;
            shared.send(Cmd::Pause(!paused));
        }
        "screenshot" => {
            std::thread::spawn(move || screenshots::take(&shared));
        }
        "quit" => quit_app(app, &shared),
        "mascot-pick" => ui_windows::open_page(app, "wishlist"),
        id if id.starts_with(mascot::HIDE_MENU_PREFIX) => {
            mascot::hide(app, &id[mascot::HIDE_MENU_PREFIX.len()..])
        }
        _ => {}
    }
}

pub fn closed_once(app: &AppHandle) {
    let shared = app.state::<Arc<Shared>>();
    let language = {
        let mut cfg = shared.config.lock().unwrap();
        if cfg.bool("tray_hint_shown") {
            return;
        }
        let mut changes = serde_json::Map::new();
        changes.insert("tray_hint_shown".into(), true.into());
        let _ = cfg.update(changes);
        lang::resolve(&cfg.str("language"))
    };
    let toast = ui_windows::Toast {
        title: lang::tr("Still running in the tray", language).to_string(),
        detail: lang::tr("Right-click its icon to quit", language).to_string(),
        ok: true,
        action: "open".into(),
        ..Default::default()
    };
    ui_windows::show_toast(app, toast, None);
}
