//! Ports of `fr.soe.a3s.constant.*`: Java names, descriptions, values, parsers.

use a3s_types::constants::{
    CONNECTION_TIMEOUT_MS, CheckRepositoriesFrequency, ConsoleCommand, DEFAULT_PROFILE_NAME,
    DownloadStatus, GAME_SYSTEM_FOLDERS, GAME_VERSION_ARMA3, GameDlc, GameExecutable, IconResize,
    LookAndFeel, MaxMemoryValue, MinimizationType, ModsetType, READ_TIMEOUT_MS, RepositoryStatus,
    StartWithOs, encryption_mode_description, encryption_mode_from_description,
    protocol_type_from_description,
};
use a3s_types::{EncryptionMode, ProtocolType};

/// Checks `ALL` order against Java names and that `from_java_name` inverts `java_name`.
macro_rules! assert_java_names {
    ($ty:ty, [$($name:literal),+ $(,)?]) => {{
        let names: Vec<&str> = <$ty>::ALL.iter().map(|v| v.java_name()).collect();
        assert_eq!(names, [$($name),+], stringify!($ty));
        for v in <$ty>::ALL {
            assert_eq!(<$ty>::from_java_name(v.java_name()), Some(v));
        }
        assert_eq!(<$ty>::from_java_name("NOT_A_CONSTANT"), None);
    }};
}

#[test]
fn java_names_and_order() {
    assert_java_names!(
        CheckRepositoriesFrequency,
        ["DISABLED", "FREQ1", "FREQ2", "FREQ3"]
    );
    assert_java_names!(
        ConsoleCommand,
        [
            "NEW", "BUILD", "UPDATE", "CHECK", "LIST", "DELETE", "HELP", "VERSION", "QUIT",
            "COMMANDS", "SYNC", "EXTRACT"
        ]
    );
    assert_java_names!(DownloadStatus, ["PENDING", "RUNNING", "DONE"]);
    assert_java_names!(
        GameDlc,
        ["Contact", "GM", "vn", "CSLA", "WS", "SPE", "EF", "RF"]
    );
    assert_java_names!(
        GameExecutable,
        [
            "GAME",
            "GAME_x64",
            "WIN_SERVER",
            "WIN_SERVER_x64",
            "LINUX_SERVER",
            "STEAM",
            "BATTLEYE"
        ]
    );
    assert_java_names!(
        IconResize,
        ["AUTO", "SIZE1", "SIZE2", "SIZE3", "SIZE4", "NONE"]
    );
    assert_java_names!(
        LookAndFeel,
        [
            "LAF_DEFAULT",
            "LAF_ALUMINIUM",
            "LAF_GRAPHITE",
            "LAF_HIFI",
            "LAF_NOIRE",
            "LAF_METAL"
        ]
    );
    assert_java_names!(MaxMemoryValue, ["MIN", "MAX32", "MAX64"]);
    assert_java_names!(MinimizationType, ["NOTHING", "TASK_BAR", "TRAY", "CLOSE"]);
    assert_java_names!(ModsetType, ["EVENT", "REPOSITORY"]);
    assert_java_names!(
        RepositoryStatus,
        ["UPDATED", "OK", "INDETERMINATED", "ERROR"]
    );
    assert_java_names!(StartWithOs, ["ENABLED", "DISABLED"]);
}

#[test]
fn descriptions_values_and_parsers() {
    let freq: Vec<(&str, i32)> = CheckRepositoriesFrequency::ALL
        .iter()
        .map(|f| (f.description(), f.minutes()))
        .collect();
    assert_eq!(
        freq,
        [
            ("Disabled", 0),
            ("Every 15 minutes", 15),
            ("Every 30 minutes", 30),
            ("Every 60 minutes", 60)
        ]
    );
    for f in CheckRepositoriesFrequency::ALL {
        assert_eq!(
            CheckRepositoriesFrequency::from_description(f.description()),
            Some(f)
        );
    }
    assert_eq!(CheckRepositoriesFrequency::from_description("never"), None);

    let icons: Vec<(&str, f64)> = IconResize::ALL
        .iter()
        .map(|i| (i.description(), i.value()))
        .collect();
    assert_eq!(
        icons,
        [
            ("Default", 0.0),
            ("x1.25", 1.25),
            ("x1.5", 1.5),
            ("x1.75", 1.75),
            ("x2", 2.0),
            ("Disabled", 1.0)
        ]
    );
    for i in IconResize::ALL {
        assert_eq!(IconResize::from_description(i.description()), Some(i));
    }
    assert_eq!(IconResize::from_description("x3"), None);

    let lafs: Vec<&str> = LookAndFeel::ALL.iter().map(|l| l.name()).collect();
    assert_eq!(
        lafs,
        ["Default", "Aluminium", "Graphite", "Hifi", "Noire", "Metal"]
    );
    for l in LookAndFeel::ALL {
        assert_eq!(LookAndFeel::from_name(l.name()), Some(l));
    }
    assert_eq!(LookAndFeel::from_name("default"), None);

    let mem: Vec<i32> = MaxMemoryValue::ALL.iter().map(|m| m.megabytes()).collect();
    assert_eq!(mem, [1024, 2047, 3071]);

    let mins: Vec<&str> = MinimizationType::ALL
        .iter()
        .map(|m| m.description())
        .collect();
    assert_eq!(
        mins,
        [
            "Do nothing",
            "Reduce to task bar",
            "Reduce to tray",
            "Close"
        ]
    );
    for m in MinimizationType::ALL {
        assert_eq!(MinimizationType::from_description(m.description()), m);
    }
    // Java quirk: anything unknown is CLOSE.
    assert_eq!(
        MinimizationType::from_description("whatever"),
        MinimizationType::Close
    );

    let statuses: Vec<&str> = RepositoryStatus::ALL
        .iter()
        .map(|s| s.description())
        .collect();
    assert_eq!(statuses, ["UPDATED", "OK", "-", "ERROR"]);

    assert_eq!(StartWithOs::Enabled.description(), "Enabled");
    assert_eq!(StartWithOs::Disabled.description(), "Disabled");
    assert_eq!(
        StartWithOs::from_description("Enabled"),
        StartWithOs::Enabled
    );
    // Java quirk: anything unknown is DISABLED.
    assert_eq!(
        StartWithOs::from_description("enabled"),
        StartWithOs::Disabled
    );

    let exes: Vec<&str> = GameExecutable::ALL.iter().map(|e| e.file_name()).collect();
    assert_eq!(
        exes,
        [
            "arma3.exe",
            "arma3_x64.exe",
            "arma3server.exe",
            "arma3server_x64.exe",
            "arma3server",
            "steam.exe",
            "arma3battleye.exe"
        ]
    );

    assert_eq!(DEFAULT_PROFILE_NAME, "Default");
    assert_eq!(GAME_VERSION_ARMA3, "Arma 3");
    assert_eq!(CONNECTION_TIMEOUT_MS, 300_000);
    assert_eq!(READ_TIMEOUT_MS, 300_000);
}

