//! The `tunwarden` command line, run as a process (PRD 7.7). Uses
//! `tw-test-cli`, which is the same code as the real binary.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::Value;

const CLI: &str = env!("CARGO_BIN_EXE_tw-test-cli");
const FAKE: &str = env!("CARGO_BIN_EXE_fake-tunnel");

/// Port tests take turns, so a port freed by one test is not grabbed by another.
static PORTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A config and state directory. Dropping it stops the daemon.
struct Env {
    dir: tempfile::TempDir,
    ports: Vec<u16>,
}

impl Env {
    fn new() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let ports: Vec<u16> = (0..2)
            .map(|_| {
                TcpListener::bind("127.0.0.1:0")
                    .unwrap()
                    .local_addr()
                    .unwrap()
                    .port()
            })
            .collect();
        let config = format!(
            "version = 1\n\
             [[profiles]]\nname = \"prod\"\nproduction = true\n\
             [[tunnels]]\nname = \"db\"\ngroups = [\"daily\"]\ncommand = ''''{FAKE}' --listen 127.0.0.1:{} --print-stdout ready'''\nforwards = [{{ local_port = {} }}]\n\
             [[tunnels]]\nname = \"api\"\nprofile = \"prod\"\ncommand = ''''{FAKE}' --listen 127.0.0.1:{}'''\nforwards = [{{ local_port = {} }}]\n",
            ports[0], ports[0], ports[1], ports[1]
        );
        std::fs::write(dir.path().join("config.toml"), config).unwrap();
        Env { dir, ports }
    }

    fn config(&self) -> PathBuf {
        self.dir.path().join("config.toml")
    }

    fn command(&self, args: &[&str]) -> Command {
        self.command_with(&self.config(), args)
    }

    fn command_with(&self, config: &std::path::Path, args: &[&str]) -> Command {
        let mut c = Command::new(CLI);
        c.arg("--config")
            .arg(config)
            .args(args)
            .env("TUNWARDEN_STATE_DIR", self.dir.path().join("state"))
            .env_remove("TUNWARDEN_CONFIG")
            .stdin(Stdio::null());
        c
    }

    fn run(&self, args: &[&str]) -> Run {
        Run(self.command(args).output().unwrap())
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = self.command(&["daemon", "stop"]).output();
    }
}

struct Run(Output);

impl Run {
    fn ok(&self) -> &Self {
        assert!(
            self.0.status.success(),
            "failed: {}\nstdout:\n{}\nstderr:\n{}",
            self.0.status,
            self.out(),
            self.err()
        );
        self
    }

    fn fails(&self) -> &Self {
        assert!(
            !self.0.status.success(),
            "expected failure\nstdout:\n{}",
            self.out()
        );
        self
    }

    fn out(&self) -> String {
        String::from_utf8_lossy(&self.0.stdout).into_owned()
    }

    fn err(&self) -> String {
        String::from_utf8_lossy(&self.0.stderr).into_owned()
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.0.stdout).unwrap()
    }

    fn code(&self) -> Option<i32> {
        self.0.status.code()
    }
}

