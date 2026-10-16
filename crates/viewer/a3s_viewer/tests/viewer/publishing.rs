//! The Publishing screen: a publish against a local FTP server, a failed build with Retry,
//! editing and uploading events, settings that persist, and the upload connection sheet.

use std::path::Path;
use std::time::{Duration, Instant};

use a3s_admin::EventsDoc;
use a3s_repo_files::services::ServiceError;
use a3s_test_servers::Fixture as Server;
use a3s_types::local::Repository;
use a3s_types::{FavoriteServer, ProtocolType};
use a3s_viewer::{SheetKind, Shell};
use a3s_viewer_context::admin::{
    PublishStep, Publishing, Remote, ServerCheck, StepDetail, StepStatus,
};
use a3s_viewer_context::{TaskKey, TaskKind};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use tempfile::TempDir;

use crate::support::{Start, fixture};

/// How long a test waits for background work.
const TIMEOUT: Duration = Duration::from_secs(60);

/// The name of the published repository.
const NAME: &str = "Pub";

fn wait_until(harness: &mut Harness<'static, Shell>, what: &str, done: impl Fn(&Shell) -> bool) {
    let start = Instant::now();
    while !done(harness.state()) {
        assert!(start.elapsed() < TIMEOUT, "timed out waiting for {what}");
        harness.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    harness.run();
}

fn publishing(shell: &Shell, name: &str) -> Publishing {
    shell.viewer().read().admin.publishing(name)
}

fn server_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("test-server")
        .enable_all()
        .build()
        .expect("runtime")
}

/// An empty FTP server, a source folder with two mods, and the repository publishing one to
/// the other.
fn setup(runtime: &tokio::runtime::Runtime) -> (Server, TempDir, Repository) {
    let server = runtime.block_on(a3s_test_servers::ftp::fixture(false));
    let source = tempfile::tempdir().expect("source");
    write(source.path(), "@Mod_A/addons/a.pbo", 3000, 1);
    write(source.path(), "@Mod_A/mod.cpp", 40, 2);
    write(source.path(), "@Mod_B/addons/b.pbo", 2000, 3);
    let mut repository = Repository::new(NAME, server.protocol.clone());
    repository.path = Some(source.path().display().to_string());
    repository.upload_protocol = Some(server.protocol.clone());
    (server, source, repository)
}

fn write(root: &Path, relative: &str, len: usize, seed: u64) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, a3s_test_servers::random_bytes(len, seed)).expect("write");
}

/// Opens the Publishing screen of `name` and waits until the server and the source folder
/// were read.
fn open(harness: &mut Harness<'static, Shell>, name: &str) {
    harness
        .get_all_by_label(name)
        .last()
        .expect("publishing row")
        .click();
    harness.run();
    wait_until(harness, "the publishing status", |shell| {
        let publishing = publishing(shell, name);
        matches!(publishing.remote, Remote::Loaded(_))
            && publishing.local.is_some()
            && publishing.events.is_some()
            && shell
                .viewer()
                .is_running(&TaskKey::repository(TaskKind::PublishingStatus, name))
                .is_none()
    });
}

/// Clicks `label` and waits until the publish it starts has finished.
///
/// The click reaches the context through the command bus, so for a moment the previous run is
/// still stored and no Publish task is registered. The new task replaces the run once it starts
/// and is registered until it ends, so "the run changed and no task runs" means it finished.
fn click_and_wait_for_publish(harness: &mut Harness<'static, Shell>, label: &str) {
    let previous = publishing(harness.state(), NAME).run;
    harness.get_by_label(label).click();
    harness.step();
    wait_until(harness, "the publish", |shell| {
        shell
            .viewer()
            .is_running(&TaskKey::repository(TaskKind::Publish, NAME))
            .is_none()
            && publishing(shell, NAME).run != previous
    });
}

