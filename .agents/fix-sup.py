import sys

root = "/Users/omjannu/Projects/tunwarden/crates/fake-tunnel/tests/"

def edit(p, pairs):
    s = open(root + p).read()
    for a, b in pairs:
        if a not in s:
            sys.exit(f"not found in {p}: {a[:80]!r}")
        s = s.replace(a, b, 1)
    open(root + p, "w").write(s)

def cut(p, start, end, new):
    s = open(root + p).read()
    i = s.index(start)
    j = s.index(end, i)
    open(root + p, "w").write(s[:i] + new + s[j:])

cut("supervisor.rs", "/// Loads a config with one project `p`", "fn start(spec: Spec)", '''/// Loads a config with a login `work` (expired when the output says
/// "Reauthentication failed"), a profile `p` that uses it, and the given tunnels.
fn load(tunnels: &str) -> Config {
    let src = format!(
        "version = 1\\n[[logins]]\\nname = \\"work\\"\\nlogin = \\"true\\"\\nexpired = [\\"Reauthentication failed\\"]\\n\\
         [[profiles]]\\nname = \\"p\\"\\nlogin = \\"work\\"\\nenv = {{ TOOL_PROJECT = \\"example-p-1\\" }}\\n\\n{tunnels}"
    );
    match config::parse(&src, Path::new("test.toml"), &SystemEnv) {
        Ok(c) => c,
        Err(e) => panic!("bad test config:\\n{e}"),
    }
}

/// A command tunnel running fake-tunnel with `args`, optionally listening on `port`.
fn tunnel(name: &str, args: &str, port: Option<u16>, profile: bool) -> String {
    let mut t = format!("[[tunnels]]\\nname = \\"{name}\\"\\ncommand = \\"'{FAKE}' {args}\\"\\n");
    if let Some(port) = port {
        t.push_str(&format!("forwards = [{{ local_port = {port} }}]\\n"));
    }
    if profile {
        t.push_str("profile = \\"p\\"\\n");
    }
    t
}

/// A spec with short timings.
fn spec(config: &Config, index: usize) -> Spec {
    let mut s = Spec::new(&config.tunnels[index], config);
    s.policy.starting_probe = Duration::from_millis(50);
    s.policy.health_interval = Duration::from_millis(200);
    s.policy.startup_grace = Duration::from_secs(5);
    s.policy.backoff_base = Duration::from_millis(100);
    s.policy.backoff_max = Duration::from_millis(400);
    s.policy.stable_after = Duration::from_secs(30);
    s.stop_grace = Duration::from_secs(1);
    s
}

''')
edit("supervisor.rs", [
("    // AC-3: a custom gcloud-style command in a project prints the real gcloud text.",
 "    // AC-3: a tunnel whose profile uses a login prints that login's expired text\n    // (here the real gcloud wording)."),
('        Some("gcloud login expired; run `tunwarden login p`")', '        Some("the work login has expired; run `tunwarden login work`")'),
])
cut("supervisor.rs", "async fn two_projects_get_their_own_environment() {", "#[tokio::test]\nasync fn config_changes_wait_for_a_restart()", '''async fn two_profiles_get_their_own_environment() {
    // AC-8: each tunnel gets its own profile's variables, set only in its environment.
    let _turn = PORTS.lock().await;
    let src = format!(
        "version = 1\\n[[profiles]]\\nname = \\"a\\"\\nenv = {{ TOOL_PROJECT = \\"example-a-1\\" }}\\n\\
         [[profiles]]\\nname = \\"b\\"\\nenv = {{ TOOL_PROJECT = \\"example-b-2\\", TOOL_ACCOUNT = \\"b@example.com\\" }}\\n\\
         [[tunnels]]\\nname = \\"ta\\"\\nprofile = \\"a\\"\\ncommand = \\"'{FAKE}' --print-env TOOL_PROJECT --print-env TOOL_ACCOUNT\\"\\n\\
         [[tunnels]]\\nname = \\"tb\\"\\nprofile = \\"b\\"\\ncommand = \\"'{FAKE}' --print-env TOOL_PROJECT --print-env TOOL_ACCOUNT\\"\\n"
    );
    let c = config::parse(&src, Path::new("t.toml"), &SystemEnv).unwrap();
    let (a, ra) = start(spec(&c, 0));
    let (b, rb) = start(spec(&c, 1));
    a.send(Event::Start);
    b.send(Event::Start);
    until("output", Duration::from_secs(5), || ra.output("TOOL_ACCOUNT")).await;
    until("output", Duration::from_secs(5), || rb.output("TOOL_ACCOUNT")).await;

    assert!(ra.output("env TOOL_PROJECT=example-a-1"));
    assert!(ra.output("env TOOL_ACCOUNT=<unset>"));
    assert!(rb.output("env TOOL_PROJECT=example-b-2"));
    assert!(rb.output("env TOOL_ACCOUNT=b@example.com"));
    assert!(std::env::var("TOOL_PROJECT").is_err(), "Tunwarden's own environment is untouched");
    a.shutdown().await;
    b.shutdown().await;
}

''')
s = open(root + "persist.rs").read()
s = s.replace("Spec::new(&c.tunnels[0], None, &c.tools)", "Spec::new(&c.tunnels[0], &c)")
open(root + "persist.rs", "w").write(s)
print("ok")
