use std::path::Path;
use std::time::Duration;

use super::*;
use crate::config::{Config, ConfigErrors, parse};
use crate::paths::tests::FakeEnv;

const SSH_DEFAULTS: &str = "-N -T -o BatchMode=yes -o ControlMaster=no -o ControlPath=none -o ExitOnForwardFailure=yes -o ServerAliveInterval=15 -o ServerAliveCountMax=3";

const EXAMPLE: &str = include_str!("../../../tunwarden-cli/example-config.toml");

fn config(src: &str) -> Config {
    let src = format!("version = 1\n{src}");
    match parse(
        &src,
        Path::new("config.toml"),
        &FakeEnv::with_home("/home/me"),
    ) {
        Ok(config) => config,
        Err(errors) => panic!("expected a valid config, got:\n{errors}"),
    }
}

fn errors(src: &str) -> Vec<String> {
    let src = format!("version = 1\n{src}");
    match parse(
        &src,
        Path::new("config.toml"),
        &FakeEnv::with_home("/home/me"),
    ) {
        Ok(_) => panic!("expected errors"),
        Err(ConfigErrors(errors)) => errors
            .into_iter()
            .map(|e| format!("{}: {}", e.field, e.message))
            .collect(),
    }
}

/// The displayed command of the only tunnel.
fn command(src: &str) -> String {
    let c = config(src);
    let t = &c.tunnels[0];
    let (p, l) = c.context_of(t);
    t.launch_plan(p, l).display()
}

// --- Commands (FR-40) --------------------------------------------------------

#[test]
fn ssh_local_command() {
    let cmd = command(
        "[[tunnels]]\nname = \"redis\"\ntype = \"ssh-local\"\nhost = \"bastion\"\n\
         forwards = [{ local_port = 6380, remote_host = \"redis.internal\", remote_port = 6379 }]\n",
    );
    assert_eq!(
        cmd,
        format!("ssh {SSH_DEFAULTS} -L 127.0.0.1:6380:redis.internal:6379 bastion")
    );
}

#[test]
fn ssh_local_with_jump_user_port_and_ipv6() {
    let cmd = command(
        "[[tunnels]]\nname = \"db\"\ntype = \"ssh-local\"\nhost = \"10.0.0.5\"\nuser = \"deploy\"\nport = 2222\n\
         jump = [\"jump1.example.com\", \"admin@jump2.example.com\"]\n\
         forwards = [{ bind = \"::1\", local_port = 5433, remote_port = 5432 }, { local_port = 5434, remote_host = \"fe80::1\", remote_port = 5432 }]\n",
    );
    assert_eq!(
        cmd,
        format!(
            "ssh {SSH_DEFAULTS} -p 2222 -l deploy -J jump1.example.com,admin@jump2.example.com \
             -L '[::1]:5433:localhost:5432' -L '127.0.0.1:5434:[fe80::1]:5432' 10.0.0.5"
        )
    );
}

#[test]
fn ssh_remote_and_dynamic_commands() {
    let cmd = command(
        "[[tunnels]]\nname = \"share\"\ntype = \"ssh-remote\"\nhost = \"vps\"\nforwards = [{ local_port = 3000, remote_port = 8080 }]\n",
    );
    assert_eq!(
        cmd,
        format!("ssh {SSH_DEFAULTS} -R 8080:localhost:3000 vps")
    );
    let cmd = command(
        "[[tunnels]]\nname = \"socks\"\ntype = \"ssh-dynamic\"\nhost = \"bastion\"\nforwards = [{ local_port = 1080 }]\n",
    );
    assert_eq!(cmd, format!("ssh {SSH_DEFAULTS} -D 127.0.0.1:1080 bastion"));
}

