//! Ports of the Java enums in `fr.soe.a3s.constant`.
//!
//! Every enum offers `ALL` (Java declaration order), `java_name()` (the Java constant name) and
//! `from_java_name()`, plus the per-enum data and parsers of the Java original, including their
//! fallbacks for unknown input. `ProtocolType` and `EncryptionMode` live in the crate root; this
//! module adds their description parsers.
//!
//! Serde uses the Rust variant names (like [`ProtocolType`]); the Java names stay available
//! through `java_name()`.

use serde::{Deserialize, Serialize};

use crate::{EncryptionMode, ProtocolType};

/// Declares a Java enum port with `ALL`, `java_name()` and `from_java_name()`.
macro_rules! java_enum {
    (
        $(#[$meta:meta])*
        $name:ident { $($(#[$vmeta:meta])* $variant:ident = $java:literal),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $name {
            /// Every constant, in Java declaration order (`values()`).
            pub const ALL: [Self; [$(stringify!($variant)),+].len()] = [$(Self::$variant),+];

            /// Java enum constant name (`name()`).
            pub fn java_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $java),+
                }
            }

            /// Parses a Java enum constant name (`valueOf`), case-sensitive.
            pub fn from_java_name(name: &str) -> Option<Self> {
                Self::ALL.into_iter().find(|v| v.java_name() == name)
            }
        }
    };
}

// ---------------------------------------------------------------------------------------------
// CheckRepositoriesFrequency

java_enum! {
    /// How often the client checks repositories for updates
    /// (`fr.soe.a3s.constant.CheckRepositoriesFrequency`).
    CheckRepositoriesFrequency {
        Disabled = "DISABLED",
        Freq1 = "FREQ1",
        Freq2 = "FREQ2",
        Freq3 = "FREQ3",
    }
}

impl CheckRepositoriesFrequency {
    /// UI description (`getDescription`).
    // Mirrors constant/CheckRepositoriesFrequency.java:5-6
    pub fn description(self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::Freq1 => "Every 15 minutes",
            Self::Freq2 => "Every 30 minutes",
            Self::Freq3 => "Every 60 minutes",
        }
    }

    /// Check interval in minutes, `0` when disabled (`getFrequency`).
    pub fn minutes(self) -> i32 {
        match self {
            Self::Disabled => 0,
            Self::Freq1 => 15,
            Self::Freq2 => 30,
            Self::Freq3 => 60,
        }
    }

    /// Parses a description (`getEnum`); `None` if unknown.
    // Mirrors constant/CheckRepositoriesFrequency.java:25-37
    pub fn from_description(description: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|v| v.description() == description)
    }
}

// ---------------------------------------------------------------------------------------------
// ConsoleCommands

java_enum! {
    /// Commands of the interactive console (`fr.soe.a3s.constant.ConsoleCommands`).
    ConsoleCommand {
        New = "NEW",
        Build = "BUILD",
        Update = "UPDATE",
        Check = "CHECK",
        List = "LIST",
        Delete = "DELETE",
        Help = "HELP",
        Version = "VERSION",
        Quit = "QUIT",
        Commands = "COMMANDS",
        Sync = "SYNC",
        Extract = "EXTRACT",
    }
}

// ---------------------------------------------------------------------------------------------
// DefaultProfileName, GameVersions, TimeOutValues

/// Name of the profile that always exists (`DefaultProfileName.DEFAULT`).
pub const DEFAULT_PROFILE_NAME: &str = "Default";

/// The only supported game (`GameVersions.ARMA3`).
pub const GAME_VERSION_ARMA3: &str = "Arma 3";

/// Default connection timeout in milliseconds (`TimeOutValues.CONNECTION_TIME_OUT`).
pub const CONNECTION_TIMEOUT_MS: u32 = crate::protocol::DEFAULT_TIMEOUT_MS;

/// Default read timeout in milliseconds (`TimeOutValues.READ_TIME_OUT`).
pub const READ_TIMEOUT_MS: u32 = crate::protocol::DEFAULT_TIMEOUT_MS;

