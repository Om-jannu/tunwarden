use std::path::Path;

use super::*;
use crate::config::parse;
use crate::paths::tests::FakeEnv;

fn parsed(src: &str) -> Config {
    parse(src, Path::new("c.toml"), &FakeEnv::with_home("/home/me")).unwrap()
}

fn empty() -> Config {
    parsed("version = 1\n")
}

fn one(line: &str) -> (Map<String, Value>, Imported) {
    let out = import(line, &empty());
    assert_eq!(out.tunnels.len(), 1, "{out:#?}");
    (out.tunnels[0].fields.clone(), out)
}

/// The import result must itself be a valid config.
fn valid(out: &Imported) {
    let text = format!("version = 1\n{}", out.to_toml());
    if let Err(e) = parse(&text, Path::new("c.toml"), &FakeEnv::with_home("/home/me")) {
        panic!("import produced an invalid config:\n{text}\n{e}");
    }
}

#[test]
fn any_tool_becomes_a_command_tunnel_with_its_local_port() {
    // The shape of a real gcloud alias, with placeholder names.
    let out = import(
        "alias uatdb='gcloud compute start-iap-tunnel uat-vm 3371 --zone=asia-south1-b --local-host-port=localhost:3371 --project=uat-project-123'",
        &empty(),
    );
    valid(&out);
    assert!(out.profiles.is_empty());
    let t = &out.tunnels[0];
    assert_eq!(t.name, "uatdb");
    assert_eq!(
        Value::Object(t.fields.clone()),
        json!({
            "type": "command",
            "command": "gcloud compute start-iap-tunnel uat-vm 3371 --zone=asia-south1-b --local-host-port=localhost:3371 --project=uat-project-123",
            "forwards": [{ "local_port": 3371 }]
        })
    );
    assert_eq!(
        out.warnings,
        ["line 1: guessed local port 3371 from the command; check them"]
    );

    let (f, _) =
        one("kubectl --context prod -n monitoring port-forward svc/grafana 3000:80 9090:9090");
    assert_eq!(f["type"], "command");
    assert_eq!(
        f["forwards"],
        json!([{ "local_port": 3000 }, { "local_port": 9090 }])
    );
    let (f, _) =
        one("aws ssm start-session --target i-1 --parameters portNumber=5432,localPortNumber=5434");
    assert!(
        f.get("forwards").is_none(),
        "nothing recognisable: no guess"
    );
}

#[test]
fn leading_variables_become_a_profile() {
    let text = "\
CLOUDSDK_CORE_PROJECT=p-1 CLOUDSDK_CORE_ACCOUNT=me@example.com gcloud compute ssh app-1 --tunnel-through-iap -- -N -L 8080:localhost:80
CLOUDSDK_CORE_ACCOUNT=me@example.com CLOUDSDK_CORE_PROJECT=p-1 my-other-tool --port=9000
AWS_PROFILE=work aws ssm start-session --target i-1
";
    let out = import(text, &empty());
    valid(&out);
    assert_eq!(
        out.profiles.len(),
        2,
        "the same variables in another order reuse a profile"
    );
    assert_eq!(
        Value::Object(out.profiles[0].fields.clone()),
        json!({ "env": { "CLOUDSDK_CORE_ACCOUNT": "me@example.com", "CLOUDSDK_CORE_PROJECT": "p-1" } })
    );
    let profiles: Vec<&Value> = out.tunnels.iter().map(|t| &t.fields["profile"]).collect();
    assert_eq!(profiles, ["imported", "imported", "imported-2"]);
    assert_eq!(
        out.tunnels[0].fields["forwards"],
        json!([{ "local_port": 8080 }])
    );
    assert_eq!(
        out.tunnels[1].fields["forwards"],
        json!([{ "local_port": 9000 }])
    );
    assert_eq!(out.tunnels[0].name, "gcloud-8080");

    // An existing profile with the same variables is reused.
    let config =
        parsed("version = 1\n[[profiles]]\nname = \"work\"\nenv = { AWS_PROFILE = \"work\" }\n");
    let out = import(
        "AWS_PROFILE=work aws ssm start-session --target i-1\n",
        &config,
    );
    assert!(out.profiles.is_empty());
    assert_eq!(out.tunnels[0].fields["profile"], "work");
}

