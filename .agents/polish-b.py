"""CLI polish, part B: pickers and live progress for start/stop/restart,
guided add, import picker, human-panic."""

P = "crates/tunwarden-cli/src/lib.rs"
s = open(P, encoding="utf-8").read()


def rep(old, new, count=1):
    global s
    assert s.count(old) == count, (s.count(old), old[:90])
    s = s.replace(old, new)


# --- add: optional name ------------------------------------------------------
rep('''    Add {
        name: String,
        #[command(flatten)]
        fields: Fields,
    },''', '''    Add {
        /// The new tunnel's name; asks when missing
        name: Option<String>,
        #[command(flatten)]
        fields: Fields,
    },''')

# --- action: picker and progress ----------------------------------------------
rep('''async fn action(ctx: &Ctx, kind: Kind, a: Action) -> Outcome {''',
    '''async fn action(ctx: &Ctx, kind: Kind, mut a: Action) -> Outcome {''')
rep('''    let (rpc, verb) = match kind {
        Kind::Start => (method::START, "start"),
        Kind::Stop => (method::STOP, "stop"),
        Kind::Restart => (method::RESTART, "restart"),
    };
''', '''    let (rpc, verb) = match kind {
        Kind::Start => (method::START, "start"),
        Kind::Stop => (method::STOP, "stop"),
        Kind::Restart => (method::RESTART, "restart"),
    };

    // Nothing selected: pick from a list on a terminal. Otherwise the daemon
    // answers "no tunnels selected" with the hint.
    let nothing = a.names.is_empty() && !a.all && a.group.is_none() && a.profile.is_none();
    if nothing && ui::interactive() {
        let all: StatusResult = client
            .call(method::STATUS, StatusParams { names: Vec::new() })
            .await?;
        let active = |t: &&TunnelView| t.state != State::Stopped;
        let choices: Vec<&TunnelView> = match kind {
            Kind::Stop => all.tunnels.iter().filter(active).collect(),
            _ => all.tunnels.iter().collect(),
        };
        if choices.is_empty() {
            println!(
                "{}",
                if kind == Kind::Stop {
                    "no tunnels are running"
                } else {
                    "no tunnels are configured"
                }
            );
            return Ok(ExitCode::SUCCESS);
        }
        let width = choices.iter().map(|t| t.name.len()).max().unwrap_or(0);
        let items: Vec<String> = choices
            .iter()
            .map(|t| {
                format!(
                    "{:<width$}  {:<12}  {}",
                    t.name,
                    ui::state(t.state),
                    ui::paint(ui::DIM, t.local.join(", "))
                )
            })
            .collect();
        let checked = vec![false; items.len()];
        let picked = ui::multi_select(&format!("Which tunnels to {verb}?"), &items, &checked)?;
        if picked.is_empty() {
            println!("nothing selected");
            return Ok(ExitCode::SUCCESS);
        }
        a.names = picked.into_iter().map(|i| choices[i].name.clone()).collect();
    }
''')
rep('''    let deadline = Instant::now() + Duration::from_secs(a.wait);
    let views = loop {''', '''    let deadline = Instant::now() + Duration::from_secs(a.wait);
    let progress = ui::live().then(|| Progress::new(&done.tunnels));
    let views = loop {''')
rep('''        if now.tunnels.iter().all(settled) || Instant::now() >= deadline {
            break now.tunnels;
        }''', '''        if let Some(p) = &progress {
            p.update(&now.tunnels, &settled);
        }
        if now.tunnels.iter().all(settled) || Instant::now() >= deadline {
            break now.tunnels;
        }''')
rep('''    let mut table = Table::new(&["NAME", "STATE", "LOCAL", "DETAIL"]);
    let mut ok = true;
    for t in &views {
        let good = match kind {
            Kind::Stop => t.state == State::Stopped,
            _ => t.state == State::Running,
        };
        ok &= good;
        let mut d = detail(t);
        if !settled(t) {
            d = format!(
                "still {} after {}s; see `tunwarden logs {}`",
                t.state.as_str(),
                a.wait,
                t.name
            );
        }
        table.row(vec![
            t.name.clone(),
            t.state.as_str().into(),
            or_dash(t.local.join(", ")),
            d,
        ]);
    }
    print!("{table}");''', '''    let mut table = Table::new(&["NAME", "STATE", "LOCAL", "DETAIL"]);
    let mut ok = true;
    let mut results = Vec::new();
    for t in &views {
        let good = match kind {
            Kind::Stop => t.state == State::Stopped,
            _ => t.state == State::Running,
        };
        ok &= good;
        let mut d = detail(t);
        if !settled(t) {
            d = format!(
                "still {} after {}s; see `tunwarden logs {}`",
                t.state.as_str(),
                a.wait,
                t.name
            );
        }
        results.push((t.name.clone(), good, d.clone()));
        table.row(vec![
            t.name.clone(),
            ui::state(t.state),
            or_dash(t.local.join(", ")),
            d,
        ]);
    }
    // On a terminal the finished progress lines are the result; a pipe gets
    // the table.
    let drawn = progress.is_some_and(|p| p.finish(&views, &results));
    if !(drawn && std::io::stdout().is_terminal()) {
        print!("{table}");
    }''')