fn statuses(shell: &Shell) -> Vec<StepStatus> {
    let run = publishing(shell, NAME).run.expect("a run");
    PublishStep::ALL
        .into_iter()
        .map(|step| run.step(step).status.clone())
        .collect()
}

#[test]
fn publish_builds_verifies_and_uploads() {
    let runtime = server_runtime();
    let (server, _source, repository) = setup(&runtime);
    let mut fixture = fixture(Start {
        repositories: vec![repository],
        ..Start::default()
    });
    let harness = &mut fixture.harness;
    open(harness, NAME);
    let local = publishing(harness.state(), NAME)
        .local
        .expect("scanned")
        .expect("readable");
    assert_eq!(local.changed_files, 3);
    assert_eq!(local.mods, ["@Mod_A", "@Mod_B"]);

    click_and_wait_for_publish(harness, "Publish revision 1");

    assert_eq!(statuses(harness.state()), vec![StepStatus::Done; 4]);
    let state = publishing(harness.state(), NAME);
    let run = state.run.as_ref().expect("a run");
    assert!(run.published);
    assert_eq!(run.revision, Some(1));
    let changes = run.changes.clone().expect("changes");
    assert_eq!(changes.added, ["@Mod_A", "@Mod_B"]);
    assert_eq!(state.published_revision(), Some(1));
    assert_eq!(
        state
            .local
            .expect("scanned")
            .expect("readable")
            .changed_files,
        0
    );
    assert!(server.base.join(".a3s/sync").is_file());
    assert!(server.base.join("@Mod_A/addons/a.pbo").is_file());
    assert!(server.base.join("@Mod_B/addons/b.pbo").is_file());
}

#[test]
fn a_failed_build_offers_retry_from_the_build() {
    let runtime = server_runtime();
    let (server, source, repository) = setup(&runtime);
    // An unreadable previous build stops the build.
    let broken = source.path().join(".a3s/serverinfo");
    std::fs::create_dir_all(broken.parent().expect("parent")).expect("mkdir");
    std::fs::write(&broken, b"not a server info").expect("write");
    let mut fixture = fixture(Start {
        repositories: vec![repository],
        ..Start::default()
    });
    let harness = &mut fixture.harness;
    open(harness, NAME);
    click_and_wait_for_publish(harness, "Publish revision 1");

    let failed = statuses(harness.state());
    assert_eq!(failed[0], StepStatus::Done);
    assert!(matches!(failed[1], StepStatus::Failed(_)));
    assert_eq!(failed[2], StepStatus::Pending);

    std::fs::remove_file(&broken).expect("remove");
    click_and_wait_for_publish(harness, "Retry");
    assert_eq!(statuses(harness.state()), vec![StepStatus::Done; 4]);
    assert!(server.base.join(".a3s/serverinfo").is_file());
}

#[test]
fn events_are_edited_saved_undone_and_uploaded() {
    let runtime = server_runtime();
    let (server, source, repository) = setup(&runtime);
    let mut fixture = fixture(Start {
        repositories: vec![repository],
        ..Start::default()
    });
    let harness = &mut fixture.harness;
    open(harness, NAME);
    click_and_wait_for_publish(harness, "Publish revision 1");

    harness.get_by_label("Events").click();
    harness.run();
    harness.get_by_label("Add").click();
    harness.run();
    let events = |shell: &Shell| publishing(shell, NAME).events.expect("events").events();
    assert_eq!(events(harness.state())[0].name, "New event");
    let local_file = source.path().join(".a3s/events");
    let saved =
        EventsDoc::decode(&std::fs::read(&local_file).expect("saved locally")).expect("decodes");
    assert_eq!(saved.events()[0].name, "New event");

    harness.get_by_label("@Mod_A").click();
    harness.run();
    harness.get_by_label("Optional").click();
    harness.run();
    let event = events(harness.state())[0].clone();
    assert_eq!(event.addon_names.get("@Mod_A"), Some(&true));
    assert!(!event.addon_names.contains_key("@Mod_B"));

    harness.get_by_label("Name").focus();
    harness.run();
    harness.get_by_label("Name").type_text(" 1");
    harness.run();
    assert_eq!(events(harness.state())[0].name, "New event 1");

    harness.get_by_label("Remove").click();
    harness.run();
    assert!(events(harness.state()).is_empty());
    harness.get_by_label("Undo").click();
    harness.run();
    assert_eq!(events(harness.state())[0].name, "New event 1");

    // Only the events changed since the publish.
    harness.get_by_label("Upload events").click();
    harness.step();
    wait_until(harness, "the events upload", |shell| {
        !publishing(shell, NAME).events_changed
    });
    let uploaded =
        EventsDoc::decode(&std::fs::read(server.base.join(".a3s/events")).expect("uploaded"))
            .expect("decodes");
    assert_eq!(uploaded.events()[0].name, "New event 1");
    assert_eq!(uploaded.events()[0].addon_names.get("@Mod_A"), Some(&true));
}

