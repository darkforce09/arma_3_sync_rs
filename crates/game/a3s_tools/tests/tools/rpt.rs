//! RPT tailing: appends, line terminators, partial lines, truncation, rotation, missing files,
//! preload hand-over and the background thread.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use a3s_tools::rpt::{LogTailer, RptEvent, RptTail, THREAD_NAME, TailUpdate, preload_text};

fn append(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
}

fn lines(update: &TailUpdate) -> Vec<&str> {
    update.lines.iter().map(String::as_str).collect()
}

#[test]
fn appended_lines_are_read_once_from_the_end() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("arma3.rpt");
    fs::write(&path, "old\n").unwrap();
    let mut tailer = LogTailer::at_end(&path);

    append(&path, b"l1\nl2\n");
    let update = tailer.poll().unwrap();
    assert!(!update.reset);
    assert_eq!(lines(&update), ["l1", "l2"]);
    assert_eq!(tailer.poll().unwrap(), TailUpdate::default());
}

#[test]
fn crlf_and_lone_cr_end_lines() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("arma3.rpt");
    fs::write(&path, "").unwrap();
    let mut tailer = LogTailer::at_end(&path);

    append(&path, b"a\r\nb\rc\n");
    assert_eq!(lines(&tailer.poll().unwrap()), ["a", "b", "c"]);

    // A CRLF split across two writes is one terminator.
    append(&path, b"d\r");
    assert_eq!(lines(&tailer.poll().unwrap()), ["d"]);
    append(&path, b"\ne\n");
    assert_eq!(lines(&tailer.poll().unwrap()), ["e"]);
}

#[test]
fn a_partial_line_waits_for_its_end_or_for_a_quiet_poll() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("arma3.rpt");
    fs::write(&path, "").unwrap();
    let mut tailer = LogTailer::at_end(&path);

    append(&path, b"par");
    assert!(tailer.poll().unwrap().lines.is_empty());
    append(&path, b"tial\n");
    assert_eq!(lines(&tailer.poll().unwrap()), ["partial"]);

    append(&path, b"frag");
    assert!(tailer.poll().unwrap().lines.is_empty());
    assert_eq!(lines(&tailer.poll().unwrap()), ["frag"]);
    assert!(tailer.poll().unwrap().lines.is_empty());
}

#[test]
fn a_truncated_file_is_read_from_its_start() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("arma3.rpt");
    fs::write(&path, "0123456789").unwrap();
    let mut tailer = LogTailer::at_offset(&path, 10);

    fs::write(&path, "new\n").unwrap();
    let update = tailer.poll().unwrap();

    assert!(update.reset);
    assert_eq!(lines(&update), ["new"]);
}

#[cfg(unix)]
#[test]
fn a_rotated_file_is_read_from_its_start_even_when_longer() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("arma3.rpt");
    fs::write(&path, "old\n").unwrap();
    let mut tailer = LogTailer::at_end(&path);

    fs::rename(&path, temp.path().join("arma3.rpt.1")).unwrap();
    fs::write(&path, "first\nsecond\n").unwrap();
    let update = tailer.poll().unwrap();

    assert!(update.reset);
    assert_eq!(lines(&update), ["first", "second"]);
}

#[test]
fn a_missing_file_is_read_from_its_start_once_it_appears() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("arma3.rpt");
    let mut tailer = LogTailer::at_end(&path);

    assert_eq!(tailer.poll().unwrap(), TailUpdate::default());
    fs::write(&path, "x\n").unwrap();
    let update = tailer.poll().unwrap();

    assert!(!update.reset);
    assert_eq!(lines(&update), ["x"]);
}

#[test]
fn preload_hands_over_at_the_exact_offset_and_decodes_utf8() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("arma3.rpt");
    fs::write(&path, "a\nb").unwrap();

    let (text, mut tailer) = LogTailer::preload(&path).unwrap();
    assert_eq!(text.as_deref(), Some("a\nb\n"));
    append(&path, b"\nc\n");
    // The preload already showed `b` as a line, so its late terminator adds nothing.
    assert_eq!(lines(&tailer.poll().unwrap()), ["c"]);

    // A character split across two writes decodes once complete.
    let euro = "\u{20ac}".as_bytes();
    append(&path, &euro[..1]);
    assert!(tailer.poll().unwrap().lines.is_empty());
    append(&path, &euro[1..]);
    append(&path, b"\n");
    assert_eq!(lines(&tailer.poll().unwrap()), ["\u{20ac}"]);

    assert_eq!(preload_text(b""), None);
    assert_eq!(preload_text(b"\xff\n").as_deref(), Some("\u{fffd}\n"));

    let empty = temp.path().join("empty.rpt");
    fs::write(&empty, "").unwrap();
    assert_eq!(LogTailer::preload(&empty).unwrap().0, None);
}

#[test]
fn the_tail_thread_preloads_follows_and_stops() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("arma3.rpt");
    fs::write(&path, "start\n").unwrap();
    let (sender, receiver) = mpsc::channel();

    let tail = RptTail::spawn(path.clone(), Duration::from_millis(20), move |event| {
        let thread = std::thread::current().name().map(str::to_owned);
        // The receiver outlives the thread in this test.
        let _sent = sender.send((thread, event));
    })
    .unwrap();

    let (thread, event) = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(thread.as_deref(), Some(THREAD_NAME));
    assert_eq!(event, RptEvent::Preloaded("start\n".to_owned()));

    append(&path, b"next\n");
    let (_, event) = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(event, RptEvent::Lines(vec!["next".to_owned()]));

    let stopping = Instant::now();
    tail.stop();
    assert!(stopping.elapsed() < Duration::from_millis(500));
    // The joined thread dropped the sink and with it the sender.
    assert!(matches!(
        receiver.recv_timeout(Duration::from_secs(1)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    ));
}

#[test]
fn dropping_the_tail_stops_its_thread() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("arma3.rpt");
    let (sender, receiver) = mpsc::channel::<RptEvent>();

    let tail = RptTail::spawn(path, Duration::from_secs(60), move |event| {
        let _sent = sender.send(event);
    })
    .unwrap();
    let stopping = Instant::now();
    drop(tail);

    assert!(stopping.elapsed() < Duration::from_millis(500));
    assert!(matches!(
        receiver.recv_timeout(Duration::from_secs(1)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    ));
}