#[test]
fn command_tunnels_run_any_program_with_the_profile_env() {
    let c = config(
        "[[profiles]]\nname = \"staging\"\nenv = { TOOL_PROJECT = \"example-123\", TOOL_ACCOUNT = \"me@example.com\" }\n\
         [[tunnels]]\nname = \"db\"\nprofile = \"staging\"\n\
         command = \"my-cloud connect 'vm 1' --zone z -i ~/.ssh/id\"\nforwards = [{ local_port = 8080 }]\n",
    );
    let t = &c.tunnels[0];
    let (p, l) = c.context_of(t);
    let plan = t.launch_plan(p, l);
    assert_eq!(plan.program, Program::Command("my-cloud".to_owned()));
    assert_eq!(
        plan.args,
        ["connect", "vm 1", "--zone", "z", "-i", "/home/me/.ssh/id"]
    );
    assert_eq!(
        plan.env,
        [
            ("TOOL_ACCOUNT".to_owned(), "me@example.com".to_owned()),
            ("TOOL_PROJECT".to_owned(), "example-123".to_owned())
        ]
    );
    assert_eq!(
        plan.display(),
        "TOOL_ACCOUNT=me@example.com TOOL_PROJECT=example-123 my-cloud connect 'vm 1' --zone z -i /home/me/.ssh/id"
    );

    // The login's env comes too, so the tunnel runs as the account the check
    // verifies; the profile wins on a clash.
    let c = config(
        "[[logins]]\nname = \"work\"\nlogin = \"tool login\"\nenv = { TOOL_ACCOUNT = \"me@example.com\", TOOL_REGION = \"eu\" }\n\
         [[profiles]]\nname = \"us\"\nlogin = \"work\"\nenv = { TOOL_REGION = \"us\" }\n\
         [[tunnels]]\nname = \"db\"\nprofile = \"us\"\ncommand = \"my-cloud connect\"\n",
    );
    let t = &c.tunnels[0];
    let (p, l) = c.context_of(t);
    assert_eq!(
        t.launch_plan(p, l).env,
        [
            ("TOOL_ACCOUNT".to_owned(), "me@example.com".to_owned()),
            ("TOOL_REGION".to_owned(), "us".to_owned())
        ]
    );

    let c = config("[[tunnels]]\nname = \"t\"\ncommand = \"ssh -N -L 1:a:2 host\"\n");
    assert_eq!(
        c.tunnels[0].launch_plan(None, None).program,
        Program::Tool(Tool::Ssh),
        "ssh is located like the ssh types"
    );
}

// --- Readiness (FR-16, FR-44) -----------------------------------------------

#[test]
fn readiness_per_type() {
    let c = config(
        "[[tunnels]]\nname = \"a\"\ntype = \"ssh-local\"\nhost = \"h\"\nforwards = [{ local_port = 1, remote_port = 2 }]\n\
         [[tunnels]]\nname = \"b\"\ntype = \"ssh-remote\"\nhost = \"h\"\nforwards = [{ local_port = 1, remote_port = 2 }]\n\
         [[tunnels]]\nname = \"c\"\ncommand = \"x\"\n\
         [[tunnels]]\nname = \"d\"\ncommand = \"x\"\nreadiness = \"process\"\nforwards = [{ local_port = 3 }]\n\
         [[tunnels]]\nname = \"e\"\ncommand = \"x\"\nforwards = [{ local_port = 4 }]\n",
    );
    let r: Vec<Readiness> = c.tunnels.iter().map(|t| t.readiness()).collect();
    assert_eq!(
        r[0],
        Readiness::Listening(vec!["127.0.0.1:1".parse().unwrap()])
    );
    assert_eq!(r[1], Readiness::AliveFor(Duration::from_secs(5)));
    assert_eq!(r[2], Readiness::AliveFor(Duration::from_secs(5)));
    assert_eq!(r[3], Readiness::AliveFor(Duration::from_secs(5)));
    assert_eq!(
        r[4],
        Readiness::Listening(vec!["127.0.0.1:4".parse().unwrap()])
    );
}

// --- Validation of type fields -----------------------------------------------

#[test]
fn required_type_fields() {
    let e = errors(
        "[[tunnels]]\nname = \"t\"\ntype = \"ssh-local\"\nforwards = [{ local_port = 1 }]\n",
    );
    assert_eq!(
        e,
        [
            "tunnels[0].host: a ssh-local tunnel needs `host`, for example `host = \"bastion\"`",
            "tunnels[0].forwards[0].remote_port: a ssh-local forward needs `remote_port`",
        ]
    );
    let e = errors("[[tunnels]]\nname = \"t\"\n");
    assert_eq!(e.len(), 1);
    assert!(
        e[0].starts_with("tunnels[0].command: a command tunnel needs `command`"),
        "{e:?}"
    );
}

