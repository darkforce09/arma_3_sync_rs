//! `Repository` settings semantics.
//!
//! A2 ports `src/test/java/fr/soe/a3s/domain/RepositoryUploadSettingsTest.java`; A4 covers the
//! check-connection clamps and the partial-transfer inversion.

use a3s_types::local::{InvalidSetting, Repository};
use a3s_types::{Protocol, ProtocolType};

fn repository() -> Repository {
    let protocol = Protocol::new(ProtocolType::Http, "host/path", "80", "", "", false);
    Repository::new("test", protocol)
}

#[test]
fn a2_uses_four_connections_by_default() {
    let repository = repository();
    assert_eq!(
        repository.parallel_upload_connections(),
        Repository::DEFAULT_PARALLEL_UPLOAD_CONNECTIONS
    );
    assert_eq!(repository.parallel_upload_connections(), 4);
}

#[test]
fn a2_caps_parallel_uploads_at_ten() {
    let mut repository = repository();
    repository.set_parallel_upload_connections(50).unwrap();
    assert_eq!(
        repository.parallel_upload_connections(),
        Repository::MAX_PARALLEL_UPLOAD_CONNECTIONS
    );
    assert_eq!(repository.parallel_upload_connections, 10);
}

#[test]
fn a2_rejects_invalid_parallel_upload_count() {
    let mut repository = repository();
    let err = repository.set_parallel_upload_connections(0).unwrap_err();
    assert_eq!(
        err,
        InvalidSetting {
            message: "Parallel upload connections must be positive."
        }
    );
    assert_eq!(
        err.to_string(),
        "Parallel upload connections must be positive."
    );
    assert!(repository.set_parallel_upload_connections(-3).is_err());
    // The stored value is untouched.
    assert_eq!(repository.parallel_upload_connections, 4);
}

#[test]
fn a2_raw_parallel_upload_values_are_clamped_on_read() {
    let mut repository = repository();
    repository.parallel_upload_connections = 0;
    assert_eq!(repository.parallel_upload_connections(), 4);
    repository.parallel_upload_connections = -1;
    assert_eq!(repository.parallel_upload_connections(), 4);
    repository.parallel_upload_connections = 25;
    assert_eq!(repository.parallel_upload_connections(), 10);
    repository.parallel_upload_connections = 7;
    assert_eq!(repository.parallel_upload_connections(), 7);
}

#[test]
fn a4_check_connections() {
    let mut repository = repository();
    assert_eq!(repository.repository_check_connections(), 4);
    assert_eq!(Repository::DEFAULT_REPOSITORY_CHECK_CONNECTIONS, 4);
    assert_eq!(Repository::MAX_REPOSITORY_CHECK_CONNECTIONS, 4);

    repository.set_repository_check_connections(10).unwrap();
    assert_eq!(repository.repository_check_connections(), 4);
    assert_eq!(repository.number_of_repository_check_connections, 4);

    repository.set_repository_check_connections(2).unwrap();
    assert_eq!(repository.repository_check_connections(), 2);

    let err = repository.set_repository_check_connections(0).unwrap_err();
    assert_eq!(
        err.to_string(),
        "Repository check connections must be positive."
    );
    assert_eq!(repository.number_of_repository_check_connections, 2);

    // Raw values from legacy files.
    repository.number_of_repository_check_connections = 0;
    assert_eq!(repository.repository_check_connections(), 4);
    repository.number_of_repository_check_connections = 7;
    assert_eq!(repository.repository_check_connections(), 4);
}

#[test]
fn a4_partial_file_transfer_inversion() {
    let mut repository = repository();
    assert!(!repository.no_partial_file_transfer);
    assert!(repository.use_partial_file_transfer());

    repository.set_use_partial_file_transfer(false);
    assert!(repository.no_partial_file_transfer);
    assert!(!repository.use_partial_file_transfer());

    repository.set_use_partial_file_transfer(true);
    assert!(!repository.no_partial_file_transfer);
    assert!(repository.use_partial_file_transfer());
}
