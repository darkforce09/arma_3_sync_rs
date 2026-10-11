//! User-visible texts of the TFAR and ACRE 2 installer wizards, as the Java client shows them.

/// TFAR wizard title.
pub const TFAR_WIZARD_TITLE: &str = "TFAR installer wizard";

/// TFAR wizard description.
pub const TFAR_WIZARD_DESCRIPTION: &str = "Install or update TFAR for ArmA 3 and TS3";

/// ACRE 2 wizard title.
pub const ACRE2_WIZARD_TITLE: &str = "ACRE 2 installer wizard";

/// ACRE 2 wizard description.
pub const ACRE2_WIZARD_DESCRIPTION: &str = "Install or update ACRE 2 for ArmA 3 and TS3";

/// First-page button that validates and installs.
pub const BUTTON_PROCEED: &str = "Proceed";

/// First-page button that closes the wizard.
pub const BUTTON_CANCEL: &str = "Cancel";

/// Second-page button back to the first page.
pub const BUTTON_BACK: &str = "Back";

/// Second-page button that closes the wizard.
pub const BUTTON_CLOSE: &str = "Close";

/// Button that opens a folder chooser for a field.
pub const BUTTON_SELECT: &str = "Select";

/// Second-page button that opens `<TS3>/plugins` or `<Arma 3>/userconfig`.
pub const BUTTON_VIEW: &str = "View";

/// Title of the folder fields group.
pub const GROUP_DIRECTORIES: &str = "Installation directories";

/// Title of the system specifications group.
pub const GROUP_SYSTEM_SPECS: &str = "System specifications";

/// Arma 3 folder field label.
pub const FIELD_ARMA3: &str = "ArmA 3";

/// TS3 folder field label.
pub const FIELD_TS3: &str = "TS3";

/// TFAR plugin folder field label.
pub const FIELD_TFAR_PLUGINS: &str = "@task_force_radio\\plugins";

/// TFAR userconfig folder field label.
pub const FIELD_TFAR_USERCONFIG: &str = "@task_force_radio\\userconfig";

/// ACRE 2 plugin folder field label.
pub const FIELD_ACRE2_PLUGIN: &str = "@ACRE2\\plugin";

/// Operating system spec label.
pub const SPEC_OS: &str = "OS: ";

/// TS3 spec label.
pub const SPEC_TS3: &str = "TS3: ";

/// TFAR plugin spec label.
pub const SPEC_TFAR_PLUGIN: &str = "TFAR plugins: ";

/// ACRE 2 plugin spec label.
pub const SPEC_ACRE2_PLUGIN: &str = "ACRE 2 plugin: ";

/// Heading of the second page.
pub const OPERATIONS_HEADING: &str = "The following operations have been performed:";

/// Second-page line of the TFAR plugin step.
pub const TFAR_PLUGIN_STEP: &str = "  - TFAR plugin copied into TS3\\plugins directory. ";

/// Second-page line of the TFAR userconfig step.
pub const TFAR_USERCONFIG_STEP: &str = "  - TFAR userconfig copied into ArmA 3 directory.";

/// Second-page line of the ACRE 2 plugin step.
pub const ACRE2_PLUGIN_STEP: &str = "  - ACRE 2 plugin copied into TS3\\plugins directory. ";

/// First closing message of a successful TFAR install.
pub const TFAR_SUCCESS_MESSAGE: &str = "You can now run your game with @task_force_radio.";

/// Second closing message of a successful TFAR install.
pub const TFAR_ADMIN_REMINDER: &str = "Don't forget to run TS3 with administrator priviledges.";

/// Closing message of a successful ACRE 2 install.
pub const ACRE2_SUCCESS_MESSAGE: &str = "You can now run your game with @ACRE2.";
