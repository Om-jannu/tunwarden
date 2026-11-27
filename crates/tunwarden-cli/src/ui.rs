//! How the CLI looks and asks: colours, prompts, progress and error display.
//!
//! Output always carries colour codes and is printed with `anstream`, which
//! removes them when stdout or stderr is not a terminal, `NO_COLOR` is set or
//! `TERM=dumb`. Prompts only appear when [`interactive`] is true, so scripts,
//! pipes and CI behave exactly as without them.

use std::io::IsTerminal;
use std::path::Path;

use anstyle::{AnsiColor, Style};
use tunwarden_core::config::ConfigErrors;
use tunwarden_core::supervisor::State;

pub const BOLD: Style = Style::new().bold();
pub const DIM: Style = Style::new().dimmed();
pub const GREEN: Style = AnsiColor::Green.on_default();
pub const YELLOW: Style = AnsiColor::Yellow.on_default();
pub const RED: Style = AnsiColor::Red.on_default();
pub const CYAN: Style = AnsiColor::Cyan.on_default();

/// `text` in `style`. The codes are removed on output that is not a terminal.
pub fn paint(style: Style, text: impl std::fmt::Display) -> String {
    format!("{style}{text}{style:#}")
}

pub fn state_style(state: State) -> Style {
    match state {
        State::Running => GREEN,
        State::Starting | State::Reconnecting | State::Unhealthy => YELLOW,
        State::Failed => RED,
        State::Stopped => DIM,
    }
}

pub fn state(state: State) -> String {
    paint(state_style(state), state.as_str())
}

/// [`state`] padded to `width` characters, so columns line up with colour on.
pub fn state_padded(state: State, width: usize) -> String {
    paint(state_style(state), format!("{:<width$}", state.as_str()))
}

/// Whether the CLI may ask questions: stdin and stderr are terminals and it is
/// not running in CI.
pub fn interactive() -> bool {
    std::io::stdin().is_terminal()
        && std::io::stderr().is_terminal()
        && std::env::var_os("CI").is_none()
}

/// Whether live progress may be drawn on stderr.
pub fn live() -> bool {
    std::io::stderr().is_terminal() && std::env::var_os("CI").is_none()
}

fn theme() -> dialoguer::theme::ColorfulTheme {
    dialoguer::theme::ColorfulTheme::default()
}

/// Ctrl-C or Esc in a prompt.
pub struct Cancelled;

/// Picks one item, with type-to-filter. `Cancelled` on Ctrl-C or Esc.
pub fn select(prompt: &str, items: &[String]) -> Result<usize, Cancelled> {
    dialoguer::FuzzySelect::with_theme(&theme())
        .with_prompt(prompt)
        .items(items)
        .default(0)
        .interact_opt()
        .ok()
        .flatten()
        .ok_or(Cancelled)
}

/// Picks several items; `checked` gives the starting selection.
pub fn multi_select(
    prompt: &str,
    items: &[String],
    checked: &[bool],
) -> Result<Vec<usize>, Cancelled> {
    dialoguer::MultiSelect::with_theme(&theme())
        .with_prompt(format!("{prompt} (space to toggle, enter to accept)"))
        .items(items)
        .defaults(checked)
        .interact_opt()
        .ok()
        .flatten()
        .ok_or(Cancelled)
}

pub fn confirm(prompt: &str) -> Result<bool, Cancelled> {
    dialoguer::Confirm::with_theme(&theme())
        .with_prompt(prompt)
        .default(false)
        .interact_opt()
        .ok()
        .flatten()
        .ok_or(Cancelled)
}

/// Asks for a line of text. An empty answer is allowed when `optional`.
pub fn input(prompt: &str, optional: bool) -> Result<String, Cancelled> {
    dialoguer::Input::<String>::with_theme(&theme())
        .with_prompt(prompt)
        .allow_empty(optional)
        .interact_text()
        .map_err(|_| Cancelled)
}

/// Config errors drawn with the offending line and a pointer, for a terminal.
/// `None` when the file cannot be read again; the caller then prints the
/// plain list.
pub fn render_config_errors(path: &Path, errors: &ConfigErrors) -> Option<String> {
    use miette::{
        GraphicalReportHandler, GraphicalTheme, LabeledSpan, MietteDiagnostic, NamedSource,
    };
    let text = std::fs::read_to_string(path).ok()?;
    let handler = GraphicalReportHandler::new_themed(GraphicalTheme::unicode());
    let mut out = String::new();
    for e in &errors.0 {
        let offset = offset_of(&text, e.position.line, e.position.column);
        let len = token_len(&text[offset..]);
        let label = if e.field.is_empty() {
            "here".to_owned()
        } else {
            e.field.clone()
        };
        let diag = MietteDiagnostic::new(e.message.clone())
            .with_label(LabeledSpan::at(offset..offset + len, label));
        let report = miette::Report::new(diag).with_source_code(
            NamedSource::new(path.display().to_string(), text.clone()).with_language("toml"),
        );
        handler.render_report(&mut out, report.as_ref()).ok()?;
    }
    Some(out)
}

/// The byte offset of a 1-based line and character column.
fn offset_of(text: &str, line: usize, column: usize) -> usize {
    let mut offset = 0;
    for (i, l) in text.split_inclusive('\n').enumerate() {
        if i + 1 == line {
            let col: usize = l
                .char_indices()
                .nth(column.saturating_sub(1))
                .map_or(l.trim_end_matches('\n').len(), |(b, _)| b);
            return offset + col;
        }
        offset += l.len();
    }
    text.len()
}

/// How much of the text at an error to underline: up to the end of the value.
fn token_len(rest: &str) -> usize {
    let line = rest.lines().next().unwrap_or("");
    let line = line.split(" #").next().unwrap_or(line).trim_end();
    line.len().max(1).min(rest.len().max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_count_characters() {
        let text = "version = 1\nname = \"é b\"\n";
        assert_eq!(offset_of(text, 1, 1), 0);
        assert_eq!(offset_of(text, 2, 8), 19);
        assert_eq!(&text[offset_of(text, 2, 9)..offset_of(text, 2, 9) + 2], "é");
        assert_eq!(offset_of(text, 9, 1), text.len());
        assert_eq!(token_len("\"x\"  # note\nnext"), 3);
    }

    #[test]
    fn painted_text_is_plain_without_a_terminal() {
        let s = paint(GREEN, "running");
        assert!(s.contains("running") && s.contains('\u{1b}'));
        let mut out = anstream::StripStream::new(Vec::new());
        std::io::Write::write_all(&mut out, s.as_bytes()).unwrap();
        assert_eq!(out.into_inner(), b"running");
    }
}
