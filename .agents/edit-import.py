import sys

p = "/Users/omjannu/Projects/tunwarden/crates/tunwarden-core/src/import.rs"
s = open(p).read()

def cut(start, end, new):
    global s
    i = s.index(start)
    j = s.index(end, i)
    s = s[:i] + new + s[j:]

cut("//! Turning pasted tunnel commands", "/// Lines with `\\` continuations joined", '''//! Turning pasted tunnel commands into config entries (PRD FR-6, FR-43).
//!
//! Each input line is a command, an `alias name='command'` line, or a comment.
//! Plain `ssh -L/-R/-D` commands become the ssh types; any other command
//! becomes a `command` tunnel with the original command line. Leading
//! `NAME=value` assignments become a profile's environment. Every assumption
//! is reported as a warning.

use std::net::IpAddr;

use serde_json::{Map, Value, json};

use crate::config::Config;

/// A profile the import needs that the config does not have yet.
#[derive(Clone, Debug, PartialEq)]
pub struct NewProfile {
    pub name: String,
    pub fields: Map<String, Value>,
}

/// One imported tunnel: its name and config fields (without `name`).
#[derive(Clone, Debug, PartialEq)]
pub struct NewTunnel {
    pub name: String,
    pub fields: Map<String, Value>,
    /// The input line it came from.
    pub source: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Imported {
    pub profiles: Vec<NewProfile>,
    pub tunnels: Vec<NewTunnel>,
    /// `line N: ...` notes about assumptions and unsupported parts.
    pub warnings: Vec<String>,
}

impl Imported {
    /// The entries as TOML, for review.
    pub fn to_toml(&self) -> String {
        let mut doc = toml_edit::DocumentMut::new();
        for p in &self.profiles {
            let _ = crate::config::edit::add_entry(&mut doc, "profiles", &p.name, &p.fields);
        }
        for t in &self.tunnels {
            let _ = crate::config::edit::add_tunnel(&mut doc, &t.name, &t.fields);
        }
        doc.to_string().trim_start().to_owned()
    }
}

/// Imports every line of `text`. `config` provides existing names and profiles.
pub fn import(text: &str, config: &Config) -> Imported {
    let mut out = Imported::default();
    let mut names: Vec<String> = config.tunnels.iter().map(|t| t.name.clone()).collect();
    let mut profiles: Vec<(String, Vec<(String, String)>)> = config
        .profiles
        .iter()
        .map(|p| (p.name.clone(), sorted(p.env.clone())))
        .collect();

    for (number, line) in joined_lines(text) {
        let warn = |out: &mut Imported, msg: String| out.warnings.push(format!("line {number}: {msg}"));
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (alias, command) = split_alias(trimmed);
        let Some(mut words) = shlex::split(&command) else {
            warn(&mut out, "has an unclosed quote; skipped".into());
            continue;
        };
        if words.is_empty() {
            continue;
        }

        // Leading NAME=value assignments become the profile's environment.
        let mut env = Vec::new();
        while words.first().is_some_and(|w| is_assignment(w)) {
            let w = words.remove(0);
            let (k, v) = w.split_once('=').unwrap_or((&w, ""));
            env.push((k.to_owned(), v.to_owned()));
        }
        if words.is_empty() {
            warn(&mut out, "has no command; skipped".into());
            continue;
        }
        // A trailing `&` backgrounds the command in a shell; the daemon does that.
        if words.last().is_some_and(|w| w == "&") {
            words.pop();
            warn(&mut out, "ends with `&`; removed, because Tunwarden runs tunnels in the background itself".into());
        }

        let mut notes = Vec::new();
        let parsed = if words[0] == "ssh" { ssh(&words[1..], &mut notes) } else { None };
        for n in notes {
            warn(&mut out, n);
        }

        let mut fields = match parsed {
            Some(fields) => fields,
            None => {
                let mut f = Map::new();
                f.insert("type".into(), json!("command"));
                let mut words = words.clone();
                if words[0] == "ssh"
                    && let Some(i) = words.iter().position(|w| w == "-f")
                {
                    // ssh -f forks into the background; the supervisor must keep the process.
                    words.remove(i);
                    warn(&mut out, "uses `ssh -f`; removed, because Tunwarden runs tunnels in the background itself".into());
                }
                f.insert(
                    "command".into(),
                    json!(shlex::try_join(words.iter().map(String::as_str)).unwrap_or(command.clone())),
                );
                let ports = guess_local_ports(&words);
                if ports.is_empty() {
                    warn(&mut out, "no local port found; the tunnel counts as ready while its process runs. Add forwards to check a port".into());
                } else {
                    warn(
                        &mut out,
                        format!(
                            "guessed local port{} {} from the command; check them",
                            if ports.len() == 1 { "" } else { "s" },
                            ports.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ")
                        ),
                    );
                    f.insert(
                        "forwards".into(),
                        Value::Array(ports.into_iter().map(|p| json!({ "local_port": p })).collect()),
                    );
                }
                f
            }
        };

        if !env.is_empty() {
            let name = profile_for(&mut profiles, &mut out, env);
            fields.insert("profile".into(), json!(name));
        }

        let kind = fields.get("type").and_then(Value::as_str).unwrap_or("command").to_owned();
        let base = alias.map(|a| sanitize(&a)).unwrap_or_else(|| default_name(&kind, &fields, &words));
        let name = unique(&mut names, &base);
        out.tunnels.push(NewTunnel {
            name,
            fields,
            source: trimmed.to_owned(),
        });
    }
    out
}

fn sorted(mut env: Vec<(String, String)>) -> Vec<(String, String)> {
    env.sort();
    env
}

/// An existing profile with exactly this environment, or a new one.
fn profile_for(known: &mut Vec<(String, Vec<(String, String)>)>, out: &mut Imported, env: Vec<(String, String)>) -> String {
    let env = sorted(env);
    if let Some((name, _)) = known.iter().find(|(_, e)| *e == env) {
        return name.clone();
    }
    let mut taken: Vec<String> = known.iter().map(|(n, _)| n.clone()).collect();
    let name = unique(&mut taken, "imported");
    let mut vars = Map::new();
    for (k, v) in &env {
        vars.insert(k.clone(), json!(v));
    }
    let mut fields = Map::new();
    fields.insert("env".into(), Value::Object(vars));
    known.push((name.clone(), env));
    out.profiles.push(NewProfile { name: name.clone(), fields });
    name
}

''')

