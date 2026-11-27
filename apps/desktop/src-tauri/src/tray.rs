//! The tray (menu bar) icon and menu (FR-31, FR-32). The icon is a coloured
//! dot drawn at runtime, so no image files are needed per state.

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Runtime};
use tunwarden_core::proto::{StatusResult, TunnelView};
use tunwarden_core::supervisor::State;

pub const TRAY_ID: &str = "main";

/// Overall health, as the icon shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    /// Nothing is running, or the daemon is down.
    Idle,
    /// Every started tunnel is running.
    Good,
    /// Something is starting, reconnecting or unhealthy.
    Busy,
    /// Something failed (including an expired login).
    Bad,
}

pub fn health(status: Option<&StatusResult>) -> Health {
    let Some(s) = status else {
        return Health::Idle;
    };
    let any = |f: &dyn Fn(&TunnelView) -> bool| s.tunnels.iter().any(f);
    if any(&|t| t.state == State::Failed) {
        Health::Bad
    } else if any(&|t| {
        matches!(
            t.state,
            State::Starting | State::Reconnecting | State::Unhealthy
        )
    }) {
        Health::Busy
    } else if any(&|t| t.state == State::Running) {
        Health::Good
    } else {
        Health::Idle
    }
}

/// "3 running, 1 failed", or "nothing running".
pub fn summary(status: Option<&StatusResult>) -> String {
    let Some(s) = status else {
        return "daemon not running".into();
    };
    let count = |st: &[State]| s.tunnels.iter().filter(|t| st.contains(&t.state)).count();
    let parts: Vec<String> = [
        (count(&[State::Running]), "running"),
        (
            count(&[State::Starting, State::Reconnecting, State::Unhealthy]),
            "connecting",
        ),
        (count(&[State::Failed]), "failed"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{n} {what}"))
    .collect();
    if parts.is_empty() {
        "nothing running".into()
    } else {
        parts.join(", ")
    }
}

/// The tray icon: the logo's tunnel portal with its signal lamp, drawn as
/// pixel art on a 32×32 grid (16 pt at 2×). The arch is in the menu bar's
/// ink (`dark` = a dark menu bar); the lamp shows the health: green, amber,
/// red, or unlit (a ring) when idle. See docs/brand/.
pub fn icon(h: Health, dark: bool) -> Image<'static> {
    const SIZE: u32 = 32;
    let ink: [u8; 3] = if dark {
        [0xf2, 0xf4, 0xf7]
    } else {
        [0x1d, 0x21, 0x28]
    };
    let lamp: Option<[u8; 3]> = match h {
        Health::Idle => None,
        Health::Good => Some([0x3c, 0xc2, 0x7a]),
        Health::Busy => Some([0xe3, 0xa4, 0x3b]),
        Health::Bad => Some([0xf0, 0x64, 0x5a]),
    };
    let portal = |x: f32, y: f32, top: f32, half: f32| -> bool {
        let cy = top + half;
        if y >= cy {
            (x - 16.0).abs() < half
        } else {
            ((x - 16.0).powi(2) + (y - cy).powi(2)).sqrt() < half
        }
    };
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let d = ((px - 16.0).powi(2) + (py - 19.0).powi(2)).sqrt();
            let arch = portal(px, py, 1.0, 14.0) && !portal(px, py, 4.0, 11.0);
            let pixel: Option<[u8; 3]> = if arch {
                Some(ink)
            } else {
                match lamp {
                    Some(c) if d <= 5.2 => Some(if (x, y) == (14, 16) || (x, y) == (15, 16) {
                        [0xff, 0xff, 0xff]
                    } else {
                        c
                    }),
                    None if d <= 5.2 && d > 3.6 => Some(ink),
                    _ => None,
                }
            };
            match pixel {
                Some([r, g, b]) => rgba.extend_from_slice(&[r, g, b, 255]),
                None => rgba.extend_from_slice(&[0, 0, 0, 0]),
            }
        }
    }
    Image::new_owned(rgba, SIZE, SIZE)
}

