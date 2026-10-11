//! `build`, `check`, `sync` and `extract`, with their legacy flags.

use std::fs;

use a3s_repo_files::local::{CacheKind, LocalStore};

use crate::support::{
    Env, HttpServer, assert_exit, closed_port, http_protocol, pbo_content, server_info, write,
    write_repository_content,
};

const A3S_FILES: [&str; 5] = ["sync", "serverinfo", "changelogs", "autoconfig", "events"];

/// An administered repository `admin` over `repo/`, served by `server`.
fn admin_repository(env: &Env, server: &HttpServer) {
    write_repository_content(&env.path("repo"));
    let path = env.path("repo").to_string_lossy().into_owned();
    env.add_repository("admin", &http_protocol(server.port), |repository| {
        repository.path = Some(path);
    });
}

#[test]
fn build_writes_the_metadata_and_stores_the_revision() {
    let env = Env::new();
    let server = HttpServer::serve(&env.path("repo"));
    admin_repository(&env, &server);

    assert_exit(&env.run(&["-BUILD", " admin "]), 0);
    for file in A3S_FILES {
        assert!(env.path("repo/.a3s").join(file).is_file(), "{file}");
    }
    assert!(env.path("repo/@mod/addons/mod.pbo.zsync").is_file());
    let stored = env.repository("admin");
    assert_eq!(stored.revision, server_info(&env.path("repo")).revision);
    assert!(stored.revision > 0);
    assert!(stored.auto_config_url.is_some());
    let store = LocalStore::new(&env.dirs);
    assert!(store.sha1_cache_path("admin", CacheKind::Build).is_file());

    // A changed file makes the next build a new revision.
    write(&env.path("repo/@mod/addons/new.pbo"), b"new");
    assert_exit(&env.run(&["build", "admin"]), 0);
    assert_eq!(env.repository("admin").revision, stored.revision + 1);
}

#[test]
fn build_fails_for_an_unknown_repository_or_a_missing_folder() {
    let env = Env::new();
    assert_exit(&env.run(&["build", "missing"]), 1);

    let missing = env.path("nowhere").to_string_lossy().into_owned();
    env.add_repository("admin", &http_protocol(80), |repository| {
        repository.path = Some(missing);
    });
    let before = env.repository("admin");
    assert_exit(&env.run(&["-build", "admin"]), 1);
    assert_eq!(env.repository("admin"), before);
    assert!(!env.path("nowhere").exists());
}

#[test]
fn check_fails_when_the_server_lost_a_file() {
    let env = Env::new();
    let server = HttpServer::serve(&env.path("repo"));
    admin_repository(&env, &server);
    assert_exit(&env.run(&["build", "admin"]), 0);

    assert_exit(&env.run(&["-check", "admin"]), 0);
    fs::remove_file(env.path("repo/@mod/addons/mod.pbo")).expect("remove");
    assert_exit(&env.run(&["check", "admin"]), 1);
    assert_exit(&env.run(&["check", "missing"]), 1);
}

#[test]
fn sync_downloads_the_repository_and_stores_the_revision() {
    let env = Env::new();
    let server = HttpServer::serve(&env.path("repo"));
    admin_repository(&env, &server);
    assert_exit(&env.run(&["build", "admin"]), 0);

    let destination = env.path("mods");
    write(&destination.join("stray.txt"), b"not in the repository");
    env.add_repository("client", &http_protocol(server.port), |_| {});
    let destination_arg = destination.to_string_lossy().into_owned();
    assert_exit(&env.run(&["-SYNC", "client", &destination_arg, "TRUE"]), 0);

    assert_eq!(
        fs::read(destination.join("@mod/addons/mod.pbo")).expect("downloaded"),
        pbo_content()
    );
    assert!(destination.join("@mod/mod.cpp").is_file());
    assert!(destination.join("readme.txt").is_file());
    assert!(
        !destination.join("stray.txt").exists(),
        "exact match deletes extra files"
    );

    let stored = env.repository("client");
    assert_eq!(stored.revision, server_info(&env.path("repo")).revision);
    assert_eq!(
        stored.default_download_location.as_deref(),
        Some(destination_arg.as_str())
    );
    assert!(stored.exact_match);
    assert_eq!(stored.protocol.connection_timeout.as_deref(), Some("0"));
    assert_eq!(stored.protocol.read_timeout.as_deref(), Some("0"));
    let store = LocalStore::new(&env.dirs);
    assert!(store.sha1_cache_path("client", CacheKind::Sync).is_file());
}

#[test]
fn sync_without_exact_match_keeps_extra_files() {
    let env = Env::new();
    let server = HttpServer::serve(&env.path("repo"));
    admin_repository(&env, &server);
    assert_exit(&env.run(&["build", "admin"]), 0);

    let destination = env.path("mods");
    write(&destination.join("stray.txt"), b"kept");
    env.add_repository("client", &http_protocol(server.port), |repository| {
        repository.exact_match = true;
    });
    let destination_arg = destination.to_string_lossy().into_owned();
    assert_exit(&env.run(&["sync", "client", &destination_arg]), 0);
    assert!(destination.join("@mod/addons/mod.pbo").is_file());
    assert!(destination.join("stray.txt").is_file());
    assert!(!env.repository("client").exact_match);
}

#[test]
fn failed_sync_stores_the_failed_revision() {
    let env = Env::new();
    let destination = env.path("mods");
    fs::create_dir_all(&destination).expect("mkdir");
    env.add_repository("client", &http_protocol(closed_port()), |repository| {
        repository.revision = 5;
    });
    let destination_arg = destination.to_string_lossy().into_owned();
    assert_exit(
        &env.run(&["sync", "client", &destination_arg, "--exact-match"]),
        1,
    );
    assert_eq!(env.repository("client").revision, -1);
}

#[test]
fn sync_refuses_a_missing_destination_or_a_bad_exact_match_value() {
    let env = Env::new();
    env.add_repository("client", &http_protocol(closed_port()), |repository| {
        repository.revision = 5;
    });
    let missing = env.path("missing").to_string_lossy().into_owned();
    assert_exit(&env.run(&["-sync", "client", &missing, "false"]), 1);
    assert_exit(&env.run(&["-sync", "client", &missing, "maybe"]), 2);
    assert_exit(&env.run(&["sync", "unknown", &missing]), 1);
    let stored = env.repository("client");
    assert_eq!(stored.revision, 5);
    assert_eq!(stored.default_download_location, None);
}

#[test]
fn extract_copies_the_bikeys() {
    let env = Env::new();
    write(&env.path("mods/@a/keys/a.bikey"), b"a");
    write(&env.path("mods/@b/keys/B.BIKEY"), b"b");
    write(&env.path("mods/@b/addons/b.pbo"), b"pbo");
    fs::create_dir_all(env.path("keys")).expect("mkdir");
    let source = env.path("mods").to_string_lossy().into_owned();
    let target = env.path("keys").to_string_lossy().into_owned();

    assert_exit(&env.run(&["-extract", &source, &target]), 0);
    let mut copied: Vec<String> = fs::read_dir(env.path("keys"))
        .expect("read_dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    copied.sort();
    assert_eq!(copied, ["B.BIKEY", "a.bikey"]);

    let missing = env.path("missing").to_string_lossy().into_owned();
    assert_exit(&env.run(&["extract", &missing, &target]), 1);
    assert_exit(&env.run(&["extract", &source, &missing]), 1);
}
