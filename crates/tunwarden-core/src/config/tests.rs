use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::*;
use crate::paths::tests::FakeEnv;

const EXAMPLE: &str = include_str!("../../../tunwarden-cli/example-config.toml");

fn env() -> FakeEnv {
    FakeEnv::with_home("/home/me")
}

fn ok(src: &str) -> Config {
    match parse(src, Path::new("config.toml"), &env()) {
        Ok(config) => config,
        Err(errors) => panic!("expected a valid config, got:\n{errors}"),
    }
}

fn errors(src: &str) -> Vec<ConfigError> {
    match parse(src, Path::new("config.toml"), &env()) {
        Ok(_) => panic!("expected errors"),
        Err(ConfigErrors(errors)) => errors,
    }
}

/// The only error, as `line:col field: message`.
fn one_error(src: &str) -> String {
    let errs = errors(src);
    assert_eq!(errs.len(), 1, "expected one error, got: {errs:#?}");
    let e = &errs[0];
    format!(
        "{}:{} {}: {}",
        e.position.line, e.position.column, e.field, e.message
    )
}

#[test]
fn example_config_is_valid() {
    let config = ok(EXAMPLE);
    assert_eq!(config.version, 1);
    assert_eq!(config.logins.len(), 2);
    assert_eq!(config.profiles.len(), 3);
    assert_eq!(config.tunnels.len(), 6);

    let google = config.login("google").unwrap();
    assert_eq!(
        google.login.as_deref().unwrap(),
        ["gcloud", "auth", "login", "me@example.com"]
    );
    assert_eq!(
        google.env,
        [(
            "CLOUDSDK_CORE_ACCOUNT".to_owned(),
            "me@example.com".to_owned()
        )]
    );
    let prod = config.profile("prod").unwrap();
    assert!(prod.production);
    assert_eq!(prod.login.as_deref(), Some("google"));

    let db = &config.tunnels[0];
    assert_eq!(db.kind, TunnelKind::Command);
    assert_eq!(db.profile.as_deref(), Some("staging"));
    assert_eq!(db.groups, ["daily"]);
    assert_eq!(db.forwards[0].bind, IpAddr::V4(Ipv4Addr::LOCALHOST));
    assert_eq!(db.forwards[0].local_port, 5433);
    let (profile, login) = config.context_of(db);
    assert_eq!(profile.map(|p| p.name.as_str()), Some("staging"));
    assert_eq!(login.map(|l| l.name.as_str()), Some("google"));
}

#[test]
fn defaults() {
    let config = ok("version = 1\n[[tunnels]]\nname = \"t\"\ncommand = \"x\"\n");
    assert_eq!(config.settings, Settings::default());
    let t = &config.tunnels[0];
    assert_eq!(t.kind, TunnelKind::Command);
    assert!(t.auto_restart);
    assert!(!t.auto_start);
    assert_eq!(t.max_retries, 10);
    assert_eq!(t.health_interval, Duration::from_secs(15));
    assert!(
        t.forwards.is_empty(),
        "command tunnels may have no forwards (process-alive readiness)"
    );
}

#[test]
fn empty_file_needs_version() {
    assert_eq!(
        one_error(""),
        "1:1 version: missing `version = 1` at the top of the file"
    );
}

#[test]
fn newer_version_is_refused() {
    assert_eq!(
        one_error("version = 2\n"),
        "1:11 version: this config uses version 2, but this Tunwarden only supports version 1; upgrade Tunwarden"
    );
}

#[test]
fn syntax_error_has_position() {
    let e = one_error("version = 1\n[[tunnels]\n");
    assert!(e.starts_with("2:"), "{e}");
}

#[test]
fn type_error_has_position() {
    let e = one_error(
        "version = 1\n[[tunnels]]\nname = \"t\"\ncommand = \"x\"\nauto_restart = \"yes\"\n",
    );
    assert!(e.starts_with("5:"), "{e}");
}

#[test]
fn unknown_top_level_and_forward_fields_are_rejected() {
    assert!(one_error("version = 1\ncolour = 1\n").contains("unknown field `colour`"));
    let e = one_error(
        "version = 1\n[[tunnels]]\nname = \"t\"\ncommand = \"x\"\nforwards = [{ local_port = 1, remote = 2 }]\n",
    );
    assert!(
        e.starts_with("5:") && e.contains("unknown field `remote`"),
        "{e}"
    );
}

