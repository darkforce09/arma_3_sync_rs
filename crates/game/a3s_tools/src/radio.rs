//! The TFAR and ACRE 2 installer wizards' logic: TS3 detection, the system
//! specifications panel, first-page validation and the second-page copy steps.
//!
//! Platform lookups (the TS3 registry entry, the process list) come in through plain inputs and
//! [`Ts3ProcessProbe`].

mod install;
mod specs;
pub mod strings;
mod ts3;
mod validate;
mod version;

pub use install::{
    Acre2Report, ERROR_TITLE, InstallStepError, STATUS_FAIL, STATUS_OK, TfarReport, install_acre2,
    install_tfar, step_status,
};
pub use specs::{HostInfo, SystemSpecs, os_label, plugin_label, system_specs, ts3_label};
pub use ts3::{
    TS3_EXE_32, TS3_EXE_64, Ts3Arch, arma3_dir_from_exe, existing_or_empty, is_ts3_32bit,
    is_ts3_64bit, parse_ts3_version, stored_or_detected, ts3_arch, ts3_version,
};
pub use validate::{
    Acre2Form, Acre2Plan, InstallerIssue, IssueKind, TfarForm, TfarPlan, Ts3ProcessProbe,
    validate_acre2, validate_tfar,
};
pub use version::{file_version, plugin_version};

/// The label shown when a value cannot be determined.
pub const UNKNOWN: &str = "Unknown";

/// The radio mod an installer is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RadioMod {
    /// Task Force Arrowhead Radio (`@task_force_radio`).
    Tfar,

    /// Advanced Combat Radio Environment 2 (`@ACRE2`).
    Acre2,
}

impl RadioMod {
    /// The name used in messages: `TFAR` or `ACRE 2`.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Tfar => "TFAR",
            Self::Acre2 => "ACRE 2",
        }
    }
}
