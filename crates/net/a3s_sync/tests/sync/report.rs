//! Download report statistics and which report a run stores.

use std::path::PathBuf;

use a3s_foundation::error::ErrorKind;
use a3s_sync::report::{ReportInput, ReportStatistics, TransferStatistics, report_for};
use a3s_sync::{DownloadStatus, DownloadTask, DownloadTaskKind, EventError, Outcome};

fn folder(relative: &str) -> DownloadTask {
    DownloadTask {
        relative_path: relative.to_owned(),
        destination_dir: PathBuf::from("/d"),
        kind: DownloadTaskKind::Directory,
        status: DownloadStatus::Done,
    }
}

fn file(relative: &str, size: u64, compressed_size: Option<u64>, complete: f64) -> DownloadTask {
    DownloadTask {
        relative_path: relative.to_owned(),
        destination_dir: PathBuf::from("/d"),
        kind: DownloadTaskKind::File {
            size,
            compressed: compressed_size.is_some(),
            compressed_size: compressed_size.unwrap_or(0),
            sha1: None,
            complete,
        },
        status: DownloadStatus::Done,
    }
}

fn stats(files: usize, disk_size: i64, downloaded: i64, saved_percent: i32) -> TransferStatistics {
    TransferStatistics {
        files,
        disk_size,
        downloaded,
        saved: disk_size - downloaded,
        saved_percent,
    }
}

#[test]
fn statistics_of_a_mixed_download() {
    let tasks = [
        folder("@a"),
        folder("@a/addons"),
        file("@a/addons/plain.pbo", 1_000_000, None, 0.0),
        file("@a/addons/packed.pbo", 4_000_000, Some(1_500_000), 0.0),
        file("@a/addons/patched.pbo", 2_000_000, None, 75.0),
    ];
    let statistics = ReportStatistics::compute(&tasks);
    // Folders count as updated files; downloaded = what the `Done` files transferred.
    assert_eq!(statistics.global, stats(5, 7_000_000, 3_000_000, 57));
    assert_eq!(statistics.partial, stats(1, 2_000_000, 500_000, 75));
    assert_eq!(statistics.compressed, stats(1, 4_000_000, 1_500_000, 62));
}

#[test]
fn statistics_edge_cases() {
    assert_eq!(ReportStatistics::compute(&[]), ReportStatistics::default());

    // A compressed file larger than its content: a negative saving, as in Java.
    let statistics = ReportStatistics::compute(&[file("x", 100, Some(150), 0.0)]);
    assert_eq!(statistics.compressed, stats(1, 100, 150, -50));
    assert_eq!(statistics.global, stats(1, 100, 150, -50));

    // Files not downloaded yet count nothing as downloaded.
    let mut pending = file("y", 1000, None, 0.0);
    pending.status = DownloadStatus::Running;
    assert_eq!(
        ReportStatistics::compute(&[pending]).global,
        stats(1, 1000, 0, 100)
    );
}

#[test]
fn stored_report_by_outcome() {
    let tasks = [file("x", 10, None, 0.0)];
    let input = ReportInput {
        repository_name: "repo",
        repository_url: "http://127.0.0.1:8080/repo",
        finished_on: "Oct 11, 2026, 2:05:09 PM",
        average_speed: 0,
        max_active_connections: 1,
        tasks: &tasks,
    };
    assert_eq!(report_for(&input, &Outcome::Cancelled), None);
    let success = report_for(&input, &Outcome::Success).expect("success report");
    assert!(success.contains(input.repository_url));

    let errors = vec![
        EventError::new(ErrorKind::NotFound, "first\nline"),
        EventError::new(ErrorKind::InvalidData, "second"),
    ];
    for outcome in [
        Outcome::CompletedWithErrors {
            errors: errors.clone(),
        },
        Outcome::TooManyErrors {
            limit: 10,
            errors: errors.clone(),
        },
    ] {
        let report = report_for(&input, &outcome).expect("error report");
        // The success report's header, a blank line, the headline, then one entry per error
        // and nothing else: no statistics block.
        let header: Vec<&str> = success.lines().take(5).collect();
        let lines: Vec<&str> = report.lines().collect();
        assert_eq!(lines[..5], header[..]);
        let error_lines: usize = errors.iter().map(|e| e.message.lines().count()).sum();
        assert_eq!(lines.len(), 6 + error_lines, "{report}");
        let entries: Vec<&str> = report.split("\n- ").skip(1).collect();
        let messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
        assert_eq!(entries, messages);
    }
    let failed = Outcome::Failed {
        error: errors[1].clone(),
    };
    let report = report_for(&input, &failed).expect("error report");
    assert!(report.contains(&errors[1].message));
    assert!(!report.contains(&errors[0].message));
}
