//! Turning pasted tunnel commands into config entries (PRD FR-6, FR-43).
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
        let warn =
            |out: &mut Imported, msg: String| out.warnings.push(format!("line {number}: {msg}"));
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
            warn(
                &mut out,
                "ends with `&`; removed, because Tunwarden runs tunnels in the background itself"
                    .into(),
            );
        }

        let mut notes = Vec::new();
        let parsed = if words[0] == "ssh" {
            ssh(&words[1..], &mut notes)
        } else {
            None
        };
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
                    json!(
                        words
                            .iter()
                            .map(|w| crate::types::quote_with(w, crate::types::Syntax::current()))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ),
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
                            ports
                                .iter()
                                .map(|p| p.to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    );
                    f.insert(
                        "forwards".into(),
                        Value::Array(
                            ports
                                .into_iter()
                                .map(|p| json!({ "local_port": p }))
                                .collect(),
                        ),
                    );
                }
                f
            }
        };

        if !env.is_empty() {
            let name = profile_for(&mut profiles, &mut out, env);
            fields.insert("profile".into(), json!(name));
        }

        let kind = fields
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("command")
            .to_owned();
        let base = alias
            .map(|a| sanitize(&a))
            .unwrap_or_else(|| default_name(&kind, &fields, &words));
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
fn profile_for(
    known: &mut Vec<(String, Vec<(String, String)>)>,
    out: &mut Imported,
    env: Vec<(String, String)>,
) -> String {
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
    out.profiles.push(NewProfile {
        name: name.clone(),
        fields,
    });
    name
}

/// Lines with `\` continuations joined, numbered from 1.
fn joined_lines(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut current: Option<(usize, String)> = None;
    for (i, line) in text.lines().enumerate() {
        let (start, mut acc) = current.take().unwrap_or((i + 1, String::new()));
        match line.strip_suffix('\\') {
            Some(part) => {
                acc.push_str(part);
                acc.push(' ');
                current = Some((start, acc));
            }
            None => {
                acc.push_str(line);
                out.push((start, acc));
            }
        }
    }
    if let Some(c) = current {
        out.push(c);
    }
    out
}

