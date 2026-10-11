use std::io;
use std::path::PathBuf;

/// Why a preset could not be imported.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// The path is missing or not a regular file.
    #[error("Preset file does not exist or is not a file: {}", .0.display())]
    NotAFile(PathBuf),

    /// The preset could not be read as UTF-8.
    #[error("Could not read {}: {source}", path.display())]
    ReadPreset {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// The preset is empty or only whitespace.
    #[error("The preset file is empty.")]
    Empty,

    /// The preset has no Workshop mod and no CDLC row.
    #[error(
        "No Arma 3 Launcher Workshop mod or CDLC entries were found in the selected HTML file."
    )]
    NoEntries,
}

/// Why an exported preset could not be written.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    /// The export result has errors or no HTML.
    #[error("Only a valid export result can be written.")]
    InvalidResult,

    /// The HTML does not end the document.
    #[error("The generated preset is incomplete.")]
    Incomplete,

    /// After the rename the target is missing, not a file or empty.
    #[error("The preset file was not created or is empty: {}", .0.display())]
    Empty(PathBuf),

    /// The written file does not end the document.
    #[error("The preset file failed the integrity check: {}", .0.display())]
    Integrity(PathBuf),

    /// Creating the folder, writing, renaming or re-reading failed.
    #[error(transparent)]
    Io(#[from] io::Error),
}