#[test]
fn bad_names() {
    let cases = [
        ("", "is empty"),
        ("has space", "contains ' '"),
        ("-x", "must start with a letter or digit"),
        ("con", "reserved on Windows"),
        ("Com3.db", "reserved on Windows"),
        ("café", "contains 'é'"),
    ];
    for (name, expected) in cases {
        let src = format!("version = 1\n[[tunnels]]\nname = \"{name}\"\ncommand = \"x\"\n");
        let e = one_error(&src);
        assert!(e.contains(expected), "{name:?}: {e}");
        assert!(e.starts_with("3:8 tunnels[0].name"), "{name:?}: {e}");
    }
    assert!(check_name("com0").is_ok());
    assert!(check_name("console").is_ok());
    assert!(check_name(&"a".repeat(65)).is_err());
}

#[test]
fn duplicate_names_ignore_case() {
    let e = one_error(
        "version = 1\n[[tunnels]]\nname = \"db\"\ncommand = \"x\"\n[[tunnels]]\nname = \"DB\"\ncommand = \"x\"\n",
    );
    assert_eq!(
        e,
        "6:8 tunnels[1].name: tunnel name \"DB\" is already used by tunnels[0] (names are compared ignoring case, because they become file names)"
    );
}

#[test]
fn unknown_type() {
    let e = one_error("version = 1\n[[tunnels]]\nname = \"t\"\ntype = \"gcloud-iap\"\n");
    assert!(
        e.starts_with(
            "4:8 tunnels[0].type: unknown type \"gcloud-iap\"; use one of: command, ssh-local,"
        ),
        "{e}"
    );
    assert!(
        e.contains("Any other tool runs as type = \"command\""),
        "{e}"
    );
}

#[test]
fn profiles_and_logins() {
    // Any tunnel type may use a profile; it must exist.
    let c = ok(
        "version = 1\n[[profiles]]\nname = \"p\"\nenv = { A_B = \"1\" }\n\
         [[tunnels]]\nname = \"t\"\ntype = \"ssh-local\"\nhost = \"h\"\nprofile = \"p\"\nforwards = [{ local_port = 1, remote_port = 2 }]\n",
    );
    assert_eq!(c.tunnels[0].profile.as_deref(), Some("p"));
    assert_eq!(c.profiles[0].env, [("A_B".to_owned(), "1".to_owned())]);

    let e = one_error(
        "version = 1\n[[profiles]]\nname = \"staging\"\n[[tunnels]]\nname = \"t\"\ncommand = \"x\"\nprofile = \"stagin\"\n",
    );
    assert_eq!(
        e,
        "7:11 tunnels[0].profile: no profile named \"stagin\"; defined profiles: staging"
    );

    let e = one_error("version = 1\n[[profiles]]\nname = \"p\"\nlogin = \"nope\"\n");
    assert_eq!(
        e,
        "4:9 profiles[0].login: no login named \"nope\"; no logins are defined; add a [[logins]] entry"
    );

    let e = one_error("version = 1\n[[logins]]\nname = \"l\"\n");
    assert!(
        e.contains("login \"l\" needs `check`, `login`, or both"),
        "{e}"
    );

    let e = one_error("version = 1\n[[logins]]\nname = \"l\"\ncheck = \"tool status | grep ok\"\n");
    assert!(e.starts_with("4:9 logins[0].check: uses a pipe"), "{e}");

    let e =
        one_error("version = 1\n[[logins]]\nname = \"l\"\nlogin = \"x\"\nexpired = [\"(bad\"]\n");
    assert!(
        e.starts_with("5:12 logins[0].expired[0]: invalid regular expression"),
        "{e}"
    );

    let e = one_error("version = 1\n[[profiles]]\nname = \"p\"\nenv = { \"BAD-NAME\" = \"1\" }\n");
    assert!(
        e.contains("\"BAD-NAME\" is not a valid environment variable name"),
        "{e}"
    );

    let e = one_error("version = 1\n[[profiles]]\nname = \"p\"\n[[profiles]]\nname = \"P\"\n");
    assert!(
        e.starts_with("5:8 profiles[1].name: profile name \"P\" is already used by profiles[0]"),
        "{e}"
    );
}

#[test]
fn built_in_types_need_a_forward() {
    let e =
        one_error("version = 1\n[[tunnels]]\nname = \"t\"\ntype = \"ssh-local\"\nhost = \"h\"\n");
    assert!(e.contains("needs at least one forward"), "{e}");
}

#[test]
fn ports_are_checked() {
    let e = one_error(
        "version = 1\n[[tunnels]]\nname = \"t\"\ncommand = \"x\"\nforwards = [{ local_port = 70000 }]\n",
    );
    assert_eq!(
        e,
        "5:28 tunnels[0].forwards[0].local_port: 70000 is out of range; use 1–65535"
    );
    let e = one_error(
        "version = 1\n[[tunnels]]\nname = \"t\"\ncommand = \"x\"\nforwards = [{ remote_port = 5 }]\n",
    );
    assert!(e.contains("missing `local_port`"), "{e}");
}

