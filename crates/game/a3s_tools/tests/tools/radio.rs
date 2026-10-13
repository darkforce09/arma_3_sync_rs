//! TFAR / ACRE 2 installers: TS3 detection, spec labels, last-wins validation, copy steps.

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};

use a3s_tools::radio::{
    Acre2Form, Acre2Plan, HostInfo, InstallStepError, IssueKind, RadioMod, TS3_EXE_32, TS3_EXE_64,
    TfarForm, TfarPlan, Ts3Arch, Ts3ProcessProbe, UNKNOWN, arma3_dir_from_exe, existing_or_empty,
    install_acre2, install_tfar, os_label, parse_ts3_version, plugin_label, stored_or_detected,
    ts3_arch, ts3_label, ts3_version, validate_acre2, validate_tfar,
};

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, path.file_name().unwrap().as_encoded_bytes()).unwrap();
}

fn text(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

fn s(path: &Path) -> String {
    path.to_str().unwrap().to_owned()
}

fn windows() -> HostInfo {
    HostInfo {
        is_windows: true,
        os_name: "Windows 11".to_owned(),
        is_64bit: true,
    }
}

struct Probe {
    running: bool,
    calls: Cell<usize>,
}

impl Probe {
    fn new(running: bool) -> Self {
        Self {
            running,
            calls: Cell::new(0),
        }
    }
}

impl Ts3ProcessProbe for Probe {
    fn is_ts3_running(&self) -> bool {
        self.calls.set(self.calls.get() + 1);
        self.running
    }
}

/// A fake TS3 installation, TFAR mod and Arma 3 folder in which every check passes.
struct Layout {
    _temp: tempfile::TempDir,
    arma3: PathBuf,
    ts3: PathBuf,
    tfar_plugins: PathBuf,
    tfar_userconfig: PathBuf,
    acre2_plugin: PathBuf,
}

impl Layout {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let layout = Self {
            arma3: root.join("Arma 3"),
            ts3: root.join("TeamSpeak 3 Client"),
            tfar_plugins: root.join("@task_force_radio/plugins"),
            tfar_userconfig: root.join("@task_force_radio/userconfig"),
            acre2_plugin: root.join("@ACRE2/plugin"),
            _temp: temp,
        };
        fs::create_dir_all(&layout.arma3).unwrap();
        touch(&layout.ts3.join(TS3_EXE_64));
        fs::create_dir_all(layout.ts3.join("plugins")).unwrap();
        touch(&layout.tfar_plugins.join("task_force_radio_win64.dll"));
        touch(
            &layout
                .tfar_plugins
                .join("task_force_radio/radio_settings_plugin.ini"),
        );
        touch(
            &layout
                .tfar_userconfig
                .join("task_force_radio/radio_settings.hpp"),
        );
        touch(&layout.acre2_plugin.join("acre2_win64.dll"));
        layout
    }

    fn tfar_form(&self) -> TfarForm {
        TfarForm {
            arma3_dir: s(&self.arma3),
            ts3_dir: s(&self.ts3),
            plugin_dir: s(&self.tfar_plugins),
            userconfig_dir: s(&self.tfar_userconfig),
        }
    }

    fn acre2_form(&self) -> Acre2Form {
        Acre2Form {
            ts3_dir: s(&self.ts3),
            plugin_dir: s(&self.acre2_plugin),
        }
    }
}

fn tfar_issue(form: &TfarForm, running: bool) -> Option<IssueKind> {
    validate_tfar(form, &windows(), &Probe::new(running))
        .err()
        .map(|issue| {
            assert_eq!(issue.radio, RadioMod::Tfar);
            issue.kind
        })
}

fn acre2_issue(form: &Acre2Form, running: bool) -> Option<IssueKind> {
    validate_acre2(form, &windows(), &Probe::new(running))
        .err()
        .map(|issue| {
            assert_eq!(issue.radio, RadioMod::Acre2);
            issue.kind
        })
}

fn plugin_missing(file: &str) -> IssueKind {
    IssueKind::PluginMissing {
        file: Some(file.to_owned()),
    }
}

