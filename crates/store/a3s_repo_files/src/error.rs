use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("decompression failed: {0}")]
    Gzip(#[source] std::io::Error),

    #[error("decompressed data exceeds {0} bytes")]
    TooLarge(u64),

    #[error(transparent)]
    Types(#[from] a3s_types::Error),

    #[error(transparent)]
    Serialization(#[from] a3s_jserial::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub(crate) trait IoContext<T> {
    fn with_path(self, path: &std::path::Path) -> Result<T>;
}

impl<T> IoContext<T> for std::io::Result<T> {
    fn with_path(self, path: &std::path::Path) -> Result<T> {
        self.map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })
    }
}
