//! Comment-preserving edits of the config text (PRD FR-5, design 3.3).
//!
//! Edits work on a `toml_edit::DocumentMut`, so comments, blank lines and key
//! order elsewhere in the file stay as the user wrote them. Tunnels are found
//! by name, never by index, so a hand edit that reordered the file cannot make
//! an edit hit the wrong tunnel. The caller validates the result before
//! writing it.

use serde_json::Value as Json;
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("the config file is not valid TOML: {0}")]
    Syntax(String),
    #[error("no tunnel named \"{0}\" in the config file")]
    UnknownTunnel(String),
    #[error("a {0} named \"{1}\" already exists")]
    Exists(&'static str, String),
    #[error("`{field}`: {problem}")]
    Value { field: String, problem: String },
    #[error("`{0}` cannot be changed here; use the tunnel name to pick the tunnel")]
    Protected(String),
}

/// The text for a new config file.
pub const NEW_FILE: &str = "version = 1\n";

/// Fields written first, in this order, so new entries read naturally.
const LEADING: [&str; 4] = ["name", "type", "profile", "groups"];

pub fn parse(text: &str) -> Result<DocumentMut, EditError> {
    text.parse::<DocumentMut>()
        .map_err(|e| EditError::Syntax(e.to_string().trim().to_owned()))
}

