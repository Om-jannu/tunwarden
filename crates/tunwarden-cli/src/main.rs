fn main() -> std::process::ExitCode {
    // A crash prints a short message and saves a report, instead of a backtrace.
    human_panic::setup_panic!();
    tunwarden_cli::run(std::env::args_os())
}
