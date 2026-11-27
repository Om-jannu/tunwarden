//! Test helper that imitates a tunnel command (design section 10).
//!
//! ```text
//! fake-tunnel [--listen ADDR:PORT]... [--listen-after DUR]
//!             [--print TEXT] [--print-stdout TEXT] [--print-after DUR] [--print-env NAME]
//!             [--spawn-child] [--ignore-sigterm] [--exit-after DUR] [--exit-code N]
//! ```
//!
//! Durations are like `500ms` or `2s`. Without `--exit-after` it runs until
//! killed. `--spawn-child` starts a second fake-tunnel that sleeps forever, to
//! test that a stop also kills grandchildren; its PID is printed to stdout as
//! `child PID`. `--ignore-sigterm` (Unix) makes it, and its child, survive SIGTERM.

use std::io::Write;
use std::net::{SocketAddr, TcpListener};
use std::process::{Command, Stdio};
use std::time::Duration;

#[derive(Default)]
struct Opts {
    listen: Vec<SocketAddr>,
    listen_after: Duration,
    print: Vec<String>,
    print_stdout: Vec<String>,
    print_after: Duration,
    print_env: Vec<String>,
    spawn_child: bool,
    ignore_sigterm: bool,
    exit_after: Option<Duration>,
    exit_code: i32,
    sleep_forever: bool,
}

fn main() {
    let opts = match parse(std::env::args().skip(1)) {
        Ok(opts) => opts,
        Err(e) => {
            eprintln!("fake-tunnel: {e}");
            std::process::exit(2);
        }
    };

    if opts.ignore_sigterm {
        ignore_sigterm();
    }
    if opts.sleep_forever {
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }

    if opts.spawn_child {
        let mut cmd = Command::new(std::env::current_exe().expect("current exe"));
        cmd.arg("--sleep-forever");
        if opts.ignore_sigterm {
            cmd.arg("--ignore-sigterm");
        }
        #[expect(
            clippy::zombie_processes,
            reason = "the child must outlive this process to test group cleanup"
        )]
        let child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn child");
        println!("child {}", child.id());
        let _ = std::io::stdout().flush();
    }

    for name in &opts.print_env {
        let value = std::env::var(name).unwrap_or_else(|_| "<unset>".to_owned());
        println!("env {name}={value}");
    }
    let _ = std::io::stdout().flush();

    if !opts.print.is_empty() || !opts.print_stdout.is_empty() {
        let (print, print_stdout, after) = (
            opts.print.clone(),
            opts.print_stdout.clone(),
            opts.print_after,
        );
        std::thread::spawn(move || {
            std::thread::sleep(after);
            for line in print_stdout {
                println!("{line}");
            }
            let _ = std::io::stdout().flush();
            for line in print {
                eprintln!("{line}");
            }
        });
    }

    if !opts.listen.is_empty() {
        let (addrs, after) = (opts.listen.clone(), opts.listen_after);
        std::thread::spawn(move || {
            std::thread::sleep(after);
            for addr in addrs {
                match TcpListener::bind(addr) {
                    Ok(listener) => {
                        std::thread::spawn(move || {
                            for conn in listener.incoming() {
                                drop(conn);
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("fake-tunnel: bind {addr}: {e}");
                        std::process::exit(3);
                    }
                }
            }
        });
    }

    match opts.exit_after {
        Some(after) => {
            std::thread::sleep(after);
            std::process::exit(opts.exit_code);
        }
        None => loop {
            std::thread::sleep(Duration::from_secs(3600));
        },
    }
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Opts, String> {
    let mut o = Opts::default();
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--listen" => o
                .listen
                .push(value()?.parse().map_err(|e| format!("--listen: {e}"))?),
            "--listen-after" => o.listen_after = duration(&value()?)?,
            "--print" => o.print.push(value()?),
            "--print-stdout" => o.print_stdout.push(value()?),
            "--print-after" => o.print_after = duration(&value()?)?,
            "--print-env" => o.print_env.push(value()?),
            "--spawn-child" => o.spawn_child = true,
            "--ignore-sigterm" => o.ignore_sigterm = true,
            "--exit-after" => o.exit_after = Some(duration(&value()?)?),
            "--exit-code" => {
                o.exit_code = value()?.parse().map_err(|e| format!("--exit-code: {e}"))?
            }
            "--sleep-forever" => o.sleep_forever = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(o)
}

fn duration(text: &str) -> Result<Duration, String> {
    let (number, scale) = if let Some(ms) = text.strip_suffix("ms") {
        (ms, 1)
    } else if let Some(s) = text.strip_suffix('s') {
        (s, 1000)
    } else {
        return Err(format!("bad duration {text}; use for example 500ms or 2s"));
    };
    let n: u64 = number.parse().map_err(|_| format!("bad duration {text}"))?;
    Ok(Duration::from_millis(n * scale))
}

#[cfg(unix)]
fn ignore_sigterm() {
    // Registering a flag replaces the default action, so SIGTERM no longer kills us.
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, flag).expect("register SIGTERM");
}

#[cfg(not(unix))]
fn ignore_sigterm() {}