/// The menu for the current status.
pub fn menu<R: Runtime>(
    app: &AppHandle<R>,
    status: Option<&StatusResult>,
) -> tauri::Result<Menu<R>> {
    let menu = Menu::new(app)?;
    menu.append(&MenuItem::with_id(
        app,
        "summary",
        summary(status),
        false,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    if let Some(s) = status {
        for t in &s.tunnels {
            let active = !matches!(t.state, State::Stopped | State::Failed);
            let label = match t.state {
                State::Running | State::Stopped => t.name.clone(),
                other => format!("{} ({})", t.name, other.as_str()),
            };
            menu.append(&CheckMenuItem::with_id(
                app,
                format!("tunnel:{}", t.name),
                label,
                true,
                active,
                None::<&str>,
            )?)?;
        }
        let mut groups: Vec<&String> = s.tunnels.iter().flat_map(|t| &t.groups).collect();
        groups.sort();
        groups.dedup();
        if !groups.is_empty() {
            let sub = Submenu::new(app, "Groups", true)?;
            for g in groups {
                sub.append(&MenuItem::with_id(
                    app,
                    format!("group-start:{g}"),
                    format!("Start {g}"),
                    true,
                    None::<&str>,
                )?)?;
                sub.append(&MenuItem::with_id(
                    app,
                    format!("group-stop:{g}"),
                    format!("Stop {g}"),
                    true,
                    None::<&str>,
                )?)?;
            }
            menu.append(&PredefinedMenuItem::separator(app)?)?;
            menu.append(&sub)?;
        }
        if !s.tunnels.is_empty() {
            menu.append(&PredefinedMenuItem::separator(app)?)?;
        }
    }
    menu.append(&MenuItem::with_id(
        app,
        "open",
        "Open Tunwarden",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "quit",
        "Quit Tunwarden…",
        true,
        None::<&str>,
    )?)?;
    Ok(menu)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tunwarden_core::state::Desired;

    fn view(name: &str, state: State) -> TunnelView {
        TunnelView {
            name: name.into(),
            kind: "command".into(),
            profile: None,
            production: false,
            groups: vec![],
            local: vec![],
            state,
            reason: None,
            last_error: None,
            pid: None,
            restarts: 0,
            retries: 0,
            since: String::new(),
            desired: Desired::Running,
            pending_restart: false,
            note: None,
        }
    }

    fn status(states: &[State]) -> StatusResult {
        StatusResult {
            tunnels: states
                .iter()
                .enumerate()
                .map(|(i, s)| view(&format!("t{i}"), *s))
                .collect(),
            config_errors: vec![],
            notes: vec![],
        }
    }

    #[test]
    fn health_and_summary() {
        assert_eq!(health(None), Health::Idle);
        assert_eq!(health(Some(&status(&[State::Stopped]))), Health::Idle);
        assert_eq!(
            health(Some(&status(&[State::Running, State::Stopped]))),
            Health::Good
        );
        assert_eq!(
            health(Some(&status(&[State::Running, State::Reconnecting]))),
            Health::Busy
        );
        assert_eq!(
            health(Some(&status(&[State::Starting, State::Failed]))),
            Health::Bad
        );
        assert_eq!(summary(None), "daemon not running");
        assert_eq!(summary(Some(&status(&[State::Stopped]))), "nothing running");
        assert_eq!(
            summary(Some(&status(&[
                State::Running,
                State::Running,
                State::Unhealthy,
                State::Failed
            ]))),
            "2 running, 1 connecting, 1 failed"
        );
    }

    #[test]
    fn icon_is_the_portal_with_a_state_lamp() {
        let px = |img: &Image<'_>, x: u32, y: u32| {
            let i = ((y * 32 + x) * 4) as usize;
            [
                img.rgba()[i],
                img.rgba()[i + 1],
                img.rgba()[i + 2],
                img.rgba()[i + 3],
            ]
        };
        let good = icon(Health::Good, false);
        assert_eq!((good.width(), good.height()), (32, 32));
        assert_eq!(px(&good, 0, 0)[3], 0, "transparent corner");
        assert_eq!(
            px(&good, 16, 2),
            [0x1d, 0x21, 0x28, 255],
            "the arch in dark ink"
        );
        assert_eq!(px(&good, 16, 19), [0x3c, 0xc2, 0x7a, 255], "a green lamp");
        assert_eq!(
            px(&icon(Health::Bad, true), 16, 2),
            [0xf2, 0xf4, 0xf7, 255],
            "light ink on a dark bar"
        );
        assert_eq!(
            px(&icon(Health::Bad, true), 16, 19),
            [0xf0, 0x64, 0x5a, 255]
        );
        let idle = icon(Health::Idle, false);
        assert_eq!(px(&idle, 16, 19)[3], 0, "an unlit lamp is hollow");
        assert_eq!(px(&idle, 16, 14)[3], 255, "its ring is drawn");
    }

    /// Writes the tray icons as raw RGBA for docs/brand/tray-preview.py when
    /// TUNWARDEN_ICON_PREVIEW names a directory.
    #[test]
    fn preview() {
        let Some(dir) = std::env::var_os("TUNWARDEN_ICON_PREVIEW") else {
            return;
        };
        for (h, name) in [
            (Health::Idle, "idle"),
            (Health::Good, "running"),
            (Health::Busy, "connecting"),
            (Health::Bad, "failed"),
        ] {
            for (dark, theme) in [(false, "light"), (true, "dark")] {
                let path = std::path::Path::new(&dir).join(format!("{name}-{theme}.rgba"));
                std::fs::write(path, icon(h, dark).rgba()).unwrap();
            }
        }
    }
}