rep('''fn confirm(verb: &str, production: &[&str]) -> Result<(), Failure> {''', '''/// One live line per tunnel on stderr while `start`, `stop` or `restart`
/// waits, ending as `✓ name running 127.0.0.1:5433` or `✗ name failed ...`.
struct Progress {
    bars: Vec<(String, indicatif::ProgressBar)>,
    width: usize,
}

impl Progress {
    fn new(names: &[String]) -> Progress {
        let multi = indicatif::MultiProgress::with_draw_target(
            indicatif::ProgressDrawTarget::stderr(),
        );
        let style = indicatif::ProgressStyle::with_template("{spinner:.yellow} {msg}")
            .expect("a valid template")
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ");
        let width = names.iter().map(|n| n.len()).max().unwrap_or(0);
        let bars = names
            .iter()
            .map(|n| {
                let bar = multi.add(indicatif::ProgressBar::new_spinner());
                bar.set_style(style.clone());
                bar.set_message(format!("{n:<width$}  waiting"));
                bar.enable_steady_tick(Duration::from_millis(100));
                (n.clone(), bar)
            })
            .collect();
        Progress { bars, width }
    }

    fn update(&self, views: &[TunnelView], settled: &dyn Fn(&TunnelView) -> bool) {
        let width = self.width;
        for (name, bar) in &self.bars {
            if let Some(t) = views.iter().find(|t| &t.name == name)
                && !settled(t)
            {
                bar.set_message(format!(
                    "{name:<width$}  {}  {}",
                    ui::state(t.state),
                    ui::paint(ui::DIM, t.local.join(", "))
                ));
            }
        }
    }

    /// Ends every line. Returns true when lines were drawn.
    fn finish(self, views: &[TunnelView], results: &[(String, bool, String)]) -> bool {
        let width = self.width;
        let done = indicatif::ProgressStyle::with_template("{msg}").expect("a valid template");
        for (name, bar) in &self.bars {
            let view = views.iter().find(|t| &t.name == name);
            let (good, detail) = results
                .iter()
                .find(|(n, _, _)| n == name)
                .map(|(_, g, d)| (*g, d.as_str()))
                .unwrap_or((false, ""));
            let mark = if good {
                ui::paint(ui::GREEN, "✓")
            } else {
                ui::paint(ui::RED, "✗")
            };
            let state = view.map(|t| ui::state(t.state)).unwrap_or_default();
            let local = view.map(|t| t.local.join(", ")).unwrap_or_default();
            let tail = if detail.is_empty() {
                ui::paint(ui::DIM, local)
            } else {
                format!("{}  {detail}", ui::paint(ui::DIM, local))
            };
            bar.set_style(done.clone());
            bar.finish_with_message(format!("{mark} {name:<width$}  {state}  {tail}"));
        }
        !self.bars.is_empty()
    }
}

fn confirm(verb: &str, production: &[&str]) -> Result<(), Failure> {''')

# --- add: guided -------------------------------------------------------------
rep('''        Cmd::Add { name, fields } => add(ctx, name, fields).await,''',
    '''        Cmd::Add { name, fields } => add(ctx, name, fields).await,''')
rep('''async fn add(ctx: &Ctx, name: String, fields: Fields) -> Outcome {
    let fields = fields.to_map()?;''', '''async fn add(ctx: &Ctx, name: Option<String>, fields: Fields) -> Outcome {
    let mut fields = fields.to_map()?;
    let name = match name {
        Some(n) => n,
        None if ui::interactive() => ui::input("Tunnel name", false)?.trim().to_owned(),
        None => {
            return Err(Failure {
                message: "name the new tunnel".into(),
                hint: Some("for example `tunwarden add grafana --command \\"...\\" --forward 3000`, or run it in a terminal to be asked".into()),
            });
        }
    };
    if fields.is_empty() && ui::interactive() {
        fields = ask_fields(ctx)?;
    }''')