#[test]
fn game_dlcs() {
    let data: Vec<(&str, &str)> = GameDlc::ALL
        .iter()
        .map(|d| (d.description(), d.steam_app_id()))
        .collect();
    assert_eq!(
        data,
        [
            ("", "1021790"),
            ("Global Mobilization", "1042220"),
            ("S.O.G Prairie Fire", "1227700"),
            ("Iron Curtain", "1294440"),
            ("Western Sahara", "1681170"),
            ("Spearhead 1944", "1175380"),
            ("Expeditionary Forces", "2647830"),
            ("Reaction Forces", "2647760")
        ]
    );
    assert_eq!(GameDlc::Contact.display_name(), "Contact");
    assert_eq!(GameDlc::Gm.display_name(), "Global Mobilization");
    assert!(GameDlc::Contact.steam_store_url().ends_with("/1021790"));
    assert_eq!(
        GameDlc::Contact.steam_store_url(),
        "https://store.steampowered.com/app/1021790"
    );
    assert_eq!(GameDlc::from_name("VN"), Some(GameDlc::Vn));
    assert_eq!(GameDlc::from_name("contact"), Some(GameDlc::Contact));
    assert_eq!(GameDlc::from_name("Global Mobilization"), None);
    assert_eq!(GameDlc::from_steam_app_id("1681170"), Some(GameDlc::Ws));
    assert_eq!(GameDlc::from_steam_app_id(" 1681170"), None);
    // `from_java_name` stays case-sensitive (`valueOf`).
    assert_eq!(GameDlc::from_java_name("VN"), None);
}

#[test]
fn game_system_folders() {
    assert_eq!(GAME_SYSTEM_FOLDERS.len(), 21);
    assert_eq!(
        GAME_SYSTEM_FOLDERS,
        [
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
            "RF"
        ]
    );
}

#[test]
fn protocol_and_encryption_descriptions() {
    let cases = [
        ("FTP", ProtocolType::Ftp),
        ("FTPS", ProtocolType::Ftps),
        ("SFTP", ProtocolType::Sftp),
        ("HTTP", ProtocolType::Http),
        ("HTTPS", ProtocolType::Https),
        ("SOCKS4", ProtocolType::Socks4),
        ("SOCKS5", ProtocolType::Socks5),
        ("HTTP/WEBDAV", ProtocolType::HttpWebdav),
        ("HTTPS/WEBDAV", ProtocolType::HttpsWebdav),
    ];
    for (description, protocol_type) in cases {
        assert_eq!(
            protocol_type_from_description(description),
            Some(protocol_type)
        );
    }
    // Java quirk: A3S has no description mapping.
    assert_eq!(protocol_type_from_description("A3S"), None);
    assert_eq!(protocol_type_from_description("HTTPS_WEBDAV"), None);
    assert_eq!(protocol_type_from_description("ftp"), None);

    let modes = [
        (EncryptionMode::NoEncryption, "No encryption"),
        (EncryptionMode::ExplicitSsl, "SSL explicit encryption"),
        (EncryptionMode::ImplicitSsl, "SSL implicit encryption"),
    ];
    for (mode, description) in modes {
        assert_eq!(encryption_mode_description(mode), description);
        assert_eq!(encryption_mode_from_description(description), mode);
    }
    // Java quirk: anything unknown is IMPLICIT_SSL.
    assert_eq!(
        encryption_mode_from_description("none"),
        EncryptionMode::ImplicitSsl
    );
}