#[test]
fn settings_persist_at_once() {
    let mut fixture = fixture(Start::default());
    let harness = &mut fixture.harness;
    harness
        .get_all_by_label("Beta")
        .last()
        .expect("publishing row")
        .click();
    harness.run();
    harness.get_by_label("Settings").click();
    harness.run();

    for label in [
        "Compress files",
        "Delta updates",
        "Upload only compressed files",
    ] {
        harness
            .get_by_role_and_label(egui::accesskit::Role::CheckBox, label)
            .click();
        harness.run();
    }
    for (label, value) in [("Parallel uploads", "2"), ("Connections per player", "3")] {
        harness.get_by_label(label).click();
        harness.run();
        harness
            .get_all_by_label(value)
            .last()
            .expect("pop-up entry")
            .click();
        harness.run();
    }
    let loaded = fixture.store.load_repositories().expect("load");
    let stored = loaded
        .repositories
        .values()
        .map(|loaded| loaded.repository.clone())
        .find(|repository| repository.name == "Beta")
        .unwrap_or_else(|| panic!("Beta not stored: {:?}", loaded.repositories.keys()));
    let defaults = Repository::new("Beta", stored.protocol.clone());
    assert_eq!(stored.compressed, !defaults.compressed);
    assert_eq!(
        stored.use_partial_file_transfer(),
        !defaults.use_partial_file_transfer()
    );
    assert_eq!(
        stored.upload_compressed_pbo_files_only,
        !defaults.upload_compressed_pbo_files_only
    );
    assert_eq!(stored.parallel_upload_connections(), 2);
    assert_eq!(stored.number_of_connections, 3);
}

#[test]
fn auto_config_servers_are_chosen_and_stored_at_once() {
    let mut fixture = fixture(Start::default());
    let harness = &mut fixture.harness;
    harness
        .state()
        .viewer()
        .mutate(None, |state| {
            for (name, address) in [("Main", "10.0.0.1"), ("Training", "10.0.0.2")] {
                state.client.add_favorite_server(FavoriteServer {
                    name: Some(name.to_owned()),
                    ip_address: Some(address.to_owned()),
                    port: 2302,
                    ..FavoriteServer::default()
                })?;
            }
            Ok::<(), ServiceError>(())
        })
        .expect("servers");
    harness
        .get_all_by_label("Beta")
        .last()
        .expect("publishing row")
        .click();
    harness.run();
    harness.get_by_label("Settings").click();
    harness.run();

    let stored = |fixture: &crate::support::Fixture| -> Vec<Option<String>> {
        let loaded = fixture.store.load_repositories().expect("load");
        loaded
            .repositories
            .values()
            .find(|loaded| loaded.repository.name == "Beta")
            .expect("Beta stored")
            .repository
            .favorite_servers_set_to_autoconfig
            .iter()
            .map(|server| server.name.clone())
            .collect()
    };
    let switch = |fixture: &mut crate::support::Fixture, label: &str| {
        let role = egui::accesskit::Role::CheckBox;
        fixture
            .harness
            .get_by_role_and_label(role, label)
            .scroll_to_me();
        fixture.harness.run();
        fixture.harness.get_by_role_and_label(role, label).click();
        fixture.harness.run();
    };

    switch(&mut fixture, "Training");
    assert_eq!(stored(&fixture), [Some("Training".to_owned())]);
    switch(&mut fixture, "Main");
    assert_eq!(
        stored(&fixture),
        [Some("Main".to_owned()), Some("Training".to_owned())]
    );
    switch(&mut fixture, "Training");
    assert_eq!(stored(&fixture), [Some("Main".to_owned())]);
}