#[test]
fn ts3_version_comes_from_the_first_complete_release_line() {
    assert_eq!(
        parse_ts3_version("x\n=== Client Release 3.1.10 ===\n").as_deref(),
        Some("3.1.10")
    );
    assert_eq!(
        parse_ts3_version("## Client Release 3.6.2 - 10 Jun 2024").as_deref(),
        Some("3.6.2")
    );
    assert_eq!(
        parse_ts3_version("Client Release 3.5.6\r\n=== CLIENT RELEASE 3.5.5 ===").as_deref(),
        Some("3.5.5")
    );
    assert_eq!(
        parse_ts3_version("===   Client Release   3.0.1  ").as_deref(),
        Some("3.0.1")
    );
    assert_eq!(parse_ts3_version("===\tClient\tRelease\t3.0.0"), None);
    assert_eq!(parse_ts3_version("nothing here\n"), None);

    let temp = tempfile::tempdir().unwrap();
    assert_eq!(ts3_version(temp.path()), None);
    fs::write(
        temp.path().join("changelog.txt"),
        "== Client Release 3.5.6 ==",
    )
    .unwrap();
    assert_eq!(ts3_version(temp.path()).as_deref(), Some("3.5.6"));
}

#[test]
fn ts3_architecture_prefers_64_bit() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    assert_eq!(ts3_arch(dir), None);
    touch(&dir.join(TS3_EXE_32));
    assert_eq!(ts3_arch(dir), Some(Ts3Arch::X86));
    touch(&dir.join(TS3_EXE_64));
    assert_eq!(ts3_arch(dir), Some(Ts3Arch::X64));
    assert_eq!(ts3_arch(Path::new("")), None);
}

#[test]
fn spec_labels_follow_the_detected_installation() {
    let temp = tempfile::tempdir().unwrap();
    let ts3_64 = temp.path().join("ts3_64");
    let ts3_32 = temp.path().join("ts3_32");
    let ts3_none = temp.path().join("ts3_none");
    touch(&ts3_64.join(TS3_EXE_64));
    touch(&ts3_32.join(TS3_EXE_32));
    fs::write(ts3_32.join("changelog.txt"), "=== Client Release 3.5.6 ===").unwrap();
    fs::create_dir_all(&ts3_none).unwrap();

    assert_eq!(ts3_label(""), UNKNOWN);
    assert_eq!(ts3_label(&s(&ts3_64)), format!("{UNKNOWN} - 64 bit"));
    assert_eq!(ts3_label(&s(&ts3_32)), "3.5.6 - 32 bit");
    assert_eq!(ts3_label(&s(&ts3_none)), UNKNOWN);

    assert_eq!(
        plugin_label(RadioMod::Tfar, &s(&ts3_64), "p"),
        "task_force_radio_win64.dll"
    );
    assert_eq!(
        plugin_label(RadioMod::Tfar, &s(&ts3_none), "p"),
        "task_force_radio_win32.dll"
    );
    assert_eq!(
        plugin_label(RadioMod::Acre2, &s(&ts3_32), "p"),
        "acre2_win32.dll"
    );
    assert_eq!(plugin_label(RadioMod::Acre2, &s(&ts3_none), "p"), UNKNOWN);
    assert_eq!(plugin_label(RadioMod::Tfar, "", "p"), UNKNOWN);
    assert_eq!(plugin_label(RadioMod::Acre2, &s(&ts3_64), ""), UNKNOWN);

    let mut host = windows();
    assert!(os_label(&host).ends_with("64 bit"));
    host.is_64bit = false;
    assert!(os_label(&host).ends_with("32 bit"));
}

#[test]
fn tfar_validation_accepts_a_complete_form() {
    let layout = Layout::new();
    let plan = validate_tfar(&layout.tfar_form(), &windows(), &Probe::new(false)).unwrap();
    assert_eq!(
        plan,
        TfarPlan {
            arma3_dir: layout.arma3.clone(),
            ts3_dir: layout.ts3.clone(),
            userconfig_dir: layout.tfar_userconfig.clone(),
            plugin_dir: layout.tfar_plugins.clone(),
        }
    );
}

#[test]
fn tfar_validation_on_another_system_never_probes() {
    let layout = Layout::new();
    let probe = Probe::new(true);
    let host = HostInfo {
        is_windows: false,
        ..windows()
    };
    let issue = validate_tfar(&layout.tfar_form(), &host, &probe).unwrap_err();
    assert_eq!(issue.kind, IssueKind::Unsupported);
    assert_eq!(probe.calls.get(), 0);
}

