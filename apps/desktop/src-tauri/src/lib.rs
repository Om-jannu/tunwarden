//! The Tunwarden desktop app (docs/desktop.md). The Rust side is only a
//! bridge to the daemon: commands call `tunwarden_core::client`, a watcher
//! relays daemon events, and the tray mirrors the tunnels. The web view never
//! touches the control socket.

mod bridge;
mod tray;

use serde::Serialize;
use tauri::menu::MenuEvent;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Runtime, WindowEvent};
use tunwarden_core::client::Client;
use tunwarden_core::proto::{ActionResult, Selector, method};
use tunwarden_core::supervisor::State;

use bridge::Bridge;

/// Asks the window to confirm starting production tunnels (FR-33).
#[derive(Clone, Serialize)]
struct ConfirmStart {
    selector: Selector,
    production: Vec<String>,
}

pub fn run() {
    let bridge = match Bridge::new() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("tunwarden-desktop: {e}");
            std::process::exit(1);
        }
    };
    // `--background`: start in the tray without showing the window (login
    // autostart on Windows, D12).
    let background = std::env::args().any(|a| a == "--background");
    tauri::Builder::default()
        // A second launch shows the running app's window instead of opening
        // another copy. Registered first, as the plugin requires.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show(app)
        }))
        .plugin(tauri_plugin_notification::init())
        .manage(bridge)
        .invoke_handler(tauri::generate_handler![
            bridge::config_info,
            bridge::init_example,
            bridge::open_config,
            bridge::import_preview,
            bridge::import_apply,
            bridge::status,
            bridge::list,
            bridge::act,
            bridge::logs_tail,
            bridge::auth_status,
            bridge::auth_check,
            bridge::restart_daemon,
            bridge::quit,
            bridge::config_entries,
            bridge::save_entry,
            bridge::remove_entry,
            bridge::doctor,
            bridge::activity,
            bridge::autostart_status,
            bridge::autostart_set,
            bridge::login_run,
            bridge::login_in_terminal,
        ])
        .setup(move |app| {
            TrayIconBuilder::with_id(tray::TRAY_ID)
                .icon(tray::icon(tray::Health::Idle, false))
                .tooltip("Tunwarden")
                .menu(&tray::menu(app.handle(), None)?)
                .show_menu_on_left_click(true)
                .on_menu_event(on_menu)
                .on_tray_icon_event(|icon, event| {
                    // Windows and Linux: a double click opens the window.
                    if let TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    }
                    | TrayIconEvent::Click {
                        button: MouseButton::Middle,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show(icon.app_handle());
                    }
                })
                .build(app)?;
            if let Some(theme) = app.get_webview_window("main").and_then(|w| w.theme().ok()) {
                set_theme(app.handle(), theme);
            }
            tauri::async_runtime::spawn(bridge::watch(app.handle().clone()));
            if !background {
                show(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // Closing the window hides it; tunnels and the tray stay (D6).
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            // The tray icon's ink follows the system appearance.
            WindowEvent::ThemeChanged(theme) => set_theme(window.app_handle(), *theme),
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running the Tunwarden app");
}

fn set_theme<R: Runtime>(app: &AppHandle<R>, theme: tauri::Theme) {
    let bridge = app.state::<Bridge>();
    bridge.dark.store(
        matches!(theme, tauri::Theme::Dark),
        std::sync::atomic::Ordering::Relaxed,
    );
    let last = bridge
        .last
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    bridge::update_tray(app, last.as_ref());
}

fn show<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn on_menu<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let id = event.id().as_ref().to_owned();
    match id.as_str() {
        "open" => show(app),
        "quit" => {
            show(app);
            let _ = app.emit("quit-requested", ());
        }
        _ => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { menu_action(&app, &id).await });
        }
    }
}

/// A tunnel or group item: start or stop, asking in the window first when a
/// production tunnel would start.
async fn menu_action<R: Runtime>(app: &AppHandle<R>, id: &str) {
    let bridge = app.state::<Bridge>();
    let last = bridge
        .last
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let Some(status) = last else {
        show(app);
        return;
    };
    let (start, selector) = if let Some(name) = id.strip_prefix("tunnel:") {
        let active = status
            .tunnels
            .iter()
            .find(|t| t.name == name)
            .is_some_and(|t| !matches!(t.state, State::Stopped | State::Failed));
        (
            !active,
            Selector {
                names: vec![name.to_owned()],
                ..Selector::default()
            },
        )
    } else if let Some(g) = id.strip_prefix("group-start:") {
        (
            true,
            Selector {
                group: Some(g.to_owned()),
                ..Selector::default()
            },
        )
    } else if let Some(g) = id.strip_prefix("group-stop:") {
        (
            false,
            Selector {
                group: Some(g.to_owned()),
                ..Selector::default()
            },
        )
    } else {
        return;
    };
    if start {
        let production: Vec<String> = status
            .tunnels
            .iter()
            .filter(|t| t.production)
            .filter(|t| {
                selector.names.contains(&t.name)
                    || selector
                        .group
                        .as_ref()
                        .is_some_and(|g| t.groups.contains(g))
            })
            .map(|t| t.name.clone())
            .collect();
        if !production.is_empty() {
            show(app);
            let _ = app.emit(
                "confirm-start",
                ConfirmStart {
                    selector,
                    production,
                },
            );
            return;
        }
    }
    let rpc = if start { method::START } else { method::STOP };
    if let Ok(mut c) = Client::connect(&bridge.state_dir, "desktop-tray").await {
        let _: Result<ActionResult, _> = c.call(rpc, selector).await;
    }
}
