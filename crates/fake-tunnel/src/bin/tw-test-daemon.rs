//! Runs the Tunwarden daemon for tests, as `tunwarden daemon run` would.
//! The tests need it as a binary of this package, because Cargo gives a
//! binary's path only to tests of the same package.
//!
//! Usage: tw-test-daemon --config PATH   (state directory from TUNWARDEN_STATE_DIR)

use std::path::PathBuf;

fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1);
    let config = match (args.next().as_deref(), args.next()) {
        (Some("--config"), Some(path)) => PathBuf::from(path),
        _ => {
            eprintln!("usage: tw-test-daemon --config PATH");
            return std::process::ExitCode::from(2);
        }
    };
    let Some(state_dir) = std::env::var_os("TUNWARDEN_STATE_DIR") else {
        eprintln!("TUNWARDEN_STATE_DIR is not set");
        return std::process::ExitCode::from(2);
    };
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let opts = tunwarden_core::daemon::Options {
        config_path: config,
        state_dir: PathBuf::from(state_dir),
        handle_signals: true,
        auth_interval: None,
    };
    match runtime.block_on(tunwarden_core::daemon::run(opts)) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tw-test-daemon: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
