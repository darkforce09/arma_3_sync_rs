//! Executable location check, kinds, BattlEye names and the BattlEye path toggle.

use std::path::{Path, PathBuf};

use a3s_foundation::paths::Os;
use a3s_launch::LaunchError;
use a3s_launch::executable::{
    ExecutableKind, battleye_toggle_path, check_arma3_executable_location, executable_kind,
    is_battleye, is_elevatable,
};
use a3s_types::local::Profile;

fn profile_with_exe(path: Option<&str>) -> Profile {
    let mut profile = Profile::new("p");
    profile.launcher_options.arma3_exe_path = path.map(str::to_owned);
    profile
}

#[test]
fn executable_location() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.exe");
    let missing = missing.to_str().unwrap();

    assert!(matches!(
        check_arma3_executable_location(None),
        Err(LaunchError::ExecutableLocation)
    ));
    for path in [None, Some(""), Some(missing)] {
        assert!(matches!(
            check_arma3_executable_location(Some(&profile_with_exe(path))),
            Err(LaunchError::ExecutableLocation)
        ));
    }

    let file = dir.path().join("arma3_x64.exe");
    std::fs::write(&file, b"").unwrap();
    for existing in [file.as_path(), dir.path()] {
        let profile = profile_with_exe(existing.to_str());
        assert_eq!(
            check_arma3_executable_location(Some(&profile)).unwrap(),
            existing
        );
    }
}

#[test]
fn kinds_by_extension() {
    for (name, kind) in [
        ("arma3_x64.exe", ExecutableKind::Exe),
        ("ARMA3.EXE", ExecutableKind::Exe),
        ("run.bat", ExecutableKind::Bat),
        ("RUN.BAT", ExecutableKind::Bat),
        ("a.sh", ExecutableKind::Sh),
        ("a.exe.sh", ExecutableKind::Sh),
        ("teamspeak3", ExecutableKind::Other),
        ("x.cmd", ExecutableKind::Other),
    ] {
        assert_eq!(executable_kind(name), kind, "{name}");
    }
}

#[test]
fn battleye_and_elevatable_names() {
    assert!(is_battleye("arma3battleye.exe"));
    assert!(is_battleye("Arma3BattlEye.EXE"));
    assert!(!is_battleye("arma3.exe"));
    for name in ["ARMA3.exe", "arma3_X64.exe", "arma3battleye.EXE"] {
        assert!(is_elevatable(name), "{name}");
    }
    assert!(!is_elevatable("arma3server_x64.exe"));
}

#[test]
fn battleye_toggle() {
    let dir = Path::new("/games/Arma 3");
    let be = dir.join("arma3battleye.exe");
    assert_eq!(
        battleye_toggle_path(&be, Os::Windows, true),
        Some(dir.join("arma3_x64.exe"))
    );
    assert_eq!(
        battleye_toggle_path(&be, Os::Windows, false),
        Some(dir.join("arma3.exe"))
    );
    assert_eq!(
        battleye_toggle_path(&be, Os::Linux, true),
        Some(dir.join("arma3.exe"))
    );
    for game in ["arma3.exe", "arma3_x64.exe"] {
        assert_eq!(
            battleye_toggle_path(&dir.join(game), Os::Windows, true),
            Some(be.clone())
        );
    }
    assert_eq!(
        battleye_toggle_path(&dir.join("Arma3.exe"), Os::Windows, true),
        None
    );
    assert_eq!(
        battleye_toggle_path(&PathBuf::from("/"), Os::Windows, true),
        None
    );
}
