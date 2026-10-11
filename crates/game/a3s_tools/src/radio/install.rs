//! Second-page copy steps of the installer wizards.

use std::io;
use std::path::{Path, PathBuf};

use super::strings;
use super::validate::{Acre2Plan, TfarPlan};
use crate::fs_util::{copy_directory, is_writable};

/// Step status shown when a copy succeeded.
pub const STATUS_OK: &str = "OK";

/// Step status shown when a copy failed.
pub const STATUS_FAIL: &str = "Fail";

/// Title of the error box of a failed step.
pub const ERROR_TITLE: &str = "Error";

/// Why an install step failed. Its Display is the error box text.
#[derive(Debug, thiserror::Error)]
pub enum InstallStepError {
    /// No file can be created in the destination folder.
    #[error("Can't to write on: {}\nCheck files permissions.", dir.display())]
    NotWritable {
        /// The folder that is not writable.
        dir: PathBuf,
    },

    /// Copying failed part way; what was copied stays.
    #[error("Failed to write on: {}\n{source}", dir.display())]
    CopyFailed {
        /// The destination folder.
        dir: PathBuf,

        /// The underlying error, naming the file that failed.
        #[source]
        source: io::Error,
    },
}

/// The status label of a step: [`STATUS_OK`] or [`STATUS_FAIL`].
pub fn step_status(result: &Result<(), InstallStepError>) -> &'static str {
    if result.is_ok() {
        STATUS_OK
    } else {
        STATUS_FAIL
    }
}

/// Results of the TFAR install. A failed step does not stop the other one; each failure is its
/// own error box, userconfig first.
#[derive(Debug)]
pub struct TfarReport {
    /// Copy of the userconfig folder into `<Arma 3>/userconfig`.
    pub userconfig: Result<(), InstallStepError>,

    /// Copy of the plugin folder into `<TS3>/plugins`.
    pub plugin: Result<(), InstallStepError>,
}

impl TfarReport {
    /// The two closing messages, shown only when both steps succeeded.
    // Mirrors ui/tools/tfar/SecondPageTFARInstallerPanel.java:240-247
    pub fn success_messages(&self) -> Option<[&'static str; 2]> {
        (self.userconfig.is_ok() && self.plugin.is_ok())
            .then_some([strings::TFAR_SUCCESS_MESSAGE, strings::TFAR_ADMIN_REMINDER])
    }
}

/// Result of the ACRE 2 install.
#[derive(Debug)]
pub struct Acre2Report {
    /// Copy of the plugin dll into `<TS3>/plugins`.
    pub plugin: Result<(), InstallStepError>,
}

impl Acre2Report {
    /// The closing message, shown only when the copy succeeded.
    // Mirrors ui/tools/acre2/SecondPageACRE2InstallerDialog.java:166-171
    pub fn success_message(&self) -> Option<&'static str> {
        self.plugin
            .is_ok()
            .then_some(strings::ACRE2_SUCCESS_MESSAGE)
    }
}

/// Copies the TFAR userconfig folder into the Arma 3 folder (merging with an existing
/// `userconfig`) and the whole TFAR plugin folder into the TS3 `plugins` folder.
// Mirrors ui/tools/tfar/SecondPageTFARInstallerPanel.java:156-226
pub fn install_tfar(plan: &TfarPlan) -> TfarReport {
    let userconfig = install_step(&plan.arma3_dir, || {
        let target = plan.arma3_dir.join("userconfig");
        // Java ignores the result of `mkdirs`; a failure surfaces in the copy.
        let _ignored: io::Result<()> = std::fs::create_dir_all(&target);
        copy_directory(&plan.userconfig_dir, &target)
    });
    let plugin = install_step(&plan.ts3_dir, || {
        copy_directory(&plan.plugin_dir, &plan.ts3_dir.join("plugins"))
    });
    TfarReport { userconfig, plugin }
}

/// Copies the ACRE 2 plugin dll into the TS3 `plugins` folder, overwriting it.
// Mirrors ui/tools/acre2/SecondPageACRE2InstallerDialog.java:124-155
pub fn install_acre2(plan: &Acre2Plan) -> Acre2Report {
    let plugin = install_step(&plan.ts3_dir, || {
        let mut target = plan.ts3_dir.join("plugins");
        if let Some(name) = plan.plugin_file.file_name() {
            target.push(name);
        }
        copy_directory(&plan.plugin_file, &target)
    });
    Acre2Report { plugin }
}

/// Runs `copy` when `dir` is writable, attributing a failure to `dir`.
fn install_step(dir: &Path, copy: impl FnOnce() -> io::Result<()>) -> Result<(), InstallStepError> {
    if !is_writable(dir) {
        return Err(InstallStepError::NotWritable {
            dir: dir.to_path_buf(),
        });
    }
    copy().map_err(|source| InstallStepError::CopyFailed {
        dir: dir.to_path_buf(),
        source,
    })
}
