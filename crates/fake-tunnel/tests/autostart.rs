//! `tunwarden autostart` run as a process (PRD FR-15). HOME and
//! XDG_CONFIG_HOME point at a temp directory, so the user's real login items
//! are never touched. Nothing is loaded into launchd or systemd here.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const CLI: &str = env!("CARGO_BIN_EXE_tw-test-cli");

struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new() -> Home {
        Home {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(CLI)
            .arg("--config")
            .arg(self.path("config.toml"))
            .args(args)
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.path(".config"))
            .env("TUNWARDEN_STATE_DIR", self.path("state"))
            .env("PATH", "/opt/tools/bin:/usr/bin:/bin")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[cfg(target_os = "macos")]
#[test]
fn enable_status_disable_on_macos() {
    let home = Home::new();
    let plist = home.path("Library/LaunchAgents/io.github.om-jannu.tunwarden.plist");

    let s = home.run(&["autostart", "status"]);
    assert_eq!(s.status.code(), Some(3), "not enabled yet");
    assert!(out(&s).contains("autostart is not enabled"));

    let e = home.run(&["autostart", "enable"]);
    assert!(e.status.success(), "{}{}", out(&e), err(&e));
    assert!(out(&e).contains(&format!("autostart enabled: {}", plist.display())));
    assert!(out(&e).contains("daemon run"), "{}", out(&e));
    assert!(out(&e).contains("note: it takes effect at your next login"));
    assert!(
        err(&e).contains("warning: this tunwarden is a build in a cargo target directory"),
        "the test binary lives in target/: {}",
        err(&e)
    );

    let text = std::fs::read_to_string(&plist).unwrap();
    assert!(text.contains(&format!("<string>{CLI}</string>")), "{text}");
    assert!(
        text.contains("<string>/opt/tools/bin:/usr/bin:/bin</string>"),
        "the shell's PATH"
    );
    assert!(
        text.contains("<key>TUNWARDEN_STATE_DIR</key>"),
        "the overridden state dir"
    );
    assert!(text.contains(&format!("{}/daemon.log", home.path("state").display())));
    let lint = Command::new("plutil")
        .arg("-lint")
        .arg(&plist)
        .output()
        .unwrap();
    assert!(lint.status.success(), "{}", out(&lint));

    let s = home.run(&["autostart", "status"]);
    assert!(s.status.success(), "{}{}", out(&s), err(&s));
    assert!(out(&s).contains(&format!("it starts {CLI}")));
    let doctor = out(&home.run(&["doctor"]));
    assert!(doctor.contains("ok       autostart on"), "{doctor}");

    // Enabling again replaces the entry.
    assert!(home.run(&["autostart", "enable"]).status.success());

    // An entry whose binary is gone is a problem.
    std::fs::write(&plist, text.replace(CLI, "/nonexistent/tunwarden")).unwrap();
    let s = home.run(&["autostart", "status"]);
    assert!(!s.status.success());
    assert!(
        err(&s).contains("it starts /nonexistent/tunwarden, which no longer exists"),
        "{}",
        err(&s)
    );
    let doctor = home.run(&["doctor"]);
    assert!(!doctor.status.success());
    assert!(
        out(&doctor).contains("problem  autostart"),
        "{}",
        out(&doctor)
    );

    let d = home.run(&["autostart", "disable"]);
    assert!(d.status.success());
    assert!(out(&d).contains("autostart disabled; removed"));
    assert!(!plist.exists());
    let d = home.run(&["autostart", "disable"]);
    assert!(out(&d).contains("autostart was not enabled"));
}

#[cfg(target_os = "linux")]
#[test]
fn enable_without_a_systemd_session_fails_cleanly() {
    // Test machines have no systemd user session for this temp HOME, so
    // `systemctl --user` fails. The unit file must not be left behind.
    let home = Home::new();
    let unit = home.path(".config/systemd/user/tunwarden.service");
    let e = home.run(&["autostart", "enable"]);
    assert!(!e.status.success(), "{}", out(&e));
    assert!(
        err(&e).contains("autostart needs a systemd user session"),
        "{}",
        err(&e)
    );
    assert!(!unit.exists());
    assert_eq!(home.run(&["autostart", "status"]).status.code(), Some(3));
}

#[test]
fn help_lists_the_subcommands() {
    let home = Home::new();
    let h = home.run(&["autostart", "--help"]);
    let text = out(&h);
    for sub in ["enable", "disable", "status"] {
        assert!(text.contains(sub), "{text}");
    }
    let _ = Path::new(CLI);
}