s += '''
/// Asks for a new tunnel's fields, one at a time (`tunwarden add` on a terminal).
fn ask_fields(ctx: &Ctx) -> Result<serde_json::Map<String, Value>, Failure> {
    let mut m = serde_json::Map::new();
    let kinds = [
        ("command", "any command that opens a local port"),
        ("ssh-local", "SSH local forward (ssh -L)"),
        ("ssh-remote", "SSH remote forward (ssh -R)"),
        ("ssh-dynamic", "SOCKS proxy over SSH (ssh -D)"),
    ];
    let items: Vec<String> = kinds
        .iter()
        .map(|(k, d)| format!("{k:<12} {}", ui::paint(ui::DIM, d)))
        .collect();
    let kind = kinds[ui::select("Type", &items)?].0;
    if kind != "command" {
        m.insert("type".into(), Value::String(kind.into()));
    }
    let list = |text: &str| -> Vec<String> {
        text.split([',', ' '])
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let forwards_prompt = match kind {
        "command" => "Local ports it opens (comma separated; empty: ready while it runs)",
        "ssh-local" => "Forwards as LOCAL:HOST:REMOTE_PORT (comma separated)",
        "ssh-remote" => "Forwards as LOCAL:REMOTE_PORT (comma separated)",
        _ => "Local SOCKS port",
    };
    match kind {
        "command" => {
            let command = ui::input("Command line", false)?;
            m.insert("command".into(), Value::String(command));
        }
        _ => {
            let host = ui::input("SSH host or alias from ~/.ssh/config", false)?;
            m.insert("host".into(), Value::String(host.trim().into()));
            let jump = list(&ui::input("Jump hosts (comma separated, optional)", true)?);
            if !jump.is_empty() {
                m.insert("jump".into(), Value::from(jump));
            }
        }
    }
    let specs = list(&ui::input(forwards_prompt, kind == "command")?);
    if !specs.is_empty() {
        let forwards: Result<Vec<Value>, Failure> = specs.iter().map(|f| forward(f)).collect();
        m.insert("forwards".into(), Value::Array(forwards?));
    }
    if let Ok(config) = config::load(&ctx.config)
        && !config.profiles.is_empty()
    {
        let mut items = vec![ui::paint(ui::DIM, "(no profile)")];
        items.extend(config.profiles.iter().map(|p| {
            let prod = if p.production {
                ui::paint(ui::YELLOW, " production")
            } else {
                String::new()
            };
            format!("{}{prod}", p.name)
        }));
        let i = ui::select("Profile", &items)?;
        if i > 0 {
            m.insert(
                "profile".into(),
                Value::String(config.profiles[i - 1].name.clone()),
            );
        }
    }
    let groups = list(&ui::input("Groups (comma separated, optional)", true)?);
    if !groups.is_empty() {
        m.insert("groups".into(), Value::from(groups));
    }
    Ok(m)
}
'''

# --- import: pick entries ------------------------------------------------------
rep('''    let mut client = connect_or_start(ctx).await?;
    for p in &imported.profiles {''', '''    // From a file on a terminal: choose which entries to add.
    let mut tunnels: Vec<_> = imported.tunnels.iter().collect();
    let from_file = file.as_ref().is_some_and(|f| f.as_os_str() != "-");
    if from_file && ui::interactive() {
        let width = tunnels.iter().map(|t| t.name.len()).max().unwrap_or(0);
        let items: Vec<String> = tunnels
            .iter()
            .map(|t| {
                let what = t
                    .fields
                    .get("command")
                    .or_else(|| t.fields.get("host"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let short: String = what.chars().take(60).collect();
                format!("{:<width$}  {}", t.name, ui::paint(ui::DIM, short))
            })
            .collect();
        let picked = ui::multi_select("Add which tunnels?", &items, &vec![true; items.len()])?;
        if picked.is_empty() {
            println!("nothing added");
            return Ok(ExitCode::SUCCESS);
        }
        tunnels = picked.into_iter().map(|i| tunnels[i]).collect();
    }
    let used: Vec<&str> = tunnels
        .iter()
        .filter_map(|t| t.fields.get("profile").and_then(Value::as_str))
        .collect();
    let mut client = connect_or_start(ctx).await?;
    for p in imported.profiles.iter().filter(|p| used.contains(&p.name.as_str())) {''')
rep('''    for t in &imported.tunnels {
        let _: ConfigEditResult = client
            .call(
                method::CONFIG_ADD,''', '''    for t in tunnels {
        let _: ConfigEditResult = client
            .call(
                method::CONFIG_ADD,''')

open(P, "w", encoding="utf-8").write(s)

M = "crates/tunwarden-cli/src/main.rs"
open(M, "w").write('''fn main() -> std::process::ExitCode {
    // A crash prints a short message and saves a report, instead of a backtrace.
    human_panic::setup_panic!();
    tunwarden_cli::run(std::env::args_os())
}
''')
print("ok")
