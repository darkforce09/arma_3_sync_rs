//! Real launches of harmless shell scripts standing in for the game, Steam and external
//! applications: arguments, events, auto-restart and the `Arma3.cfg` clean-up.
#![cfg(unix)]

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use a3s_addons::AddonRegistry;
use a3s_foundation::paths::Os;
use a3s_launch::cfg::ARMA3_CFG;
use a3s_launch::command::{CommandSpec, LaunchMethod, LaunchPlan, LinuxLaunchMode};
use a3s_launch::params::{ModPathStyle, RunParameterInput, determine_run_parameters};
use a3s_launch::{
    Arma3LaunchRequest, EventSink, LaunchError, LaunchEvent, LaunchHandle, LaunchTarget,
    RestartGuard, RunOptions, SteamCommand, SteamWatch, launch_arma3, launch_external_applications,
    spawn_plan,
};
use a3s_platform::process::{FixedProcessProbe, ProcessProbe, ProcessSnapshot};
use a3s_types::local::{Addon, Configuration, ExternalApplication, Profile, TreeLeaf, TreeNode};
use parking_lot::Mutex;

const MOD_LIST_CFG: &[u8] = b"a=1;\nclass ModLauncherList\n{\n};\n";

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<LaunchEvent>>>);

impl Recorder {
    fn sink(&self) -> EventSink {
        let events = Arc::clone(&self.0);
        Arc::new(move |event| events.lock().push(event))
    }

    fn events(&self) -> Vec<LaunchEvent> {
        self.0.lock().clone()
    }

