//! Reading a tunnel type's own fields from its config table, with errors that
//! point at the exact field.

use crate::config::{FieldPath, Forward};

/// Where type validation sends its errors. Implemented by the config loader.
pub trait Report {
    fn error(&mut self, path: &FieldPath, message: String);
}

/// The type-specific fields of one tunnel table.
pub struct Fields<'a> {
    table: &'a toml::Table,
    path: &'a FieldPath,
    report: &'a mut dyn Report,
    known: Vec<&'static str>,
    failed: bool,
}

impl<'a> Fields<'a> {
    pub fn new(table: &'a toml::Table, path: &'a FieldPath, report: &'a mut dyn Report) -> Self {
        Fields {
            table,
            path,
            report,
            known: Vec::new(),
            failed: false,
        }
    }

    /// Reports an error at any path, for example a forward.
    pub fn error_at(&mut self, path: &FieldPath, message: String) {
        self.failed = true;
        self.report.error(path, message);
    }

    pub fn error(&mut self, key: &str, message: String) {
        let path = self.path.key(key);
        self.error_at(&path, message);
    }

    /// The path of one of this tunnel's fields.
    pub fn path(&self, key: &str) -> FieldPath {
        self.path.key(key)
    }

    pub fn forwards_path(&self, index: usize) -> FieldPath {
        self.path.key("forwards").index(index)
    }

    /// Whether any error was reported for this tunnel's type fields.
    pub fn failed(&self) -> bool {
        self.failed
    }

    /// An optional string.
    pub fn string(&mut self, key: &'static str) -> Option<String> {
        self.known.push(key);
        match self.table.get(key)? {
            toml::Value::String(s) => Some(s.clone()),
            other => {
                self.error(
                    key,
                    format!("`{key}` must be a string, not {}", other.type_str()),
                );
                None
            }
        }
    }

    /// A required string; `example` is shown when it is missing.
    pub fn required_string(
        &mut self,
        key: &'static str,
        kind: &str,
        example: &str,
    ) -> Option<String> {
        let missing = !self.table.contains_key(key);
        let value = self.string(key);
        if missing {
            self.error(
                key,
                format!("a {kind} tunnel needs `{key}`, for example `{key} = {example}`"),
            );
        }
        value
    }

    /// An optional string that is passed to a tool as an argument value. It must
    /// not look like an option, so a config value can never add flags.
    pub fn argument(&mut self, key: &'static str) -> Option<String> {
        let value = self.string(key)?;
        self.check_argument(&self.path.key(key), &value)
            .then_some(value)
    }

    pub fn required_argument(
        &mut self,
        key: &'static str,
        kind: &str,
        example: &str,
    ) -> Option<String> {
        let value = self.required_string(key, kind, example)?;
        self.check_argument(&self.path.key(key), &value)
            .then_some(value)
    }

    /// An optional list of argument values.
    pub fn argument_list(&mut self, key: &'static str) -> Vec<String> {
        self.known.push(key);
        let Some(value) = self.table.get(key) else {
            return Vec::new();
        };
        let Some(items) = value.as_array() else {
            self.error(
                key,
                format!("`{key}` must be a list of strings, for example `{key} = [\"a\", \"b\"]`"),
            );
            return Vec::new();
        };
        let mut out = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let path = self.path.key(key).index(i);
            match item.as_str() {
                Some(s) if self.check_argument(&path, s) => out.push(s.to_owned()),
                Some(_) => {}
                None => self.error_at(&path, format!("`{key}` must contain only strings")),
            }
        }
        out
    }

    /// An optional list of strings with no argument check.
    pub fn string_list(&mut self, key: &'static str) -> Vec<String> {
        self.known.push(key);
        let Some(value) = self.table.get(key) else {
            return Vec::new();
        };
        let Some(items) = value.as_array() else {
            self.error(key, format!("`{key}` must be a list of strings"));
            return Vec::new();
        };
        let mut out = Vec::new();
        for (i, item) in items.iter().enumerate() {
            match item.as_str() {
                Some(s) => out.push(s.to_owned()),
                None => {
                    let path = self.path.key(key).index(i);
                    self.error_at(&path, format!("`{key}` must contain only strings"));
                }
            }
        }
        out
    }

    /// An optional TCP port.
    pub fn port(&mut self, key: &'static str) -> Option<u16> {
        self.known.push(key);
        let value = self.table.get(key)?;
        match value
            .as_integer()
            .and_then(|v| u16::try_from(v).ok())
            .filter(|v| *v > 0)
        {
            Some(port) => Some(port),
            None => {
                self.error(
                    key,
                    format!("`{key}` must be a port number from 1 to 65535"),
                );
                None
            }
        }
    }

    /// Reports every field the type did not read. `common` lists the core
    /// fields, so the message can show all valid names. Returns whether any
    /// error was reported for this tunnel's type fields.
    pub fn finish(mut self, kind: &str, common: &[&str]) -> bool {
        let unknown: Vec<String> = self
            .table
            .keys()
            .filter(|k| !self.known.contains(&k.as_str()))
            .cloned()
            .collect();
        for key in unknown {
            let mut valid: Vec<&str> = common
                .iter()
                .copied()
                .chain(self.known.iter().copied())
                .collect();
            valid.sort_unstable();
            valid.dedup();
            self.error(
                &key,
                format!(
                    "unknown field `{key}` for a {kind} tunnel; valid fields: {}",
                    valid.join(", ")
                ),
            );
        }
        self.failed
    }

    fn check_argument(&mut self, path: &FieldPath, value: &str) -> bool {
        let problem = if value.is_empty() {
            Some("is empty".to_owned())
        } else if value.starts_with('-') {
            Some("must not start with '-'".to_owned())
        } else {
            value
                .chars()
                .find(|c| c.is_whitespace() || c.is_control())
                .map(|c| format!("must not contain {c:?}"))
        };
        match problem {
            Some(problem) => {
                self.error_at(path, format!("\"{value}\" {problem}"));
                false
            }
            None => true,
        }
    }
}

/// Reports forward fields a type does not use. `allowed` lists the ones it does.
pub fn forbid_forward_fields(
    fields: &mut Fields<'_>,
    kind: &str,
    forwards: &[Forward],
    allowed: &[&str],
) {
    for (i, f) in forwards.iter().enumerate() {
        let present = [
            ("remote_host", f.remote_host.is_some()),
            ("remote_port", f.remote_port.is_some()),
            ("local_host", f.local_host.is_some()),
        ];
        for (name, is_set) in present {
            if is_set && !allowed.contains(&name) {
                let path = fields.forwards_path(i).key(name);
                fields.error_at(&path, format!("a {kind} forward does not use `{name}`"));
            }
        }
    }
}

/// Reports a forward that is missing `remote_port`.
pub fn require_remote_port(fields: &mut Fields<'_>, kind: &str, forwards: &[Forward]) {
    for (i, f) in forwards.iter().enumerate() {
        if f.remote_port.is_none() {
            let path = fields.forwards_path(i).key("remote_port");
            fields.error_at(&path, format!("a {kind} forward needs `remote_port`"));
        }
    }
}
