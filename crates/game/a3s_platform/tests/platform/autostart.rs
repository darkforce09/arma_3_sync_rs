//! Autostart entries: the Windows `Run` value, XDG autostart files and macOS launch agents.

use std::fs;
use std::path::PathBuf;

use a3s_foundation::paths::{Os, PlatformEnv};
use a3s_platform::autostart::{
    AutostartBackend, AutostartEntry, AutostartStatus, autostart_backend, launch_agent_plist,
    windows_run_command, xdg_autostart_file,
};
use a3s_platform::registry::{Hive, MemoryRegistry};

fn entry(exe: &str, args: &[&str]) -> AutostartEntry {
    AutostartEntry {
        exe: PathBuf::from(exe),
        args: args.iter().map(|&arg| arg.to_owned()).collect(),
    }
}

#[test]
fn run_command_quotes_the_exe_and_each_argument() {
    assert_eq!(
        windows_run_command(&entry(r"C:\Program Files\A3S\arma3sync.exe", &["-run"])),
        r#""C:\Program Files\A3S\arma3sync.exe" -run"#
    );
    assert_eq!(
        windows_run_command(&entry(
            r"C:\a.exe",
            &["a b", "", "x\\\"y", "tail\\", r"c:\dir\"]
        )),
        r#""C:\a.exe" "a b" "" "x\\\"y" tail\ c:\dir\"#
    );
    assert_eq!(
        windows_run_command(&entry(r"C:\a.exe", &["two words\\"])),
        r#""C:\a.exe" "two words\\""#
    );
}

#[test]
fn windows_backend_writes_only_the_user_run_value() {
    let registry = MemoryRegistry::default();
    let env = PlatformEnv::for_tests(Os::Windows, None, &[], None);
    let backend = autostart_backend(&env, &registry);
    assert!(matches!(backend, AutostartBackend::WindowsRun(_)));
    assert_eq!(backend.status().unwrap(), AutostartStatus::Disabled);

    let entry = entry(r"C:\A3S\arma3sync.exe", &["-run"]);
    backend.enable(&entry).unwrap();
    assert_eq!(
        registry.keys(),
        [(
            Hive::CurrentUser,
            r"software\microsoft\windows\currentversion\run".to_owned(),
            "arma3sync-rs".to_owned()
        )]
    );
    assert_eq!(
        backend.status().unwrap(),
        AutostartStatus::Enabled {
            command: windows_run_command(&entry)
        }
    );

    backend.disable().unwrap();
    assert_eq!(backend.status().unwrap(), AutostartStatus::Disabled);
    assert!(registry.keys().is_empty());
    backend.disable().unwrap();
}

#[test]
fn xdg_entry_lives_in_the_config_autostart_folder() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("cfg");
    let env = PlatformEnv::for_tests(
        Os::Linux,
        Some(temp.path().join("home")),
        &[("XDG_CONFIG_HOME", config.to_str().unwrap())],
        None,
    );
    let registry = MemoryRegistry::default();
    let backend = autostart_backend(&env, &registry);
    let entry = entry("/opt/a3s/arma3sync", &["-run"]);
    let expected = "[Desktop Entry]\nType=Application\nVersion=1.0\nName=ArmA3Sync\n\
                    Exec=/opt/a3s/arma3sync -run\nTerminal=false\nX-GNOME-Autostart-enabled=true\n";
    assert_eq!(xdg_autostart_file(&entry).unwrap(), expected);

    backend.enable(&entry).unwrap();
    let file = config.join("autostart/arma3sync-rs.desktop");
    assert_eq!(fs::read_to_string(&file).unwrap(), expected);
    assert_eq!(
        backend.status().unwrap(),
        AutostartStatus::Enabled {
            command: "/opt/a3s/arma3sync -run".to_owned()
        }
    );

    fs::write(&file, format!("{expected}Hidden=true\n")).unwrap();
    assert_eq!(backend.status().unwrap(), AutostartStatus::Disabled);

    backend.disable().unwrap();
    assert!(!file.exists());
    backend.disable().unwrap();
    assert!(registry.keys().is_empty());
}

#[test]
fn relative_xdg_config_home_is_ignored() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().to_path_buf();
    let env = PlatformEnv::for_tests(
        Os::Linux,
        Some(home.clone()),
        &[("XDG_CONFIG_HOME", "relative/cfg")],
        None,
    );
    let registry = MemoryRegistry::default();
    let backend = autostart_backend(&env, &registry);
    backend.enable(&entry("/opt/a3s/arma3sync", &[])).unwrap();
    assert!(
        home.join(".config/autostart/arma3sync-rs.desktop")
            .is_file()
    );
}

#[test]
fn appimage_replaces_the_current_executable_on_linux() {
    let env = PlatformEnv::for_tests(
        Os::Linux,
        None,
        &[("APPIMAGE", "/home/u/A3S.AppImage")],
        None,
    );
    let entry = AutostartEntry::for_current_process(&env, vec!["-run".to_owned()]).unwrap();
    assert_eq!(entry.exe, PathBuf::from("/home/u/A3S.AppImage"));
    assert_eq!(entry.args, ["-run"]);

    let windows = PlatformEnv::for_tests(
        Os::Windows,
        None,
        &[("APPIMAGE", "/home/u/A3S.AppImage")],
        None,
    );
    let entry = AutostartEntry::for_current_process(&windows, Vec::new()).unwrap();
    assert_eq!(entry.exe, std::env::current_exe().unwrap());
}

#[test]
fn launch_agent_plist_escapes_xml_and_round_trips() {
    let entry = entry(
        "/Applications/A & B.app/Contents/MacOS/arma3sync",
        &["-run"],
    );
    let expected = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
\t<key>Label</key>\n\
\t<string>com.github.vpzbrig21.arma3sync-rs</string>\n\
\t<key>ProgramArguments</key>\n\
\t<array>\n\
\t\t<string>/Applications/A &amp; B.app/Contents/MacOS/arma3sync</string>\n\
\t\t<string>-run</string>\n\
\t</array>\n\
\t<key>RunAtLoad</key>\n\
\t<true/>\n\
</dict>\n\
</plist>\n";
    assert_eq!(launch_agent_plist(&entry), expected);

    let temp = tempfile::tempdir().unwrap();
    let env = PlatformEnv::for_tests(Os::MacOs, Some(temp.path().to_path_buf()), &[], None);
    let registry = MemoryRegistry::default();
    let backend = autostart_backend(&env, &registry);
    assert_eq!(backend.status().unwrap(), AutostartStatus::Disabled);
    backend.enable(&entry).unwrap();
    let file = temp
        .path()
        .join("Library/LaunchAgents/com.github.vpzbrig21.arma3sync-rs.plist");
    assert_eq!(fs::read_to_string(&file).unwrap(), expected);
    assert_eq!(
        backend.status().unwrap(),
        AutostartStatus::Enabled {
            command: "/Applications/A & B.app/Contents/MacOS/arma3sync -run".to_owned()
        }
    );
    backend.disable().unwrap();
    assert!(!file.exists());
    assert_eq!(backend.status().unwrap(), AutostartStatus::Disabled);
}

#[test]
fn other_oses_have_no_autostart() {
    let env = PlatformEnv::for_tests(Os::Other, None, &[], None);
    let registry = MemoryRegistry::default();
    let backend = autostart_backend(&env, &registry);
    assert!(backend.enable(&entry("/x", &[])).is_err());
    assert!(backend.disable().is_err());
    assert!(backend.status().is_err());
}