// ---------------------------------------------------------------------------------------------
// DownloadStatus

java_enum! {
    /// State of one download task (`fr.soe.a3s.constant.DownloadStatus`).
    DownloadStatus {
        Pending = "PENDING",
        Running = "RUNNING",
        Done = "DONE",
    }
}

// ---------------------------------------------------------------------------------------------
// GameDLCs

java_enum! {
    /// Arma 3 creator DLCs (`fr.soe.a3s.constant.GameDLCs`).
    ///
    /// The Java names (note the lowercase `vn`) are part of the repository format: they are the
    /// keys of [`crate::Event::dlc_names`].
    GameDlc {
        Contact = "Contact",
        Gm = "GM",
        Vn = "vn",
        Csla = "CSLA",
        Ws = "WS",
        Spe = "SPE",
        Ef = "EF",
        Rf = "RF",
    }
}

impl GameDlc {
    /// Description (`GetDescription`); empty for `Contact`.
    // Mirrors constant/GameDLCs.java:9-16
    pub fn description(self) -> &'static str {
        match self {
            Self::Contact => "",
            Self::Gm => "Global Mobilization",
            Self::Vn => "S.O.G Prairie Fire",
            Self::Csla => "Iron Curtain",
            Self::Ws => "Western Sahara",
            Self::Spe => "Spearhead 1944",
            Self::Ef => "Expeditionary Forces",
            Self::Rf => "Reaction Forces",
        }
    }

    /// The description, or the Java name when the description is empty (`getDisplayName`).
    // Mirrors constant/GameDLCs.java:30-32
    pub fn display_name(self) -> &'static str {
        let description = self.description();
        if description.is_empty() {
            self.java_name()
        } else {
            description
        }
    }

    /// Steam application id (`getSteamAppId`).
    pub fn steam_app_id(self) -> &'static str {
        match self {
            Self::Contact => "1021790",
            Self::Gm => "1042220",
            Self::Vn => "1227700",
            Self::Csla => "1294440",
            Self::Ws => "1681170",
            Self::Spe => "1175380",
            Self::Ef => "2647830",
            Self::Rf => "2647760",
        }
    }

    /// Steam store page (`getSteamStoreUrl`).
    pub fn steam_store_url(self) -> String {
        format!("https://store.steampowered.com/app/{}", self.steam_app_id())
    }

    /// Finds a DLC by Java name, ignoring case (`fromName`, `equalsIgnoreCase`).
    // Mirrors constant/GameDLCs.java:42-52
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|dlc| crate::java_string::compare_ignore_case(dlc.java_name(), name).is_eq())
    }

    /// Finds a DLC by exact Steam application id (`fromSteamAppId`).
    // Mirrors constant/GameDLCs.java:54-64
    pub fn from_steam_app_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|dlc| dlc.steam_app_id() == id)
    }
}

// ---------------------------------------------------------------------------------------------
// GameExecutables

java_enum! {
    /// Executable file names of the game, server and Steam (`fr.soe.a3s.constant.GameExecutables`).
    GameExecutable {
        Game = "GAME",
        GameX64 = "GAME_x64",
        WinServer = "WIN_SERVER",
        WinServerX64 = "WIN_SERVER_x64",
        LinuxServer = "LINUX_SERVER",
        Steam = "STEAM",
        Battleye = "BATTLEYE",
    }
}

impl GameExecutable {
    /// File name (`getDescription`).
    // Mirrors constant/GameExecutables.java:5-6
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Game => "arma3.exe",
            Self::GameX64 => "arma3_x64.exe",
            Self::WinServer => "arma3server.exe",
            Self::WinServerX64 => "arma3server_x64.exe",
            Self::LinuxServer => "arma3server",
            Self::Steam => "steam.exe",
            Self::Battleye => "arma3battleye.exe",
        }
    }
}

// ---------------------------------------------------------------------------------------------
// GameSystemFolders

