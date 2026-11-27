//! The SSH types against a real OpenSSH server in Docker (PRD AC-11, FR-42,
//! FR-45). Each test starts two containers from `tests/sshd`: `target`, with
//! an echo service on its 127.0.0.1:7000, and `jump`, a jump host that reaches
//! `target` by name. The user's own `~/.ssh` is never used: `tools.ssh` points
//! at a wrapper that runs ssh with a config file in the test directory.
//!
//! Needs Docker and an ssh client, so the tests are ignored by default:
//! `cargo test -p fake-tunnel --test ssh -- --ignored`
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;

const CLI: &str = env!("CARGO_BIN_EXE_tw-test-cli");
const IMAGE: &str = "tunwarden-it-sshd:1";
const LABEL: &str = "tunwarden-it=1";

/// One lab at a time: each test owns its containers, daemon and ports.
static TURN: Mutex<()> = Mutex::new(());
static COUNTER: AtomicU32 = AtomicU32::new(0);

fn docker(args: &[&str]) -> Output {
    let out = Command::new("docker")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("docker is installed");
    assert!(
        out.status.success(),
        "docker {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn build_image() {
    static BUILT: OnceLock<()> = OnceLock::new();
    BUILT.get_or_init(|| {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sshd");
        docker(&["build", "-q", "-t", IMAGE, dir.to_str().unwrap()]);
    });
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn keygen(path: &Path) -> String {
    let out = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success());
    std::fs::read_to_string(path.with_extension("pub"))
        .unwrap()
        .trim()
        .to_owned()
}

/// Waits until an SSH server answers with its banner on `port`.
fn wait_for_sshd(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) {
            s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut banner = [0u8; 4];
            if s.read_exact(&mut banner).is_ok() && &banner == b"SSH-" {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "sshd on port {port} never answered"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Keys {
    /// The servers allow the test key.
    Allowed,
    /// The servers allow a different key.
    Refused,
}

/// Two SSH servers, a private ssh config, and a Tunwarden config and state dir.
struct Lab {
    dir: tempfile::TempDir,
    network: String,
    target: String,
    jump: String,
    target_port: u16,
    jump_port: u16,
}

impl Lab {
    fn new(keys: Keys) -> Lab {
        build_image();
        let dir = tempfile::tempdir().unwrap();
        let key = keygen(&dir.path().join("id"));
        let allowed = match keys {
            Keys::Allowed => key,
            Keys::Refused => keygen(&dir.path().join("other")),
        };
        let name = format!(
            "tw-it-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let network = format!("{name}-net");
        docker(&["network", "create", "--label", LABEL, &network]);
        let target = format!("{name}-target");
        let jump = format!("{name}-jump");
        let env = format!("AUTHORIZED_KEY={allowed}");
        for (container, alias) in [(&target, "target"), (&jump, "jump")] {
            docker(&[
                "run",
                "-d",
                "--rm",
                "--label",
                LABEL,
                "--name",
                container,
                "--network",
                &network,
                "--network-alias",
                alias,
                "-e",
                &env,
                "-p",
                "127.0.0.1::22",
                IMAGE,
            ]);
        }
        let published = |c: &str| -> u16 {
            let out = text(&docker(&["port", c, "22/tcp"]));
            out.lines()
                .find(|l| l.starts_with("127.0.0.1:"))
                .and_then(|l| l.rsplit(':').next())
                .unwrap()
                .parse()
                .unwrap()
        };
        let lab = Lab {
            target_port: published(&target),
            jump_port: published(&jump),
            dir,
            network,
            target,
            jump,
        };
        wait_for_sshd(lab.target_port);
        wait_for_sshd(lab.jump_port);
        lab.write_ssh_setup();
        lab.trust_hosts();
        lab
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// An ssh config that uses only the test key and known_hosts, and a
    /// wrapper that passes it to ssh. ssh hands `-F` on to jump-host hops.
    fn write_ssh_setup(&self) {
        let d = self.dir.path().display();
        std::fs::write(
            self.path("ssh_config"),
            format!(
                "Host *\n  IdentityFile {d}/id\n  IdentitiesOnly yes\n  IdentityAgent none\n  \
                 UserKnownHostsFile {d}/known_hosts\n  StrictHostKeyChecking yes\n  LogLevel ERROR\n"
            ),
        )
        .unwrap();
        let ssh = Command::new("sh")
            .args(["-c", "command -v ssh"])
            .output()
            .unwrap();
        let ssh = String::from_utf8(ssh.stdout).unwrap().trim().to_owned();
        assert!(!ssh.is_empty(), "an ssh client is installed");
        let wrapper = self.path("ssh");
        std::fs::write(
            &wrapper,
            format!("#!/bin/sh\nexec '{ssh}' -F '{d}/ssh_config' \"$@\"\n"),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Writes known_hosts entries for both servers, as `trust` would.
    fn trust_hosts(&self) {
        let key = |c: &str| {
            text(&docker(&[
                "exec",
                c,
                "cat",
                "/etc/ssh/ssh_host_ed25519_key.pub",
            ]))
        };
        let lines = format!(
            "[127.0.0.1]:{} {}\n[127.0.0.1]:{} {}\ntarget {}\n",
            self.target_port,
            key(&self.target),
            self.jump_port,
            key(&self.jump),
            key(&self.target)
        );
        std::fs::write(self.path("known_hosts"), lines).unwrap();
    }

    fn forget_hosts(&self) {
        std::fs::write(self.path("known_hosts"), "").unwrap();
    }

    /// Writes the Tunwarden config: `[tools] ssh` is the wrapper.
    fn config(&self, tunnels: &str) {
        std::fs::write(
            self.path("config.toml"),
            format!(
                "version = 1\n[tools]\nssh = \"{}\"\n{tunnels}",
                self.path("ssh").display()
            ),
        )
        .unwrap();
    }

    fn cli(&self, args: &[&str]) -> Output {
        Command::new(CLI)
            .arg("--config")
            .arg(self.path("config.toml"))
            .args(args)
            .env("TUNWARDEN_STATE_DIR", self.path("state"))
            .env_remove("TUNWARDEN_CONFIG")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn start(&self, name: &str) -> Output {
        self.cli(&["start", name, "--wait", "40"])
    }

    fn start_ok(&self, name: &str) {
        let out = self.start(name);
        assert!(
            out.status.success(),
            "start {name}:\n{}\n{}\nlog:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
            self.log(name)
        );
    }

    fn status(&self, name: &str) -> Value {
        let out = self.cli(&["status", "--json", name]);
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        v["tunnels"][0].clone()
    }

    fn wait_status(&self, name: &str, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let s = self.status(name);
            if f(&s) {
                return s;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: {s:#}\nlog:\n{}",
                self.log(name)
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    fn log(&self, name: &str) -> String {
        text(&self.cli(&["logs", name, "-n", "40"]))
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = self.cli(&["daemon", "stop"]);
        let _ = Command::new("docker")
            .args(["rm", "-f", &self.target, &self.jump])
            .output();
        let _ = Command::new("docker")
            .args(["network", "rm", &self.network])
            .output();
    }
}

fn stream(port: u16) -> TcpStream {
    let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s
}

/// Sends one line through `s` and returns the line that comes back.
fn echo_on(mut s: TcpStream, message: &str) -> String {
    s.write_all(format!("{message}\n").as_bytes()).unwrap();
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).unwrap();
    line.trim_end().to_owned()
}

fn echo(port: u16, message: &str) -> String {
    echo_on(stream(port), message)
}

/// Connects to `host:port` through a SOCKS5 proxy on `proxy` (RFC 1928, no auth).
fn socks5(proxy: u16, host: &str, port: u16) -> TcpStream {
    let mut s = stream(proxy);
    s.write_all(&[5, 1, 0]).unwrap();
    let mut reply = [0u8; 2];
    s.read_exact(&mut reply).unwrap();
    assert_eq!(reply, [5, 0], "no authentication");
    let mut request = vec![5, 1, 0, 3, host.len() as u8];
    request.extend_from_slice(host.as_bytes());
    request.extend_from_slice(&port.to_be_bytes());
    s.write_all(&request).unwrap();
    let mut head = [0u8; 4];
    s.read_exact(&mut head).unwrap();
    assert_eq!(head[..2], [5, 0], "the proxy connected");
    let rest = match head[3] {
        1 => 4 + 2,
        4 => 16 + 2,
        3 => {
            let mut len = [0u8; 1];
            s.read_exact(&mut len).unwrap();
            len[0] as usize + 2
        }
        other => panic!("unknown address type {other}"),
    };
    let mut addr = vec![0u8; rest];
    s.read_exact(&mut addr).unwrap();
    s
}

fn local_tunnel(name: &str, port: u16, local: u16) -> String {
    format!(
        "[[tunnels]]\nname = \"{name}\"\ntype = \"ssh-local\"\nhost = \"127.0.0.1\"\nport = {port}\nuser = \"tw\"\n\
         forwards = [{{ local_port = {local}, remote_host = \"localhost\", remote_port = 7000 }}]\n"
    )
}

#[test]
#[ignore = "needs Docker; run with --ignored"]
fn local_forward_reaches_a_service_behind_the_server() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let lab = Lab::new(Keys::Allowed);
    let local = free_port();
    lab.config(&local_tunnel("db", lab.target_port, local));
    lab.start_ok("db");
    assert_eq!(echo(local, "hello"), "hello");
    let s = lab.status("db");
    assert_eq!(s["state"], "running");
    assert!(s["pid"].as_u64().is_some());
}

#[test]
#[ignore = "needs Docker; run with --ignored"]
fn local_forward_through_a_jump_host() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let lab = Lab::new(Keys::Allowed);
    let local = free_port();
    // `target` is a name only the jump host can resolve.
    lab.config(&format!(
        "[[tunnels]]\nname = \"inner\"\ntype = \"ssh-local\"\nhost = \"target\"\nuser = \"tw\"\n\
         jump = [\"tw@127.0.0.1:{}\"]\n\
         forwards = [{{ local_port = {local}, remote_host = \"localhost\", remote_port = 7000 }}]\n",
        lab.jump_port
    ));
    lab.start_ok("inner");
    assert_eq!(echo(local, "via jump"), "via jump");
}

#[test]
#[ignore = "needs Docker; run with --ignored"]
fn dynamic_forward_is_a_socks_proxy() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let lab = Lab::new(Keys::Allowed);
    let local = free_port();
    lab.config(&format!(
        "[[tunnels]]\nname = \"socks\"\ntype = \"ssh-dynamic\"\nhost = \"127.0.0.1\"\nport = {}\nuser = \"tw\"\n\
         forwards = [{{ local_port = {local} }}]\n",
        lab.target_port
    ));
    lab.start_ok("socks");
    // `localhost` is resolved on the server, where the echo service runs.
    assert_eq!(echo_on(socks5(local, "localhost", 7000), "socks"), "socks");
}

#[test]
#[ignore = "needs Docker; run with --ignored"]
fn remote_forward_reaches_a_local_service() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let lab = Lab::new(Keys::Allowed);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let local = listener.local_addr().unwrap().port();
    lab.config(&format!(
        "[[tunnels]]\nname = \"share\"\ntype = \"ssh-remote\"\nhost = \"127.0.0.1\"\nport = {}\nuser = \"tw\"\n\
         forwards = [{{ local_port = {local}, remote_port = 9000 }}]\n",
        lab.target_port
    ));
    lab.start_ok("share");
    let server = std::thread::spawn(move || {
        let (s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut line = String::new();
        BufReader::new(&s).read_line(&mut line).unwrap();
        (&s).write_all(b"pong\n").unwrap();
        line.trim_end().to_owned()
    });
    let out = docker(&[
        "exec",
        &lab.target,
        "sh",
        "-c",
        "echo ping | socat -T 5 - TCP:127.0.0.1:9000",
    ]);
    assert_eq!(server.join().unwrap(), "ping");
    assert_eq!(text(&out), "pong");
}

#[test]
#[ignore = "needs Docker; run with --ignored"]
fn a_remote_port_in_use_is_a_port_conflict() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let lab = Lab::new(Keys::Allowed);
    // sshd binds a remote forward on `localhost`, both 127.0.0.1 and ::1, and
    // succeeds if either works. So take port 7100 on every address.
    docker(&[
        "exec",
        "-d",
        &lab.target,
        "socat",
        "TCP6-LISTEN:7100,ipv6only=0,fork",
        "EXEC:cat",
    ]);
    std::thread::sleep(Duration::from_millis(500));
    lab.config(&format!(
        "[[tunnels]]\nname = \"share\"\ntype = \"ssh-remote\"\nhost = \"127.0.0.1\"\nport = {}\nuser = \"tw\"\n\
         forwards = [{{ local_port = {}, remote_port = 7100 }}]\n",
        lab.target_port,
        free_port()
    ));
    let out = lab.start("share");
    assert!(!out.status.success());
    let s = lab.status("share");
    assert_eq!(s["state"], "failed", "{s:#}");
    assert_eq!(s["reason"], "port conflict", "{s:#}");
    assert!(
        s["last_error"]
            .as_str()
            .unwrap()
            .contains("the remote port is already in use on 127.0.0.1"),
        "{s:#}"
    );
    assert_eq!(s["restarts"], 0, "no retries");
}

#[test]
#[ignore = "needs Docker; run with --ignored"]
fn an_unknown_host_key_fails_with_the_trust_hint_until_trusted() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let lab = Lab::new(Keys::Allowed);
    lab.forget_hosts();
    let local = free_port();
    lab.config(&local_tunnel("db", lab.target_port, local));
    assert!(!lab.start("db").status.success());
    let s = lab.status("db");
    assert_eq!(s["state"], "failed", "{s:#}");
    assert_eq!(s["reason"], "host key", "{s:#}");
    assert!(
        s["last_error"]
            .as_str()
            .unwrap()
            .contains("run `tunwarden trust 127.0.0.1`"),
        "{s:#}"
    );
    assert_eq!(s["restarts"], 0, "no retries");

    // What `tunwarden trust` would write, then a plain start works.
    lab.trust_hosts();
    lab.start_ok("db");
    assert_eq!(echo(local, "trusted"), "trusted");
}

#[test]
#[ignore = "needs Docker; run with --ignored"]
fn a_refused_key_fails_with_credentials() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let lab = Lab::new(Keys::Refused);
    lab.config(&local_tunnel("db", lab.target_port, free_port()));
    assert!(!lab.start("db").status.success());
    let s = lab.status("db");
    assert_eq!(s["state"], "failed", "{s:#}");
    assert_eq!(s["reason"], "credentials", "{s:#}");
    assert!(
        s["last_error"]
            .as_str()
            .unwrap()
            .contains("127.0.0.1 refused your SSH key"),
        "{s:#}"
    );
    assert_eq!(s["restarts"], 0, "no retries");
}

#[test]
#[ignore = "needs Docker; run with --ignored"]
fn a_dropped_connection_reconnects() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let lab = Lab::new(Keys::Allowed);
    let local = free_port();
    lab.config(&local_tunnel("db", lab.target_port, local));
    lab.start_ok("db");
    let before = lab.status("db")["pid"].as_u64().unwrap();

    // Kill the server side of the session; ssh exits and Tunwarden reconnects.
    docker(&["exec", &lab.target, "pkill", "-u", "tw"]);
    let s = lab.wait_status("db", "a new ssh process", |s| {
        s["state"] == "running"
            && s["restarts"].as_u64() >= Some(1)
            && s["pid"].as_u64().is_some_and(|p| p != before)
    });
    assert_eq!(s["reason"], Value::Null);
    assert_eq!(echo(local, "again"), "again");
}

#[test]
#[ignore = "needs Docker; run with --ignored"]
fn a_command_tunnel_running_ssh_gets_the_ssh_defaults_for_failures() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let lab = Lab::new(Keys::Allowed);
    lab.forget_hosts();
    let local = free_port();
    // `ssh` as a command is located like the ssh types, so it uses the wrapper.
    lab.config(&format!(
        "[[tunnels]]\nname = \"raw\"\ncommand = \"ssh -N -o BatchMode=yes -p {} -L {local}:localhost:7000 tw@127.0.0.1\"\n\
         forwards = [{{ local_port = {local} }}]\n",
        lab.target_port
    ));
    assert!(!lab.start("raw").status.success());
    let s = lab.status("raw");
    assert_eq!(s["type"], "command");
    assert_eq!(s["reason"], "host key", "{s:#}");

    lab.trust_hosts();
    lab.start_ok("raw");
    assert_eq!(echo(local, "raw"), "raw");
}
