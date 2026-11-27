"""M3b core: edit and remove profiles and logins (config.edit_entry, config.remove_entry)."""

def edit(p, pairs):
    s = open(p, encoding="utf-8").read()
    for a, b in pairs:
        assert s.count(a) == 1, (p, s.count(a), a[:80])
        s = s.replace(a, b)
    open(p, "w", encoding="utf-8").write(s)


E = "crates/tunwarden-core/src/config/edit.rs"
edit(E, [
    ('''    #[error("no tunnel named \\"{0}\\" in the config file")]
    UnknownTunnel(String),''', '''    #[error("no tunnel named \\"{0}\\" in the config file")]
    UnknownTunnel(String),
    #[error("no {0} named \\"{1}\\" in the config file")]
    Unknown(&'static str, String),
    #[error("the {0} \\"{1}\\" is used by {2}; change or remove them first")]
    InUse(&'static str, String, String),'''),
    ('''/// Sets and removes fields of the tunnel named `name`.
pub fn edit_tunnel(
    doc: &mut DocumentMut,
    name: &str,
    set: &serde_json::Map<String, Json>,
    unset: &[String],
) -> Result<(), EditError> {
    let index = find(doc, name).ok_or_else(|| EditError::UnknownTunnel(name.to_owned()))?;''', '''/// Sets and removes fields of the tunnel named `name`.
pub fn edit_tunnel(
    doc: &mut DocumentMut,
    name: &str,
    set: &serde_json::Map<String, Json>,
    unset: &[String],
) -> Result<(), EditError> {
    let index = find(doc, name).ok_or_else(|| EditError::UnknownTunnel(name.to_owned()))?;
    set_fields(doc, "tunnels", index, set, unset)
}

/// What one entry of `table` is called in messages.
fn entry_kind(table: &str) -> &'static str {
    match table {
        "logins" => "login",
        "profiles" => "profile",
        _ => "tunnel",
    }
}

/// Sets and removes fields of the `[[logins]]` or `[[profiles]]` entry `name`.
pub fn edit_entry(
    doc: &mut DocumentMut,
    table: &str,
    name: &str,
    set: &serde_json::Map<String, Json>,
    unset: &[String],
) -> Result<(), EditError> {
    let index = find_in(doc, table, name)
        .ok_or_else(|| EditError::Unknown(entry_kind(table), name.to_owned()))?;
    set_fields(doc, table, index, set, unset)
}

/// Removes the `[[logins]]` or `[[profiles]]` entry `name`. A profile still
/// used by a tunnel, or a login still used by a profile, is refused.
pub fn remove_entry(doc: &mut DocumentMut, table: &str, name: &str) -> Result<(), EditError> {
    let kind = entry_kind(table);
    let index =
        find_in(doc, table, name).ok_or_else(|| EditError::Unknown(kind, name.to_owned()))?;
    let (users_table, field) = match table {
        "profiles" => ("tunnels", "profile"),
        _ => ("profiles", "login"),
    };
    let users: Vec<String> = doc
        .get(users_table)
        .and_then(Item::as_array_of_tables)
        .map(|entries| {
            entries
                .iter()
                .filter(|e| e.get(field).and_then(Item::as_str) == Some(name))
                .filter_map(|e| e.get("name").and_then(Item::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if !users.is_empty() {
        let who = format!("{} {}", entry_kind(users_table), users.join(", "));
        return Err(EditError::InUse(kind, name.to_owned(), who));
    }
    let entries = doc
        .get_mut(table)
        .and_then(Item::as_array_of_tables_mut)
        .expect("found above");
    entries.remove(index);
    if entries.is_empty() {
        doc.remove(table);
    }
    Ok(())
}

fn set_fields(
    doc: &mut DocumentMut,
    table_name: &str,
    index: usize,
    set: &serde_json::Map<String, Json>,
    unset: &[String],
) -> Result<(), EditError> {'''),
    ('''    let table = tunnels_mut(doc)
        .and_then(|t| t.get_mut(index))
        .expect("found above");''', '''    let table = doc
        .get_mut(table_name)
        .and_then(Item::as_array_of_tables_mut)
        .and_then(|t| t.get_mut(index))
        .expect("found above");'''),
    ('''fn find(doc: &DocumentMut, name: &str) -> Option<usize> {
    doc.get("tunnels")?''', '''fn find(doc: &DocumentMut, name: &str) -> Option<usize> {
    find_in(doc, "tunnels", name)
}

fn find_in(doc: &DocumentMut, table: &str, name: &str) -> Option<usize> {
    doc.get(table)?'''),
])

