import sys,re
p="crates/tunwarden-core/src/config/edit.rs"; s=open(p).read()
i=s.index("/// Appends a `[[projects]]` entry"); j=s.index("/// Sets and removes fields of the tunnel named")
s=s[:i]+'''/// Appends a `[[logins]]` or `[[profiles]]` entry with `fields` (without
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
        .is_some_and(|ps| ps.iter().any(|p| p.get("name").and_then(Item::as_str) == Some(name)));
    if exists {
        return Err(EditError::Exists(name.to_owned()));
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

'''+s[j:]
s=s.replace('const LEADING: [&str; 4] = ["name", "type", "project", "groups"];','const LEADING: [&str; 4] = ["name", "type", "profile", "groups"];')
s=s.replace('#[error("a tunnel named \\"{0}\\" already exists")]','#[error("an entry named \\"{0}\\" already exists")]')
open(p,"w").write(s)
print("ok", s.count("project"))