cut("fn default_name(kind: &str, fields: &Map<String, Value>) -> String {", "/// An IP for a bind host;", '''fn default_name(kind: &str, fields: &Map<String, Value>, words: &[String]) -> String {
    let port = fields
        .get("forwards")
        .and_then(|f| f.get(0))
        .and_then(|f| f.get("local_port"))
        .and_then(Value::as_u64);
    let stem = match kind {
        "ssh-local" | "ssh-remote" | "ssh-dynamic" => fields.get("host").and_then(Value::as_str).unwrap_or("ssh").to_owned(),
        _ => std::path::Path::new(&words[0])
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "tunnel".into()),
    };
    match port {
        Some(p) => sanitize(&format!("{stem}-{p}")),
        None => sanitize(&stem),
    }
}

''')

cut("// --- gcloud ---", "/// Local ports in a command kept as", "")

cut("/// Local ports in a command kept as", "#[cfg(test)]", '''/// Local ports guessed from a `command` tunnel's words: ssh-style `-L`/`-D`
/// specs, `--...port...=N` or `HOST:N` values with a local host, and
/// `LOCAL:REMOTE` pairs such as a port-forward's `5433:5432`.
fn guess_local_ports(words: &[String]) -> Vec<u64> {
    let mut ports = Vec::new();
    let mut add = |p: Option<u64>| {
        if let Some(p) = p.filter(|p| !ports.contains(p)) {
            ports.push(p);
        }
    };
    let local_host = |h: &str| matches!(h, "localhost" | "127.0.0.1" | "0.0.0.0" | "[::1]" | "");
    let mut i = 0;
    while i < words.len() {
        let w = &words[i];
        let spec = match w.as_str() {
            "-L" | "-D" => {
                i += 1;
                words.get(i).cloned()
            }
            _ if w.starts_with("-L") || w.starts_with("-D") => Some(w[2..].to_owned()),
            _ => None,
        };
        if let Some(spec) = spec {
            let parts = colon_parts(&spec);
            add(match parts.len() {
                1 => port(&parts[0]),
                2 => port(&parts[1]),
                3 => port(&parts[0]),
                _ => port(&parts[1]),
            });
        } else {
            let value = w.split_once('=').filter(|(k, _)| k.starts_with("--") && k.contains("port")).map(|(_, v)| v).unwrap_or(w);
            let parts = colon_parts(value);
            match parts.as_slice() {
                [h, p] if local_host(h) => add(port(p)),
                [l, r] if port(l).is_some() && port(r).is_some() => add(port(l)),
                [p] if value != w.as_str() => add(port(p)),
                _ => {}
            }
        }
        i += 1;
    }
    ports
}

''')

open(p, "w").write(s)
print("import ok")
