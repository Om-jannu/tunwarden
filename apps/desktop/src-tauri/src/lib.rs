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
use tauri::{PhysicalPosition, Rect};
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
            open_main,
            hide_popover,
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
                // A left click opens the popover and a right click the menu.
                // Linux trays report no clicks, so there the menu opens on any.
                .show_menu_on_left_click(cfg!(target_os = "linux"))
                .on_menu_event(on_menu)
                .on_tray_icon_event(|icon, event| match event {
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        rect,
                        ..
                    } => toggle_popover(icon.app_handle(), rect),
                    TrayIconEvent::Click {
                        button: MouseButton::Middle,
                        button_state: MouseButtonState::Up,
                        ..
                    } => show(icon.app_handle()),
                    _ => {}
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
            // The popover closes as soon as it loses focus, like a menu.
            WindowEvent::Focused(false) if window.label() == POPOVER => {
                if window.is_visible().unwrap_or(false) {
                    *BLURRED_AT.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(std::time::Instant::now());
                }
                let _ = window.hide();
            }
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

const POPOVER: &str = "popover";

/// When the popover last hid because it lost focus. Clicking the tray icon to
/// close it first takes its focus, then delivers the click: without this the
/// click would open it again at once.
static BLURRED_AT: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// Shows the popover next to the tray icon, or hides it when it is open.
/// It opens below an icon in a top bar (macOS) and above one in a bottom
/// taskbar (Windows), kept inside the screen.
fn toggle_popover<R: Runtime>(app: &AppHandle<R>, icon: Rect) {
    let Some(w) = app.get_webview_window(POPOVER) else {
        return;
    };
    if w.is_visible().unwrap_or(false) {
        let _ = w.hide();
        return;
    }
    let just_closed = BLURRED_AT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(300));
    if just_closed {
        return;
    }
    let scale = w.scale_factor().unwrap_or(1.0);
    let at = icon.position.to_physical::<f64>(scale);
    let size = icon.size.to_physical::<f64>(scale);
    let pop = w.outer_size().unwrap_or_default();
    let (pw, ph) = (f64::from(pop.width), f64::from(pop.height));
    let gap = 6.0 * scale;
    let mut x = at.x + size.width / 2.0 - pw / 2.0;
    let mut y = at.y + size.height + gap;
    if let Ok(Some(m)) = w.monitor_from_point(at.x, at.y) {
        let area = m.work_area();
        let (left, top) = (f64::from(area.position.x), f64::from(area.position.y));
        let (right, bottom) = (
            left + f64::from(area.size.width),
            top + f64::from(area.size.height),
        );
        if at.y > (top + bottom) / 2.0 {
            y = at.y - ph - gap;
        }
        x = x.clamp(left + gap, (right - pw - gap).max(left + gap));
        y = y.clamp(top + gap, (bottom - ph - gap).max(top + gap));
    }
    let _ = w.set_position(PhysicalPosition::new(x, y));
    let _ = w.show();
    let _ = w.set_focus();
}

/// From the popover: opens the main window, optionally straight into a login.
#[tauri::command]
fn open_main<R: Runtime>(app: AppHandle<R>, login: Option<String>) {
    if let Some(p) = app.get_webview_window(POPOVER) {
        let _ = p.hide();
    }
    show(&app);
    if let Some(name) = login {
        let _ = app.emit_to("main", "open-login", name);
    }
}

#[tauri::command]
fn hide_popover<R: Runtime>(app: AppHandle<R>) {
    if let Some(p) = app.get_webview_window(POPOVER) {
        let _ = p.hide();
    }
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
            let _ = app.emit_to("main", "quit-requested", ());
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
            let _ = app.emit_to(
                "main",
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
