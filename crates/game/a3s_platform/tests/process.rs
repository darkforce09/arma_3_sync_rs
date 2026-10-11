//! Process probes: `tasklist` CSV parsing, a fake `/proc`, and failure handling.

use std::fs;

use a3s_platform::process::{
    FixedProcessProbe, ProcFsProbe, ProcessSnapshot, is_arma3_running, is_running,
    is_steam_running, is_ts3_running, parse_tasklist_csv,
};

#[test]
fn tasklist_csv_gives_whole_image_names() {
    let stdout = "\"System Idle Process\",\"0\",\"Services\",\"0\",\"8 K\"\r\n\
                  \r\n\
                  \"ts3client_win64.exe\",\"4242\",\"Console\",\"1\",\"120,000 K\"\r\n\
                  \"say \"\"hi\"\".exe\",\"7\",\"Console\",\"1\",\"1 K\"\r\n";
    assert_eq!(
        parse_tasklist_csv(stdout),
        [
            "System Idle Process",
            "ts3client_win64.exe",
            "say \"hi\".exe"
        ]
    );
}

#[test]
fn snapshots_match_whole_names_case_insensitively() {
    let snapshot = |names: &[&str]| {
        FixedProcessProbe(Some(names.iter().map(|&name| name.to_owned()).collect()))
    };
    assert!(is_ts3_running(&snapshot(&["ts3client_win64.exe"])));
    assert!(!is_arma3_running(&snapshot(&[
        "xarma3.exe",
        "arma3.exe.bak"
    ])));
    assert!(is_arma3_running(&snapshot(&["ARMA3.EXE"])));
    assert!(is_steam_running(&snapshot(&["Steam.exe"])));
    assert!(
        ProcessSnapshot::from_names(["Arma3server_x64.EXE".to_owned()])
            .is_any_running(&["arma3server.exe", "arma3server_x64.exe"])
    );
}

#[test]
fn probe_failures_mean_not_running() {
    let failing = FixedProcessProbe(None);
    assert!(!is_arma3_running(&failing));
    assert!(!is_running(&failing, "anything.exe"));
}

#[test]
fn proc_scan_sees_wine_and_truncated_names() {
    let proc_root = tempfile::tempdir().unwrap();
    let pid = |pid: &str| {
        let dir = proc_root.path().join(pid);
        fs::create_dir_all(&dir).unwrap();
        dir
    };

    let arma = pid("100");
    fs::write(arma.join("comm"), "arma3_x64.exe\n").unwrap();
    fs::write(
        arma.join("cmdline"),
        b"Z:\\home\\u\\.steam\\steamapps\\common\\Arma 3\\arma3_x64.exe\0-mod=@ace\0",
    )
    .unwrap();

    // `comm` cut at 15 bytes; `cmdline` unreadable (a directory here).
    let ts3 = pid("101");
    fs::write(ts3.join("comm"), "ts3client_win64\n").unwrap();
    fs::create_dir(ts3.join("cmdline")).unwrap();

    fs::create_dir_all(proc_root.path().join("self")).unwrap();
    fs::write(proc_root.path().join("self/comm"), "steam.exe\n").unwrap();
    pid("102");

    let probe = ProcFsProbe::new(proc_root.path());
    assert!(is_arma3_running(&probe));
    assert!(is_ts3_running(&probe));
    assert!(!is_steam_running(&probe));
    // A 15-byte `comm` only matches names that start with it.
    assert!(!is_running(&probe, "ts3client_win32.exe"));
}

#[test]
fn a_missing_proc_root_means_not_running() {
    let temp = tempfile::tempdir().unwrap();
    assert!(!is_arma3_running(&ProcFsProbe::new(
        temp.path().join("absent")
    )));
}
