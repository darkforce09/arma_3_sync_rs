//! Command planning for every OS, UAC elevation through PowerShell, and Steam.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_addons::AddonRegistry;
use a3s_foundation::command_line::encode_powershell_command;
use a3s_foundation::paths::Os;
use a3s_launch::command::{
    CommandSpec, LaunchMethod, LinuxLaunchMode, arma3_method, command_line, log_line, plan_arma3,
    plan_external,
};
use a3s_launch::elevation::{elevated_command, elevation_script};
use a3s_launch::steam::{FLATPAK_STEAM_APP, SteamCommand};
use a3s_launch::{Arma3LaunchRequest, LaunchError, launch_arma3, plan_launch};
use a3s_platform::process::FixedProcessProbe;
use a3s_types::local::{Configuration, GameStart, LauncherOptions, Profile};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn os_strings(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn admin() -> LauncherOptions {
    LauncherOptions {
        launch_arma3_as_administrator: true,
        ..Default::default()
    }
}

fn direct(os: Os, exe: &str, params: &[String]) -> Result<CommandSpec, LaunchError> {
    plan_arma3(os, Path::new(exe), params, LaunchMethod::Direct, None).map(|plan| plan.command)
}

fn decode_script(command: &CommandSpec) -> String {
    let encoded = command.args.last().unwrap().to_str().unwrap();
    let bytes = STANDARD.decode(encoded).unwrap();
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16(&units).unwrap()
}

#[test]
fn elevation_script_matches_java() {
    let script = elevation_script(r"C:\A\arma3.exe", Some(r"C:\A"), &strings(&["-window"]));
    let expected = r"$ErrorActionPreference='Stop'; try { $arguments=@('-window'); $process=Start-Process -FilePath 'C:\A\arma3.exe' -WorkingDirectory 'C:\A' -ArgumentList $arguments -Verb RunAs -Wait -PassThru; exit $process.ExitCode } catch { exit 1 }";
    assert_eq!(script, expected);
    assert_eq!(script.chars().count(), 232);

    let encoded = encode_powershell_command(&script);
    assert!(encoded.starts_with("JABFAHIAcgBvAHIA"));
    let utf16le: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    assert_eq!(STANDARD.decode(&encoded).unwrap(), utf16le);

    let command = elevated_command(r"C:\A\arma3.exe", Some(r"C:\A"), &strings(&["-window"]));
    assert_eq!(command.program, "powershell.exe");
    let mut args = os_strings(&[
        "-NoProfile",
        "-NonInteractive",
        "-WindowStyle",
        "Hidden",
        "-EncodedCommand",
    ]);
    args.push(encoded.into());
    assert_eq!(command.args, args);
    assert!(command.hide_window);
    assert!(command.discard_output);
}

#[test]
fn elevation_script_arguments() {
    let script = elevation_script(r"C:\A\arma3.exe", None, &[]);
    assert!(script.contains("$arguments=@();"));
    assert!(!script.contains("-ArgumentList"));
    assert!(!script.contains("-WorkingDirectory"));

    let script = elevation_script(r"C:\A\arma3.exe", None, &strings(&[r"-mod=C:\My Mods\@a;"]));
    assert!(script.contains(r#"$arguments=@('"-mod=C:\My Mods\@a;"');"#));
}

#[test]
fn windows_plans() {
    let params = strings(&[" -window ", "-mod=a;"]);
    let exe = "/games/arma3_x64.exe";
    let command = direct(Os::Windows, exe, &params).unwrap();
    assert_eq!(command.program, exe);
    assert_eq!(command.args, os_strings(&[" -window ", "-mod=a;"]));
    assert_eq!(command.current_dir, Some(PathBuf::from("/games")));
    assert!(!command.discard_output);

    let command = direct(Os::Windows, "/games/arma3battleye.exe", &params).unwrap();
    assert_eq!(
        command.args,
        os_strings(&["2", "1", " -window ", "-mod=a;"])
    );

    let command = direct(Os::Windows, "/games/run.bat", &params).unwrap();
    assert_eq!(command.program, "cmd.exe");
    assert_eq!(
        command.args,
        os_strings(&["/C", "start", "", "/games/run.bat", "-window", "-mod=a;"])
    );

    assert!(matches!(
        direct(Os::Windows, "/games/x.sh", &params),
        Err(LaunchError::InvalidExecutable { name }) if name == "x.sh"
    ));
}

#[test]
fn windows_methods() {
    let none = LauncherOptions::default();
    let exe = Path::new("/games/arma3_x64.exe");
    let mode = LinuxLaunchMode::Auto;
    assert_eq!(
        arma3_method(Os::Windows, exe, &none, mode).unwrap(),
        LaunchMethod::Direct
    );
    assert_eq!(
        arma3_method(Os::Windows, exe, &admin(), mode).unwrap(),
        LaunchMethod::Elevated
    );
    let server = Path::new("/games/arma3server_x64.exe");
    assert_eq!(
        arma3_method(Os::Windows, server, &admin(), mode).unwrap(),
        LaunchMethod::Direct
    );
    let batch = Path::new("/games/arma3.bat");
    assert_eq!(
        arma3_method(Os::Windows, batch, &admin(), mode).unwrap(),
        LaunchMethod::Direct
    );
    assert!(matches!(
        arma3_method(Os::Windows, Path::new("/games/x.sh"), &none, mode),
        Err(LaunchError::InvalidExecutable { .. })
    ));
}

#[test]
fn elevated_plan_logs_the_plain_command() {
    let exe = Path::new("/games/arma3battleye.exe");
    let params = strings(&["-window"]);
    let plan = plan_arma3(Os::Windows, exe, &params, LaunchMethod::Elevated, None).unwrap();
    assert_eq!(plan.method, LaunchMethod::Elevated);
    assert_eq!(
        plan.shown_command,
        strings(&["/games/arma3battleye.exe", "2", "1", "-window"])
    );
    assert_eq!(plan.command.program, "powershell.exe");
    let script = decode_script(&plan.command);
    assert!(script.contains("$arguments=@('2','1','-window');"));
    assert!(script.contains("-FilePath '/games/arma3battleye.exe'"));
}

#[test]
fn linux_and_other_plans() {
    let params = strings(&[" -window ", "-mod=a b;"]);
    let command = direct(Os::Linux, "/srv/game.sh", &params).unwrap();
    assert_eq!(command.program, "/bin/bash");
    assert_eq!(
        command.args,
        os_strings(&["/srv/game.sh", "-window", "-mod=a b;"])
    );
    assert_eq!(command.current_dir, Some(PathBuf::from("/srv")));

    let command = direct(Os::Linux, "/srv/arma3server", &params).unwrap();
    assert_eq!(command.program, "/srv/arma3server");
    assert_eq!(command.args, os_strings(&["-window", "-mod=a b;"]));

    let command = direct(Os::Linux, "arma3server", &params).unwrap();
    assert_eq!(command.current_dir, None);

    let exe = Path::new("/games/arma3_x64.exe");
    let options = LauncherOptions::default();
    assert_eq!(
        arma3_method(Os::Linux, exe, &options, LinuxLaunchMode::Direct).unwrap(),
        LaunchMethod::Direct
    );
    assert_eq!(
        arma3_method(Os::Linux, exe, &options, LinuxLaunchMode::Auto).unwrap(),
        LaunchMethod::Steam
    );
    let script = Path::new("/srv/game.sh");
    assert_eq!(
        arma3_method(Os::Linux, script, &options, LinuxLaunchMode::Auto).unwrap(),
        LaunchMethod::Direct
    );
    assert_eq!(
        arma3_method(Os::Linux, script, &options, LinuxLaunchMode::Steam).unwrap(),
        LaunchMethod::Steam
    );

    for os in [Os::MacOs, Os::OtherUnix, Os::Other] {
        assert!(matches!(
            arma3_method(os, exe, &options, LinuxLaunchMode::Direct),
            Err(LaunchError::InvalidExecutable { .. })
        ));
        assert!(matches!(
            direct(os, "/srv/game.sh", &params),
            Err(LaunchError::InvalidExecutable { .. })
        ));
    }
}

#[test]
fn passwords_are_hidden_in_the_log() {
    let elements = strings(&["a", "-password=x", "-PASSWORD=y"]);
    assert_eq!(command_line(&elements), "a -password=***** -PASSWORD=*****");
    let line = log_line(&elements);
    assert!(line.ends_with("a -password=***** -PASSWORD=***** "));
}

#[test]
fn external_plans() {
    let params = strings(&[" -a ", "b"]);
    for exe in ["/t/app.exe", "/t/app.bat"] {
        assert_eq!(
            plan_external(Os::Linux, Path::new(exe), &params).unwrap(),
            None
        );
    }
    assert_eq!(
        plan_external(Os::Windows, Path::new("/t/app.sh"), &params).unwrap(),
        None
    );

    let command = plan_external(Os::Linux, Path::new("/t/app.sh"), &params)
        .unwrap()
        .unwrap();
    assert_eq!(command.program, "/bin/bash");
    assert_eq!(command.args, os_strings(&["/t/app.sh", "-a", "b"]));
    assert_eq!(command.current_dir, Some(PathBuf::from("/t")));

    let command = plan_external(Os::Windows, Path::new("/t/app.exe"), &params)
        .unwrap()
        .unwrap();
    assert_eq!(command.program, "/t/app.exe");
    assert_eq!(command.args, os_strings(&["-a", "b"]));

    let command = plan_external(Os::Windows, Path::new("/t/app.bat"), &params)
        .unwrap()
        .unwrap();
    assert_eq!(
        command.args,
        os_strings(&["/C", "start", "", "/t/app.bat", "-a", "b"])
    );

    assert!(matches!(
        plan_external(Os::Linux, Path::new("/t/tool"), &params),
        Err(LaunchError::InvalidExecutable { name }) if name == "tool"
    ));
}

#[test]
fn steam_plan() {
    let steam = SteamCommand {
        prefix: os_strings(&["steam"]),
    };
    let params = strings(&["-window", r" -mod=Z:\mods\@ace; "]);
    let exe = Path::new("/games/arma3battleye.exe");
    let plan = plan_arma3(Os::Linux, exe, &params, LaunchMethod::Steam, Some(&steam)).unwrap();
    assert_eq!(plan.command.program, "steam");
    assert_eq!(
        plan.command.args,
        os_strings(&[
            "-applaunch",
            "107410",
            "-noLauncher",
            "-window",
            r"-mod=Z:\mods\@ace;"
        ])
    );
    assert_eq!(plan.command.current_dir, None);

    let flatpak = SteamCommand {
        prefix: os_strings(&["flatpak", "run", FLATPAK_STEAM_APP]),
    };
    let plan = plan_arma3(Os::Linux, exe, &[], LaunchMethod::Steam, Some(&flatpak)).unwrap();
    assert_eq!(plan.command.program, "flatpak");
    assert_eq!(
        plan.command.args,
        os_strings(&[
            "run",
            FLATPAK_STEAM_APP,
            "-applaunch",
            "107410",
            "-noLauncher"
        ])
    );

    assert!(matches!(
        plan_arma3(Os::Linux, exe, &[], LaunchMethod::Steam, None),
        Err(LaunchError::SteamNotFound)
    ));
}

#[test]
fn steam_not_found_fails_before_launching() {
    let mut profile = Profile::new("p");
    profile.launcher_options.arma3_exe_path = Some("/games/Arma 3/arma3_x64.exe".to_owned());
    let configuration = Configuration::default();
    let registry = AddonRegistry::new();
    let request = Arma3LaunchRequest {
        os: Os::Linux,
        profile: Some(&profile),
        configuration: &configuration,
        registry: &registry,
        steam: None,
        arma3_documents_dir: None,
        restart_guard: Default::default(),
        steam_watch: Default::default(),
    };
    let events: a3s_launch::EventSink = Arc::new(|event| panic!("unexpected event {event:?}"));
    let probe = Arc::new(FixedProcessProbe::default());
    assert!(matches!(
        launch_arma3(&request, probe.clone(), events.clone()),
        Err(LaunchError::SteamNotFound)
    ));

    let no_profile = Arma3LaunchRequest {
        profile: None,
        ..request.clone()
    };
    assert!(
        launch_arma3(&no_profile, probe.clone(), events.clone())
            .unwrap()
            .is_none()
    );

    let mut unset = profile.clone();
    unset.launcher_options.arma3_exe_path = None;
    let unset_request = Arma3LaunchRequest {
        profile: Some(&unset),
        ..request
    };
    assert!(matches!(
        launch_arma3(&unset_request, probe, events),
        Err(LaunchError::ExecutableLocation)
    ));
}

#[cfg(unix)]
fn executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(path, b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[test]
fn finds_steam() {
    use a3s_foundation::paths::PlatformEnv;
    use a3s_launch::steam::find_steam_command;

    let home = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    let bin_text = bin.path().to_str().unwrap();
    let env = |path: &str| {
        PlatformEnv::for_tests(
            Os::Linux,
            Some(home.path().to_path_buf()),
            &[("PATH", path)],
            None,
        )
    };

    // Nothing on PATH, no Flatpak data.
    assert_eq!(find_steam_command(&env("")), None);

    // `flatpak` alone is not enough without the Flatpak Steam data folder.
    executable(&bin.path().join("flatpak"));
    assert_eq!(find_steam_command(&env(bin_text)), None);

    std::fs::create_dir_all(home.path().join(".var/app").join(FLATPAK_STEAM_APP)).unwrap();
    assert_eq!(
        find_steam_command(&env(bin_text)),
        Some(SteamCommand {
            prefix: vec![
                bin.path().join("flatpak").into_os_string(),
                "run".into(),
                FLATPAK_STEAM_APP.into(),
            ],
        })
    );

    // A non-executable `steam` is skipped; an executable one wins.
    let other = tempfile::tempdir().unwrap();
    std::fs::write(other.path().join("steam"), b"").unwrap();
    executable(&bin.path().join("steam"));
    let path = format!("{}:{bin_text}", other.path().to_str().unwrap());
    assert_eq!(
        find_steam_command(&env(&path)),
        Some(SteamCommand {
            prefix: vec![bin.path().join("steam").into_os_string()],
        })
    );
}

#[test]
fn the_profile_game_start_chooses_the_linux_launch() {
    let configuration = Configuration::default();
    let registry = AddonRegistry::new();
    let method = |exe: &str, start: GameStart| {
        let mut profile = Profile::new("p");
        profile.launcher_options.arma3_exe_path = Some(exe.to_owned());
        profile.launcher_options.game_start = start;
        let request = Arma3LaunchRequest {
            os: Os::Linux,
            profile: Some(&profile),
            configuration: &configuration,
            registry: &registry,
            steam: Some(SteamCommand {
                prefix: os_strings(&["steam"]),
            }),
            arma3_documents_dir: None,
            restart_guard: Default::default(),
            steam_watch: Default::default(),
        };
        plan_launch(&request)
            .expect("planned")
            .expect("profile")
            .plan
            .method
    };
    let windows_exe = "/games/Arma 3/arma3_x64.exe";
    let native = "/games/Arma 3/arma3_x64";
    assert_eq!(
        method(windows_exe, GameStart::Automatic),
        LaunchMethod::Steam
    );
    assert_eq!(method(native, GameStart::Automatic), LaunchMethod::Direct);
    assert_eq!(method(native, GameStart::Steam), LaunchMethod::Steam);
    assert_eq!(
        method(windows_exe, GameStart::Executable),
        LaunchMethod::Direct
    );
}