/// `alias db='gcloud ...'` → (Some("db"), "gcloud ..."). Other lines are commands.
fn split_alias(line: &str) -> (Option<String>, String) {
    let Some(rest) = line.strip_prefix("alias ") else {
        return (None, line.to_owned());
    };
    let rest = rest.trim_start().trim_start_matches("-g ").trim_start();
    let Some((name, value)) = rest.split_once('=') else {
        return (None, line.to_owned());
    };
    let value = value.trim();
    let unquoted = shlex::split(value)
        .filter(|w| w.len() == 1)
        .map(|mut w| w.remove(0))
        .unwrap_or_else(|| value.to_owned());
    let name = shlex::split(name.trim())
        .filter(|w| w.len() == 1)
        .map(|mut w| w.remove(0))
        .unwrap_or_else(|| name.trim().to_owned());
    (Some(name), unquoted)
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(k, _)| {
        !k.is_empty()
            && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !k.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// A valid tunnel name from free text.
fn sanitize(text: &str) -> String {
    let mut s: String = text
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    s = s
        .trim_start_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_owned();
    s.truncate(64);
    if s.is_empty() { "tunnel".into() } else { s }
}

fn unique(taken: &mut Vec<String>, base: &str) -> String {
    let mut name = base.to_owned();
    let mut n = 2;
    while taken.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
        name = format!("{base}-{n}");
        n += 1;
    }
    taken.push(name.clone());
    name
}

fn default_name(kind: &str, fields: &Map<String, Value>, words: &[String]) -> String {
    let port = fields
        .get("forwards")
        .and_then(|f| f.get(0))
        .and_then(|f| f.get("local_port"))
        .and_then(Value::as_u64);
    let stem = match kind {
        "ssh-local" | "ssh-remote" | "ssh-dynamic" => fields
            .get("host")
            .and_then(Value::as_str)
            .unwrap_or("ssh")
            .to_owned(),
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

/// An IP for a bind host; `localhost` means loopback. `None` if not expressible.
fn bind_ip(host: &str) -> Option<String> {
    match host {
        "localhost" => Some("127.0.0.1".into()),
        "" | "*" => Some("0.0.0.0".into()),
        h => h
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .ok()
            .map(|ip| ip.to_string()),
    }
}

fn port(text: &str) -> Option<u64> {
    text.parse::<u16>().ok().filter(|p| *p > 0).map(u64::from)
}

/// Splits on `:` outside `[...]`.
fn colon_parts(spec: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut depth = 0;
    for c in spec.chars() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            ':' if depth == 0 => {
                parts.push(String::new());
                continue;
            }
            _ => {}
        }
        parts.last_mut().unwrap().push(c);
    }
    parts
}

// --- ssh -------------------------------------------------------------------------

fn ssh(args: &[String], notes: &mut Vec<String>) -> Option<Map<String, Value>> {
    const WITH_VALUE: &str = "BbcDEeFIiJLlmOopQRSWw";
    const IGNORED: &str = "NnTqfC46v";
    let (mut locals, mut remotes, mut dynamics) = (Vec::new(), Vec::new(), Vec::new());
    let (mut jump, mut user, mut ssh_port, mut host) = (None, None, None, None);
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        // OpenSSH also reads options after the host; anything else there is a remote command.
        if host.is_some() && !a.starts_with('-') {
            notes.push("passes a remote command to ssh; kept as a command tunnel".into());
            return None;
        }
        if let Some(flags) = a.strip_prefix('-').filter(|f| !f.is_empty()) {
            let opt = flags.chars().next().unwrap();
            let value = if WITH_VALUE.contains(opt) {
                if flags.len() > 1 {
                    Some(flags[1..].to_owned())
                } else {
                    i += 1;
                    Some(args.get(i)?.clone())
                }
            } else {
                None
            };
            match (opt, value) {
                ('L', Some(v)) => locals.push(v),
                ('R', Some(v)) => remotes.push(v),
                ('D', Some(v)) => dynamics.push(v),
                ('J', Some(v)) => jump = Some(v),
                ('l', Some(v)) => user = Some(v),
                ('p', Some(v)) => ssh_port = Some(v),
                (o, Some(_)) => {
                    notes.push(format!(
                        "uses ssh option -{o}, which the ssh types do not set; kept as a command tunnel (or move it to ~/.ssh/config)"
                    ));
                    return None;
                }
                (_, None) if flags.chars().all(|c| IGNORED.contains(c)) => {
                    if flags.contains('f') {
                        notes.push("uses `ssh -f`; dropped, because Tunwarden runs tunnels in the background itself".into());
                    }
                }
                (_, None) => {
                    notes.push(format!("uses ssh flag -{flags}, which the ssh types do not set; kept as a command tunnel"));
                    return None;
                }
            }
        } else {
            host = Some(a.clone());
        }
        i += 1;
    }
    let host = host?;
    let (host_user, host) = match host.split_once('@') {
        Some((u, h)) => (Some(u.to_owned()), h.to_owned()),
        None => (None, host),
    };
    let user = user.or(host_user);

    let kinds = [
        !locals.is_empty(),
        !remotes.is_empty(),
        !dynamics.is_empty(),
    ]
    .iter()
    .filter(|x| **x)
    .count();
    if kinds != 1 {
        notes.push(if kinds == 0 {
            "ssh without -L, -R or -D is not a tunnel; kept as a command tunnel".into()
        } else {
            "mixes -L, -R and -D; one tunnel per kind is needed, so it was kept as a command tunnel"
                .into()
        });
        return None;
    }

    let mut forwards = Vec::new();
    let kind = if !locals.is_empty() {
        for spec in &locals {
            let p = colon_parts(spec);
            let (bind, l, h, r) = match p.as_slice() {
                [l, h, r] => ("localhost", l, h, r),
                [b, l, h, r] => (b.as_str(), l, h, r),
                _ => return None,
            };
            let mut f = Map::new();
            f.insert("bind".into(), json!(bind_ip(bind)?));
            f.insert("local_port".into(), json!(port(l)?));
            f.insert("remote_host".into(), json!(h.trim_matches(['[', ']'])));
            f.insert("remote_port".into(), json!(port(r)?));
            forwards.push(Value::Object(f));
        }
        "ssh-local"
    } else if !remotes.is_empty() {
        for spec in &remotes {
            let p = colon_parts(spec);
            let [r, h, l] = p.as_slice() else {
                notes.push("a remote forward with a bind address is not supported; kept as a command tunnel".into());
                return None;
            };
            forwards.push(json!({ "remote_port": port(r)?, "local_host": h.trim_matches(['[', ']']), "local_port": port(l)? }));
        }
        "ssh-remote"
    } else {
        for spec in &dynamics {
            let p = colon_parts(spec);
            let (bind, l) = match p.as_slice() {
                [l] => ("localhost", l),
                [b, l] => (b.as_str(), l),
                _ => return None,
            };
            forwards.push(json!({ "bind": bind_ip(bind)?, "local_port": port(l)? }));
        }
        "ssh-dynamic"
    };
    for f in &mut forwards {
        if f.get("bind").and_then(Value::as_str) == Some("127.0.0.1") {
            f.as_object_mut().unwrap().remove("bind");
        }
        if f.get("remote_host").and_then(Value::as_str) == Some("localhost") {
            f.as_object_mut().unwrap().remove("remote_host");
        }
        if f.get("local_host").and_then(Value::as_str) == Some("localhost") {
            f.as_object_mut().unwrap().remove("local_host");
        }
    }

    let mut m = Map::new();
    m.insert("type".into(), json!(kind));
    m.insert("host".into(), json!(host));
    if let Some(u) = user {
        m.insert("user".into(), json!(u));
    }
    if let Some(p) = ssh_port {
        m.insert("port".into(), json!(port(&p)?));
    }
    if let Some(j) = jump {
        m.insert(
            "jump".into(),
            Value::Array(j.split(',').map(|s| json!(s)).collect()),
        );
    }
    m.insert("forwards".into(), Value::Array(forwards));
    Some(m)
}

/// Local ports guessed from a `command` tunnel's words: ssh-style `-L`/`-D`
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
            let value = w
                .split_once('=')
                .filter(|(k, _)| k.starts_with("--") && k.contains("port"))
                .map(|(_, v)| v)
                .unwrap_or(w);
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

#[cfg(test)]
mod tests;