/// Appends a `[[tunnels]]` entry with `fields` (without `name`).
pub fn add_tunnel(
    doc: &mut DocumentMut,
    name: &str,
    fields: &serde_json::Map<String, Json>,
) -> Result<(), EditError> {
    if find(doc, name).is_some() {
        return Err(EditError::Exists("tunnel", name.to_owned()));
    }
    let mut table = Table::new();
    table.insert("name", toml_edit::value(name));
    let mut keys: Vec<&String> = fields.keys().collect();
    keys.sort_by_key(|k| {
        let lead = LEADING.iter().position(|l| l == k).unwrap_or(LEADING.len());
        (lead, *k == "forwards", k.to_string())
    });
    for key in keys {
        if key == "name" {
            continue;
        }
        table.insert(key, Item::Value(to_toml(key, &fields[key])?));
    }
    if !doc.contains_key("tunnels") {
        doc.insert("tunnels", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    match doc.get_mut("tunnels") {
        Some(Item::ArrayOfTables(tunnels)) => {
            tunnels.push(table);
            Ok(())
        }
        _ => Err(EditError::Value {
            field: "tunnels".into(),
            problem: "must be written as [[tunnels]] entries to be edited here".into(),
        }),
    }
}

/// Appends a `[[logins]]` or `[[profiles]]` entry with `fields` (without
/// `name`). `table` is `logins` or `profiles`.
pub fn add_entry(
    doc: &mut DocumentMut,
    table: &str,
    name: &str,
    fields: &serde_json::Map<String, Json>,
) -> Result<(), EditError> {
    let exists = doc
        .get(table)
        .and_then(Item::as_array_of_tables)
        .is_some_and(|ps| {
            ps.iter()
                .any(|p| p.get("name").and_then(Item::as_str) == Some(name))
        });
    if exists {
        let what = if table == "logins" {
            "login"
        } else {
            "profile"
        };
        return Err(EditError::Exists(what, name.to_owned()));
    }
    let mut entry = Table::new();
    entry.insert("name", toml_edit::value(name));
    for (key, v) in fields {
        if key != "name" {
            entry.insert(key, Item::Value(to_toml(key, v)?));
        }
    }
    if !doc.contains_key(table) {
        doc.insert(table, Item::ArrayOfTables(ArrayOfTables::new()));
    }
    match doc.get_mut(table) {
        Some(Item::ArrayOfTables(entries)) => {
            entries.push(entry);
            Ok(())
        }
        _ => Err(EditError::Value {
            field: table.into(),
            problem: format!("must be written as [[{table}]] entries to be edited here"),
        }),
    }
}

/// Sets and removes fields of the tunnel named `name`.
pub fn edit_tunnel(
    doc: &mut DocumentMut,
    name: &str,
    set: &serde_json::Map<String, Json>,
    unset: &[String],
) -> Result<(), EditError> {
    let index = find(doc, name).ok_or_else(|| EditError::UnknownTunnel(name.to_owned()))?;
    if let Some(key) = set.keys().chain(unset).find(|k| k.as_str() == "name") {
        return Err(EditError::Protected(key.clone()));
    }
    let mut values = Vec::new();
    for (key, v) in set {
        values.push((key.clone(), to_toml(key, v)?));
    }
    let table = tunnels_mut(doc)
        .and_then(|t| t.get_mut(index))
        .expect("found above");
    for key in unset {
        table.remove(key);
    }
    for (key, v) in values {
        match table.get_mut(&key) {
            // Keep the old value's surrounding spaces and trailing comment.
            Some(Item::Value(old)) => {
                let decor = old.decor().clone();
                *old = v;
                *old.decor_mut() = decor;
            }
            _ => {
                table.insert(&key, Item::Value(v));
            }
        }
    }
    Ok(())
}

/// Removes the tunnel named `name`.
pub fn remove_tunnel(doc: &mut DocumentMut, name: &str) -> Result<(), EditError> {
    let index = find(doc, name).ok_or_else(|| EditError::UnknownTunnel(name.to_owned()))?;
    let tunnels = tunnels_mut(doc).expect("found above");
    tunnels.remove(index);
    if tunnels.is_empty() {
        doc.remove("tunnels");
    }
    Ok(())
}

fn tunnels_mut(doc: &mut DocumentMut) -> Option<&mut ArrayOfTables> {
    doc.get_mut("tunnels")
        .and_then(Item::as_array_of_tables_mut)
}

fn find(doc: &DocumentMut, name: &str) -> Option<usize> {
    doc.get("tunnels")?
        .as_array_of_tables()?
        .iter()
        .position(|t| t.get("name").and_then(Item::as_str) == Some(name))
}

/// Converts a JSON value from the protocol into a TOML value. Objects become
/// inline tables, so `forwards` reads as `[{ local_port = 5433, ... }]`.
fn to_toml(field: &str, v: &Json) -> Result<Value, EditError> {
    let bad = |problem: &str| EditError::Value {
        field: field.to_owned(),
        problem: problem.to_owned(),
    };
    Ok(match v {
        Json::Null => return Err(bad("has no value; to remove a field use unset")),
        Json::Bool(b) => Value::from(*b),
        Json::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => Value::from(i),
            (None, Some(f)) => Value::from(f),
            _ => return Err(bad("is a number TOML cannot store")),
        },
        Json::String(s) => Value::from(s.as_str()),
        Json::Array(items) => {
            let mut array = Array::new();
            for item in items {
                array.push(to_toml(field, item)?);
            }
            Value::Array(array)
        }
        Json::Object(map) => {
            let mut table = InlineTable::new();
            for (k, item) in map {
                table.insert(k, to_toml(&format!("{field}.{k}"), item)?);
            }
            Value::InlineTable(table)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ORIGINAL: &str = "\
# My tunnels
version = 1

[[tunnels]]
# the staging database
name = \"db\"
command = \"x\"  # keep this comment
forwards = [{ local_port = 5433 }]

[[tunnels]]
name = \"redis\" # cache
command = \"y\"
";

    fn map(v: Json) -> serde_json::Map<String, Json> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn add_appends_and_keeps_comments() {
        let mut doc = parse(ORIGINAL).unwrap();
        add_tunnel(
            &mut doc,
            "bastion-redis",
            &map(json!({
                "forwards": [{"local_port": 6380, "remote_host": "redis.internal", "remote_port": 6379}],
                "host": "bastion",
                "type": "ssh-local",
                "groups": ["daily"],
            })),
        )
        .unwrap();
        let text = doc.to_string();
        assert!(
            text.starts_with(ORIGINAL),
            "existing text is untouched:\n{text}"
        );
        assert!(
            text.ends_with(
                "\n[[tunnels]]\nname = \"bastion-redis\"\ntype = \"ssh-local\"\ngroups = [\"daily\"]\nhost = \"bastion\"\nforwards = [{ local_port = 6380, remote_host = \"redis.internal\", remote_port = 6379 }]\n"
            ),
            "{text}"
        );
        assert_eq!(
            add_tunnel(&mut doc, "db", &map(json!({}))),
            Err(EditError::Exists("tunnel", "db".into()))
        );
    }

    #[test]
    fn add_to_a_file_without_tunnels() {
        let mut doc = parse(NEW_FILE).unwrap();
        add_tunnel(&mut doc, "t", &map(json!({"command": "z"}))).unwrap();
        assert_eq!(
            doc.to_string(),
            "version = 1\n\n[[tunnels]]\nname = \"t\"\ncommand = \"z\"\n"
        );
    }

    #[test]
    fn edit_keeps_comments_and_finds_by_name() {
        let mut doc = parse(ORIGINAL).unwrap();
        edit_tunnel(
            &mut doc,
            "db",
            &map(json!({"command": "x2", "auto_start": true})),
            &["forwards".into()],
        )
        .unwrap();
        let text = doc.to_string();
        assert!(
            text.contains(
                "# the staging database\nname = \"db\"\ncommand = \"x2\"  # keep this comment\n"
            ),
            "{text}"
        );
        assert!(text.contains("auto_start = true"), "{text}");
        assert!(!text.contains("5433"), "{text}");
        assert!(
            text.contains("name = \"redis\" # cache"),
            "the other tunnel is untouched:\n{text}"
        );

        assert_eq!(
            edit_tunnel(&mut doc, "nope", &map(json!({})), &[]),
            Err(EditError::UnknownTunnel("nope".into()))
        );
        assert_eq!(
            edit_tunnel(&mut doc, "db", &map(json!({"name": "x"})), &[]),
            Err(EditError::Protected("name".into()))
        );
    }

    #[test]
    fn remove_keeps_the_rest() {
        let mut doc = parse(ORIGINAL).unwrap();
        remove_tunnel(&mut doc, "db").unwrap();
        let text = doc.to_string();
        assert!(text.starts_with("# My tunnels\nversion = 1\n"), "{text}");
        assert!(
            !text.contains("staging database"),
            "the removed entry's comment goes too"
        );
        assert!(text.contains("name = \"redis\" # cache"));
        remove_tunnel(&mut doc, "redis").unwrap();
        assert!(!doc.to_string().contains("[[tunnels]]"));
    }

    #[test]
    fn values_convert() {
        assert_eq!(
            to_toml("x", &json!({"a": [1, true, "s"]}))
                .unwrap()
                .to_string(),
            "{ a = [1, true, \"s\"] }"
        );
        assert!(matches!(
            to_toml("x", &Json::Null),
            Err(EditError::Value { .. })
        ));
        assert!(matches!(parse("version = \n"), Err(EditError::Syntax(_))));
    }
}
