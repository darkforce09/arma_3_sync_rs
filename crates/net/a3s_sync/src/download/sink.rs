//! Hand-off of downloaded `.zip` files to the unzipper (WP10).

use std::path::PathBuf;

use parking_lot::Mutex;

/// Receives every finished `.zip` as soon as it is complete.
///
/// Java hands `.pbo.zip` files to `UnZipFlowProcessor.unZipAsynchronously`
/// (`service/connection/ConnectionDownloadProcessor.java:98-104`). Here every compressed leaf is
/// handed over, `.ebo.zip` included (DV9).
pub trait ZipSink: Send + Sync {
    /// Called once per finished `.zip`, from a worker. Must not block.
    fn submit(&self, zip: DownloadedZip);

    /// Whether no unzip is in progress (Java `UnZipFlowProcessor.uncompressionIsFinished`).
    /// While it returns `false` the connection-waiting counter stays at 0.
    fn is_idle(&self) -> bool {
        true
    }
}

/// A downloaded `.zip` whose content replaces [`DownloadedZip::target_path`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadedZip {
    /// Tree path of the leaf, e.g. `@ace/addons/ace_main.pbo`.
    pub relative_path: String,

    /// The downloaded archive: `destination_dir/"{name}.zip"`.
    pub zip_path: PathBuf,

    /// The file to extract: `destination_dir/name`.
    pub target_path: PathBuf,

    /// Remote SHA-1 of the uncompressed file.
    pub sha1: Option<String>,

    /// Uncompressed size in bytes.
    pub size: u64,
}

/// A sink that ignores every submission.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoZipSink;

impl ZipSink for NoZipSink {
    fn submit(&self, _zip: DownloadedZip) {}
}

/// A sink that collects submissions, for tests and simple callers.
#[derive(Debug, Default)]
pub struct CollectZips {
    zips: Mutex<Vec<DownloadedZip>>,
}

impl CollectZips {
    /// An empty collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Removes and returns the zips received so far, in submission order.
    pub fn take(&self) -> Vec<DownloadedZip> {
        std::mem::take(&mut *self.zips.lock())
    }
}

impl ZipSink for CollectZips {
    fn submit(&self, zip: DownloadedZip) {
        self.zips.lock().push(zip);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn a16_sink_is_object_safe() {
        let collect = Arc::new(CollectZips::new());
        let sink: Arc<dyn ZipSink> = collect.clone();
        let zip = DownloadedZip {
            relative_path: "@a/x.pbo".to_owned(),
            zip_path: PathBuf::from("/d/x.pbo.zip"),
            target_path: PathBuf::from("/d/x.pbo"),
            sha1: Some("abc".to_owned()),
            size: 3,
        };
        sink.submit(zip.clone());
        assert!(sink.is_idle());
        assert_eq!(collect.take(), vec![zip.clone()]);
        assert!(collect.take().is_empty());
        let none: Arc<dyn ZipSink> = Arc::new(NoZipSink);
        none.submit(zip);
        assert!(none.is_idle());
    }
}
