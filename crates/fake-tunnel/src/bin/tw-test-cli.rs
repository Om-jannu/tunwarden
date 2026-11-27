//! The `tunwarden` command line for system tests. Same code as the real
//! binary; it lives in this package so its tests can run it next to
//! `fake-tunnel`.

fn main() -> std::process::ExitCode {
    tunwarden_cli::run(std::env::args_os())
}