/// Folders of the game installation that are not addons (`GameSystemFolders.values()`), in Java
/// declaration order and case.
// Mirrors constant/GameSystemFolders.java:4
pub const GAME_SYSTEM_FOLDERS: [&str; 21] = [
    "Addons",
    "Curator",
    "Kart",
    "Heli",
    "Mark",
    "Expansion",
    "Argo",
    "Jets",
    "Orange",
    "Tank",
    "Tacops",
    "Enoch",
    "AoW",
    "Contact",
    "GM",
    "vn",
    "CSLA",
    "WS",
    "SPE",
    "EF",
    "RF",
];

// ---------------------------------------------------------------------------------------------
// IconResize

java_enum! {
    /// UI icon scale (`fr.soe.a3s.constant.IconResize`). `Disabled` is the Java `NONE`.
    IconResize {
        Auto = "AUTO",
        Size1 = "SIZE1",
        Size2 = "SIZE2",
        Size3 = "SIZE3",
        Size4 = "SIZE4",
        Disabled = "NONE",
    }
}

impl IconResize {
    /// UI description (`getDescription`).
    // Mirrors constant/IconResize.java:5-6
    pub fn description(self) -> &'static str {
        match self {
            Self::Auto => "Default",
            Self::Size1 => "x1.25",
            Self::Size2 => "x1.5",
            Self::Size3 => "x1.75",
            Self::Size4 => "x2",
            Self::Disabled => "Disabled",
        }
    }

    /// Scale factor (`getValue`); `0` means automatic.
    pub fn value(self) -> f64 {
        match self {
            Self::Auto => 0.0,
            Self::Size1 => 1.25,
            Self::Size2 => 1.5,
            Self::Size3 => 1.75,
            Self::Size4 => 2.0,
            Self::Disabled => 1.0,
        }
    }

    /// Parses a description (`getEnum`); `None` if unknown.
    // Mirrors constant/IconResize.java:25-42
    pub fn from_description(description: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|v| v.description() == description)
    }
}

// ---------------------------------------------------------------------------------------------
// LookAndFeel

java_enum! {
    /// Swing look and feel of the Java UI (`fr.soe.a3s.constant.LookAndFeel`).
    LookAndFeel {
        LafDefault = "LAF_DEFAULT",
        LafAluminium = "LAF_ALUMINIUM",
        LafGraphite = "LAF_GRAPHITE",
        LafHifi = "LAF_HIFI",
        LafNoire = "LAF_NOIRE",
        LafMetal = "LAF_METAL",
    }
}

impl LookAndFeel {
    /// Display name (`getName`).
    // Mirrors constant/LookAndFeel.java:5-6
    pub fn name(self) -> &'static str {
        match self {
            Self::LafDefault => "Default",
            Self::LafAluminium => "Aluminium",
            Self::LafGraphite => "Graphite",
            Self::LafHifi => "Hifi",
            Self::LafNoire => "Noire",
            Self::LafMetal => "Metal",
        }
    }

    /// Parses a display name (`getEnum`); `None` if unknown.
    // Mirrors constant/LookAndFeel.java:18-29
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.name() == name)
    }
}

// ---------------------------------------------------------------------------------------------
// MaxMemoryValues

java_enum! {
    /// Presets of the `-maxMem` launch option (`fr.soe.a3s.constant.MaxMemoryValues`).
    MaxMemoryValue {
        Min = "MIN",
        Max32 = "MAX32",
        Max64 = "MAX64",
    }
}

