//! Adversarial checks of `redact` (Java `DebugLogger.safe`) and `RotatingFile` edge cases.

use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;

use a3s_foundation::log::{RotatingFile, describe_endpoint, redact};
use tracing_subscriber::fmt::MakeWriter as _;

#[test]
fn verify_redact_java_regex_semantics() {
    let table: &[(&str, &str)] = &[
        // `[^&\s]+` needs one char; backtracking `\s*` cannot help.
        ("password = &x", "password = &x"),
        ("password=", "password="),
        ("password=\t", "password=\t"),
        // The separator may be followed by another separator, which is part of the value.
        ("pwd::x", "pwd=<redacted>"),
        ("token==a=b", "token=<redacted>"),
        // Keyword inside a longer word, and keyword repeated.
        ("passwordpassword=1", "passwordpassword=<redacted>"),
        (
            "tokentoken:1&secret=2",
            "tokentoken=<redacted>&secret=<redacted>",
        ),
        // Value consumes everything up to `&` or Java `\s`.
        ("token=a?b#c d", "token=<redacted> d"),
        ("PassWd = a b", "PassWd=<redacted> b"),
        // Java `\s` includes \x0B and \f.
        ("token=\u{0B}x", "token=<redacted>"),
        ("token\u{0C}=\u{0C}x\u{0C}y", "token=<redacted>\u{0C}y"),
        // Non-ASCII whitespace is not `\s` in Java: part of the value.
        ("token=\u{A0}x y", "token=<redacted> y"),
        ("secret:\u{85}", "secret=<redacted>"),
        // `(?i)` without UNICODE_CASE: the Kelvin sign is not `k`.
        ("to\u{212A}en=x", "to\u{212A}en=x"),
        // `passw0rd` is not a keyword; `pwd` inside `passwd` matches only after `passwd` fails.
        ("passw0rd=x", "passw0rd=x"),
        ("passwd x pwd=1", "passwd x pwd=<redacted>"),
        // Multi-byte text around matches stays intact.
        ("日本 token=秘密&ok", "日本 token=<redacted>&ok"),
    ];
    for &(input, expected) in table {
        assert_eq!(redact(Some(input)), expected, "redact({input:?})");
    }
}

#[test]
fn verify_redact_credentials_separator() {
    let table: &[(&str, &str)] = &[
        ("@", "<credentials-redacted>@"),
        ("user@", "<credentials-redacted>@"),
        // Last `@` after the first `://`.
        ("a://b@c://d@e", "<credentials-redacted>@e"),
        // `@` before the scheme separator: untouched.
        ("x@y://z", "x@y://z"),
        // A line break is replaced before searching.
        ("ftp://u:p@\nhost", "<credentials-redacted>@ host"),
        ("\r", " "),
        // Credentials removed first, then keywords in the remainder.
        (
            "u@h/?token=abc",
            "<credentials-redacted>@h/?token=<redacted>",
        ),
        ("token=abc@host", "<credentials-redacted>@host"),
    ];
    for &(input, expected) in table {
        assert_eq!(redact(Some(input)), expected, "redact({input:?})");
    }
}

#[test]
fn verify_describe_endpoint_redacts_everything_but_protocol() {
    assert_eq!(
        describe_endpoint(Some("HTTPS"), None, None, Some("/a?pwd=x")),
        "protocol=HTTPS, host=<empty>, port=<empty>, remotePath=/a?pwd=<redacted>"
    );
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read_dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn verify_rotation_exact_limit_and_oversized_record() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("x.log");
    let mut file = RotatingFile::open(path.clone(), 10, 3).expect("open");
    // Exactly reaching the limit does not rotate.
    file.write_all(b"0123456789").expect("write");
    assert_eq!(names(tmp.path()), ["x.log"]);
    // The next byte would exceed it.
    file.write_all(b"a").expect("write");
    assert_eq!(names(tmp.path()), ["x.log", "x.log.1"]);
    assert_eq!(fs::read(&path).expect("read"), b"a");
    // A record larger than the limit is never split.
    file.write_all(b"BIGGER-THAN-TEN-BYTES").expect("write");
    assert_eq!(fs::read(&path).expect("read"), b"BIGGER-THAN-TEN-BYTES");
    assert_eq!(
        fs::read(tmp.path().join("x.log.1")).expect("read"),
        b"a",
        "previous file shifted"
    );
    assert_eq!(
        fs::read(tmp.path().join("x.log.2")).expect("read"),
        b"0123456789"
    );
    file.write_all(b"z").expect("write");
    assert_eq!(names(tmp.path()), ["x.log", "x.log.1", "x.log.2"]);
    assert_eq!(fs::read(&path).expect("read"), b"z");
    assert_eq!(
        fs::read(tmp.path().join("x.log.2")).expect("read"),
        b"a",
        "oldest dropped"
    );
}

