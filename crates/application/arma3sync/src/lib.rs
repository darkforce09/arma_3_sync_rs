//! Arma3Sync: Arma 3 mod repository synchronization and launcher.
//!
//! The library half of the `arma3sync` binary: the command line ([`run`]), the window
//! (`a3s_viewer`), the interactive console and the build information baked into the binary.

/// Writes one line to standard output; a closed output is ignored.
macro_rules! say {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        writeln!(std::io::stdout(), $($arg)*).ok();
    }};
}

/// Writes one line to standard error; a closed output is ignored.
macro_rules! say_error {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        writeln!(std::io::stderr(), $($arg)*).ok();
    }};
}

mod app;
pub mod build_info;
mod cli;
mod client_store;
mod commands;
mod console;

use std::ffi::OsString;
use std::process::ExitCode;

/// Exit code of a successful command.
pub const EXIT_SUCCESS: u8 = 0;

/// Exit code of a failed command.
pub const EXIT_FAILURE: u8 = 1;

/// Exit code of invalid arguments.
pub const EXIT_USAGE: u8 = 2;

/// Entry point of the `arma3sync` binary.
///
/// The elevated updater subcommand is handled before anything else; then the arguments are
/// parsed, and a command or the window runs with the application directories and logging set
/// up.
pub fn run() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg == a3s_updater::apply::ELEVATED_SUBCOMMAND)
    {
        let code = a3s_updater::apply::run_elevated_install(&args[1..], build_info::VERSION);
        return exit_code(code);
    }
    match cli::parse(&args) {
        cli::Invocation::Run { debug, command } => ExitCode::from(app::run(debug, command)),
        cli::Invocation::Gui { debug, window } => ExitCode::from(app::run_window(debug, window)),
        cli::Invocation::Info(text) => {
            say!("{}", text.trim_end());
            ExitCode::from(EXIT_SUCCESS)
        }
        cli::Invocation::Usage(text) => {
            say_error!("{}", text.trim_end());
            ExitCode::from(EXIT_USAGE)
        }
    }
}

/// A process exit code from an `i32` status (anything outside `0..=255` is a failure).
fn exit_code(code: i32) -> ExitCode {
    ExitCode::from(u8::try_from(code).unwrap_or(EXIT_FAILURE))
}