impl MaxMemoryValue {
    /// Value in megabytes (`getValue`).
    pub fn megabytes(self) -> i32 {
        match self {
            Self::Min => 1024,
            Self::Max32 => 2047,
            Self::Max64 => 3071,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// MinimizationType

java_enum! {
    /// What the main window does on game launch or minimize
    /// (`fr.soe.a3s.constant.MinimizationType`).
    MinimizationType {
        Nothing = "NOTHING",
        TaskBar = "TASK_BAR",
        Tray = "TRAY",
        Close = "CLOSE",
    }
}

impl MinimizationType {
    /// UI description (`getDescription`).
    // Mirrors constant/MinimizationType.java:4-5
    pub fn description(self) -> &'static str {
        match self {
            Self::Nothing => "Do nothing",
            Self::TaskBar => "Reduce to task bar",
            Self::Tray => "Reduce to tray",
            Self::Close => "Close",
        }
    }

    /// Parses a description (`getEnum`). Anything unknown is **`Close`**, as in Java.
    // Mirrors constant/MinimizationType.java:17-24
    pub fn from_description(description: &str) -> Self {
        match description {
            "Do nothing" => Self::Nothing,
            "Reduce to task bar" => Self::TaskBar,
            "Reduce to tray" => Self::Tray,
            _ => Self::Close,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// ModsetType

java_enum! {
    /// Origin of an addon group created from a repository or event
    /// (`fr.soe.a3s.constant.ModsetType`). Written to `export.a3s.autoconfig`.
    ModsetType {
        Event = "EVENT",
        Repository = "REPOSITORY",
    }
}

// ---------------------------------------------------------------------------------------------
// RepositoryStatus

java_enum! {
    /// Synchronization state of a repository (`fr.soe.a3s.constant.RepositoryStatus`).
    RepositoryStatus {
        Updated = "UPDATED",
        Ok = "OK",
        Indeterminated = "INDETERMINATED",
        Error = "ERROR",
    }
}

impl RepositoryStatus {
    /// UI description (`getDescription`).
    // Mirrors constant/RepositoryStatus.java:5-6
    pub fn description(self) -> &'static str {
        match self {
            Self::Updated => "UPDATED",
            Self::Ok => "OK",
            Self::Indeterminated => "-",
            Self::Error => "ERROR",
        }
    }
}

// ---------------------------------------------------------------------------------------------
// StartWithOS

java_enum! {
    /// Whether the client starts with the operating system (`fr.soe.a3s.constant.StartWithOS`).
    StartWithOs {
        Enabled = "ENABLED",
        Disabled = "DISABLED",
    }
}

impl StartWithOs {
    /// UI description (`getDescription`).
    pub fn description(self) -> &'static str {
        match self {
            Self::Enabled => "Enabled",
            Self::Disabled => "Disabled",
        }
    }

    /// Parses a description (`getEnum`). Anything unknown is **`Disabled`**, as in Java.
    // Mirrors constant/StartWithOS.java:17-23
    pub fn from_description(description: &str) -> Self {
        if description == "Enabled" {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
}

// ---------------------------------------------------------------------------------------------
// ProtocolType, EncryptionMode

/// Parses a [`ProtocolType::description`] (`ProtocolType.getEnum`).
///
/// `"A3S"` is **not** recognised and gives `None`, as in Java.
// Mirrors constant/ProtocolType.java:27-48
pub fn protocol_type_from_description(description: &str) -> Option<ProtocolType> {
    ProtocolType::ALL
        .into_iter()
        .filter(|t| *t != ProtocolType::A3s)
        .find(|t| t.description() == description)
}

/// UI description of an [`EncryptionMode`] (`EncryptionMode.getDescription`).
// Mirrors constant/EncryptionMode.java:5-6
pub fn encryption_mode_description(mode: EncryptionMode) -> &'static str {
    match mode {
        EncryptionMode::NoEncryption => "No encryption",
        EncryptionMode::ExplicitSsl => "SSL explicit encryption",
        EncryptionMode::ImplicitSsl => "SSL implicit encryption",
    }
}

/// Parses an encryption mode description (`EncryptionMode.getEnum`). Anything unknown is
/// **`ImplicitSsl`**, as in Java.
// Mirrors constant/EncryptionMode.java:18-26
pub fn encryption_mode_from_description(description: &str) -> EncryptionMode {
    match description {
        "No encryption" => EncryptionMode::NoEncryption,
        "SSL explicit encryption" => EncryptionMode::ExplicitSsl,
        _ => EncryptionMode::ImplicitSsl,
    }
}