#[test]
fn verify_rotation_of_preexisting_oversized_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("x.log");
    fs::write(&path, vec![b'o'; 500]).expect("write");
    fs::write(tmp.path().join("x.log.1"), "one").expect("write");
    fs::write(tmp.path().join("x.log.2"), "two").expect("write");
    let mut file = RotatingFile::open(path.clone(), 100, 3).expect("open");
    file.write_all(b"new").expect("write");
    assert_eq!(fs::read(&path).expect("read"), b"new");
    assert_eq!(
        fs::read(tmp.path().join("x.log.1")).expect("read"),
        vec![b'o'; 500]
    );
    assert_eq!(
        fs::read_to_string(tmp.path().join("x.log.2")).expect("read"),
        "one"
    );
    assert_eq!(names(tmp.path()), ["x.log", "x.log.1", "x.log.2"]);
}

#[test]
fn verify_rotation_survives_deleted_directory() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("logs");
    let mut file = RotatingFile::open(dir.join("x.log"), 8, 3).expect("open");
    file.write_all(b"12345678").expect("write");
    fs::remove_dir_all(&dir).expect("remove");
    // Rotation and writes fail silently; never an error or a panic.
    for _ in 0..5 {
        file.write_all(b"abcdefgh").expect("swallowed");
        file.flush().expect("flush");
    }
}

#[test]
fn verify_rotation_keep_zero_behaves_like_one() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("x.log");
    let mut file = RotatingFile::open(path.clone(), 4, 0).expect("open");
    file.write_all(b"1234").expect("write");
    file.write_all(b"5").expect("write");
    assert_eq!(names(tmp.path()), ["x.log"]);
    assert_eq!(fs::read(&path).expect("read"), b"5");
}

#[test]
fn verify_concurrent_records_are_never_interleaved() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("x.log");
    let file = Arc::new(RotatingFile::open(path.clone(), 1 << 30, 3).expect("open"));
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let file = Arc::clone(&file);
            std::thread::Builder::new()
                .name(format!("writer-{t}"))
                .spawn(move || {
                    for i in 0..200 {
                        let line = format!("thread-{t}-record-{i:04}-{}\n", "x".repeat(64));
                        file.make_writer()
                            .write_all(line.as_bytes())
                            .expect("write");
                    }
                })
                .expect("spawn")
        })
        .collect();
    for thread in threads {
        thread.join().expect("join");
    }
    let contents = fs::read_to_string(&path).expect("read");
    let lines: Vec<&str> = contents.lines().collect();
    assert_eq!(lines.len(), 8 * 200);
    for line in lines {
        assert!(
            line.starts_with("thread-") && line.ends_with(&"x".repeat(64)),
            "{line:?}"
        );
    }
}

#[test]
fn verify_concurrent_rotation_keeps_at_most_three_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("x.log");
    let file = Arc::new(RotatingFile::open(path.clone(), 256, 3).expect("open"));
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let file = Arc::clone(&file);
            std::thread::Builder::new()
                .name(format!("writer-{t}"))
                .spawn(move || {
                    for i in 0..100 {
                        let line = format!("t{t}-{i:04}\n");
                        file.make_writer()
                            .write_all(line.as_bytes())
                            .expect("write");
                    }
                })
                .expect("spawn")
        })
        .collect();
    for thread in threads {
        thread.join().expect("join");
    }
    assert_eq!(names(tmp.path()), ["x.log", "x.log.1", "x.log.2"]);
    for name in names(tmp.path()) {
        let contents = fs::read_to_string(tmp.path().join(&name)).expect("read");
        assert!(contents.len() <= 256, "{name}: {}", contents.len());
        assert!(
            contents
                .lines()
                .all(|line| line.starts_with('t') && line.len() == 7),
            "{name}: {contents:?}"
        );
    }
}
