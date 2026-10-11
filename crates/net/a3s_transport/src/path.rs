//! Remote path helpers.

use crate::{Error, Result};

/// Joins the protocol's remote path with a path relative to the repository root.
///
/// `("/Repo/WOG", ".a3s/sync")` → `/Repo/WOG/.a3s/sync`.
pub fn join_remote(remote_path: &str, relative: &str) -> String {
    let mut out = String::with_capacity(remote_path.len() + relative.len() + 2);
    for segment in remote_path.split('/').chain(relative.split('/')) {
        if !segment.is_empty() {
            out.push('/');
            out.push_str(segment);
        }
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

/// Non-empty path segments of `path`.
pub fn segments(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|s| !s.is_empty())
}

/// Checks that a repository-relative path is safe to send to a server.
///
/// `\` is treated as `/`; empty segments and a leading `/` are allowed. Rejected with
/// [`Error::InvalidPath`]: `..` and `.` segments, NUL, CR and LF (FTP command injection).
// Mirrors dao/connection/sftp/SftpDAO.java:451-457 (extended to all protocols and to `.`).
pub fn validate_relative(relative: &str) -> Result<()> {
    normalize_relative(relative).map(|_| ())
}

/// Validates `relative` ([`validate_relative`]) and returns its canonical form: non-empty
/// segments joined with `/`, without leading or trailing `/` (`""` for the root).
pub(crate) fn normalize_relative(relative: &str) -> Result<String> {
    if relative.contains(['\0', '\r', '\n']) {
        return Err(Error::InvalidPath(relative.escape_debug().to_string()));
    }
    let replaced = relative.replace('\\', "/");
    let mut out = String::with_capacity(replaced.len());
    for segment in segments(&replaced) {
        if segment == ".." || segment == "." {
            return Err(Error::InvalidPath(relative.to_owned()));
        }
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(segment);
    }
    Ok(out)
}

/// Splits a canonical relative path into `(parent, name)`; the parent of a top-level entry
/// is `""`.
pub(crate) fn split_parent(normalized: &str) -> (&str, &str) {
    normalized.rsplit_once('/').unwrap_or(("", normalized))
}

/// Joins two canonical relative paths.
pub(crate) fn join_relative(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::{join_remote, normalize_relative, split_parent, validate_relative};
    use crate::Error;

    #[test]
    fn joins() {
        assert_eq!(join_remote("/Repo/WOG", ".a3s/sync"), "/Repo/WOG/.a3s/sync");
        assert_eq!(join_remote("", ".a3s/sync"), "/.a3s/sync");
        assert_eq!(join_remote("/a/", "/b//c"), "/a/b/c");
        assert_eq!(join_remote("", ""), "/");
    }

    #[test]
    fn validate_relative_cases() {
        for ok in [
            "a/b",
            "/a//b",
            "@ace/addons/x.pbo",
            "a..b",
            "",
            "/",
            ".a3s/sync",
        ] {
            assert!(validate_relative(ok).is_ok(), "{ok:?} should be accepted");
        }
        for bad in [
            "..",
            "a/../b",
            "a\\..\\b",
            "./a",
            "a/./b",
            "a\0b",
            "a\r\nDELE x",
        ] {
            assert!(
                matches!(validate_relative(bad), Err(Error::InvalidPath(_))),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn normalize_relative_cases() {
        assert_eq!(normalize_relative("/a//b/").unwrap(), "a/b");
        assert_eq!(normalize_relative("a\\b\\c.pbo").unwrap(), "a/b/c.pbo");
        assert_eq!(normalize_relative("/").unwrap(), "");
        assert_eq!(split_parent("a/b/c"), ("a/b", "c"));
        assert_eq!(split_parent("c"), ("", "c"));
    }
}
