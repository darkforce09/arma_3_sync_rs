use std::io;
use std::path::{Component, Path, PathBuf};

/// Java `Path.toAbsolutePath().normalize()`: joined to the current directory when relative,
/// then `.` and `name/..` removed lexically (no symbolic link is resolved; `..` above the root
/// stays at the root).
pub(crate) fn absolute_normalized(path: &Path) -> io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                }
            }
            other => out.push(other),
        }
    }
    Ok(out)
}
