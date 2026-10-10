//! Verbatim user-facing message texts of the Java client.
//!
//! The texts keep the Java typos (`Incompete`, `no found`, `checkout`, …) and line breaks for
//! parity with the Java UI and console.

use std::path::Path;

use crate::ErrorKind;

/// Message of an authentication failure.
// Mirrors exception/ConnectionExceptionFactory.java:20
pub const WRONG_LOGIN_PASSWORD: &str = "login or password wrong or missing";

/// Message of a TLS certificate validation failure.
// Mirrors exception/ConnectionExceptionFactory.java:21
pub const SSL_CERTIFICATE_VALIDATION_FAILED: &str = "Failed to validate SSL certificate";

// Mirrors exception/ConnectionExceptionFactory.java:13-19
const CONNECTION_FAILED: &str = "Connection failed";
const CONNECTION_TIME_OUT_REACHED: &str =
    "Connection closed by ArmA3Sync.\nServer didn't respond, connection timeout reached";
const READ_TIME_OUT_REACHED: &str =
    "Connection closed by ArmA3Sync\nServer didn't respond, read timeout reached";
const TIME_OUT_REACHED: &str =
    "Connection closed by ArmA3Sync\nServer didn't respond, connection or read timeout reached";

/// Builds the message of a failed network operation.
///
/// `core` describes the operation, `detail` is the message of the underlying error.
/// Connection failures and timeouts replace `detail` with a fixed explanation; an
/// [`ErrorKind::IncompleteTransfer`] returns `detail` alone (or `core` if there is none); every
/// other kind appends `detail` on its own line.
// Mirrors exception/ConnectionExceptionFactory.java:23-60
pub fn connection(core: &str, kind: ErrorKind, detail: Option<&str>) -> String {
    match kind {
        ErrorKind::ConnectionFailed => format!("{core}\n{CONNECTION_FAILED}"),
        ErrorKind::ConnectTimeout => format!("{core}\n{CONNECTION_TIME_OUT_REACHED}"),
        ErrorKind::ReadTimeout => format!("{core}\n{READ_TIME_OUT_REACHED}"),
        ErrorKind::Timeout => format!("{core}\n{TIME_OUT_REACHED}"),
        ErrorKind::IncompleteTransfer => detail.unwrap_or(core).to_owned(),
        _ => match detail {
            Some(detail) => format!("{core}\n{detail}"),
            None => core.to_owned(),
        },
    }
}

/// Message of a transfer that ended with the wrong size.
// Mirrors exception/IncompleteFileTransferException.java:9-15
pub fn incomplete_transfer(file_name: &str, actual_size: u64, remote_size: u64) -> String {
    format!(
        "Incompete file size transfer: {file_name} Remote size: {remote_size} Bytes, Actual size: {actual_size} Bytes"
    )
}

/// Message of a directory that could not be created.
// Mirrors exception/CreateDirectoryException.java:17-21
pub fn cannot_create_directory(path: &Path) -> String {
    format!(
        "Cannot create directory {}\nPlease checkout file access permissions.",
        path.display()
    )
}

/// Message of a directory that could not be deleted.
// Mirrors exception/DeleteDirectoryException.java:17-21
pub fn cannot_delete_directory(path: &Path) -> String {
    format!(
        "Cannot delete directory {}\nPlease checkout file access permissions.",
        path.display()
    )
}

/// A metadata file in a repository's `.a3s` folder.
///
/// Local mirror of `a3s_repo_files::A3sFile`, which this crate cannot depend on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RemoteA3sFile {
    /// `.a3s/sync`
    Sync,

    /// `.a3s/serverinfo`
    ServerInfo,

    /// `.a3s/changelogs`
    Changelogs,

    /// `.a3s/events`
    Events,

    /// `.a3s/autoconfig`
    AutoConfig,
}

/// Message of a metadata file missing on the remote server.
///
/// Quirks: `no found` for `sync`, and no leading `/` for `serverinfo`.
// Mirrors exception/remote/Remote*FileNotFoundException.java:8-9
pub fn remote_file_not_found(file: RemoteA3sFile) -> &'static str {
    match file {
        RemoteA3sFile::Sync => "Remote file no found: /.a3s/sync",
        RemoteA3sFile::ServerInfo => "Remote file not found: .a3s/serverinfo",
        RemoteA3sFile::Changelogs => "Remote file not found: /.a3s/changelogs",
        RemoteA3sFile::Events => "Remote file not found: /.a3s/events",
        RemoteA3sFile::AutoConfig => "Remote file not found: /.a3s/autoconfig",
    }
}

/// Message of a metadata file missing for a repository.
///
/// Quirk: the `serverinfo` text puts the repository name on its own line.
// Mirrors exception/repository/{Sync,ServerInfo,Changelogs,Events,AutoConfig}FileNotFound*.java
pub fn repository_file_not_found(file: RemoteA3sFile, repository_name: &str) -> String {
    match file {
        RemoteA3sFile::Sync => {
            format!("File /.a3s/sync not found on repository: {repository_name}")
        }
        RemoteA3sFile::ServerInfo => {
            format!("File /.a3s/serverinfo not found on repository: \n{repository_name}")
        }
        RemoteA3sFile::Changelogs => {
            format!("File /.a3s/changelogs not found on repository: {repository_name}")
        }
        RemoteA3sFile::Events => {
            format!("File /.a3s/events not found on repository: {repository_name}")
        }
        RemoteA3sFile::AutoConfig => {
            format!("File /.a3s/autoconfig not found on repository: {repository_name}")
        }
    }
}

/// Message of an unknown repository.
// Mirrors exception/repository/RepositoryNotFoundException.java
pub fn repository_not_found(repository_name: &str) -> String {
    format!("Repository {repository_name} not found!")
}

