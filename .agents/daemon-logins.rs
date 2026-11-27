// --- Logins (FR-22 to FR-25) ---------------------------------------------------

/// The login named `name` (or used by the profile named `name`), or all logins.
async fn login_names(ctx: &Ctx, name: Option<&str>) -> Result<Vec<String>, RpcError> {
    let config = ctx.config();
    let Some(name) = name else {
        return Ok(config.logins.iter().map(|l| l.name.clone()).collect());
    };
    if config.login(name).is_some() {
        return Ok(vec![name.to_owned()]);
    }
    if let Some(profile) = config.profile(name) {
        return match &profile.login {
            Some(login) => Ok(vec![login.clone()]),
            None => Err(RpcError::app(
                "no_login",
                format!("profile \"{name}\" has no login"),
                Some("set `login = \"NAME\"` in the profile to use one".into()),
            )),
        };
    }
    let known: Vec<&str> = config.logins.iter().map(|l| l.name.as_str()).collect();
    Err(RpcError::app(
        "unknown_login",
        format!("no login or profile named \"{name}\""),
        Some(if known.is_empty() {
            "no logins are configured; add a [[logins]] entry".into()
        } else {
            format!("configured logins: {}", known.join(", "))
        }),
    ))
}

fn is_active(state: State) -> bool {
    matches!(state, State::Starting | State::Running | State::Unhealthy | State::Reconnecting)
}

/// Active tunnels that use the login `name`.
fn active_in(ctx: &Ctx, name: &str) -> Vec<String> {
    ctx.tunnels()
        .values()
        .filter(|e| e.login.as_deref() == Some(name) && is_active(e.supervisor.status().state))
        .map(|e| e.config.name.clone())
        .collect()
}

/// Checks one login now. If it has expired, its active tunnels fail with
/// `auth expired`. An unknown result changes nothing (FR-22).
async fn check_login(ctx: &Ctx, name: &str) -> Outcome {
    let config = ctx.config();
    let Some(login) = config.login(name) else {
        return Outcome::Unknown {
            detail: format!("no login named {name}"),
        };
    };
    let outcome = auth::check(login, &config.tools).await;
    if login.check.is_some()
        && let Some(state) = ctx.auth.lock().await.get_mut(name)
    {
        state.last = Some((outcome.clone(), now(), Instant::now()));
    }
    match &outcome {
        Outcome::Expired { .. } => {
            let tunnels = ctx.tunnels();
            for e in tunnels.values().filter(|e| e.login.as_deref() == Some(name)) {
                e.supervisor.send(Event::AuthExpired(format!(
                    "the {name} login has expired; run `tunwarden login {name}`"
                )));
            }
            diag(&format!("the {name} login has expired"));
        }
        Outcome::Unknown { detail } => {
            diag(&format!("the check of the {name} login was inconclusive: {detail}"));
        }
        Outcome::Ok => {}
    }
    outcome
}

async fn login_views(ctx: &Ctx, names: &[String]) -> Vec<LoginView> {
    let config = ctx.config();
    let states = ctx.auth.lock().await;
    names
        .iter()
        .filter_map(|name| {
            let s = states.get(name)?;
            Some(LoginView {
                name: name.clone(),
                profiles: s.profiles.clone(),
                checkable: config.login(name).is_some_and(|l| l.check.is_some()),
                result: s.last.as_ref().map_or("not checked", |(o, _, _)| o.name()).to_owned(),
                detail: s.last.as_ref().and_then(|(o, _, _)| o.detail()),
                checked_at: s.last.as_ref().map(|(_, at, _)| at.clone()),
                active_tunnels: active_in(ctx, name),
            })
        })
        .collect()
}

/// Checks logins that have a `check` command and active tunnels on a
/// schedule, and right away when a tunnel fails because its login expired,
/// so the other tunnels sharing that login are caught too.
async fn auth_watcher(ctx: Arc<Ctx>, interval: Duration) {
    const MIN_GAP: Duration = Duration::from_secs(10);
    let mut notices = ctx.hub.tx.subscribe();
    let mut tick = tokio::time::interval(interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tick.tick().await; // the first tick is immediate; nothing is running yet
    loop {
        let due: Vec<String> = tokio::select! {
            _ = tick.tick() => {
                let config = ctx.config();
                config
                    .logins
                    .iter()
                    .filter(|l| l.check.is_some() && !active_in(&ctx, &l.name).is_empty())
                    .map(|l| l.name.clone())
                    .collect()
            }
            notice = notices.recv() => match notice {
                Ok(Notice::State(s)) if s.state == State::Failed && s.reason == Some(FailureReason::AuthExpired) => {
                    let login = ctx.tunnels().get(&s.name).and_then(|e| e.login.clone());
                    let Some(login) = login else { continue };
                    let recent = ctx
                        .auth
                        .lock()
                        .await
                        .get(&login)
                        .and_then(|st| st.last.as_ref())
                        .is_some_and(|(_, _, at)| at.elapsed() < MIN_GAP);
                    let checkable = ctx.config().login(&login).is_some_and(|l| l.check.is_some());
                    if recent || !checkable { continue }
                    vec![login]
                }
                Err(broadcast::error::RecvError::Closed) => return,
                _ => continue,
            },
        };
        for name in due {
            check_login(&ctx, &name).await;
        }
    }
}

