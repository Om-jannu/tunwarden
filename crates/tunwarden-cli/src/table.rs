//! Tables without borders, columns two spaces apart. On a terminal they fit
//! its width and wrap the last columns; elsewhere lines are never wrapped, so
//! scripts can split them. Cells may carry colour codes.

use std::fmt;

use comfy_table::{CellAlignment, ContentArrangement, Table as Inner};

pub struct Table {
    inner: Inner,
}

impl Table {
    pub fn new(header: &[&str]) -> Table {
        let mut inner = Inner::new();
        inner
            .load_style(comfy_table::presets::NOTHING)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(header.iter().map(|h| crate::ui::paint(crate::ui::BOLD, h)));
        inner.column_count();
        let last = header.len().saturating_sub(1);
        for (i, column) in inner.column_iter_mut().enumerate() {
            column.set_padding((0, if i == last { 0 } else { 2 }));
            column.set_cell_alignment(CellAlignment::Left);
        }
        Table { inner }
    }

    pub fn row(&mut self, cells: Vec<String>) {
        self.inner.add_row(cells);
    }
}

/// Below this width, wrapping would squash columns; lines run long instead.
const MIN_WRAP_WIDTH: u16 = 60;

impl fmt::Display for Table {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut inner = self.inner.clone();
        if inner.width().is_none_or(|w| w < MIN_WRAP_WIDTH) {
            inner.set_content_arrangement(ContentArrangement::Disabled);
        }
        for line in inner.lines() {
            let line = line.trim_end();
            if !line.trim().is_empty() {
                writeln!(f, "{line}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(s: String) -> String {
        let mut out = anstream::StripStream::new(Vec::new());
        std::io::Write::write_all(&mut out, s.as_bytes()).unwrap();
        String::from_utf8(out.into_inner()).unwrap()
    }

    #[test]
    fn aligns_columns() {
        let mut t = Table::new(&["NAME", "STATE", "DETAIL"]);
        t.row(vec!["staging-db".into(), "running".into(), String::new()]);
        t.row(vec![
            "redis".into(),
            crate::ui::paint(crate::ui::RED, "failed"),
            "port conflict".into(),
        ]);
        assert_eq!(
            plain(t.to_string()),
            "NAME        STATE    DETAIL\nstaging-db  running\nredis       failed   port conflict\n"
        );
    }
}
