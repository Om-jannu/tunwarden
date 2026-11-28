//! The brand's pixel warden and portal, drawn in the terminal with half-block
//! characters: each character cell holds two pixels, the upper one as the
//! foreground of `▀` and the lower one as its background.
//!
//! Only shown at a few moments (a bare `tunwarden`, `init`, `doctor`, `login`,
//! `daemon stop`), never in output people scan, such as `status` or `logs`.
//! Off unless stdout is a colour terminal: pipes, scripts, CI, `NO_COLOR`,
//! `TERM=dumb` and `TUNWARDEN_NO_ART=1` get the text alone.

use std::io::IsTerminal;

#[rustfmt::skip]
mod drawings;

pub use drawings::{WARDEN_LANTERN, WARDEN_REST, WARDEN_WAVE};

/// A drawing: rows of palette indices (`0-9a-z`), `.` for transparent.
pub struct Art {
    pub palette: &'static [Ink],
    pub rows: &'static [&'static str],
}

#[derive(Clone, Copy)]
pub enum Ink {
    Rgb(u8, u8, u8),
    /// The lantern or portal lamp: filled in with the state it shows.
    Lamp,
}

/// Lamp colours, the same as the tray's.
pub const RUNNING: (u8, u8, u8) = (0x3c, 0xc2, 0x7a);
pub const FAILED: (u8, u8, u8) = (0xf0, 0x64, 0x5a);
pub const UNLIT: (u8, u8, u8) = (0x3a, 0x41, 0x50);

/// Whether art may be drawn on stdout.
pub fn enabled() -> bool {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
    std::io::stdout().is_terminal()
        && env("NO_COLOR").is_none()
        && env("CI").is_none()
        && env("TUNWARDEN_NO_ART").is_none()
        && std::env::var("TERM").map_or(true, |t| t != "dumb")
}

/// Whether the terminal takes 24-bit colour; otherwise the 256-colour palette.
fn truecolor() -> bool {
    if cfg!(windows) {
        return true;
    }
    matches!(
        std::env::var("COLORTERM").as_deref(),
        Ok("truecolor" | "24bit")
    )
}

/// Prints `text` beside `art`, or the text alone when art is off.
pub fn say(art: &Art, lamp: (u8, u8, u8), text: &[String]) {
    if enabled() {
        for line in beside(&render(art, lamp, truecolor()), text) {
            anstream::println!("{line}");
        }
    } else {
        for line in text {
            anstream::println!("{line}");
        }
    }
}

/// The drawing as terminal lines, two pixel rows per line.
pub fn render(art: &Art, lamp: (u8, u8, u8), rgb: bool) -> Vec<String> {
    let colour = |ch: char| -> Option<(u8, u8, u8)> {
        let i = ch.to_digit(36)? as usize;
        Some(match art.palette[i] {
            Ink::Rgb(r, g, b) => (r, g, b),
            Ink::Lamp => lamp,
        })
    };
    let width = art.rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut lines = Vec::new();
    for pair in art.rows.chunks(2) {
        let top: Vec<char> = pair[0].chars().collect();
        let bottom: Vec<char> = pair.get(1).map_or(Vec::new(), |r| r.chars().collect());
        let mut line = String::new();
        for x in 0..width {
            let t = top.get(x).copied().and_then(colour);
            let b = bottom.get(x).copied().and_then(colour);
            match (t, b) {
                (None, None) => line.push(' '),
                (Some(t), None) => line.push_str(&format!("{}▀\x1b[0m", fg(t, rgb))),
                (None, Some(b)) => line.push_str(&format!("{}▄\x1b[0m", fg(b, rgb))),
                (Some(t), Some(b)) => {
                    line.push_str(&format!("{}{}▀\x1b[0m", fg(t, rgb), bg(b, rgb)));
                }
            }
        }
        lines.push(line);
    }
    lines
}

/// `art` lines with `text` to their right, the text centred on the art.
fn beside(art: &[String], text: &[String]) -> Vec<String> {
    let rows = art.len().max(text.len());
    let art_pad = (rows - art.len()) / 2;
    let text_pad = (rows - text.len()) / 2;
    let blank = " ".repeat(art.first().map_or(0, |l| visible_width(l)));
    (0..rows)
        .map(|i| {
            let a = i
                .checked_sub(art_pad)
                .and_then(|j| art.get(j))
                .map_or(blank.as_str(), String::as_str);
            let t = i
                .checked_sub(text_pad)
                .and_then(|j| text.get(j))
                .map_or("", String::as_str);
            if t.is_empty() {
                a.trim_end().to_owned()
            } else {
                format!("{a}   {t}")
            }
        })
        .collect()
}