#[test]
fn tfar_validation_reports_the_last_failing_check() {
    let layout = Layout::new();
    let base = layout.tfar_form();
    let without_plugins_folder = || fs::remove_dir(layout.ts3.join("plugins")).unwrap();

    let only_arma3_empty = TfarForm {
        arma3_dir: String::new(),
        ..base.clone()
    };
    assert_eq!(
        tfar_issue(&only_arma3_empty, false),
        Some(IssueKind::Arma3DirMissing)
    );

    without_plugins_folder();
    assert_eq!(
        tfar_issue(&only_arma3_empty, false),
        Some(IssueKind::Ts3PluginsFolderMissing)
    );

    fs::remove_file(layout.tfar_plugins.join("task_force_radio_win64.dll")).unwrap();
    assert_eq!(
        tfar_issue(&base, false),
        Some(plugin_missing("task_force_radio_win64.dll"))
    );

    fs::remove_file(
        layout
            .tfar_userconfig
            .join("task_force_radio/radio_settings.hpp"),
    )
    .unwrap();
    assert_eq!(
        tfar_issue(&base, false),
        Some(IssueKind::UserconfigHppMissing)
    );

    assert_eq!(tfar_issue(&base, true), Some(IssueKind::Ts3Running));
    assert_eq!(
        tfar_issue(&TfarForm::default(), true),
        Some(IssueKind::Ts3Running)
    );
}

#[test]
fn tfar_validation_without_a_detectable_ts3_architecture() {
    let layout = Layout::new();
    fs::remove_file(layout.ts3.join(TS3_EXE_64)).unwrap();
    let form = layout.tfar_form();

    // The plugin label falls back to the 32-bit dll, whose absence wins over the unknown arch.
    assert_eq!(
        tfar_issue(&form, false),
        Some(plugin_missing("task_force_radio_win32.dll"))
    );
    touch(&layout.tfar_plugins.join("task_force_radio_win32.dll"));
    assert_eq!(tfar_issue(&form, false), Some(IssueKind::Ts3ArchUnknown));
}

#[test]
fn tfar_validation_with_an_empty_plugin_field_reports_an_unknown_plugin() {
    let layout = Layout::new();
    let form = TfarForm {
        plugin_dir: String::new(),
        ..layout.tfar_form()
    };
    assert_eq!(
        tfar_issue(&form, false),
        Some(IssueKind::PluginMissing { file: None })
    );
}

#[test]
fn acre2_validation_reports_the_last_failing_check() {
    let layout = Layout::new();
    let base = layout.acre2_form();

    let plan = validate_acre2(&base, &windows(), &Probe::new(false)).unwrap();
    assert_eq!(
        plan,
        Acre2Plan {
            ts3_dir: layout.ts3.clone(),
            plugin_file: layout.acre2_plugin.join("acre2_win64.dll"),
        }
    );

    let probe = Probe::new(true);
    let host = HostInfo {
        is_windows: false,
        ..windows()
    };
    let unsupported = validate_acre2(&base, &host, &probe).unwrap_err();
    assert_eq!(unsupported.kind, IssueKind::Unsupported);
    assert_ne!(
        unsupported.dialog_title(),
        validate_acre2(&base, &windows(), &probe)
            .unwrap_err()
            .dialog_title()
    );
    assert_eq!(probe.calls.get(), 1);

    let no_plugin_dir = Acre2Form {
        plugin_dir: String::new(),
        ..base.clone()
    };
    assert_eq!(
        acre2_issue(&no_plugin_dir, false),
        Some(IssueKind::PluginMissing { file: None })
    );

    fs::remove_file(layout.acre2_plugin.join("acre2_win64.dll")).unwrap();
    assert_eq!(
        acre2_issue(&base, false),
        Some(plugin_missing("acre2_win64.dll"))
    );

    fs::remove_dir(layout.ts3.join("plugins")).unwrap();
    fs::remove_file(layout.ts3.join(TS3_EXE_64)).unwrap();
    // No executable: arch unknown, then no plugins folder, then an unknown plugin.
    assert_eq!(
        acre2_issue(&base, false),
        Some(IssueKind::PluginMissing { file: None })
    );
    assert_eq!(acre2_issue(&base, true), Some(IssueKind::Ts3Running));
}

#[test]
fn tfar_install_copies_both_trees_and_merges_userconfig() {
    let layout = Layout::new();
    touch(&layout.arma3.join("userconfig/other/keep.hpp"));
    let plan = validate_tfar(&layout.tfar_form(), &windows(), &Probe::new(false)).unwrap();

    let report = install_tfar(&plan);

    assert!(report.userconfig.is_ok(), "{:?}", report.userconfig);
    assert!(report.plugin.is_ok(), "{:?}", report.plugin);
    assert!(report.success_messages().is_some());
    assert!(
        layout
            .arma3
            .join("userconfig/task_force_radio/radio_settings.hpp")
            .is_file()
    );
    assert!(layout.arma3.join("userconfig/other/keep.hpp").is_file());
    assert!(
        layout
            .ts3
            .join("plugins/task_force_radio/radio_settings_plugin.ini")
            .is_file()
    );
    assert!(
        layout
            .ts3
            .join("plugins/task_force_radio_win64.dll")
            .is_file()
    );
}

