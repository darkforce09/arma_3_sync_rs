//! Registry-based detection on Windows, its absence elsewhere, and the special-folder rules.

use std::path::PathBuf;

use a3s_foundation::paths::{Os, PlatformEnv};
use a3s_platform::detect::{
    arma3_dir_from_registry, detect_arma3_dir, java_install_dirs, rpt_dir, steam_dir_from_registry,
    ts3_dir, windows_rpt_dir_from_appdata,
};
use a3s_platform::folders::{MemoryFolders, arma3_documents_dir, log_export_dir};
use a3s_platform::registry::{Hive, MemoryRegistry, RegData};

fn windows() -> PlatformEnv {
    PlatformEnv::for_tests(Os::Windows, Some(PathBuf::from(r"C:\Users\Bob")), &[], None)
}

fn sz(text: &str) -> RegData {
    RegData::Sz(text.to_owned())
}

#[test]
fn other_oses_never_query_the_registry() {
    let registry = MemoryRegistry::default();
    registry.insert(
        Hive::LocalMachine,
        r"SOFTWARE\Wow6432Node\Bohemia Interactive\Arma 3",
        Some("MAIN"),
        sz(r"D:\Arma 3"),
    );
    let env = PlatformEnv::for_tests(Os::Linux, None, &[], None);
    assert_eq!(arma3_dir_from_registry(&env, &registry), None);
    assert_eq!(steam_dir_from_registry(&env, &registry), None);
    assert_eq!(ts3_dir(&env, &registry), None);
    assert!(java_install_dirs(&env, &registry).is_empty());
    assert_eq!(registry.query_count(), 0);
}

#[test]
fn arma3_registry_path_is_returned_unchecked() {
    let registry = MemoryRegistry::default();
    registry.insert(
        Hive::LocalMachine,
        r"SOFTWARE\Wow6432Node\Bohemia Interactive\Arma 3",
        Some("MAIN"),
        sz(r"D:\Steam\steamapps\common\Arma 3"),
    );
    assert_eq!(
        detect_arma3_dir(&windows(), &registry),
        Some(PathBuf::from(r"D:\Steam\steamapps\common\Arma 3"))
    );
}

#[test]
fn ts3_prefers_the_user_install_then_falls_back_to_all_users() {
    let registry = MemoryRegistry::default();
    registry.insert(
        Hive::LocalMachine,
        r"SOFTWARE\Wow6432Node\TeamSpeak 3 Client",
        None,
        sz(r"C:\Program Files (x86)\TeamSpeak 3 Client"),
    );
    assert_eq!(
        ts3_dir(&windows(), &registry),
        Some(PathBuf::from(r"C:\Program Files (x86)\TeamSpeak 3 Client"))
    );
    registry.insert(
        Hive::LocalMachine,
        r"SOFTWARE\TeamSpeak 3 Client",
        None,
        sz(r"C:\Program Files\TeamSpeak 3 Client"),
    );
    assert_eq!(
        ts3_dir(&windows(), &registry),
        Some(PathBuf::from(r"C:\Program Files\TeamSpeak 3 Client"))
    );
    registry.insert(
        Hive::CurrentUser,
        r"Software\TeamSpeak 3 Client",
        None,
        sz(r"C:\Users\Bob\AppData\Local\TeamSpeak 3 Client"),
    );
    assert_eq!(
        ts3_dir(&windows(), &registry),
        Some(PathBuf::from(
            r"C:\Users\Bob\AppData\Local\TeamSpeak 3 Client"
        ))
    );
}

#[test]
fn steam_prefers_install_path_then_user_steam_path() {
    let registry = MemoryRegistry::default();
    registry.insert(
        Hive::CurrentUser,
        r"Software\Valve\Steam",
        Some("SteamPath"),
        sz("c:/program files (x86)/steam"),
    );
    assert_eq!(
        steam_dir_from_registry(&windows(), &registry),
        Some(PathBuf::from(r"c:\program files (x86)\steam"))
    );
    registry.insert(
        Hive::LocalMachine,
        r"SOFTWARE\Wow6432Node\Valve\Steam",
        Some("InstallPath"),
        sz(r"D:\Steam"),
    );
    assert_eq!(
        steam_dir_from_registry(&windows(), &registry),
        Some(PathBuf::from(r"D:\Steam"))
    );
}

