//! Desktop shortcuts: Exec quoting, `.desktop` and `.lnk` writers.

use std::fs;
use std::path::{Path, PathBuf};

use a3s_foundation::paths::{Os, PlatformEnv};
use a3s_platform::Error;
use a3s_platform::folders::MemoryFolders;
use a3s_platform::shortcut::{
    ShortcutSpec, create_desktop_shortcut, desktop_escape_value, desktop_exec,
    render_desktop_entry, validate_shortcut_name, write_lnk,
};

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|&arg| arg.to_owned()).collect()
}

fn spec(name: &str, target: &Path, args: &[&str]) -> ShortcutSpec {
    ShortcutSpec {
        name: name.to_owned(),
        target: target.to_path_buf(),
        args: strings(args),
        working_dir: None,
        icon: None,
        comment: None,
    }
}

#[test]
fn exec_quoting_follows_the_desktop_entry_rules() {
    let exec = desktop_exec(
        Path::new("/usr/bin/steam"),
        &strings(&[
            "-applaunch",
            "107410",
            "-mod=@a;@b c",
            "100%",
            "$HOME",
            "a\\b",
        ]),
    )
    .unwrap();
    assert_eq!(
        desktop_escape_value(&exec),
        r#"/usr/bin/steam -applaunch 107410 "-mod=@a;@b c" 100%% "\\$HOME" "a\\\\b""#
    );
    assert_eq!(
        desktop_exec(Path::new("/my games/run"), &strings(&["", "it's"])).unwrap(),
        r#""/my games/run" "" "it's""#
    );
    assert!(matches!(
        desktop_exec(Path::new("/bin/x"), &strings(&["a\nb"])),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        desktop_exec(Path::new("/bin/x"), &strings(&["a\0b"])),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn escape_value_handles_controls_and_a_leading_space() {
    assert_eq!(desktop_escape_value(" a\\b\nc\td\re "), r"\sa\\b\nc\td\re ");
}

#[test]
fn desktop_entry_lists_optional_keys_only_when_set() {
    let mut spec = spec(
        "Default",
        Path::new("/opt/arma3/arma3.x86_64"),
        &["-noSplash"],
    );
    assert_eq!(
        render_desktop_entry(&spec).unwrap(),
        "[Desktop Entry]\nType=Application\nVersion=1.0\nName=Default\n\
         Exec=/opt/arma3/arma3.x86_64 -noSplash\nTerminal=false\n"
    );
    spec.working_dir = Some(PathBuf::from("/opt/arma3"));
    spec.icon = Some(PathBuf::from("/opt/arma3/icon.png"));
    spec.comment = Some("Start Arma 3\nwith mods".to_owned());
    assert_eq!(
        render_desktop_entry(&spec).unwrap(),
        "[Desktop Entry]\nType=Application\nVersion=1.0\nName=Default\n\
         Exec=/opt/arma3/arma3.x86_64 -noSplash\nPath=/opt/arma3\nIcon=/opt/arma3/icon.png\n\
         Comment=Start Arma 3\\nwith mods\nTerminal=false\n"
    );
}

#[test]
fn shortcut_names_must_be_file_names() {
    validate_shortcut_name("My Profile").unwrap();
    validate_shortcut_name("Équipe 1").unwrap();
    for name in [
        "a/b",
        "a\\b",
        "a:b",
        "con?",
        "",
        "  ",
        ".",
        "..",
        "tab\there",
    ] {
        assert!(
            matches!(
                validate_shortcut_name(name),
                Err(Error::InvalidShortcutName(_))
            ),
            "{name:?}"
        );
    }
}

#[test]
fn invalid_lnk_specs_are_rejected_before_writing() {
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("x.lnk");
    let target = temp.path().join("x/arma3_x64.exe");

    assert!(matches!(
        write_lnk(&spec("x", &target, &[]), &out),
        Err(Error::ShortcutTargetMissing(_))
    ));
    assert!(matches!(
        write_lnk(&spec("x", Path::new("x/arma3_x64.exe"), &[]), &out),
        Err(Error::InvalidInput(_))
    ));

    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"MZ").unwrap();
    let long_arg = "a".repeat(32_768);
    assert!(matches!(
        write_lnk(&spec("x", &target, &[long_arg.as_str()]), &out),
        Err(Error::InvalidInput(_))
    ));
    assert!(!out.exists());
}

#[test]
fn desktop_shortcuts_per_os() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("game/arma3_x64.exe");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"MZ").unwrap();
    let folders = MemoryFolders {
        desktop: Some(temp.path().to_path_buf()),
        ..MemoryFolders::default()
    };
    let env = |os| PlatformEnv::for_tests(os, None, &[], None);
    let spec = spec("Default", &target, &["-noSplash"]);

    let path = create_desktop_shortcut(&env(Os::Linux), &folders, &spec).unwrap();
    assert_eq!(path, temp.path().join("Default.desktop"));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        render_desktop_entry(&spec).unwrap()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    assert!(matches!(
        create_desktop_shortcut(&env(Os::MacOs), &folders, &spec),
        Err(Error::Unsupported)
    ));
    assert!(matches!(
        create_desktop_shortcut(&env(Os::Linux), &MemoryFolders::default(), &spec),
        Err(Error::NoDesktopDir)
    ));
    let bad_name = ShortcutSpec {
        name: "a/b".to_owned(),
        ..spec.clone()
    };
    assert!(matches!(
        create_desktop_shortcut(&env(Os::Linux), &folders, &bad_name),
        Err(Error::InvalidShortcutName(_))
    ));

    // The `.lnk` writer exists only on Windows.
    let lnk = create_desktop_shortcut(&env(Os::Windows), &folders, &spec);
    if cfg!(windows) {
        assert_eq!(lnk.unwrap(), temp.path().join("Default.lnk"));
    } else {
        assert!(matches!(lnk, Err(Error::Unsupported)));
    }
}

#[cfg(windows)]
#[test]
fn lnk_file_has_the_shell_link_layout() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("x/arma3_x64.exe");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"MZ").unwrap();
    let out = temp.path().join("Équipe 1.lnk");
    write_lnk(
        &spec("Équipe 1", &target, &["-mod=@CBA_A3;@ace", "-noSplash"]),
        &out,
    )
    .unwrap();

    let bytes = fs::read(&out).unwrap();
    // HeaderSize 0x4C and the ShellLink CLSID 00021401-0000-0000-C000-000000000046.
    assert_eq!(
        bytes[..20],
        [
            0x4C, 0x00, 0x00, 0x00, 0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x46
        ]
    );
    const HAS_ARGUMENTS: u32 = 0x20;
    const IS_UNICODE: u32 = 0x80;
    let flags = u32::from_le_bytes(bytes[0x14..0x18].try_into().unwrap());
    assert_eq!(
        flags & (HAS_ARGUMENTS | IS_UNICODE),
        HAS_ARGUMENTS | IS_UNICODE
    );
    let arguments: Vec<u8> = "-mod=@CBA_A3;@ace -noSplash"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    assert!(
        bytes
            .windows(arguments.len())
            .any(|window| window == arguments)
    );
}