/// Message of a repository without a main folder location.
// Mirrors exception/repository/RepositoryMainFolderLocationNotFoundException.java
pub fn repository_main_folder_empty(repository_name: &str) -> String {
    format!("Repository {repository_name} main folder location is empty!")
}

/// Message of a missing default download location (Java ignores the repository name).
// Mirrors exception/repository/RepositoryDefaultDownloadLocationNotFoundException.java:7-8
pub const DEFAULT_DESTINATION_FOLDER_EMPTY: &str = "Default destination folder is empty!";

/// Message of a server info file that could not be reloaded; `: {details}` is appended only for
/// non-empty details.
// Mirrors exception/repository/RepositoryServerInfoReadException.java:11-17
pub fn server_info_read_failed(repository_name: &str, details: Option<&str>) -> String {
    match details {
        Some(details) if !details.is_empty() => {
            format!("Failed to reload server info for repository \"{repository_name}\": {details}")
        }
        _ => format!("Failed to reload server info for repository \"{repository_name}\""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_e1_verbatim_texts() {
        assert_eq!(WRONG_LOGIN_PASSWORD, "login or password wrong or missing");
        assert_eq!(
            SSL_CERTIFICATE_VALIDATION_FAILED,
            "Failed to validate SSL certificate"
        );
        assert_eq!(
            incomplete_transfer("a.pbo", 10, 20),
            "Incompete file size transfer: a.pbo Remote size: 20 Bytes, Actual size: 10 Bytes"
        );
        assert_eq!(
            cannot_create_directory(Path::new("/x/y")),
            "Cannot create directory /x/y\nPlease checkout file access permissions."
        );
        assert_eq!(
            cannot_delete_directory(Path::new("/x/y")),
            "Cannot delete directory /x/y\nPlease checkout file access permissions."
        );
        assert_eq!(
            remote_file_not_found(RemoteA3sFile::AutoConfig),
            "Remote file not found: /.a3s/autoconfig"
        );
        assert_eq!(
            remote_file_not_found(RemoteA3sFile::Changelogs),
            "Remote file not found: /.a3s/changelogs"
        );
        assert_eq!(
            remote_file_not_found(RemoteA3sFile::Events),
            "Remote file not found: /.a3s/events"
        );
        assert_eq!(
            remote_file_not_found(RemoteA3sFile::ServerInfo),
            "Remote file not found: .a3s/serverinfo"
        );
        assert_eq!(
            remote_file_not_found(RemoteA3sFile::Sync),
            "Remote file no found: /.a3s/sync"
        );
        assert_eq!(
            repository_file_not_found(RemoteA3sFile::AutoConfig, "WOG"),
            "File /.a3s/autoconfig not found on repository: WOG"
        );
        assert_eq!(
            repository_file_not_found(RemoteA3sFile::Changelogs, "WOG"),
            "File /.a3s/changelogs not found on repository: WOG"
        );
        assert_eq!(
            repository_file_not_found(RemoteA3sFile::Events, "WOG"),
            "File /.a3s/events not found on repository: WOG"
        );
        assert_eq!(
            repository_file_not_found(RemoteA3sFile::ServerInfo, "WOG"),
            "File /.a3s/serverinfo not found on repository: \nWOG"
        );
        assert_eq!(
            repository_file_not_found(RemoteA3sFile::Sync, "WOG"),
            "File /.a3s/sync not found on repository: WOG"
        );
        assert_eq!(repository_not_found("WOG"), "Repository WOG not found!");
        assert_eq!(
            repository_main_folder_empty("WOG"),
            "Repository WOG main folder location is empty!"
        );
        assert_eq!(
            DEFAULT_DESTINATION_FOLDER_EMPTY,
            "Default destination folder is empty!"
        );
        assert_eq!(
            server_info_read_failed("WOG", Some("bad header")),
            "Failed to reload server info for repository \"WOG\": bad header"
        );
        assert_eq!(
            server_info_read_failed("WOG", Some("")),
            "Failed to reload server info for repository \"WOG\""
        );
        assert_eq!(
            server_info_read_failed("WOG", None),
            "Failed to reload server info for repository \"WOG\""
        );
    }

    #[test]
    fn a_e2_connection_messages() {
        let core = "Failed to connect to repository WOG";
        assert_eq!(
            connection(core, ErrorKind::ConnectionFailed, Some("ignored")),
            format!("{core}\nConnection failed")
        );
        assert_eq!(
            connection(core, ErrorKind::ConnectTimeout, None),
            format!(
                "{core}\nConnection closed by ArmA3Sync.\nServer didn't respond, connection timeout reached"
            )
        );
        assert_eq!(
            connection(core, ErrorKind::ReadTimeout, None),
            format!(
                "{core}\nConnection closed by ArmA3Sync\nServer didn't respond, read timeout reached"
            )
        );
        assert_eq!(
            connection(core, ErrorKind::Timeout, Some("x")),
            format!(
                "{core}\nConnection closed by ArmA3Sync\nServer didn't respond, connection or read timeout reached"
            )
        );
        let incomplete = incomplete_transfer("a.pbo", 1, 2);
        assert_eq!(
            connection(core, ErrorKind::IncompleteTransfer, Some(&incomplete)),
            incomplete
        );
        assert_eq!(
            connection(core, ErrorKind::NotFound, Some("/a/b")),
            format!("{core}\n/a/b")
        );
        assert_eq!(connection(core, ErrorKind::NotFound, None), core);
        assert_eq!(
            connection(core, ErrorKind::Unauthorized, Some(WRONG_LOGIN_PASSWORD)),
            format!("{core}\nlogin or password wrong or missing")
        );
        assert_eq!(
            connection(core, ErrorKind::Io, Some("broken pipe")),
            format!("{core}\nbroken pipe")
        );
    }
}