#[test]
fn ssh_types() {
    let (f, out) = one(
        "ssh -N -f -J jump1,jump2 -p 2222 deploy@bastion -L 5433:db.internal:5432 -L 127.0.0.1:6380:redis:6379",
    );
    valid(&out);
    assert_eq!(
        Value::Object(f),
        json!({
            "type": "ssh-local", "host": "bastion", "user": "deploy", "port": 2222, "jump": ["jump1", "jump2"],
            "forwards": [
                { "local_port": 5433, "remote_host": "db.internal", "remote_port": 5432 },
                { "local_port": 6380, "remote_host": "redis", "remote_port": 6379 }
            ]
        })
    );
    assert!(out.warnings[0].contains("ssh -f"), "{:?}", out.warnings);
    assert_eq!(out.tunnels[0].name, "bastion-5433");

    let (f, _) = one("ssh -D 1080 -N bastion");
    assert_eq!(f["type"], "ssh-dynamic");
    assert_eq!(f["forwards"], json!([{ "local_port": 1080 }]));
    let (f, _) = one("ssh -N -R 8080:localhost:3000 vps");
    assert_eq!(f["type"], "ssh-remote");
    assert_eq!(
        f["forwards"],
        json!([{ "remote_port": 8080, "local_port": 3000 }])
    );
    let (f, _) = one("ssh -N -L 0.0.0.0:8080:web:80 bastion");
    assert_eq!(
        f["forwards"][0]["bind"], "0.0.0.0",
        "a non-default bind is kept"
    );
}

#[test]
fn ssh_with_options_the_types_do_not_set_stays_a_command() {
    let (f, out) = one("ssh -f -N -i ~/.ssh/work -L 5433:db:5432 bastion");
    valid(&out);
    assert_eq!(f["type"], "command");
    assert_eq!(
        f["command"], "ssh -N -i '~/.ssh/work' -L 5433:db:5432 bastion",
        "-f is removed"
    );
    assert_eq!(f["forwards"], json!([{ "local_port": 5433 }]));
    assert!(
        out.warnings.iter().any(|w| w.contains("ssh option -i")),
        "{:?}",
        out.warnings
    );
    let (f, out) = one("ssh -L 1:a:2 -D 3 host");
    assert_eq!(f["type"], "command");
    assert!(out.warnings[0].contains("mixes -L, -R and -D"));
}

#[test]
fn other_commands_names_and_shell_lines() {
    let text = "\
# my tunnels
alias db='ssh -N -L 5433:db:5432 bastion'
alias db=\"ssh -N -L 5434:db:5432 bastion\"
alias proxy=cloud-sql-proxy
ssh -N -L 7000:a:7000 \\
    bastion &
";
    let out = import(text, &empty());
    valid(&out);
    let names: Vec<&str> = out.tunnels.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["db", "db-2", "proxy", "bastion-7000"]);
    assert_eq!(out.tunnels[2].fields["command"], "cloud-sql-proxy");
    assert!(
        out.warnings
            .iter()
            .any(|w| w.starts_with("line 4: no local port found")),
        "{:?}",
        out.warnings
    );
    assert!(
        out.warnings
            .iter()
            .any(|w| w.starts_with("line 5: ends with `&`")),
        "continued lines keep their first line number: {:?}",
        out.warnings
    );
    assert_eq!(
        out.tunnels[3].fields["forwards"],
        json!([{ "local_port": 7000, "remote_host": "a", "remote_port": 7000 }])
    );

    let out = import("ssh -N -D 1 'host\n", &empty());
    assert!(out.tunnels.is_empty(), "{out:?}");
    assert_eq!(out.warnings, ["line 1: has an unclosed quote; skipped"]);
}

#[test]
fn names_are_sanitized_and_unique_against_the_config() {
    let config = parsed("version = 1\n[[tunnels]]\nname = \"db\"\ncommand = \"x\"\n");
    let out = import(
        "alias db='ssh -N -D 1 h'\nalias 'my tunnel!'='ssh -N -D 2 h'\n",
        &config,
    );
    let names: Vec<&str> = out.tunnels.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["db-2", "my-tunnel-"]);
}
