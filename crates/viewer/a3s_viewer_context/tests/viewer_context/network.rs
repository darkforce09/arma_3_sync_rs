//! The network checker against a refused local connection.

use std::time::{Duration, Instant};

use a3s_foundation::error::ErrorKind;
use a3s_types::constants::RepositoryStatus;
use a3s_types::local::Repository;
use a3s_types::{Protocol, ProtocolType};
use a3s_viewer_context::repositories::{Command as RepositoryCommand, Event as RepositoryEvent};
use a3s_viewer_context::{
    AppEvent, CheckOutcome, Command, ContextOptions, NetworkChecker, ViewerContext,
};

use crate::support::{EVENT_TIMEOUT, store};

#[test]
fn a_refused_connection_fails_the_check() {
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.local_addr().expect("address").port()
    };
    let dir = tempfile::tempdir().expect("temp dir");
    let repository = Repository::new(
        "Local",
        Protocol::new(
            ProtocolType::Http,
            "127.0.0.1/repo",
            port.to_string(),
            "anonymous",
            "",
            true,
        ),
    );
    let client = a3s_repo_files::services::ClientData::from_parts(
        store(dir.path()),
        Default::default(),
        Default::default(),
        Vec::new(),
        vec![repository],
    );
    let ctx = ViewerContext::new(
        client,
        ContextOptions {
            worker_threads: 2,
            checker: std::sync::Arc::new(NetworkChecker),
            synchronizer: None,
            ..ContextOptions::default()
        },
    )
    .expect("context");

    let started = Instant::now();
    ctx.send(Command::Repositories(RepositoryCommand::Check {
        names: vec!["Local".to_owned()],
    }))
    .expect("send");
    let results = loop {
        match ctx.next_event_timeout(EVENT_TIMEOUT) {
            Some(AppEvent::Repositories(RepositoryEvent::CheckCompleted { results, .. })) => {
                break results;
            }
            Some(_) => {}
            None => panic!("no CheckCompleted"),
        }
    };
    assert!(started.elapsed() < Duration::from_secs(5));
    let CheckOutcome::Failed(error) = &results[0].outcome else {
        panic!("expected a failure, got {:?}", results[0].outcome);
    };
    assert_ne!(error.kind, ErrorKind::Cancelled);
    assert_eq!(error.kind, ErrorKind::ConnectionFailed);
    assert_eq!(results[0].status, Some(RepositoryStatus::Indeterminated));
}
