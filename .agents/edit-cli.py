import sys, re

p = "/Users/omjannu/Projects/tunwarden/crates/tunwarden-cli/src/lib.rs"
s = open(p).read()

def rep(a, b):
    global s
    if a not in s:
        sys.exit(f"not found: {a[:90]!r}")
    s = s.replace(a, b, 1)

def cut(start, end, new):
    global s
    i = s.index(start)
    j = s.index(end, i)
    s = s[:i] + new + s[j:]

rep("use tunwarden_core::auth::{self as gauth, ContextKey, Outcome as AuthOutcome};",
    "use tunwarden_core::auth::{self as gauth, Outcome as AuthOutcome};")
s = s.replace("AuthContextView,", "LoginView, ConfigAddEntryParams,", 1)
rep("use tunwarden_core::config::{self, Config, LoadError};", "use tunwarden_core::config::{self, Config, LoadError, Login};")
rep('about = "Manage SSH, gcloud IAP, kubectl and custom tunnels"', 'about = "Keep tunnels running: any command that opens a local port, plus built-in SSH forwards"')
s = re.sub(r'    /// Check gcloud logins\n    Auth \{\n        /// Only the login this project uses\n        project: Option<String>,',
           '    /// Check logins (runs each login\'s `check` command)\n    Auth {\n        /// A login, or a profile meaning its login; all logins if missing\n        name: Option<String>,', s)
rep("    /// Log in to gcloud for a project, then restart its tunnels that need it\n    Login { project: String },",
    "    /// Run a login's `login` command, then restart the tunnels waiting for it\n    Login {\n        /// A login, or a profile meaning its login\n        name: String,\n    },")
rep("    /// The tunnels of a project\n", "    /// The tunnels of a profile\n")
rep("    project: Option<String>,\n    /// Start production", "    profile: Option<String>,\n    /// Start production")
rep("            project: self.project.clone(),", "            profile: self.profile.clone(),")
rep("        Cmd::Auth { project, json } => auth(ctx, project, json).await,\n        Cmd::Login { project } => login(ctx, &project).await,",
    "        Cmd::Auth { name, json } => auth(ctx, name, json).await,\n        Cmd::Login { name } => login(ctx, &name).await,")
rep("Replace the example projects and tunnels with your own", "Replace the example logins, profiles and tunnels with your own")
rep('    let mut table = Table::new(&["NAME", "TYPE", "PROJECT", "LOCAL", "GROUPS"]);\n    for t in &tunnels {\n        let project = match (&t.project, t.production) {',
    '    let mut table = Table::new(&["NAME", "TYPE", "PROFILE", "LOCAL", "GROUPS"]);\n    for t in &tunnels {\n        let profile = match (&t.profile, t.production) {')
rep("            t.kind.clone(),\n            project,", "            t.kind.clone(),\n            profile,")
rep("                project: info.project,", "                profile: info.profile,")
rep("belong to a production project.", "belong to a production profile.")
rep("    /// Tunnel type: ssh-local, ssh-remote, ssh-dynamic, gcloud-iap, kubectl-port-forward, custom",
    "    /// Tunnel type: command (the default), ssh-local, ssh-remote or ssh-dynamic")
rep("    /// Project (for gcloud-iap and custom tunnels)\n    #[arg(long)]\n    project: Option<String>,",
    "    /// Profile: shared environment, production flag and login\n    #[arg(long)]\n    profile: Option<String>,")
rep('        if let Some(p) = &self.project {\n            m.insert("project".into(), Value::String(p.clone()));',
    '        if let Some(p) = &self.profile {\n            m.insert("profile".into(), Value::String(p.clone()));')
rep('    /// The command line of a custom tunnel', '    /// The command line of a command tunnel')
rep('format!("{} tunnels, {} projects", c.tunnels.len(), c.projects.len()),',
    'format!("{} tunnels, {} profiles, {} logins", c.tunnels.len(), c.profiles.len(), c.logins.len()),')

cut("    // Logins\n    if config.projects.is_empty() {", "    // Ports of tunnels that are not running", '''    // Logins
    if config.logins.is_empty() {
        r.add("Logins", Mark::Ok, "none", "no logins are configured");
    } else {
        let auth: AuthResult = match client.as_mut() {
            Some(c) => c.call(method::AUTH_CHECK, AuthParams::default()).await?,
            None => check_logins(&config, None).await?,
        };
        for l in &auth.logins {
            let item = if l.profiles.is_empty() { l.name.clone() } else { format!("{} ({})", l.name, l.profiles.join(", ")) };
            let detail = l.detail.clone().unwrap_or_default();
            match l.result.as_str() {
                "ok" => r.add("Logins", Mark::Ok, item, detail),
                "not checked" => r.add("Logins", Mark::Ok, item, "no check command"),
                "expired" => r.add("Logins", Mark::Problem, item, format!("expired; run `tunwarden login {}`", l.name)),
                _ => r.add("Logins", Mark::Warning, item, format!("could not check: {detail}")),
            }
        }
    }

''')

cut("    for p in &imported.projects {", "    for t in &imported.tunnels {", '''    for p in &imported.profiles {
        let _: ConfigEditResult = client
            .call(
                method::CONFIG_ADD_ENTRY,
                ConfigAddEntryParams {
                    table: "profiles".into(),
                    name: p.name.clone(),
                    fields: p.fields.clone(),
                },
            )
            .await?;
        println!("added profile {}", p.name);
    }
''')

cut("    use tunwarden_core::types::{SshTarget, TypeSettings};", "    let ssh = tunwarden_core::tools::resolve(", '''    use tunwarden_core::types::SshTarget;
    let config = config::load(&ctx.config).ok();
    let tool_paths = config.as_ref().map(|c| c.tools.clone()).unwrap_or_default();
    let ssh_target: SshTarget = match config.as_ref().and_then(|c| c.tunnels.iter().find(|t| t.name == target)) {
        Some(t) => match t.type_settings.ssh_target() {
            Some(s) => s.clone(),
            None => {
                return Err(Failure {
                    message: format!("{target} is a {} tunnel, not an SSH tunnel", t.kind),
                    hint: Some("pass an SSH tunnel's name or a host".into()),
                });
            }
        },
        None => SshTarget {
            host: target.to_owned(),
            user: None,
            port: None,
            jump: Vec::new(),
        },
    };
''')

cut("// --- gcloud logins (FR-22, FR-25)", "// --- daemon ---", open("/Users/omjannu/Projects/tunwarden/.agents/cli-logins.rs").read())

open(p, "w").write(s)
print("cli ok", s.count("project"))
