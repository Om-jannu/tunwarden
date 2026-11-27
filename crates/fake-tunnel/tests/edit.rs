//! Config edits from the CLI and hand edits while the daemon runs
//! (PRD FR-5, AC-12).

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

const CLI: &str = env!("CARGO_BIN_EXE_tw-test-cli");
const FAKE: &str = env!("CARGO_BIN_EXE_fake-tunnel");

static PORTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

    fn text(&self) -> String {
        std::fs::read_to_string(self.config()).unwrap()
    }

    fn write(&self, text: &str) {
        std::fs::write(self.config(), text).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(CLI)
            .arg("--config")
            .arg(self.config())
            .args(args)
            .env("TUNWARDEN_STATE_DIR", self.dir.path().join("state"))
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "tunwarden {args:?} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn fails(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(!out.status.success(), "tunwarden {args:?} should fail");
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    fn status(&self) -> Value {
        serde_json::from_str(&self.ok(&["status", "--json"])).unwrap()
    }

    /// Polls `status --json` until `f` holds (hand edits are picked up within a second).
    fn wait(&self, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let s = self.status();
            if f(&s) {
                return s;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: {s:#}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = self.run(&["daemon", "stop"]);
    }
}

fn tunnel<'a>(status: &'a Value, name: &str) -> Option<&'a Value> {
    status["tunnels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
}

fn names(status: &Value) -> Vec<String> {
    status["tunnels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn listening(port: u16) -> bool {
    TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(200),
    )
    .is_ok()
}

fn fake(port: u16) -> String {
    format!("'{FAKE}' --listen 127.0.0.1:{port}")
}

const HEADER: &str = "# My tunnels, edited by hand and by tunwarden.\nversion = 1\n";

#[test]
fn add_edit_remove_keep_comments() {
    let _turn = PORTS.lock().unwrap_or_else(|e| e.into_inner());
    let env = Env::new(HEADER);
    let (p1, p2) = (free_port(), free_port());

    let out = env.ok(&[
        "add",
        "db",
        "--command",
        &fake(p1),
        "--forward",
        &p1.to_string(),
        "--group",
        "daily",
    ]);
    assert!(out.contains("added db to"), "{out}");
    let text = env.text();
    assert!(text.starts_with(HEADER), "the comment survives:\n{text}");
    assert!(
        text.contains("name = \"db\"\ngroups = [\"daily\"]\ncommand = "),
        "{text}"
    );
    assert!(
        text.contains(&format!("forwards = [{{ local_port = {p1} }}]")),
        "{text}"
    );

    env.ok(&["start", "db"]);
    assert!(listening(p1));

    // Editing a running tunnel keeps it running until a restart.
    let out = env.ok(&[
        "edit",
        "db",
        "--command",
        &fake(p2),
        "--forward",
        &p2.to_string(),
    ]);
    assert!(out.contains("run `tunwarden restart db` to apply"), "{out}");
    let s = env.status();
    let db = tunnel(&s, "db").unwrap();
    assert_eq!(db["state"], "running");
    assert_eq!(db["pending_restart"], true);
    assert!(listening(p1), "not interrupted");

    env.ok(&["restart", "db"]);
    assert!(
        listening(p2) && !listening(p1),
        "the new settings apply after a restart"
    );
    assert_eq!(
        tunnel(&env.status(), "db").unwrap()["pending_restart"],
        false
    );

    // A stopped tunnel takes changes at once.
    env.ok(&["stop", "db"]);
    env.ok(&[
        "edit",
        "db",
        "--set",
        "auto_restart=false",
        "--unset",
        "groups",
    ]);
    let text = env.text();
    assert!(
        text.contains("auto_restart = false") && !text.contains("groups"),
        "{text}"
    );
    assert_eq!(
        tunnel(&env.status(), "db").unwrap()["pending_restart"],
        false
    );

    // Removing a running tunnel stops it.
    env.ok(&["start", "db"]);
    let out = env.ok(&["remove", "db"]);
    assert!(out.contains("removed db"), "{out}");
    assert!(names(&env.status()).is_empty());
    assert!(!listening(p2), "removed tunnels are stopped");
    assert_eq!(env.text(), HEADER, "back to the original file");
}

#[test]
fn invalid_edits_write_nothing() {
    let env = Env::new(HEADER);
    let before = env.text();
    let err = env.fails(&[
        "add",
        "redis",
        "--type",
        "ssh-local",
        "--forward",
        "6380:redis.internal:6379",
    ]);
    assert!(
        err.contains("the change would make the config invalid, so nothing was written"),
        "{err}"
    );
    assert!(
        err.contains("tunnels[0].host: a ssh-local tunnel needs `host`"),
        "{err}"
    );
    assert_eq!(env.text(), before);

    env.ok(&[
        "add",
        "redis",
        "--type",
        "ssh-local",
        "--set",
        "host=bastion",
        "--set",
        "jump=[\"j1\", \"j2\"]",
        "--forward",
        "6380:redis.internal:6379",
    ]);
    let text = env.text();
    assert!(
        text.contains("host = \"bastion\"\njump = [\"j1\", \"j2\"]\n"),
        "TOML values in --set: {text}"
    );
    assert!(text.contains("forwards = [{ local_port = 6380, remote_host = \"redis.internal\", remote_port = 6379 }]"), "{text}");

    let err = env.fails(&["add", "redis", "--command", "x"]);
    assert!(
        err.contains("a tunnel named \"redis\" already exists"),
        "{err}"
    );
    let err = env.fails(&["edit", "nope", "--set", "a=b"]);
    assert!(err.contains("no tunnel named \"nope\""), "{err}");
    let err = env.fails(&["edit", "redis"]);
    assert!(err.contains("nothing to change"), "{err}");
    let err = env.fails(&["add", "x", "--forward", "1:2:3:4"]);
    assert!(err.contains("--forward 1:2:3:4: not a forward"), "{err}");
}

#[test]
fn hand_edits_are_picked_up_while_the_daemon_runs() {
    // AC-12
    let _turn = PORTS.lock().unwrap_or_else(|e| e.into_inner());
    let (p1, p2) = (free_port(), free_port());
    let env = Env::new(&format!(
        "{HEADER}\n# the main database\n[[tunnels]]\nname = \"db\"\ncommand = \"{}\"\nforwards = [{{ local_port = {p1} }}]\n",
        fake(p1)
    ));
    env.ok(&["start", "db"]);

    // A tunnel added by hand appears, stopped; the running one is untouched.
    let hand = format!(
        "\n# added by hand\n[[tunnels]]\nname = \"api\"\ncommand = \"{}\"\nforwards = [{{ local_port = {p2} }}]\n",
        fake(p2)
    );
    env.write(&(env.text() + &hand));
    let s = env.wait("api to appear", |s| tunnel(s, "api").is_some());
    assert_eq!(tunnel(&s, "api").unwrap()["state"], "stopped");
    assert_eq!(tunnel(&s, "db").unwrap()["state"], "running");
    assert_eq!(
        tunnel(&s, "db").unwrap()["pending_restart"],
        false,
        "db did not change"
    );

    // An invalid hand edit is reported with its location; the daemon keeps going.
    let good = env.text();
    env.write(&good.replace("name = \"api\"", "name = \"api v2\""));
    let s = env.wait("the error", |s| {
        !s["config_errors"].as_array().unwrap().is_empty()
    });
    let error = s["config_errors"][0].as_str().unwrap();
    assert!(
        error.contains("tunnels[1].name: tunnel name \"api v2\" contains ' '"),
        "{error}"
    );
    assert!(
        error.contains("config.toml:"),
        "with the file and line: {error}"
    );
    assert_eq!(names(&s), ["api", "db"], "the last valid config stays");
    assert!(listening(p1));
    let err = env.fails(&["add", "x", "--command", "y"]);
    assert!(
        err.contains("cannot be edited") && err.contains("hint: fix"),
        "{err}"
    );

    // Fixing it clears the error.
    env.write(&good);
    env.wait("no errors", |s| {
        s["config_errors"].as_array().unwrap().is_empty()
    });

    // A tool edit after a hand edit keeps both, and every comment.
    env.write(&(env.text() + "\n# a hand comment at the end\n"));
    env.ok(&["add", "cache", "--command", "z"]);
    let text = env.text();
    for keep in [
        "# My tunnels",
        "# the main database",
        "# added by hand",
        "# a hand comment at the end",
        "name = \"api\"",
        "name = \"cache\"",
    ] {
        assert!(text.contains(keep), "{keep:?} missing:\n{text}");
    }
    assert_eq!(names(&env.status()), ["api", "cache", "db"]);

    // Removing a running tunnel by hand stops it.
    let without_db = env.text().replace(
        &format!(
            "[[tunnels]]\nname = \"db\"\ncommand = \"{}\"\nforwards = [{{ local_port = {p1} }}]\n",
            fake(p1)
        ),
        "",
    );
    assert_ne!(without_db, env.text(), "the test removed db's entry");
    env.write(&without_db);
    env.wait("db gone", |s| tunnel(s, "db").is_none());
    let deadline = Instant::now() + Duration::from_secs(5);
    while listening(p1) {
        assert!(
            Instant::now() < deadline,
            "db still listens after being removed"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn profiles_and_logins_are_edited_and_removed_through_the_daemon() {
    // config.edit_entry and config.remove_entry, which the desktop app uses.
    use tunwarden_core::client::{Client, ClientError};
    use tunwarden_core::proto::{
        ConfigEditEntryParams, ConfigEditResult, ConfigRemoveEntryParams, method,
    };
    let env = Env::new(
        "version = 1\n\
         [[logins]]\nname = \"google\"\nlogin = \"gcloud auth login\"\n\
         [[profiles]]\nname = \"staging\"\nlogin = \"google\"  # shared\n\
         [[profiles]]\nname = \"spare\"\n\
         [[tunnels]]\nname = \"db\"\nprofile = \"staging\"\ncommand = \"x\"\n",
    );
    env.ok(&["daemon", "start"]);
    let state = env.dir.path().join("state");
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut c = Client::connect(&state, "test").await.unwrap();
        let code = |e: ClientError| match e {
            ClientError::Rpc(r) => r.data.map(|d| d.code).unwrap_or_default(),
            other => panic!("{other}"),
        };
        let mut set = serde_json::Map::new();
        set.insert("production".into(), Value::Bool(true));
        let r: ConfigEditResult = c
            .call(
                method::CONFIG_EDIT_ENTRY,
                ConfigEditEntryParams {
                    table: "profiles".into(),
                    name: "staging".into(),
                    set,
                    unset: vec![],
                },
            )
            .await
            .unwrap();
        assert_eq!(r.change, "updated");

        let e = c
            .call::<_, ConfigEditResult>(
                method::CONFIG_REMOVE_ENTRY,
                ConfigRemoveEntryParams {
                    table: "profiles".into(),
                    name: "staging".into(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(code(e), "in_use");

        let mut bad = serde_json::Map::new();
        bad.insert("login".into(), Value::String("nope".into()));
        let before = env.text();
        let e = c
            .call::<_, ConfigEditResult>(
                method::CONFIG_EDIT_ENTRY,
                ConfigEditEntryParams {
                    table: "profiles".into(),
                    name: "spare".into(),
                    set: bad,
                    unset: vec![],
                },
            )
            .await
            .unwrap_err();
        assert_eq!(code(e), "invalid_config");
        assert_eq!(env.text(), before, "an invalid edit writes nothing");

        let _: ConfigEditResult = c
            .call(
                method::CONFIG_REMOVE_ENTRY,
                ConfigRemoveEntryParams {
                    table: "profiles".into(),
                    name: "spare".into(),
                },
            )
            .await
            .unwrap();
        let e = c
            .call::<_, ConfigEditResult>(
                method::CONFIG_REMOVE_ENTRY,
                ConfigRemoveEntryParams {
                    table: "tunnels".into(),
                    name: "db".into(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(code(e), "invalid_edit", "tunnels use config.remove");
    });
    let text = env.text();
    assert!(
        text.contains("name = \"staging\"\nlogin = \"google\"  # shared\nproduction = true\n"),
        "{text}"
    );
    assert!(!text.contains("spare"), "{text}");
    let list = env.ok(&["list"]);
    assert!(list.contains("staging (production)"), "{list}");
}
