//! Command-line grammar: the subcommands, and the Java client's single-dash flags mapped onto
//! them.

use std::ffi::{OsStr, OsString};

use clap::{CommandFactory as _, Parser, Subcommand};

use crate::build_info;

/// What the process was asked to do.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Invocation {
    /// Run a command; `debug` enables the diagnostic log file.
    Run { debug: bool, command: Command },

    /// The graphical client (no arguments, `-dev`, `-run`, `-unlock`).
    Gui,

    /// Help or version text was requested; print it and succeed.
    Info(String),

    /// The arguments are invalid; print the text and fail with a usage error.
    Usage(String),
}

/// The commands of the command line.
#[derive(Debug, PartialEq, Eq, Subcommand)]
pub(crate) enum Command {
    /// Build a repository from its main folder (legacy `-build NAME`).
    Build {
        /// Name of the repository.
        repository: String,
    },

    /// Check that the server holds every file of a repository (legacy `-check NAME`).
    Check {
        /// Name of the repository.
        repository: String,
    },

    /// Synchronize the content of a repository into a folder (legacy
    /// `-sync NAME FOLDER true|false`).
    Sync {
        /// Name of the repository.
        repository: String,

        /// Destination folder; it must exist.
        destination: String,

        /// Delete every extra file in the destination folder.
        #[arg(long)]
        exact_match: bool,
    },

    /// Copy every `*.bikey` file below a folder into another one (legacy `-extract SOURCE
    /// TARGET`).
    Extract {
        /// Folder searched for `*.bikey` files.
        source: String,

        /// Folder the keys are copied into.
        target: String,
    },

    /// Check for an Arma3Sync update and install it (legacy `-update`). Exits with 0 when
    /// updated, 2 when already up to date, 1 on failure.
    Update {
        /// Use the development release channel.
        #[arg(long)]
        dev: bool,
    },

    /// Run the interactive console, reading commands from standard input (legacy `-console`).
    Console {
        /// Use the development release channel for `UPDATE`.
        #[arg(long)]
        dev: bool,
    },
}

#[derive(Debug, Parser)]
#[command(
    name = "arma3sync",
    version = build_info::VERSION,
    about = "Arma 3 mod repository synchronizer, repository builder and launcher.",
    after_help = LEGACY_HELP
)]
struct Cli {
    /// Write a diagnostic log into the data folder (also `-debug` or `A3S_DEBUG=true`).
    #[arg(long, global = true)]
    debug: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

/// The Java client's flags, listed under the help.
// Mirrors main/ArmA3Sync.java:160-170
const LEGACY_HELP: &str = "\
Legacy flags (case-insensitive):
  -BUILD NAME                     build repository
  -CHECK NAME                     check repository
  -CONSOLE                        run the console management
  -EXTRACT SOURCE TARGET          extract *.bikey files
  -SYNC NAME FOLDER true|false    synchronize with repository (true: exact content matching)
  -UPDATE                         check for ArmA3Sync updates
  -DEBUG                          enable diagnostic logging";

/// The full help text.
pub(crate) fn help() -> String {
    Cli::command().render_help().to_string()
}

/// Parses the arguments after the program name.
///
/// `-debug` is removed wherever it appears, ignoring case (Java's `removeDebugArgument`). An
/// invocation starting with one of the Java flags then follows Java's grammar exactly (flag
/// case ignored, values trimmed); anything else goes to the subcommand parser.
// Mirrors main/ArmA3Sync.java:41-49,63-86,118-172
pub(crate) fn parse(args: &[OsString]) -> Invocation {
    let legacy_debug = args.iter().any(|arg| is_flag(arg, "-debug"));
    let rest: Vec<&OsString> = args.iter().filter(|arg| !is_flag(arg, "-debug")).collect();
    if let Some(invocation) = parse_legacy(&rest, legacy_debug) {
        return invocation;
    }
    let argv =
        std::iter::once(OsStr::new("arma3sync")).chain(rest.iter().map(|arg| arg.as_os_str()));
    match Cli::try_parse_from(argv) {
        Ok(cli) => match cli.command {
            Some(command) => Invocation::Run {
                debug: cli.debug || legacy_debug,
                command,
            },
            None => Invocation::Gui,
        },
        Err(err) if err.use_stderr() => Invocation::Usage(err.render().to_string()),
        Err(err) => Invocation::Info(err.render().to_string()),
    }
}

/// The Java grammar, or `None` when the first argument is not a Java flag.
fn parse_legacy(args: &[&OsString], debug: bool) -> Option<Invocation> {
    const FLAGS: [&str; 9] = [
        "-build", "-check", "-console", "-dev", "-extract", "-run", "-sync", "-unlock", "-update",
    ];
    let first = args.first()?;
    if !FLAGS.iter().any(|flag| is_flag(first, flag)) {
        return None;
    }
    let flags: Vec<String> = args
        .iter()
        .map(|arg| arg.to_string_lossy().to_ascii_lowercase())
        .collect();
    let value = |index: usize| args[index].to_string_lossy().trim().to_owned();
    let flags: Vec<&str> = flags.iter().map(String::as_str).collect();
    let run = |command| Invocation::Run { debug, command };
    Some(match flags.as_slice() {
        [] => return None,
        ["-dev" | "-run" | "-unlock"] | ["-dev", "-run"] => Invocation::Gui,
        ["-console"] => run(Command::Console { dev: false }),
        ["-dev", "-console"] => run(Command::Console { dev: true }),
        ["-build", _] => run(Command::Build {
            repository: value(1),
        }),
        ["-check", _] => run(Command::Check {
            repository: value(1),
        }),
        ["-extract", _, _] => run(Command::Extract {
            source: value(1),
            target: value(2),
        }),
        ["-sync", _, _, _] => match value(3).to_ascii_lowercase().as_str() {
            "true" => run(sync(value(1), value(2), true)),
            "false" => run(sync(value(1), value(2), false)),
            // Mirrors console/CommandLine.java:91-96
            _ => Invocation::Usage(format!(
                "Unrecognized exact math parameter (true/false).\n\n{}",
                help()
            )),
        },
        ["-update"] => run(Command::Update { dev: false }),
        // Mirrors main/ArmA3Sync.java:159-171
        _ => Invocation::Usage(format!("ArmA3Sync - bad command.\n\n{}", help())),
    })
}

fn sync(repository: String, destination: String, exact_match: bool) -> Command {
    Command::Sync {
        repository,
        destination,
        exact_match,
    }
}

/// Whether `arg` is the single-dash flag `flag` (lowercase), ignoring ASCII case.
fn is_flag(arg: &OsStr, flag: &str) -> bool {
    arg.to_str()
        .is_some_and(|arg| arg.eq_ignore_ascii_case(flag))
}
