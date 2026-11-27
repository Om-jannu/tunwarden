import sys

p = "/Users/omjannu/Projects/tunwarden/crates/tunwarden-core/src/config/tests.rs"
s = open(p).read()

def cut(start_fn, new):
    """Replace the whole test function starting at `#[test]\\nfn start_fn(`."""
    global s
    key = f"fn {start_fn}("
    i = s.index(key)
    i = s.rindex("#[test]", 0, i)
    j = s.find("\n#[test]", i + 1)
    j = len(s) if j < 0 else j + 1
    s = s[:i] + new + s[j:]

cut("example_config_is_valid", '''#[test]
fn example_config_is_valid() {
    let config = ok(EXAMPLE);
    assert_eq!(config.version, 1);
    assert_eq!(config.logins.len(), 2);
    assert_eq!(config.profiles.len(), 3);
    assert_eq!(config.tunnels.len(), 6);

    let google = config.login("google").unwrap();
    assert_eq!(google.login.as_deref().unwrap(), ["gcloud", "auth", "login", "me@example.com"]);
    assert_eq!(google.env, [("CLOUDSDK_CORE_ACCOUNT".to_owned(), "me@example.com".to_owned())]);
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

''')
s = s.replace("    assert_eq!(t.kind, TunnelKind::Custom);", "    assert_eq!(t.kind, TunnelKind::Command);")
s = s.replace('"custom may have no forwards (process-alive readiness)"', '"command tunnels may have no forwards (process-alive readiness)"')

cut("unknown_type", '''#[test]
fn unknown_type() {
    let e = one_error("version = 1\\n[[tunnels]]\\nname = \\"t\\"\\ntype = \\"gcloud-iap\\"\\n");
    assert!(
        e.starts_with("4:8 tunnels[0].type: unknown type \\"gcloud-iap\\"; use one of: command, ssh-local,"),
        "{e}"
    );
    assert!(e.contains("Any other tool runs as type = \\"command\\""), "{e}");
}

''')

cut("project_rules", '''#[test]
fn profiles_and_logins() {
    // Any tunnel type may use a profile; it must exist.
    let c = ok(
        "version = 1\\n[[profiles]]\\nname = \\"p\\"\\nenv = { A_B = \\"1\\" }\\n\\
         [[tunnels]]\\nname = \\"t\\"\\ntype = \\"ssh-local\\"\\nhost = \\"h\\"\\nprofile = \\"p\\"\\nforwards = [{ local_port = 1, remote_port = 2 }]\\n",
    );
    assert_eq!(c.tunnels[0].profile.as_deref(), Some("p"));
    assert_eq!(c.profiles[0].env, [("A_B".to_owned(), "1".to_owned())]);

    let e = one_error(
        "version = 1\\n[[profiles]]\\nname = \\"staging\\"\\n[[tunnels]]\\nname = \\"t\\"\\ncommand = \\"x\\"\\nprofile = \\"stagin\\"\\n",
    );
    assert_eq!(e, "7:11 tunnels[0].profile: no profile named \\"stagin\\"; defined profiles: staging");

    let e = one_error("version = 1\\n[[profiles]]\\nname = \\"p\\"\\nlogin = \\"nope\\"\\n");
    assert_eq!(e, "4:9 profiles[0].login: no login named \\"nope\\"; no logins are defined; add a [[logins]] entry");

    let e = one_error("version = 1\\n[[logins]]\\nname = \\"l\\"\\n");
    assert!(e.contains("login \\"l\\" needs `check`, `login`, or both"), "{e}");

    let e = one_error("version = 1\\n[[logins]]\\nname = \\"l\\"\\ncheck = \\"tool status | grep ok\\"\\n");
    assert!(e.starts_with("4:9 logins[0].check: uses a pipe"), "{e}");

    let e = one_error("version = 1\\n[[logins]]\\nname = \\"l\\"\\nlogin = \\"x\\"\\nexpired = [\\"(bad\\"]\\n");
    assert!(e.starts_with("5:12 logins[0].expired[0]: invalid regular expression"), "{e}");

    let e = one_error("version = 1\\n[[profiles]]\\nname = \\"p\\"\\nenv = { \\"BAD-NAME\\" = \\"1\\" }\\n");
    assert!(e.contains("\\"BAD-NAME\\" is not a valid environment variable name"), "{e}");

    let e = one_error("version = 1\\n[[profiles]]\\nname = \\"p\\"\\n[[profiles]]\\nname = \\"P\\"\\n");
    assert!(e.starts_with("6:8 profiles[1].name: profile name \\"P\\" is already used by profiles[0]"), "{e}");
}

''')

s = s.replace('auth_check_interval_minutes = 0', 'login_check_interval_minutes = 0')
s = s.replace('settings.auth_check_interval_minutes: 0 is out of range', 'settings.login_check_interval_minutes: 0 is out of range')
s = s.replace('"3:31 settings.login_check_interval_minutes', '"3:32 settings.login_check_interval_minutes')

cut("tool_and_config_dir_paths", '''#[test]
fn tool_paths() {
    let c = ok("version = 1\\n[tools]\\nssh = \\"~/bin/ssh\\"\\npath = [\\"~/sdk/bin\\", \\"/opt/tools\\"]\\n");
    assert_eq!(c.tools.ssh, Some(PathBuf::from("/home/me").join("bin/ssh")));
    assert_eq!(c.tools.path, [PathBuf::from("/home/me").join("sdk/bin"), PathBuf::from("/opt/tools")]);
    let e = one_error("version = 1\\n[tools]\\nssh = \\"bin/ssh\\"\\n");
    assert!(e.contains("must be an absolute path"), "{e}");
}

''')
cut("empty_project_fields", "")

open(p, "w").write(s)
print("ok", s.count("project"))
