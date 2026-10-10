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

#[cfg(test)]
mod tests {
    use super::join_remote;

    #[test]
    fn joins() {
        assert_eq!(join_remote("/Repo/WOG", ".a3s/sync"), "/Repo/WOG/.a3s/sync");
        assert_eq!(join_remote("", ".a3s/sync"), "/.a3s/sync");
        assert_eq!(join_remote("/a/", "/b//c"), "/a/b/c");
        assert_eq!(join_remote("", ""), "/");
    }
}
