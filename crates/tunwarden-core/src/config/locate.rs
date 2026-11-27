//! Turning a field path such as `tunnels[2].forwards[0].local_port` into a
//! line and column in the config file.

use std::fmt;
use std::ops::Range;

use toml_edit::{Document, Item, Table, Value};

/// One step in a field path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seg {
    Key(String),
    Index(usize),
}

/// A path to a field in the config file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldPath(pub Vec<Seg>);

impl FieldPath {
    pub fn root() -> Self {
        FieldPath(Vec::new())
    }

    pub fn key(&self, key: &str) -> Self {
        let mut segs = self.0.clone();
        segs.push(Seg::Key(key.to_owned()));
        FieldPath(segs)
    }

    pub fn index(&self, index: usize) -> Self {
        let mut segs = self.0.clone();
        segs.push(Seg::Index(index));
        FieldPath(segs)
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for FieldPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, seg) in self.0.iter().enumerate() {
            match seg {
                Seg::Key(key) if i == 0 => write!(f, "{key}")?,
                Seg::Key(key) => write!(f, ".{key}")?,
                Seg::Index(index) => write!(f, "[{index}]")?,
            }
        }
        Ok(())
    }
}

/// A 1-based line and column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

impl Position {
    pub const START: Position = Position { line: 1, column: 1 };

    /// The position of byte `offset` in `source`. Columns count characters.
    pub fn of_offset(source: &str, offset: usize) -> Position {
        let offset = offset.min(source.len());
        let before = source.get(..offset).unwrap_or(source);
        let line = before.matches('\n').count() + 1;
        let line_start = before.rfind('\n').map_or(0, |i| i + 1);
        let column = before[line_start..].chars().count() + 1;
        Position { line, column }
    }
}

/// Finds field paths in a parsed document.
pub struct Locator<'a> {
    source: &'a str,
    doc: Option<Document<&'a str>>,
}

enum Node<'a> {
    Table(&'a Table),
    Item(&'a Item),
    Value(&'a Value),
}

impl<'a> Locator<'a> {
    pub fn new(source: &'a str) -> Self {
        Locator {
            source,
            doc: Document::parse(source).ok(),
        }
    }

    /// The position of the field at `path`. If the field itself is missing,
    /// the position of its closest existing parent.
    pub fn position(&self, path: &FieldPath) -> Position {
        self.span(path).map_or(Position::START, |span| {
            Position::of_offset(self.source, span.start)
        })
    }

    fn span(&self, path: &FieldPath) -> Option<Range<usize>> {
        let doc = self.doc.as_ref()?;
        let mut node = Node::Table(doc.as_table());
        let mut best = None;
        for seg in &path.0 {
            let (next, key_span) = match (&node, seg) {
                (Node::Table(t), Seg::Key(k)) => {
                    (t.get(k).map(Node::Item), t.key(k).and_then(|k| k.span()))
                }
                (Node::Item(Item::Table(t)), Seg::Key(k)) => {
                    (t.get(k).map(Node::Item), t.key(k).and_then(|k| k.span()))
                }
                (Node::Item(Item::Value(Value::InlineTable(t))), Seg::Key(k))
                | (Node::Value(Value::InlineTable(t)), Seg::Key(k)) => {
                    (t.get(k).map(Node::Value), t.key(k).and_then(|k| k.span()))
                }
                (Node::Item(Item::ArrayOfTables(a)), Seg::Index(i)) => {
                    (a.get(*i).map(Node::Table), None)
                }
                (Node::Item(Item::Value(Value::Array(a))), Seg::Index(i))
                | (Node::Value(Value::Array(a)), Seg::Index(i)) => {
                    (a.get(*i).map(Node::Value), None)
                }
                _ => (None, None),
            };
            let Some(next) = next else { break };
            best = node_span(&next).or(key_span).or(best);
            node = next;
        }
        best
    }
}

fn node_span(node: &Node<'_>) -> Option<Range<usize>> {
    match node {
        Node::Table(t) => t.span(),
        Node::Item(item) => item.span(),
        Node::Value(value) => value.span(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "version = 1\n\n[[tunnels]]\nname = \"a\"\nforwards = [{ local_port = 1 }, { local_port = 2 }]\n\n[[tunnels]]\nname = \"b\"\n";

    fn path(segs: &[Seg]) -> FieldPath {
        FieldPath(segs.to_vec())
    }

    fn k(s: &str) -> Seg {
        Seg::Key(s.to_owned())
    }

    #[test]
    fn display() {
        let p = FieldPath::root()
            .key("tunnels")
            .index(2)
            .key("forwards")
            .index(0)
            .key("local_port");
        assert_eq!(p.to_string(), "tunnels[2].forwards[0].local_port");
    }

    #[test]
    fn offsets_to_positions() {
        assert_eq!(
            Position::of_offset("ab\ncd", 0),
            Position { line: 1, column: 1 }
        );
        assert_eq!(
            Position::of_offset("ab\ncd", 4),
            Position { line: 2, column: 2 }
        );
        assert_eq!(
            Position::of_offset("é\nx", 3),
            Position { line: 2, column: 1 }
        );
    }

    #[test]
    fn finds_nested_fields() {
        let loc = Locator::new(SRC);
        assert_eq!(loc.position(&path(&[k("version")])).line, 1);
        assert_eq!(
            loc.position(&path(&[k("tunnels"), Seg::Index(1), k("name")])),
            Position { line: 8, column: 8 }
        );
        let second_port = loc.position(&path(&[
            k("tunnels"),
            Seg::Index(0),
            k("forwards"),
            Seg::Index(1),
            k("local_port"),
        ]));
        assert_eq!(second_port.line, 5);
        assert_eq!(second_port.column, 48);
    }

    #[test]
    fn missing_field_falls_back_to_parent() {
        let loc = Locator::new(SRC);
        let p = loc.position(&path(&[k("tunnels"), Seg::Index(1), k("type")]));
        assert_eq!(p.line, 7);
    }
}