#[test]
fn the_upload_connection_sheet_stores_the_connection() {
    let mut fixture = fixture(Start::default());
    let harness = &mut fixture.harness;
    harness
        .get_all_by_label("Beta")
        .last()
        .expect("publishing row")
        .click();
    harness.run();
    harness.get_by_label("Settings").click();
    harness.run();
    harness.get_by_label("Edit connection…").click();
    harness.run();
    assert_eq!(harness.state().sheet(), Some(SheetKind::UploadConnection));

    harness.get_by_label("SFTP").click();
    harness.run();
    // The sheet starts from the download address.
    assert_eq!(
        harness.get_by_label("Host").value().as_deref(),
        Some("127.0.0.1")
    );
    harness.get_by_label("Path").focus();
    harness.run();
    harness.get_by_label("Path").type_text("/upload");
    harness.run();
    harness.get_by_label("Save").click();
    harness.run();
    assert_eq!(harness.state().sheet(), None);
    let state = harness.state().viewer().read();
    let upload = state
        .client
        .repository("Beta")
        .expect("Beta")
        .upload_protocol
        .clone()
        .expect("an upload connection");
    assert_eq!(upload.protocol_type, ProtocolType::Sftp);
    assert_eq!(upload.url, "127.0.0.1/repo/upload");
    assert_eq!(upload.port, ProtocolType::Sftp.default_port());
}

#[test]
fn without_an_upload_connection_a_served_folder_is_built_in_place() {
    let runtime = server_runtime();
    let server = runtime.block_on(a3s_test_servers::ftp::fixture(false));
    write(&server.base, "@Mod_A/addons/a.pbo", 3000, 1);
    write(&server.base, "@Mod_B/addons/b.pbo", 2000, 3);
    // The server serves the source folder itself: no upload connection.
    let mut repository = Repository::new(NAME, server.protocol.clone());
    repository.path = Some(server.base.display().to_string());
    let mut fixture = fixture(Start {
        repositories: vec![repository],
        ..Start::default()
    });
    let harness = &mut fixture.harness;
    open(harness, NAME);

    click_and_wait_for_publish(harness, "Build revision 1");
    assert_eq!(statuses(harness.state()), vec![StepStatus::Done; 4]);
    let state = publishing(harness.state(), NAME);
    let run = state.run.as_ref().expect("a run");
    assert!(run.published);
    assert_eq!(
        run.step(PublishStep::Upload).detail,
        StepDetail::ServedDirectly
    );
    assert!(
        !state
            .local
            .expect("scanned")
            .expect("readable")
            .history
            .is_empty(),
        "the build's changelog is the history"
    );
    harness.get_by_label("History");

    // "Check the server files" finds every file of the build.
    harness
        .get_all_by_label(a3s_ui::icon::icons::DOTS_THREE_CIRCLE)
        .last()
        .expect("Publishing menu")
        .click();
    harness.run();
    harness.get_by_label("Check the server files").click();
    wait_until(harness, "the server check", |shell| {
        matches!(
            publishing(shell, NAME).server_check,
            Some(ServerCheck::Done { .. })
        )
    });
    harness.get_by_label_contains("are on the server");
}