#[cfg(unix)]
#[test]
fn tfar_install_runs_the_plugin_step_after_a_failed_userconfig_step() {
    use std::os::unix::fs::PermissionsExt as _;

    let layout = Layout::new();
    let plan = validate_tfar(&layout.tfar_form(), &windows(), &Probe::new(false)).unwrap();
    fs::set_permissions(&layout.arma3, fs::Permissions::from_mode(0o555)).unwrap();
    if fs::write(layout.arma3.join("probe"), "").is_ok() {
        return; // Running as root: nothing is read-only.
    }

    let report = install_tfar(&plan);
    fs::set_permissions(&layout.arma3, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(matches!(
        &report.userconfig,
        Err(InstallStepError::NotWritable { dir }) if *dir == layout.arma3
    ));
    assert!(report.plugin.is_ok(), "{:?}", report.plugin);
    assert!(report.success_messages().is_none());
    assert!(
        layout
            .ts3
            .join("plugins/task_force_radio_win64.dll")
            .is_file()
    );
}

#[test]
fn acre2_install_copies_and_overwrites_the_plugin() {
    let layout = Layout::new();
    fs::write(layout.ts3.join("plugins/acre2_win64.dll"), "old").unwrap();
    let plan = validate_acre2(&layout.acre2_form(), &windows(), &Probe::new(false)).unwrap();

    let report = install_acre2(&plan);

    assert!(report.plugin.is_ok(), "{:?}", report.plugin);
    assert!(report.success_message().is_some());
    assert_eq!(
        text(&layout.ts3.join("plugins/acre2_win64.dll")),
        "acre2_win64.dll"
    );

    let missing = Acre2Plan {
        plugin_file: layout.acre2_plugin.join("gone.dll"),
        ..plan
    };
    let report = install_acre2(&missing);
    assert!(matches!(
        &report.plugin,
        Err(InstallStepError::CopyFailed { dir, .. }) if *dir == layout.ts3
    ));
    assert!(report.success_message().is_none());
}

#[test]
fn initial_field_values() {
    let detector_called = Cell::new(false);
    let stored = stored_or_detected(Some(""), || {
        detector_called.set(true);
        None
    });
    assert_eq!(stored.as_deref(), Some(""));
    assert!(!detector_called.get());
    assert_eq!(
        stored_or_detected(None, || Some("C:\\TS3".to_owned())).as_deref(),
        Some("C:\\TS3")
    );

    let temp = tempfile::tempdir().unwrap();
    let existing = s(temp.path());
    assert_eq!(existing_or_empty(Some(&existing)), existing);
    assert_eq!(existing_or_empty(Some(&s(&temp.path().join("gone")))), "");
    assert_eq!(existing_or_empty(None), "");

    assert_eq!(arma3_dir_from_exe(Some("arma3_x64.exe")), None);
    assert_eq!(arma3_dir_from_exe(None), None);
    let exe = temp.path().join("Arma 3/arma3_x64.exe");
    assert_eq!(
        arma3_dir_from_exe(Some(&s(&exe))),
        Some(s(&temp.path().join("Arma 3")))
    );
}

/// A dll stand-in whose version resource holds `VS_FIXEDFILEINFO` for version 1.2.3.4.
fn dll_with_version() -> Vec<u8> {
    let mut bytes = b"MZ\0\0 padding VS_VERSION_INFO".to_vec();
    bytes.extend_from_slice(&0xFEEF_04BD_u32.to_le_bytes());
    bytes.extend_from_slice(&0x0001_0000_u32.to_le_bytes());
    bytes.extend_from_slice(&0x0001_0002_u32.to_le_bytes());
    bytes.extend_from_slice(&0x0003_0004_u32.to_le_bytes());
    bytes.extend_from_slice(&[0; 36]);
    bytes
}

#[test]
fn file_version_reads_the_fixed_file_info() {
    assert_eq!(
        a3s_tools::radio::file_version(&dll_with_version()).as_deref(),
        Some("1.2.3.4")
    );
    assert_eq!(a3s_tools::radio::file_version(b"MZ no resource"), None);
    let truncated = &dll_with_version()[..30];
    assert_eq!(a3s_tools::radio::file_version(truncated), None);
}
