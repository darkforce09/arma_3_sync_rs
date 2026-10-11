//! `extract`: copies the `*.bikey` files below a folder into another one.

use std::path::Path;

use a3s_tools::CancelToken;
use a3s_tools::bikey::{self, BikeyOutcome};

use crate::app::{CommandResult, Failed};

/// Checks both folders, then copies every key found below `source` flat into `target`. Finding
/// no key is not a failure.
// Mirrors console/CommandGeneral.java:532-563
pub(crate) fn run(source: &str, target: &str) -> CommandResult {
    say!("{}", bikey::PROGRESS_TITLE);
    if let Err(err) = bikey::validate(source, target) {
        say_error!("{err}");
        return Err(Failed);
    }
    match bikey::extract(Path::new(source), Path::new(target), &CancelToken::new()) {
        Ok(outcome) => {
            say!("{}", outcome.message());
            say!("{}", bikey::CLI_DONE);
            match outcome {
                BikeyOutcome::Copied { .. } | BikeyOutcome::NoneFound => Ok(()),
                BikeyOutcome::Cancelled { .. } => Err(Failed),
            }
        }
        Err(err) => {
            say_error!("{}", bikey::CLI_FAILED);
            say_error!("{err}");
            Err(Failed)
        }
    }
}
