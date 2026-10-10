//! Remote path formatting (`dao/connection/RemoteFile.java`).

/// A file or folder on the repository server, relative to the protocol's remote path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFile {
    /// File or folder name.
    pub filename: String,

    /// Parent folder below the repository root; leading and trailing `/` are ignored.
    pub parent_relative_path: String,

    /// A folder (gets a trailing `/` in [`Self::relative_file_path`]).
    pub is_directory: bool,
}

impl RemoteFile {
    /// A remote file named `filename` in `parent_relative_path`.
    // Mirrors dao/connection/RemoteFile.java:9-16
    pub fn new(
        filename: impl Into<String>,
        parent_relative_path: impl Into<String>,
        is_directory: bool,
    ) -> Self {
        Self {
            filename: filename.into(),
            parent_relative_path: parent_relative_path.into(),
            is_directory,
        }
    }

    /// `/parent/name` (or `/name`), with a trailing `/` for folders. Used in messages.
    // Mirrors dao/connection/RemoteFile.java:38-65
    pub fn relative_file_path(&self) -> String {
        let parent = self.normalized_parent();
        let mut path = if parent.is_empty() {
            format!("/{}", self.filename)
        } else {
            format!("/{parent}/{}", self.filename)
        };
        if self.is_directory {
            path.push('/');
        }
        path
    }

    /// `parent/name` without leading or trailing `/`, as passed to
    /// [`a3s_transport::Transport::fetch`].
    pub fn transport_path(&self) -> String {
        let parent = self.normalized_parent();
        if parent.is_empty() {
            self.filename.clone()
        } else {
            format!("{parent}/{}", self.filename)
        }
    }

    /// The parent without one leading and one trailing `/`.
    fn normalized_parent(&self) -> &str {
        let parent = self.parent_relative_path.as_str();
        let parent = parent.strip_prefix('/').unwrap_or(parent);
        parent.strip_suffix('/').unwrap_or(parent)
    }
}

#[cfg(test)]
mod tests {
    use super::RemoteFile;

    #[test]
    fn a12_relative_file_path_table() {
        let table = [
            (("sync", ".a3s", false), "/.a3s/sync", ".a3s/sync"),
            (("autoconfig", "", false), "/autoconfig", "autoconfig"),
            (("x", "/a/b/", false), "/a/b/x", "a/b/x"),
            (("d", "a", true), "/a/d/", "a/d"),
            (("x", "/", false), "/x", "x"),
        ];
        for ((name, parent, is_directory), relative, transport) in table {
            let file = RemoteFile::new(name, parent, is_directory);
            assert_eq!(file.relative_file_path(), relative, "{file:?}");
            assert_eq!(file.transport_path(), transport, "{file:?}");
        }
    }
}
