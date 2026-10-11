//! The download report shown after a synchronization.

use a3s_foundation::format;

use crate::events::{EventError, Outcome};
use crate::model::DownloadTask;
use crate::plan::{ReportTotals, resumed_size};

/// What a report is made of.
#[derive(Clone, Copy, Debug)]
pub struct ReportInput<'a> {
    /// The repository name.
    pub repository_name: &'a str,

    /// The repository URL (`a3s_types::local::protocol::host_url`).
    pub repository_url: &'a str,

    /// When the download finished, already formatted.
    pub finished_on: &'a str,

    /// Average download speed in bytes per second; 0 when unknown.
    pub average_speed: u64,

    /// Most connections active at once.
    pub max_active_connections: u32,

    /// The download tasks of the session, with their final statuses.
    pub tasks: &'a [DownloadTask],
}

/// One block of transfer statistics. Sizes are signed: Java computes them as `long`, and a
/// negative saving prints as such.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransferStatistics {
    /// Number of files (the global block counts folders too, as Java does).
    pub files: usize,

    /// Bytes on disk once synchronized.
    pub disk_size: i64,

    /// Bytes downloaded.
    pub downloaded: i64,

    /// `disk_size - downloaded`.
    pub saved: i64,

    /// `saved * 100 / disk_size`, 0 when `disk_size` is 0, with Java's `long` wrap-around and
    /// `(int)` cast.
    pub saved_percent: i32,
}

impl TransferStatistics {
    fn new(files: usize, disk_size: u64, downloaded: u64) -> Self {
        let disk_size = disk_size as i64;
        let downloaded = downloaded as i64;
        let saved = disk_size.wrapping_sub(downloaded);
        // Mirrors service/synchronization/FilesSynchronizationReportManager.java:57-61
        let saved_percent = if disk_size == 0 {
            0
        } else {
            (saved.wrapping_mul(100) / disk_size) as i32
        };
        Self {
            files,
            disk_size,
            downloaded,
            saved,
            saved_percent,
        }
    }
}

/// The three statistics blocks of a successful download.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReportStatistics {
    /// Every task: disk size of the files against the size of those downloaded (`Done`).
    pub global: TransferStatistics,

    /// zsync-patched files: their size against the bytes expected to transfer.
    pub partial: TransferStatistics,

    /// Compressed files: uncompressed against compressed size.
    pub compressed: TransferStatistics,
}

impl ReportStatistics {
    /// The statistics of `tasks`.
    // Mirrors service/synchronization/FilesSynchronizationReportManager.java:52-94,
    // service/synchronization/FilesSynchronizationManager.java:105-134,183-196
    pub fn compute(tasks: &[DownloadTask]) -> Self {
        let totals = ReportTotals::compute(tasks);
        Self {
            global: TransferStatistics::new(
                tasks.len(),
                totals.disk_files_size,
                resumed_size(tasks),
            ),
            partial: TransferStatistics::new(
                totals.incomplete_files,
                totals.incomplete_disk_size,
                totals.incomplete_expected_size,
            ),
            compressed: TransferStatistics::new(
                totals.compressed_files,
                totals.uncompressed_files_size,
                totals.compressed_files_size,
            ),
        }
    }
}

/// The report of a successful download.
// Mirrors service/synchronization/FilesSynchronizationReportManager.java:37-107
pub fn success_report(input: &ReportInput<'_>) -> String {
    let message = Outcome::Success
        .download_report_message()
        .unwrap_or_default();
    let speed = if input.average_speed == 0 {
        "unavailable".to_owned()
    } else {
        format::speed(input.average_speed as i64)
    };
    let statistics = ReportStatistics::compute(input.tasks);
    let block = |title: &str, s: &TransferStatistics| {
        format!(
            "{title}:\n- Number of files updated: {}\n- Total files size on disk: {}\n\
             - Downloaded data: {}\n- Saved: {} ({}%)",
            s.files,
            format::size(s.disk_size),
            format::size(s.downloaded),
            format::size(s.saved),
            s.saved_percent
        )
    };
    format!(
        "{}\n\n{message}\n\nServer connection:\n- Average download speed: {speed}\n\
         - Number of active connections used: {}\n\n{}\n\n{}\n\n{}",
        header(input),
        input.max_active_connections,
        block("Global file transfer", &statistics.global),
        block("Partial file transfer", &statistics.partial),
        block("Compressed file transfer", &statistics.compressed),
    )
}

/// The report of a failed download: `message`, then one line per error.
// Mirrors service/synchronization/FilesSynchronizationReportManager.java:109-136
pub fn error_report(input: &ReportInput<'_>, message: &str, errors: &[EventError]) -> String {
    let mut report = format!("{}\n\n{message}", header(input));
    for error in errors {
        report.push_str("\n- ");
        report.push_str(&error.message);
    }
    report
}

/// The report a download run stores for `outcome`: none after a cancel.
// Mirrors service/synchronization/FilesSynchronizationProcessor.java:314-332
pub fn report_for(input: &ReportInput<'_>, outcome: &Outcome) -> Option<String> {
    let message = outcome.download_report_message()?;
    Some(match outcome {
        Outcome::Success => success_report(input),
        Outcome::CompletedWithErrors { errors } | Outcome::TooManyErrors { errors, .. } => {
            error_report(input, &message, errors)
        }
        Outcome::Failed { error } => error_report(input, &message, std::slice::from_ref(error)),
        Outcome::Cancelled => return None,
    })
}

fn header(input: &ReportInput<'_>) -> String {
    format!(
        "--- Download report ---\nRepository name: {}\nRepository url: {}\nDownload finished on: {}",
        input.repository_name, input.repository_url, input.finished_on
    )
}
