//! `a3s_dev_tools`: golden fixture regeneration and Java cross-checks.
//!
//! Exit codes: 0 success, 1 differences or verification failures, 2 usage error or missing
//! prerequisites (JDK, harness, bash, non-Unix host), 3 any other error.

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "command-line tool"
)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use a3s_dev_tools::fixtures;
use a3s_dev_tools::{DumpMode, Harness, HarnessError, Part, TreeDiff};
use clap::{Parser, Subcommand};

/// Developer tooling for the Arma3Sync Rust rewrite.
#[derive(Debug, Parser)]
#[command(name = "a3s_dev_tools", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Regenerates the golden fixtures with the Java harness (JDK 25).
    RegenFixtures {
        /// Output directory (empty or missing). Default: a new temporary directory, kept.
        #[arg(long, conflicts_with_all = ["write_fixtures", "check"])]
        out: Option<PathBuf>,

        /// Comma-separated parts: descriptors, local, built, presets, roundtrip. Default: all.
        #[arg(long, value_delimiter = ',', value_parser = parse_part, conflicts_with_all = ["write_fixtures", "check"])]
        only: Vec<Part>,

        /// Generate everything and install it into tests/fixtures.
        #[arg(long, conflicts_with = "check")]
        write_fixtures: bool,

        /// Generate everything and compare it with tests/fixtures; exit 1 on differences.
        #[arg(long)]
        check: bool,
    },

    /// Reads files with Arma3Sync's own Java readers; exit 1 if any file fails.
    JavaVerify {
        /// Write a JSON dump of every object file read successfully into this directory.
        #[arg(long)]
        dump_dir: Option<PathBuf>,

        /// Semantic dumps (no identities, sorted sets and maps).
        #[arg(long)]
        semantic: bool,

        /// Files or directories (walked recursively).
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },
}

fn parse_part(arg: &str) -> Result<Part, String> {
    Part::from_arg(arg).ok_or_else(|| {
        let parts: Vec<&str> = Part::ALL.iter().map(|part| part.as_arg()).collect();
        format!("unknown part `{arg}` (parts: {})", parts.join(", "))
    })
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("a3s_dev_tools: {err:#}");
            let prerequisite = err.downcast_ref::<HarnessError>().is_some_and(|err| {
                matches!(
                    err,
                    HarnessError::NoJdk(_)
                        | HarnessError::NotFound(_)
                        | HarnessError::Unsupported
                        | HarnessError::NoBash
                )
            });
            ExitCode::from(if prerequisite { 2 } else { 3 })
        }
    }
}

fn run(command: Command) -> anyhow::Result<ExitCode> {
    let harness = Harness::locate()?;
    match command {
        Command::RegenFixtures {
            out,
            only,
            write_fixtures,
            check,
        } => regen_fixtures(&harness, out, &only, write_fixtures, check),
        Command::JavaVerify {
            dump_dir,
            semantic,
            paths,
        } => {
            let report = harness.verify(&paths, dump_dir.as_deref(), dump_mode(semantic))?;
            for line in &report.ok {
                println!("OK\t{}\t{}", line.path.display(), line.detail);
            }
            for line in &report.failed {
                println!("FAIL\t{}\t{}", line.path.display(), line.detail);
            }
            for path in &report.skipped {
                println!("SKIP\t{}", path.display());
            }
            println!(
                "SUMMARY\tok={}\tfail={}\tskip={}",
                report.ok.len(),
                report.failed.len(),
                report.skipped.len()
            );
            Ok(exit_code(report.is_success()))
        }
    }
}

fn regen_fixtures(
    harness: &Harness,
    out: Option<PathBuf>,
    only: &[Part],
    write_fixtures: bool,
    check: bool,
) -> anyhow::Result<ExitCode> {
    let fixtures_dir = harness.repo_root().join("tests").join("fixtures");
    if write_fixtures || check {
        let tmp = tempfile::Builder::new().prefix("a3s-fixtures").tempdir()?;
        let generated = tmp.path().join("generated");
        fixtures::regenerate(harness, &generated, &Part::ALL)?;
        if write_fixtures {
            fixtures::install(&generated, &fixtures_dir)?;
            println!(
                "installed {} entries into {}",
                fixtures::GENERATED_ENTRIES.len(),
                fixtures_dir.display()
            );
            return Ok(ExitCode::SUCCESS);
        }
        let diffs = fixtures::compare_generated(&fixtures_dir, &generated)?;
        for diff in &diffs {
            match diff {
                TreeDiff::Missing(path) => {
                    println!("only in tests/fixtures:     {}", path.display());
                }
                TreeDiff::Extra(path) => println!("only in the Java output:   {}", path.display()),
                TreeDiff::Differs(path) => println!("differs from Java output: {}", path.display()),
            }
        }
        if diffs.is_empty() {
            println!("tests/fixtures matches the Java harness output");
        }
        return Ok(exit_code(diffs.is_empty()));
    }
    let parts = if only.is_empty() {
        &Part::ALL[..]
    } else {
        only
    };
    let out = match out {
        Some(out) => out,
        None => tempfile::Builder::new()
            .prefix("a3s-fixtures")
            .tempdir()?
            .keep(),
    };
    fixtures::regenerate(harness, &out, parts)?;
    println!("generated fixtures in {}", display_abs(&out));
    Ok(ExitCode::SUCCESS)
}

fn display_abs(path: &Path) -> String {
    std::path::absolute(path)
        .unwrap_or_else(|_err| path.to_path_buf())
        .display()
        .to_string()
}

fn dump_mode(semantic: bool) -> DumpMode {
    if semantic {
        DumpMode::Semantic
    } else {
        DumpMode::Identity
    }
}

fn exit_code(success: bool) -> ExitCode {
    if success {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
