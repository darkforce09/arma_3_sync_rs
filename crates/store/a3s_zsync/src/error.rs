#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid .zsync file: {0}")]
    InvalidControlFile(String),

    #[error("reassembled file has SHA-1 {actual}, expected {expected}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("remote range {start}..{end} returned {got} bytes")]
    ShortRange { start: u64, end: u64, got: usize },

    #[error("cancelled")]
    Cancelled,

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