#[test]
fn rpt_dir_is_the_local_sibling_of_appdata() {
    let table = [
        (
            r"C:\Users\Bob\AppData\Roaming",
            Some(r"C:\Users\Bob\AppData\Local\Arma 3"),
        ),
        (
            r"C:\Users\Bob\AppData\Roaming\",
            Some(r"C:\Users\Bob\AppData\Local\Arma 3"),
        ),
        (
            "C:/Users/Bob/AppData/Roaming",
            Some(r"C:/Users/Bob/AppData\Local\Arma 3"),
        ),
        (r"C:\", None),
        ("Roaming", None),
    ];
    for (appdata, expected) in table {
        assert_eq!(
            windows_rpt_dir_from_appdata(appdata).as_deref(),
            expected,
            "{appdata}"
        );
    }

    let registry = MemoryRegistry::default();
    let with_appdata = PlatformEnv::for_tests(
        Os::Windows,
        None,
        &[("APPDATA", r"C:\Users\Bob\AppData\Roaming")],
        None,
    );
    assert_eq!(
        rpt_dir(&with_appdata, &registry),
        Some(PathBuf::from(r"C:\Users\Bob\AppData\Local\Arma 3"))
    );
    assert_eq!(rpt_dir(&windows(), &registry), None);
    let blank = PlatformEnv::for_tests(Os::Windows, None, &[("APPDATA", "  ")], None);
    assert_eq!(rpt_dir(&blank, &registry), None);
}

#[test]
fn java_install_dirs_follow_publisher_order_without_duplicates() {
    let registry = MemoryRegistry::default();
    registry.insert(
        Hive::LocalMachine,
        r"SOFTWARE\Arma3Sync community\Arma3Sync",
        Some("InstallDir"),
        sz(r"C:\Program Files\Arma3Sync"),
    );
    registry.insert(
        Hive::LocalMachine,
        r"SOFTWARE\Wow6432Node\Arma3Sync community\Arma3Sync",
        Some("InstallDir"),
        sz(r"D:\Old Arma3Sync"),
    );
    registry.insert(
        Hive::LocalMachine,
        r"SOFTWARE\Wow6432Node\Virtuelle Panzerbrigade 21 (vPzBrig21)\Arma3Sync",
        Some("InstallDir"),
        sz(r"c:\program files\arma3sync"),
    );
    assert_eq!(
        java_install_dirs(&windows(), &registry),
        vec![
            PathBuf::from(r"c:\program files\arma3sync"),
            PathBuf::from(r"D:\Old Arma3Sync"),
        ]
    );
}

#[test]
fn log_export_goes_to_the_desktop_on_windows_and_home_elsewhere() {
    let folders = MemoryFolders {
        home: Some(PathBuf::from("/home/bob")),
        documents: None,
        desktop: Some(PathBuf::from("/home/bob/Desktop")),
    };
    assert_eq!(
        log_export_dir(&windows(), &folders),
        Some(PathBuf::from("/home/bob/Desktop"))
    );
    let no_desktop = MemoryFolders {
        desktop: None,
        ..folders.clone()
    };
    assert_eq!(
        log_export_dir(&windows(), &no_desktop),
        Some(PathBuf::from("/home/bob"))
    );
    let linux = PlatformEnv::for_tests(Os::Linux, None, &[], None);
    assert_eq!(
        log_export_dir(&linux, &folders),
        Some(PathBuf::from("/home/bob"))
    );
}

#[test]
fn arma3_documents_on_windows_is_below_documents() {
    let folders = MemoryFolders {
        documents: Some(PathBuf::from(r"C:\Users\Bob\Documents")),
        ..MemoryFolders::default()
    };
    assert_eq!(
        arma3_documents_dir(&windows(), &folders, &MemoryRegistry::default()),
        Some(PathBuf::from(r"C:\Users\Bob\Documents").join("Arma 3"))
    );
}
