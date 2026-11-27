//! `tunwarden import`, `doctor` and `trust` run as processes (PRD FR-6,
//! FR-36, FR-42, FR-43).

use std::io::Write;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::Value;

const CLI: &str = env!("CARGO_BIN_EXE_tw-test-cli");
const FAKE: &str = env!("CARGO_BIN_EXE_fake-tunnel");

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new(config: &str) -> Env {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), config).unwrap();
        Env { dir }
    }

    fn config(&self) -> PathBuf {
        self.dir.path().join("config.toml")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new(CLI);
        c.arg("--config")
            .arg(self.config())
            .args(args)
            .env("TUNWARDEN_STATE_DIR", self.dir.path().join("state"));
        c
    }

    fn run(&self, args: &[&str], input: &str) -> Output {
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = self
            .command(&["daemon", "stop"])
            .stdin(Stdio::null())
            .output();
    }
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

const ALIASES: &str = "\
# from my .zshrc
alias uatdb='CLOUDSDK_CORE_PROJECT=uat-project-123 gcloud compute start-iap-tunnel uat-vm 3371 --zone=asia-south1-b --local-host-port=localhost:3371'
alias redis='ssh -f -N -L 6380:redis.internal:6379 bastion'
alias grafana='kubectl --context example -n monitoring port-forward svc/grafana 3000:80'
alias vpn='my-vpn-helper --up'
";

#[test]
fn import_prints_then_writes() {
    let env = Env::new("# keep me\nversion = 1\n");
    let out = env.run(&["import"], ALIASES);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let toml = text(&out.stdout);
    assert!(
        toml.contains(
            "[[profiles]]\nname = \"imported\"\nenv = { CLOUDSDK_CORE_PROJECT = \"uat-project-123\" }"
        ),
        "{toml}"
    );
    assert!(
        toml.contains("name = \"uatdb\"\ntype = \"command\"\nprofile = \"imported\"\ncommand = \"gcloud compute"),
        "{toml}"
    );
    assert!(
        toml.contains("name = \"redis\"\ntype = \"ssh-local\""),
        "{toml}"
    );
    assert!(
        toml.contains("name = \"grafana\"\ntype = \"command\""),
        "{toml}"
    );
    assert!(
        toml.contains("name = \"vpn\"\ntype = \"command\"\ncommand = \"my-vpn-helper --up\""),
        "{toml}"
    );
    let err = text(&out.stderr);
    assert!(err.contains("warning: line 3: uses `ssh -f`"), "{err}");
    assert!(
        err.contains("warning: line 5: no local port found"),
        "{err}"
    );
    assert!(err.contains("run the same command with --write"), "{err}");
    assert_eq!(
        std::fs::read_to_string(env.config()).unwrap(),
        "# keep me\nversion = 1\n",
        "nothing written yet"
    );

    let out = env.run(&["import", "--write"], ALIASES);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(
        stdout.contains("added profile imported") && stdout.contains("added tunnel vpn"),
        "{stdout}"
    );
    let file = std::fs::read_to_string(env.config()).unwrap();
    assert!(file.starts_with("# keep me\n"), "{file}");
    let list = env.run(&["list", "--json"], "");
    let json: Value = serde_json::from_slice(&list.stdout).unwrap();
    let names: Vec<&str> = json["tunnels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["uatdb", "redis", "grafana", "vpn"]);

    // Importing again picks new names instead of clashing.
    let out = env.run(&["import"], "alias redis='ssh -N -L 6381:r:6379 bastion'\n");
    assert!(
        text(&out.stdout).contains("name = \"redis-2\""),
        "{}",
        text(&out.stdout)
    );

    let out = env.run(&["import"], "# only comments\n\n");
    assert!(!out.status.success());
    assert!(text(&out.stderr).contains("no tunnel commands found"));
}

#[test]
fn doctor_reports_problems_and_exits_non_zero() {
    let squatter = TcpListener::bind("127.0.0.1:0").unwrap();
    let busy = squatter.local_addr().unwrap().port();
    let free = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let env = Env::new(&format!(
        "version = 1\n[tools]\nssh = \"/nonexistent/ssh\"\n\
         [[tunnels]]\nname = \"ok\"\ncommand = \"'{FAKE}'\"\nforwards = [{{ local_port = {free} }}]\n\
         [[tunnels]]\nname = \"taken\"\ncommand = \"'{FAKE}'\"\nforwards = [{{ local_port = {busy} }}]\n\
         [[tunnels]]\nname = \"k\"\ntype = \"ssh-local\"\nhost = \"bastion\"\nforwards = [{{ local_port = 1, remote_port = 2 }}]\n"
    ));
    let out = env.run(&["doctor"], "");
    let report = text(&out.stdout);
    assert!(
        !out.status.success(),
        "problems give a non-zero exit:\n{report}"
    );
    assert!(report.contains("Config\n  ok"), "{report}");
    assert!(report.contains("ok       not running"), "{report}");
    assert!(
        report.contains("ok       fake-tunnel"),
        "commands show their file name:\n{report}"
    );
    assert!(
        report.contains("problem  ssh")
            && report.contains("/nonexistent/ssh in [tools], but that file does not exist"),
        "{report}"
    );
    assert!(
        report.contains("problem  taken")
            && report.contains(&format!("127.0.0.1:{busy} is in use by")),
        "{report}"
    );
    assert!(!report.contains("problem  ok"), "{report}");
    assert!(report.contains("Logins\n  ok       none"), "{report}");
    drop(squatter);

    let env = Env::new("version = 1\n[[tunnels]]\nname = \"bad name\"\ncommand = \"x\"\n");
    let out = env.run(&["doctor"], "");
    assert!(!out.status.success());
    assert!(
        text(&out.stdout).contains("problem")
            && text(&out.stdout).contains("tunnel name \"bad name\""),
        "{}",
        text(&out.stdout)
    );
}

#[test]
fn trust_refuses_non_ssh_tunnels() {
    let env = Env::new("version = 1\n[[tunnels]]\nname = \"vpn\"\ncommand = \"x\"\n");
    let out = env.run(&["trust", "vpn"], "");
    assert!(!out.status.success());
    let err = text(&out.stderr);
    assert!(
        err.contains("vpn is a command tunnel, not an SSH tunnel"),
        "{err}"
    );
}