P = "crates/tunwarden-core/src/proto.rs"
edit(P, [
    ('''    pub const CONFIG_ADD_ENTRY: &str = "config.add_entry";''', '''    pub const CONFIG_ADD_ENTRY: &str = "config.add_entry";
    pub const CONFIG_EDIT_ENTRY: &str = "config.edit_entry";
    pub const CONFIG_REMOVE_ENTRY: &str = "config.remove_entry";'''),
    ('''pub struct ConfigRemoveParams {
    pub name: String,
}''', '''pub struct ConfigRemoveParams {
    pub name: String,
}

/// Changes a `[[logins]]` or `[[profiles]]` entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ConfigEditEntryParams {
    /// `logins` or `profiles`.
    pub table: String,
    pub name: String,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub set: serde_json::Map<String, Value>,
    #[serde(default)]
    pub unset: Vec<String>,
}

/// Removes a `[[logins]]` or `[[profiles]]` entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ConfigRemoveEntryParams {
    pub table: String,
    pub name: String,
}'''),
])

D = "crates/tunwarden-core/src/daemon.rs"
edit(D, [
    ('''        method::CONFIG_ADD_ENTRY => {
            let p: ConfigAddEntryParams = params(req)?;
            if !matches!(p.table.as_str(), "logins" | "profiles") {
                return Err(RpcError::app(
                    "invalid_edit",
                    format!("cannot add to `{}`; use logins or profiles", p.table),
                    None,
                ));
            }
            edit_config(ctx, EditOp::AddEntry(p.table, p.name, p.fields)).await
        }''', '''        method::CONFIG_ADD_ENTRY => {
            let p: ConfigAddEntryParams = params(req)?;
            entry_table(&p.table)?;
            edit_config(ctx, EditOp::AddEntry(p.table, p.name, p.fields)).await
        }
        method::CONFIG_EDIT_ENTRY => {
            let p: ConfigEditEntryParams = params(req)?;
            entry_table(&p.table)?;
            edit_config(ctx, EditOp::EditEntry(p.table, p.name, p.set, p.unset)).await
        }
        method::CONFIG_REMOVE_ENTRY => {
            let p: ConfigRemoveEntryParams = params(req)?;
            entry_table(&p.table)?;
            edit_config(ctx, EditOp::RemoveEntry(p.table, p.name)).await
        }'''),
    ('''        EditOp::AddEntry(table, name, fields) => {
            edit::add_entry(&mut doc, table, name, fields).map_err(edit_error)?;
            (name.clone(), "added")
        }''', '''        EditOp::AddEntry(table, name, fields) => {
            edit::add_entry(&mut doc, table, name, fields).map_err(edit_error)?;
            (name.clone(), "added")
        }
        EditOp::EditEntry(table, name, set, unset) => {
            edit::edit_entry(&mut doc, table, name, set, unset).map_err(edit_error)?;
            (name.clone(), "updated")
        }
        EditOp::RemoveEntry(table, name) => {
            edit::remove_entry(&mut doc, table, name).map_err(edit_error)?;
            (name.clone(), "removed")
        }'''),
    ('''        EditError::UnknownTunnel(_) => "unknown_tunnel",
        EditError::Exists(..) => "already_exists",''', '''        EditError::UnknownTunnel(_) => "unknown_tunnel",
        EditError::Unknown("profile", _) => "unknown_profile",
        EditError::Unknown(..) => "unknown_login",
        EditError::InUse(..) => "in_use",
        EditError::Exists(..) => "already_exists",'''),
    ('''fn edit_error(e: crate::config::edit::EditError) -> RpcError {''', '''/// Only `[[logins]]` and `[[profiles]]` are edited as entries.
fn entry_table(table: &str) -> Result<(), RpcError> {
    if matches!(table, "logins" | "profiles") {
        Ok(())
    } else {
        Err(RpcError::app(
            "invalid_edit",
            format!("cannot edit `{table}` entries; use logins or profiles"),
            None,
        ))
    }
}

fn edit_error(e: crate::config::edit::EditError) -> RpcError {'''),
])
s = open(D).read()
import re
m = re.search(r"enum EditOp \{[^}]*\}", s)
assert m, "EditOp"
block = m.group(0)
new = block.replace("AddEntry(", "EditEntry(\n        String,\n        String,\n        serde_json::Map<String, Value>,\n        Vec<String>,\n    ),\n    RemoveEntry(String, String),\n    AddEntry(", 1)
s = s.replace(block, new)
open(D, "w").write(s)
print("ok")