#[test]
fn unknown_type_field_lists_valid_ones() {
    let e = errors(
        "[[tunnels]]\nname = \"t\"\ntype = \"ssh-dynamic\"\nhots = \"x\"\nhost = \"h\"\nforwards = [{ local_port = 1 }]\n",
    );
    assert_eq!(e.len(), 1, "{e:?}");
    assert!(
        e[0].starts_with(
            "tunnels[0].hots: unknown field `hots` for a ssh-dynamic tunnel; valid fields: "
        ),
        "{e:?}"
    );
    assert!(e[0].contains("host") && e[0].contains("profile"), "{e:?}");
}

#[test]
fn values_cannot_inject_options() {
    let e = errors(
        "[[tunnels]]\nname = \"t\"\ntype = \"ssh-local\"\nhost = \"-oProxyCommand=evil\"\njump = [\"ok\", \"bad host\"]\n\
         forwards = [{ local_port = 1, remote_port = 2 }]\n",
    );
    assert_eq!(
        e,
        [
            "tunnels[0].host: \"-oProxyCommand=evil\" must not start with '-'",
            "tunnels[0].jump[1]: \"bad host\" must not contain ' '",
        ]
    );
}

#[test]
fn forward_fields_per_type() {
    let e = errors(
        "[[tunnels]]\nname = \"t\"\ntype = \"ssh-dynamic\"\nhost = \"h\"\nforwards = [{ local_port = 1, remote_port = 2 }]\n",
    );
    assert_eq!(
        e,
        ["tunnels[0].forwards[0].remote_port: a ssh-dynamic forward does not use `remote_port`"]
    );
    let e = errors(
        "[[tunnels]]\nname = \"t\"\ncommand = \"x\"\nforwards = [{ local_port = 1, remote_port = 2 }]\n",
    );
    assert_eq!(
        e,
        ["tunnels[0].forwards[0].remote_port: a command forward does not use `remote_port`"]
    );
}

#[test]
fn command_rejects_shell_features() {
    let e = errors("[[tunnels]]\nname = \"t\"\ncommand = \"ssh -N bastion | tee log\"\n");
    assert_eq!(
        e,
        [
            "tunnels[0].command: uses a pipe (`|`), which needs a shell, but Tunwarden runs commands without one; put the command in a script and run the script instead"
        ]
    );
    let e = errors("[[tunnels]]\nname = \"t\"\ncommand = \"ssh $HOST\"\n");
    assert!(e[0].contains("variable or command expansion"), "{e:?}");
    let e = errors("[[tunnels]]\nname = \"t\"\ncommand = \"ssh 'unclosed\"\n");
    assert_eq!(e, ["tunnels[0].command: has an unclosed quote"]);
    let e = errors("[[tunnels]]\nname = \"t\"\ncommand = \"  \"\n");
    assert_eq!(e, ["tunnels[0].command: is empty"]);
}

#[test]
fn command_readiness_and_patterns() {
    let e = errors("[[tunnels]]\nname = \"t\"\ncommand = \"x\"\nreadiness = \"port\"\n");
    assert_eq!(
        e,
        [
            "tunnels[0].readiness: `readiness = \"port\"` needs at least one forward with a local_port"
        ]
    );
    let e = errors(
        "[[tunnels]]\nname = \"t\"\ncommand = \"x\"\ncredential_patterns = [\"ok\", \"(bad\"]\n",
    );
    assert_eq!(e.len(), 1);
    assert!(
        e[0].starts_with("tunnels[0].credential_patterns[1]: invalid regular expression"),
        "{e:?}"
    );

    let c = config(
        "[[tunnels]]\nname = \"t\"\ncommand = \"vpn-helper\"\ncredential_patterns = [\"token expired\"]\n",
    );
    let patterns = c.tunnels[0].output_patterns(None);
    let hit = classify(&patterns, "error: Token Expired at 10:00").unwrap();
    assert_eq!(
        hit.reason,
        FailureReason::Credentials,
        "user patterns ignore case"
    );
}

// --- Output patterns (FR-24, FR-45) ------------------------------------------