/// The row of `name` in a table, split into cells.
fn row(table: &str, name: &str) -> Vec<String> {
    table
        .lines()
        .find(|l| l.split_whitespace().next() == Some(name))
        .unwrap_or_else(|| panic!("no row {name} in:\n{table}"))
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

#[test]
fn config_path_and_init() {
    let env = Env::new();
    let out = env.run(&["config-path"]).ok().out();
    assert_eq!(out.trim(), env.config().display().to_string());

    let fresh = env.dir.path().join("new/config.toml");
    let r = Run(env.command_with(&fresh, &["init"]).output().unwrap());
    r.ok();
    assert!(r.out().contains("created"), "{}", r.out());
    let written = std::fs::read_to_string(&fresh).unwrap();
    assert!(
        written.contains("example-staging-123"),
        "the example config"
    );

    // A second init does not overwrite.
    let r = Run(env.command_with(&fresh, &["init"]).output().unwrap());
    r.fails();
    assert!(r.err().contains("already exists"), "{}", r.err());

    // The example config is valid.
    let r = Run(env
        .command_with(&fresh, &["list", "--json"])
        .output()
        .unwrap());
    assert_eq!(r.ok().json()["tunnels"].as_array().unwrap().len(), 6);
}

#[test]
fn list_and_show_command_need_no_daemon() {
    let env = Env::new();
    let out = env.run(&["list"]).ok().out();
    assert_eq!(row(&out, "db")[..2], ["db", "command"]);
    assert!(out.contains("prod (production)"), "{out}");

    let json = env.run(&["list", "--json"]).ok().json();
    assert_eq!(json["tunnels"][1]["name"], "api");
    assert_eq!(json["tunnels"][1]["production"], true);

    let cmd = env.run(&["show-command", "db"]).ok().out();
    assert_eq!(
        cmd.trim(),
        // A Windows path has backslashes, so the CLI prints it quoted.
        format!(
            "{} --listen 127.0.0.1:{} --print-stdout ready",
            tunwarden_core::types::quote(FAKE),
            env.ports[0]
        )
    );

    let r = env.run(&["show-command", "nope"]);
    r.fails();
    assert!(
        r.err().contains("no tunnel named \"nope\"")
            && r.err().contains("hint: configured tunnels: db, api"),
        "{}",
        r.err()
    );

    let status = env.run(&["daemon", "status"]);
    assert_eq!(status.code(), Some(3), "listing did not start a daemon");
}

#[test]
fn status_without_a_daemon_shows_everything_stopped() {
    let env = Env::new();
    let r = env.run(&["status"]);
    r.ok();
    assert!(r.err().contains("the daemon is not running"), "{}", r.err());
    assert_eq!(row(&r.out(), "db")[1], "stopped");
    let json = env.run(&["status", "--json"]).ok().json();
    assert!(
        json["tunnels"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["state"] == "stopped")
    );
}

#[test]
fn start_status_logs_stop() {
    let _turn = PORTS.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();

    // Starting starts the daemon on demand and waits for the tunnel.
    let out = env.run(&["start", "db"]).ok().out();
    assert_eq!(row(&out, "db")[1], "running");

    let out = env.run(&["status"]).ok().out();
    let db = row(&out, "db");
    assert_eq!(db[1], "running");
    assert!(db[3].parse::<u32>().is_ok(), "a PID: {db:?}");
    assert_eq!(row(&out, "api")[1], "stopped");

    let json = env.run(&["status", "--json", "db"]).ok().json();
    assert_eq!(json["tunnels"].as_array().unwrap().len(), 1);
    assert_eq!(json["tunnels"][0]["state"], "running");
    assert_eq!(json["tunnels"][0]["desired"], "running");

    let logs = env.run(&["logs", "db", "-n", "100"]).ok().out();
    assert!(logs.lines().any(|l| l.ends_with(" out ready")), "{logs}");
    assert!(
        logs.lines().any(|l| l.ends_with(" tw starting -> running")),
        "{logs}"
    );
    assert_eq!(
        env.run(&["logs", "db", "-n", "2"])
            .ok()
            .out()
            .lines()
            .count(),
        2
    );

    let out = env.run(&["stop", "--group", "daily"]).ok().out();
    assert_eq!(row(&out, "db")[1], "stopped");

    // Logs are still readable after the daemon stops.
    env.run(&["daemon", "stop"]).ok();
    let logs = env.run(&["logs", "db", "-n", "100"]).ok().out();
    assert!(logs.contains(" tw stopped"), "{logs}");
}

#[test]
fn production_tunnels_need_confirmation() {
    // FR-33
    let _turn = PORTS.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    let r = env.run(&["start", "--all"]);
    r.fails();
    assert!(
        r.err()
            .contains("refusing to start production tunnels (api) without confirmation"),
        "{}",
        r.err()
    );
    assert!(r.err().contains("hint: pass --yes to confirm"));
    let out = env.run(&["status"]).ok().out();
    assert_eq!(row(&out, "db")[1], "stopped", "nothing was started");

    let out = env.run(&["start", "--all", "--yes"]).ok().out();
    assert_eq!(row(&out, "db")[1], "running");
    assert_eq!(row(&out, "api")[1], "running");
    // Stopping needs no confirmation.
    env.run(&["stop", "--profile", "prod"]).ok();
    env.run(&["start", "db"]).ok();
}

#[test]
fn a_failed_start_exits_non_zero_with_the_reason() {
    let _turn = PORTS.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    let squatter = TcpListener::bind(("127.0.0.1", env.ports[0])).unwrap();
    let r = env.run(&["start", "db"]);
    r.fails();
    let line = r
        .out()
        .lines()
        .find(|l| l.starts_with("db"))
        .unwrap()
        .to_owned();
    assert!(
        line.contains("failed") && line.contains("port conflict: local address"),
        "{line}"
    );
    assert!(
        line.contains(&format!("(PID {})", std::process::id())),
        "{line}"
    );
    drop(squatter);
}

#[test]
fn selectors_and_errors() {
    let _turn = PORTS.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    let r = env.run(&["start"]);
    r.fails();
    assert!(r.err().contains("no tunnels selected"), "{}", r.err());
    assert!(
        r.err()
            .contains("hint: name one or more tunnels, or use --all, --group or --profile")
    );
    let r = env.run(&["start", "nope"]);
    r.fails();
    assert!(
        r.err().contains("hint: configured tunnels: api, db"),
        "{}",
        r.err()
    );
    let r = env.run(&["start", "--group", "weekly"]);
    r.fails();
    assert!(
        r.err().contains("no tunnel is in group \"weekly\""),
        "{}",
        r.err()
    );
    let out = env.run(&["start", "db", "--wait", "0"]).ok().out();
    assert!(out.contains("start requested for db"), "{out}");
    // Stop without a daemon is a no-op, not an error.
    env.run(&["daemon", "stop"]).ok();
    let out = env.run(&["stop", "--all"]).ok().out();
    assert!(out.contains("the daemon is not running"), "{out}");
}

#[test]
fn an_invalid_config_points_at_the_problem() {
    let env = Env::new();
    std::fs::write(
        env.config(),
        "version = 1\n[[tunnels]]\nname = \"db\"\ntype = \"ssh-local\"\n",
    )
    .unwrap();
    let r = env.run(&["list"]);
    r.fails();
    let err = r.err();
    assert!(err.contains("the config file has 2 problem(s)"), "{err}");
    assert!(
        err.contains(":2:1: tunnels[0].host: a ssh-local tunnel needs `host`"),
        "{err}"
    );

    std::fs::remove_file(env.config()).unwrap();
    let r = env.run(&["list"]);
    r.fails();
    assert!(
        r.err().contains("hint: run `tunwarden init` to create one"),
        "{}",
        r.err()
    );
}

#[test]
fn logs_follow_prints_new_lines() {
    let _turn = PORTS.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new();
    env.run(&["start", "db"]).ok();
    let mut follow = env
        .command(&["logs", "db", "-n", "1", "-f"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    env.run(&["restart", "db"]).ok();
    std::thread::sleep(std::time::Duration::from_millis(500));
    follow.kill().unwrap();
    let out = follow.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains(" tw restart requested"), "{text}");
    assert!(
        text.contains(" out ready"),
        "the new process's output: {text}"
    );
}

#[test]
fn completions() {
    let env = Env::new();
    for shell in ["bash", "zsh", "fish", "powershell"] {
        let out = env.run(&["completions", shell]).ok().out();
        assert!(
            out.contains("tunwarden") && out.contains("COMPLETE"),
            "{shell}: the script calls back into tunwarden: {out}"
        );
    }
    // The shell's callback completes names from the config.
    let out = env
        .command(&["--", "tunwarden", "start", ""])
        .env("COMPLETE", "fish")
        .env("TUNWARDEN_CONFIG", env.config())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.lines().any(|l| l.starts_with("db\t")), "{text}");
    assert!(text.lines().any(|l| l.starts_with("api\t")), "{text}");
    let out = env
        .command(&["--", "tunwarden", "start", "--group", ""])
        .env("COMPLETE", "fish")
        .env("TUNWARDEN_CONFIG", env.config())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "daily");
}

#[test]
fn man_pages_and_help_examples() {
    let env = Env::new();
    let dir = env.dir.path().join("man");
    env.run(&["man", dir.to_str().unwrap()]).ok();
    for page in ["tunwarden.1", "tunwarden-start.1", "tunwarden-login.1"] {
        assert!(dir.join(page).is_file(), "{page}");
    }
    let help = env.run(&["start", "--help"]).ok().out();
    assert!(help.contains("tunwarden start --group daily"), "{help}");
}

#[test]
fn missing_names_without_a_terminal_list_the_choices() {
    // Pickers only appear on a terminal; scripts get an error with the choices.
    let env = Env::new();
    let r = env.run(&["logs"]);
    r.fails();
    assert!(r.err().contains("error: name a tunnel"), "{}", r.err());
    assert!(
        r.err().contains("hint: configured tunnels: db, api"),
        "{}",
        r.err()
    );
    let r = env.run(&["add"]);
    r.fails();
    assert!(
        r.err().contains("error: name the new tunnel"),
        "{}",
        r.err()
    );
    assert!(
        !r.out().contains('\u{1b}') && !r.err().contains('\u{1b}'),
        "no colour codes in pipes"
    );
}