#[test]
fn bind_address() {
    let c = ok(
        "version = 1\n[[tunnels]]\nname = \"t\"\ncommand = \"x\"\nforwards = [{ bind = \"::1\", local_port = 5 }]\n",
    );
    assert_eq!(
        c.tunnels[0].forwards[0].bind,
        "::1".parse::<IpAddr>().unwrap()
    );
    let e = one_error(
        "version = 1\n[[tunnels]]\nname = \"t\"\ncommand = \"x\"\nforwards = [{ bind = \"localhost\", local_port = 5 }]\n",
    );
    assert!(e.contains("is not an IP address"), "{e}");
}

#[test]
fn local_address_conflicts() {
    let base = "version = 1\n[[tunnels]]\nname = \"a\"\ncommand = \"x\"\nforwards = [{ local_port = 5433 }]\n[[tunnels]]\nname = \"b\"\ncommand = \"x\"\n";

    let e = one_error(&format!("{base}forwards = [{{ local_port = 5433 }}]\n"));
    assert_eq!(
        e,
        "9:28 tunnels[1].forwards[0].local_port: local address 127.0.0.1:5433 is already used by tunnel \"a\" (tunnels[0].forwards[0]); choose another local_port"
    );

    // A wildcard bind conflicts with any address on the same port.
    let e = one_error(&format!(
        "{base}forwards = [{{ bind = \"0.0.0.0\", local_port = 5433 }}]\n"
    ));
    assert!(e.contains("0.0.0.0:5433 is already used"), "{e}");

    // Different addresses on the same port are fine.
    ok(&format!(
        "{base}forwards = [{{ bind = \"127.0.0.2\", local_port = 5433 }}]\n"
    ));

    // ssh-remote's local port is a target, not a listen address.
    ok(
        "version = 1\n[[tunnels]]\nname = \"a\"\ncommand = \"x\"\nforwards = [{ local_port = 5433 }]\n\
        [[tunnels]]\nname = \"b\"\ntype = \"ssh-remote\"\nhost = \"h\"\nforwards = [{ local_port = 5433, remote_port = 9000 }]\n",
    );
}

#[test]
fn ranges_for_settings() {
    let e = one_error("version = 1\n[settings]\nlogin_check_interval_minutes = 0\n");
    assert_eq!(
        e,
        "3:32 settings.login_check_interval_minutes: 0 is out of range; use 1–1440 minutes"
    );
    let e = one_error(
        "version = 1\n[[tunnels]]\nname = \"t\"\ncommand = \"x\"\nhealth_interval_secs = -1\n",
    );
    assert!(e.contains("-1 is out of range; use 1–3600 seconds"), "{e}");
}

#[test]
fn tool_paths() {
    let c =
        ok("version = 1\n[tools]\nssh = \"~/bin/ssh\"\npath = [\"~/sdk/bin\", \"/opt/tools\"]\n");
    assert_eq!(c.tools.ssh, Some(PathBuf::from("/home/me").join("bin/ssh")));
    assert_eq!(
        c.tools.path,
        [
            PathBuf::from("/home/me").join("sdk/bin"),
            PathBuf::from("/opt/tools")
        ]
    );
    let e = one_error("version = 1\n[tools]\nssh = \"bin/ssh\"\n");
    assert!(e.contains("must be an absolute path"), "{e}");
}

#[test]
fn all_errors_are_reported_in_file_order() {
    let src = "version = 1\n\
        [[tunnels]]\nname = \"bad name\"\ncommand = \"x\"\nforwards = [{ local_port = 1 }]\n\
        [[tunnels]]\nname = \"b\"\ntype = \"nope\"\n\
        [[tunnels]]\nname = \"c\"\ncommand = \"x\"\nforwards = [{ local_port = 1 }]\n";
    let errs = errors(src);
    let fields: Vec<&str> = errs.iter().map(|e| e.field.as_str()).collect();
    assert_eq!(
        fields,
        [
            "tunnels[0].name",
            "tunnels[1].type",
            "tunnels[2].forwards[0].local_port"
        ]
    );
    let shown = ConfigErrors(errs).to_string();
    assert!(
        shown.starts_with("config.toml:3:8: tunnels[0].name: tunnel name \"bad name\""),
        "{shown}"
    );
    assert_eq!(shown.lines().count(), 3);
}

#[test]
fn load_reports_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.toml");
    let err = load(&missing).unwrap_err();
    assert!(matches!(err, LoadError::Io { .. }));
    assert!(err.to_string().contains("cannot read config file"));
}

#[test]
fn load_reads_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config.toml");
    std::fs::write(&file, "version = 1\n").unwrap();
    assert_eq!(load(&file).unwrap().tunnels.len(), 0);
}