/// Real OpenSSH 10.3 output, mostly captured during the spikes
/// (`docs/spikes.md`). `None` means "retry".
const SSH_CORPUS: &[(&str, Option<FailureReason>)] = &[
    (
        "Host key verification failed.",
        Some(FailureReason::HostKey),
    ),
    (
        "@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @",
        Some(FailureReason::HostKey),
    ),
    (
        "root@127.0.0.1: Permission denied (publickey,password,keyboard-interactive).",
        Some(FailureReason::Credentials),
    ),
    (
        "Received disconnect from 10.0.0.5 port 22:2: Too many authentication failures",
        Some(FailureReason::Credentials),
    ),
    (
        "bind [127.0.0.1]:15437: Address already in use",
        Some(FailureReason::PortConflict),
    ),
    (
        "Could not request local forwarding.",
        Some(FailureReason::PortConflict),
    ),
    (
        "Error: remote port forwarding failed for listen port 8080",
        Some(FailureReason::PortConflict),
    ),
    ("Timeout, server 127.0.0.1 not responding.", None),
    (
        "ssh: connect to host 10.0.0.5 port 22: Connection refused",
        None,
    ),
    (
        "ssh: Could not resolve hostname bastion: nodename nor servname provided, or not known",
        None,
    ),
];

#[test]
fn ssh_output_is_classified() {
    let patterns = Tool::Ssh.patterns();
    for (line, expected) in SSH_CORPUS {
        assert_eq!(
            classify(&patterns, line).map(|p| p.reason),
            *expected,
            "{line}"
        );
    }
}

/// Real gcloud 591 and kubectl 1.36 output from the spikes, checked against
/// the example config's `google` login and `grafana` tunnel: what used to be
/// built-in knowledge now lives in config.
#[test]
fn the_example_config_patterns_classify_real_output() {
    let c = parse(
        EXAMPLE,
        Path::new("config.toml"),
        &FakeEnv::with_home("/home/me"),
    )
    .unwrap();
    let db = c.tunnels.iter().find(|t| t.name == "staging-db").unwrap();
    let gcloud = db.output_patterns(c.context_of(db).1);
    let cases = [
        (
            "ERROR: (gcloud.compute.start-iap-tunnel) There was a problem refreshing your current auth tokens: Reauthentication failed. cannot prompt during non-interactive execution.",
            Some(FailureReason::AuthExpired),
        ),
        (
            "ERROR: (gcloud.compute.start-iap-tunnel) You do not currently have an active account selected.",
            Some(FailureReason::AuthExpired),
        ),
        ("Testing if tunnel connection works.", None),
        ("Listening on port [18099].", None),
    ];
    for (line, expected) in cases {
        assert_eq!(
            classify(&gcloud, line).map(|p| p.reason),
            expected,
            "{line}"
        );
    }
    let hit = classify(&gcloud, "Reauthentication failed").unwrap();
    assert_eq!(
        db.render_hint(&hit.hint),
        "the google login has expired; run `tunwarden login google`"
    );

    let grafana = c.tunnels.iter().find(|t| t.name == "grafana").unwrap();
    let kubectl = grafana.output_patterns(None);
    assert_eq!(
        classify(
            &kubectl,
            "error: You must be logged in to the server (Unauthorized)"
        )
        .map(|p| p.reason),
        Some(FailureReason::Credentials)
    );
    assert!(classify(&kubectl, "Handling connection for 3000").is_none());
}

#[test]
fn ssh_hints_are_rendered() {
    let c = config(
        "[[tunnels]]\nname = \"r\"\ntype = \"ssh-local\"\nhost = \"bastion\"\nforwards = [{ local_port = 3, remote_port = 4 }]\n",
    );
    let ssh = &c.tunnels[0];
    let patterns = ssh.output_patterns(None);
    let p = classify(&patterns, "Host key verification failed.").unwrap();
    assert_eq!(
        ssh.render_hint(&p.hint),
        "the SSH host key for bastion is unknown or has changed; run `tunwarden trust bastion`"
    );
    for p in &patterns {
        assert!(
            !ssh.render_hint(&p.hint).contains('{'),
            "unfilled placeholder in: {}",
            p.hint
        );
    }
}

#[test]
fn example_config_builds_every_command() {
    let c = parse(
        EXAMPLE,
        Path::new("config.toml"),
        &FakeEnv::with_home("/home/me"),
    )
    .unwrap();
    for t in &c.tunnels {
        let (p, l) = c.context_of(t);
        assert!(!t.launch_plan(p, l).display().is_empty());
    }
}