    fn wait_until(&self, done: impl Fn(&[LaunchEvent]) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !done(&self.0.lock()) {
            assert!(Instant::now() < deadline, "events: {:?}", self.events());
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn restarts(events: &[LaunchEvent]) -> Vec<u32> {
    events
        .iter()
        .filter_map(|event| match event {
            LaunchEvent::Restarting { run } => Some(*run),
            _ => None,
        })
        .collect()
}

fn exits(events: &[LaunchEvent]) -> Vec<Option<i32>> {
    events
        .iter()
        .filter_map(|event| match event {
            LaunchEvent::Exited { code, .. } => Some(*code),
            _ => None,
        })
        .collect()
}

fn errors(events: &[LaunchEvent]) -> Vec<(LaunchTarget, Arc<LaunchError>)> {
    events
        .iter()
        .filter_map(|event| match event {
            LaunchEvent::Error { target, error } => Some((target.clone(), Arc::clone(error))),
            _ => None,
        })
        .collect()
}

fn script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(path, format!("#!/bin/bash\n{body}")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A game script that records its arguments and counts its runs next to itself.
fn game_script(dir: &Path, tail: &str) -> PathBuf {
    let path = dir.join("game.sh");
    script(
        &path,
        &format!(
            "dir=\"$(dirname \"$0\")\"\nprintf '%s\\n' \"$@\" > \"$dir/args.txt\"\necho run >> \"$dir/runs.txt\"\n{tail}"
        ),
    );
    path
}

fn read_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

struct Launch {
    profile: Profile,
    configuration: Configuration,
    registry: AddonRegistry,
    documents: PathBuf,
    steam: Option<SteamCommand>,
    guard: RestartGuard,
    steam_watch: SteamWatch,
}

impl Launch {
    fn new(exe: &Path, documents: &Path) -> Self {
        let mut profile = Profile::new("p");
        profile.launcher_options.arma3_exe_path = Some(exe.to_str().unwrap().to_owned());
        Self {
            profile,
            configuration: Configuration::default(),
            registry: AddonRegistry::new(),
            documents: documents.to_path_buf(),
            steam: None,
            guard: RestartGuard::default(),
            steam_watch: SteamWatch::default(),
        }
    }

    fn request(&self) -> Arma3LaunchRequest<'_> {
        Arma3LaunchRequest {
            os: Os::Linux,
            profile: Some(&self.profile),
            configuration: &self.configuration,
            registry: &self.registry,
            linux_mode: LinuxLaunchMode::Auto,
            steam: self.steam.clone(),
            arma3_documents_dir: Some(self.documents.clone()),
            restart_guard: self.guard,
            steam_watch: self.steam_watch,
        }
    }

    fn start(&self, probe: Arc<dyn ProcessProbe>, recorder: &Recorder) -> LaunchHandle {
        launch_arma3(&self.request(), probe, recorder.sink())
            .unwrap()
            .unwrap()
    }
}

fn no_probe() -> Arc<dyn ProcessProbe> {
    Arc::new(FixedProcessProbe(Some(Vec::new())))
}

#[test]
fn script_receives_the_exact_parameters() {
    let dir = tempfile::tempdir().unwrap();
    let exe = game_script(dir.path(), "exit 0\n");
    std::fs::write(dir.path().join(ARMA3_CFG), MOD_LIST_CFG).unwrap();

    let mut launch = Launch::new(&exe, dir.path());
    launch.profile.additional_parameters = Some("-x  -malloc=a".to_owned());
    launch.profile.launcher_options.window_mode = true;
    launch.profile.launcher_options.game_profile = Some("Some Player".to_owned());
    launch.profile.tree.add_child(TreeNode::Leaf(TreeLeaf {
        selected: true,
        ..TreeLeaf::new("@ace")
    }));
    launch
        .registry
        .insert(Addon::new("@ace", "@ace", "/my mods"));

    let recorder = Recorder::default();
    launch.start(no_probe(), &recorder).join().unwrap();

    let expected = determine_run_parameters(&RunParameterInput {
        profile: Some(&launch.profile),
        configuration: &launch.configuration,
        registry: &launch.registry,
        mod_path_style: ModPathStyle::native(),
    });
    assert_eq!(
        expected,
        [
            "-malloc=a",
            "-x",
            "-name=Some Player",
            "-window",
            "-mod=/my mods/@ace;"
        ]
    );
    assert_eq!(read_lines(&dir.path().join("args.txt")), expected);

    let events = recorder.events();
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(matches!(
        &events[0],
        LaunchEvent::Started { target: LaunchTarget::Arma3, command_line }
            if command_line.starts_with("/bin/bash ")
    ));
    assert_eq!(exits(&events), [Some(0)]);
    assert_eq!(
        std::fs::read(dir.path().join(ARMA3_CFG)).unwrap(),
        b"a=1;\n"
    );
}

#[test]
fn auto_restart_until_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let exe = game_script(dir.path(), "sleep 1.2\nexit 3\n");
    std::fs::write(dir.path().join(ARMA3_CFG), MOD_LIST_CFG).unwrap();

    let mut launch = Launch::new(&exe, dir.path());
    launch.profile.launcher_options.auto_restart = true;
    launch.guard = RestartGuard {
        min_run: Duration::from_secs(1),
        max_fast_exits: 3,
    };
    let recorder = Recorder::default();
    let handle = launch.start(no_probe(), &recorder);

    // Put a mod list back once the game runs: restarts must leave it alone.
    recorder.wait_until(|events| !events.is_empty());
    assert_eq!(
        std::fs::read(dir.path().join(ARMA3_CFG)).unwrap(),
        b"a=1;\n"
    );
    std::fs::write(dir.path().join(ARMA3_CFG), MOD_LIST_CFG).unwrap();

    recorder.wait_until(|events| restarts(events).contains(&2));
    handle.stop_auto_restart();
    handle.join().unwrap();

    let events = recorder.events();
    assert_eq!(restarts(&events), [1, 2]);
    assert_eq!(exits(&events), [Some(3), Some(3), Some(3)]);
    assert!(errors(&events).is_empty(), "{events:?}");
    assert_eq!(read_lines(&dir.path().join("runs.txt")).len(), 3);
    assert_eq!(
        std::fs::read(dir.path().join(ARMA3_CFG)).unwrap(),
        MOD_LIST_CFG
    );
}

#[test]
fn auto_restart_stops_on_fast_exits() {
    let dir = tempfile::tempdir().unwrap();
    let exe = game_script(dir.path(), "exit 0\n");
    let mut launch = Launch::new(&exe, dir.path());
    launch.profile.launcher_options.auto_restart = true;
    launch.guard = RestartGuard {
        min_run: Duration::from_secs(1),
        max_fast_exits: 3,
    };
    let recorder = Recorder::default();
    launch.start(no_probe(), &recorder).join().unwrap();

    let events = recorder.events();
    assert_eq!(restarts(&events), [1, 2]);
    assert_eq!(read_lines(&dir.path().join("runs.txt")).len(), 3);
    let errors = errors(&events);
    assert_eq!(errors.len(), 1);
    assert!(matches!(
        *errors[0].1,
        LaunchError::RestartLoop { count: 3, .. }
    ));
    assert!(matches!(events.last(), Some(LaunchEvent::Error { .. })));
}

#[test]
fn spawn_failure_is_an_error_event() {
    let plan = LaunchPlan {
        method: LaunchMethod::Direct,
        command: CommandSpec {
            program: "/nonexistent/a3s-launch-test-program".into(),
            args: Vec::new(),
            current_dir: None,
            hide_window: false,
            discard_output: false,
        },
        shown_command: vec!["/nonexistent/a3s-launch-test-program".to_owned()],
    };
    let recorder = Recorder::default();
    let options = RunOptions {
        auto_restart: true,
        ..Default::default()
    };
    spawn_plan(plan, options, no_probe(), recorder.sink())
        .unwrap()
        .join()
        .unwrap();

    let events = recorder.events();
    assert_eq!(events.len(), 1, "{events:?}");
    let errors = errors(&events);
    assert_eq!(errors[0].0, LaunchTarget::Arma3);
    assert!(matches!(*errors[0].1, LaunchError::Spawn { .. }));
}

#[test]
fn external_applications() {
    let dir = tempfile::tempdir().unwrap();
    let app = |name: &str, enable: bool, parameters: Option<&str>| {
        let path = dir.path().join(name);
        script(
            &path,
            &format!(
                "printf '%s\\n' \"$@\" > \"{}.out\"\n",
                path.to_str().unwrap()
            ),
        );
        ExternalApplication {
            name: Some(name.to_owned()),
            executable_path: Some(path.to_str().unwrap().to_owned()),
            parameters: parameters.map(str::to_owned),
            enable,
        }
    };
    let tool = dir.path().join("tool");
    std::fs::write(&tool, b"").unwrap();
    let apps = vec![
        app("a.sh", true, Some("-x  y")),
        app("off.sh", false, Some("-x")),
        ExternalApplication {
            name: Some("missing".to_owned()),
            executable_path: Some(dir.path().join("missing.sh").to_str().unwrap().to_owned()),
            parameters: None,
            enable: true,
        },
        app("b.sh", true, None),
        app("running.sh", true, None),
        ExternalApplication {
            name: Some("tool".to_owned()),
            executable_path: Some(tool.to_str().unwrap().to_owned()),
            parameters: None,
            enable: true,
        },
        app("c.sh", true, Some(" z ")),
    ];
    let probe = FixedProcessProbe(Some(vec!["running.sh".to_owned()]));
    let recorder = Recorder::default();
    let threads = launch_external_applications(Os::Linux, &apps, &probe, &recorder.sink());
    assert_eq!(threads.len(), 3);
    for thread in threads {
        thread.join().unwrap();
    }

    let output = |name: &str| {
        let path = dir.path().join(format!("{name}.out"));
        path.exists().then(|| read_lines(&path))
    };
    assert_eq!(output("a.sh"), Some(vec!["-x".to_owned(), "y".to_owned()]));
    assert_eq!(output("b.sh"), Some(vec![String::new()]));
    assert_eq!(output("c.sh"), Some(vec!["z".to_owned()]));
    assert_eq!(output("off.sh"), None);
    assert_eq!(output("running.sh"), None);

    let events = recorder.events();
    assert_eq!(exits(&events), [Some(0); 3]);
    let errors = errors(&events);
    assert_eq!(errors.len(), 1, "{events:?}");
    assert_eq!(
        errors[0].0,
        LaunchTarget::External {
            name: "tool".to_owned()
        }
    );
    assert!(matches!(
        &*errors[0].1,
        LaunchError::InvalidExecutable { name } if name == "tool"
    ));
}

/// A probe that answers "is the game running" from a script, then "no" forever.
struct ScriptedProbe(Mutex<VecDeque<bool>>);

impl ProcessProbe for ScriptedProbe {
    fn snapshot(&self) -> io::Result<ProcessSnapshot> {
        let running = self.0.lock().pop_front().unwrap_or(false);
        let names = running.then(|| "arma3_x64.exe".to_owned());
        Ok(ProcessSnapshot::from_names(names))
    }
}

fn steam_launch(dir: &Path) -> Launch {
    let steam = dir.join("steam");
    script(
        &steam,
        &format!(
            "printf '%s\\n' \"$@\" > \"{}\"\n",
            dir.join("steam-args.txt").to_str().unwrap()
        ),
    );
    let mut launch = Launch::new(Path::new("/games/Arma 3/arma3_x64.exe"), dir);
    launch.steam = Some(SteamCommand {
        prefix: vec![steam.into_os_string()],
    });
    launch.profile.launcher_options.auto_restart = true;
    launch.profile.launcher_options.window_mode = true;
    launch.steam_watch = SteamWatch {
        appear_timeout: Duration::from_secs(1),
        poll: Duration::from_millis(10),
    };
    launch
}

#[test]
fn steam_launch_follows_the_game() {
    let dir = tempfile::tempdir().unwrap();
    let launch = steam_launch(dir.path());
    let probe = Arc::new(ScriptedProbe(Mutex::new(VecDeque::from([
        false, true, true, false,
    ]))));
    let recorder = Recorder::default();
    let handle = launch.start(probe, &recorder);
    recorder.wait_until(|events| restarts(events).contains(&1));
    handle.stop_auto_restart();
    handle.join().unwrap();

    let events = recorder.events();
    assert!(matches!(
        &events[..3],
        [
            LaunchEvent::Started {
                target: LaunchTarget::Arma3,
                ..
            },
            LaunchEvent::Exited { code: None, .. },
            LaunchEvent::Restarting { run: 1 },
        ]
    ));
    assert!(errors(&events).is_empty(), "{events:?}");

    let deadline = Instant::now() + Duration::from_secs(10);
    let args = dir.path().join("steam-args.txt");
    while !args.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        read_lines(&args),
        ["-applaunch", "107410", "-noLauncher", "-window"]
    );
}

#[test]
fn steam_launch_gives_up_when_the_game_never_appears() {
    let dir = tempfile::tempdir().unwrap();
    let launch = steam_launch(dir.path());
    let recorder = Recorder::default();
    launch.start(no_probe(), &recorder).join().unwrap();

    let events = recorder.events();
    assert!(matches!(events[0], LaunchEvent::Started { .. }));
    let errors = errors(&events);
    assert_eq!(errors.len(), 1, "{events:?}");
    assert!(matches!(*errors[0].1, LaunchError::SteamGameNotSeen { .. }));
}
