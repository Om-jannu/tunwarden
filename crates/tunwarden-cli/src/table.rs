//! Plain text tables: columns padded to the widest cell, two spaces apart.
//! The last column is not padded, so long details do not leave trailing spaces.

use std::fmt;

pub struct Table {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(header: &[&str]) -> Table {
        Table {
            header: header.iter().map(|h| h.to_string()).collect(),
            rows: Vec::new(),
        }
    }

    pub fn row(&mut self, cells: Vec<String>) {
        self.rows.push(cells);
    }
}

impl fmt::Display for Table {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let columns = self.header.len();
        let mut widths: Vec<usize> = self.header.iter().map(|h| h.chars().count()).collect();
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate().take(columns) {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }
        for row in std::iter::once(&self.header).chain(&self.rows) {
            let mut line = String::new();
            for (i, cell) in row.iter().enumerate().take(columns) {
                if i + 1 == columns {
                    line.push_str(cell);
                } else {
                    line.push_str(cell);
                    line.push_str(&" ".repeat(widths[i] - cell.chars().count() + 2));
                }
            }
            writeln!(f, "{}", line.trim_end())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_columns() {
        let mut t = Table::new(&["NAME", "STATE", "DETAIL"]);
        t.row(vec!["staging-db".into(), "running".into(), String::new()]);
        t.row(vec![
            "redis".into(),
            "failed".into(),
            "port conflict".into(),
        ]);
        assert_eq!(
            t.to_string(),
            "NAME        STATE    DETAIL\nstaging-db  running\nredis       failed   port conflict\n"
        );
    }
}
