// --- Logins (FR-22, FR-25) -------------------------------------------------------

/// The login named `name`, or the login of the profile named `name`.
fn find_login<'c>(config: &'c Config, name: &str) -> Result<&'c Login, Failure> {
    if let Some(l) = config.login(name) {
        return Ok(l);
    }
    if let Some(p) = config.profile(name) {
        return match p.login.as_deref().and_then(|l| config.login(l)) {
            Some(l) => Ok(l),
            None => Err(Failure {
                message: format!("profile \"{name}\" has no login"),
                hint: Some("set `login = \"NAME\"` in the profile to use one".into()),
            }),
        };
    }
    let known: Vec<&str> = config.logins.iter().map(|l| l.name.as_str()).collect();
    Err(Failure {
        message: format!("no login or profile named \"{name}\""),
        hint: Some(if known.is_empty() {
            "no logins are configured; add a [[logins]] entry".into()
        } else {
            format!("configured logins: {}", known.join(", "))
        }),
    })
}

fn login_view(config: &Config, login: &Login, outcome: Option<&AuthOutcome>) -> LoginView {
    LoginView {
        name: login.name.clone(),
        profiles: config
            .profiles
            .iter()
            .filter(|p| p.login.as_ref() == Some(&login.name))
            .map(|p| p.name.clone())
            .collect(),
        checkable: login.check.is_some(),
        result: outcome.map_or("not checked", AuthOutcome::name).to_owned(),
        detail: outcome.and_then(AuthOutcome::detail),
        checked_at: outcome.map(|_| tunwarden_core::fsutil::format_timestamp_now()),
        active_tunnels: Vec::new(),
    }
}

/// Checks logins directly, without a daemon.
async fn check_logins(config: &Config, name: Option<&str>) -> Result<AuthResult, Failure> {
    let logins: Vec<&Login> = match name {
        Some(n) => vec![find_login(config, n)?],
        None => config.logins.iter().collect(),
    };
    let mut views = Vec::new();
    for l in logins {
        let outcome = l.check.is_some().then_some(gauth::check(l, &config.tools).await);
        views.push(login_view(config, l, outcome.as_ref()));
    }
    Ok(AuthResult { logins: views })
}

/// Checks logins through the daemon if it runs (so it acts on the result),
/// otherwise directly.
async fn auth(ctx: &Ctx, name: Option<String>, json: bool) -> Outcome {
    let config = load(ctx)?;
    let result: AuthResult = match Client::connect(&ctx.state_dir, "cli").await {
        Ok(mut c) => c.call(method::AUTH_CHECK, AuthParams { name }).await?,
        Err(ClientError::NotRunning) => check_logins(&config, name.as_deref()).await?,
        Err(e) => return Err(e.into()),
    };
    if json {
        print_json(&result);
    } else if result.logins.is_empty() {
        println!("no logins are configured");
    } else {
        let mut table = Table::new(&["LOGIN", "PROFILES", "RESULT", "DETAIL"]);
        for l in &result.logins {
            let detail = match l.result.as_str() {
                "expired" => format!("{}; run `tunwarden login {}`", l.detail.clone().unwrap_or_default(), l.name),
                _ if !l.checkable => "no check command".into(),
                _ => l.detail.clone().unwrap_or_default(),
            };
            table.row(vec![l.name.clone(), or_dash(l.profiles.join(", ")), l.result.clone(), detail]);
        }
        print!("{table}");
    }
    let expired = result.logins.iter().any(|l| l.result == "expired");
    Ok(if expired { ExitCode::FAILURE } else { ExitCode::SUCCESS })
}

/// Runs the login's `login` command here in the terminal, then lets the
/// daemon check it and restart the tunnels that were waiting for it (FR-25).
async fn login(ctx: &Ctx, name: &str) -> Outcome {
    let config = load(ctx)?;
    let login = find_login(&config, name)?;
    let Some(words) = &login.login else {
        return Err(Failure {
            message: format!("the {} login has no `login` command", login.name),
            hint: Some(format!("log in with your tool, then run `tunwarden auth {}`", login.name)),
        });
    };
    let program = tunwarden_core::tools::resolve(
        &tunwarden_core::types::Program::Command(words[0].clone()),
        &config.tools,
        &SystemEnv,
        Os::current(),
    )
    .map_err(|e| e.to_string())?
    .path;

    let view = login_view(&config, login, None);
    let used_by = if view.profiles.is_empty() { String::new() } else { format!(" (used by {})", view.profiles.join(", ")) };
    eprintln!("Running the {} login{used_by}: {}", login.name, shell_line(words));
    let status = gauth::login_command(login, &program)
        .expect("checked above")
        .stdin(std::process::Stdio::inherit())
        .status()
        .map_err(|e| format!("cannot run {}: {e}", program.display()))?;
    if !status.success() {
        return Err(Failure {
            message: format!("the login command did not finish ({status})"),
            hint: Some(format!("run `tunwarden login {}` again", login.name)),
        });
    }

    match Client::connect(&ctx.state_dir, "cli").await {
        Ok(mut c) => {
            let result: RecoveredResult = c
                .call(method::AUTH_RECOVERED, RecoveredParams { login: login.name.clone() })
                .await?;
            if result.restarted.is_empty() {
                println!("logged in; no tunnels were waiting for this login");
            } else {
                println!("logged in; restarting {}", result.restarted.join(", "));
            }
        }
        Err(ClientError::NotRunning) => match gauth::check(login, &config.tools).await {
            AuthOutcome::Ok => println!("logged in"),
            other => {
                return Err(Failure {
                    message: format!("the {} login still fails its check: {}", login.name, other.detail().unwrap_or_default()),
                    hint: Some(format!("run `tunwarden login {}` again", login.name)),
                });
            }
        },
        Err(e) => return Err(e.into()),
    }
    Ok(ExitCode::SUCCESS)
}

fn shell_line(words: &[String]) -> String {
    words.iter().map(|w| tunwarden_core::types::quote(w)).collect::<Vec<_>>().join(" ")
}

