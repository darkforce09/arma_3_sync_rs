//! Argument dispatch, exit codes, logging and the updater entry points.

use crate::support::{Env, assert_exit, closed_port};

/// File name of the diagnostic log.
const LOG_FILE: &str = "logs/arma3sync-rs.log";

#[test]
fn the_window_modes_fail_without_touching_the_data() {
    let env = Env::new();
    for args in [
        &[][..],
        &["-dev"],
        &["-RUN"],
        &["-unlock"],
        &["-dev", "-run"],
        &["--debug"],
    ] {
        assert_exit(&env.run(args), 1);
    }
    assert!(!env.dirs.config_dir().exists());
    assert!(!env.dirs.data_dir().exists());
}

#[test]
fn invalid_arguments_are_usage_errors() {
    let env = Env::new();
    for args in [
        &["-build"][..],
        &["-build", "a", "b"],
        &["-extract", "a"],
        &["-sync", "a", "b"],
        &["-console", "extra"],
        &["-update", "now"],
        &["bogus"],
        &["build"],
        &["sync", "a"],
    ] {
        assert_exit(&env.run(args), 2);
    }
    assert!(!env.dirs.config_dir().exists());
    assert_exit(&env.run(&["--help"]), 0);
    assert_exit(&env.run(&["--version"]), 0);
}

#[test]
fn debug_writes_the_log_file() {
    let env = Env::new();
    assert_exit(&env.run(&["build", "missing"]), 1);
    assert!(env.dirs.data_dir().is_dir());
    assert!(!env.dirs.data_dir().join(LOG_FILE).exists());

    for args in [
        &["-DEBUG", "-build", "missing"][..],
        &["build", "missing", "--debug"],
    ] {
        let env = Env::new();
        assert_exit(&env.run(args), 1);
        assert!(env.dirs.data_dir().join(LOG_FILE).is_file(), "{args:?}");
    }

    let env = Env::new();
    let output = env
        .command()
        .args(["check", "missing"])
        .env("A3S_DEBUG", "TRUE")
        .output()
        .expect("run");
    assert_exit(&output, 1);
    assert!(env.dirs.data_dir().join(LOG_FILE).is_file());
}

#[test]
fn the_elevated_updater_runs_before_any_setup() {
    let env = Env::new();
    assert_exit(&env.run(&["__a3s-elevated-install"]), 1);
    assert!(!env.dirs.config_dir().exists());
    assert!(!env.dirs.data_dir().exists());
}

#[test]
fn update_fails_when_no_release_source_answers() {
    let env = Env::new();
    let unreachable = format!("http://127.0.0.1:{}/", closed_port());
    for args in [&["update"][..], &["-update"]] {
        let output = env
            .command()
            .args(args)
            .env("A3S_UPDATER_ALLOW_HTTP", "true")
            .env("A3S_UPDATER_URL", &unreachable)
            .env("A3S_UPDATER_MANIFEST_URL", format!("{unreachable}a3s.json"))
            .env(
                "A3S_UPDATER_LEGACY_XML_URL",
                format!("{unreachable}a3s.xml"),
            )
            .env(
                "A3S_UPDATER_GITHUB_API_URL",
                format!("{unreachable}releases"),
            )
            .output()
            .expect("run");
        assert_exit(&output, 1);
    }
}