fn visible_width(line: &str) -> usize {
    let mut n = 0;
    let mut escape = false;
    for c in line.chars() {
        match (escape, c) {
            (false, '\x1b') => escape = true,
            (true, 'm') => escape = false,
            (true, _) => {}
            (false, _) => n += 1,
        }
    }
    n
}

fn fg((r, g, b): (u8, u8, u8), rgb: bool) -> String {
    if rgb {
        format!("\x1b[38;2;{r};{g};{b}m")
    } else {
        format!("\x1b[38;5;{}m", ansi256(r, g, b))
    }
}

fn bg((r, g, b): (u8, u8, u8), rgb: bool) -> String {
    if rgb {
        format!("\x1b[48;2;{r};{g};{b}m")
    } else {
        format!("\x1b[48;5;{}m", ansi256(r, g, b))
    }
}

/// The nearest colour in the 256-colour palette's 6×6×6 cube or grey ramp.
fn ansi256(r: u8, g: u8, b: u8) -> u8 {
    const STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let near = |v: u8| -> usize {
        STEPS
            .iter()
            .enumerate()
            .min_by_key(|(_, s)| (i32::from(**s) - i32::from(v)).abs())
            .map_or(0, |(i, _)| i)
    };
    let (ri, gi, bi) = (near(r), near(g), near(b));
    let cube = (STEPS[ri], STEPS[gi], STEPS[bi]);
    let dist = |(a, b2, c): (u8, u8, u8)| {
        let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
        d(a, r) + d(b2, g) + d(c, b)
    };
    let avg = (u16::from(r) + u16::from(g) + u16::from(b)) / 3;
    let grey_i = (avg.saturating_sub(3) / 10).min(23) as u8;
    let grey = 8 + grey_i * 10;
    if dist((grey, grey, grey)) < dist(cube) {
        232 + grey_i
    } else {
        16 + 36 * ri as u8 + 6 * gi as u8 + bi as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TINY: Art = Art {
        palette: &[Ink::Rgb(255, 0, 0), Ink::Lamp],
        rows: &["0.1", ".00"],
    };

    #[test]
    fn two_pixel_rows_make_one_line() {
        let lines = render(&TINY, (0, 0, 255), true);
        assert_eq!(lines.len(), 1);
        let l = &lines[0];
        // red over nothing, nothing over red, lamp (blue) over red
        assert!(l.starts_with("\x1b[38;2;255;0;0m▀\x1b[0m"), "{l:?}");
        assert!(l.contains("\x1b[38;2;255;0;0m▄\x1b[0m"), "{l:?}");
        assert!(
            l.ends_with("\x1b[38;2;0;0;255m\x1b[48;2;255;0;0m▀\x1b[0m"),
            "{l:?}"
        );
        assert_eq!(visible_width(l), 3);
    }

    #[test]
    fn text_sits_beside_the_art_centred() {
        let art: Vec<String> = ["AA", "BB", "CC", "DD"].map(String::from).to_vec();
        let out = beside(&art, &["hello".to_owned(), "world".to_owned()]);
        assert_eq!(out, ["AA", "BB   hello", "CC   world", "DD"]);
    }

    #[test]
    fn ansi256_picks_cube_and_greys() {
        assert_eq!(ansi256(255, 0, 0), 196);
        assert_eq!(ansi256(0, 0, 0), 16);
        assert_eq!(ansi256(128, 128, 128), 244);
    }

    #[test]
    fn every_drawing_uses_its_palette() {
        for art in [&WARDEN_WAVE, &WARDEN_LANTERN, &WARDEN_REST] {
            for row in art.rows {
                for ch in row.chars().filter(|c| *c != '.') {
                    let i = ch.to_digit(36).unwrap() as usize;
                    assert!(i < art.palette.len(), "{ch} in {row}");
                }
            }
            assert_eq!(render(art, RUNNING, true).len(), art.rows.len().div_ceil(2));
        }
    }
}
