// Its own test binary: it holds its own harness lock, separate from the one in `jdk.rs`.
//! Adversarial tests for the harness wrapper and the generated fixtures.
//!
//! The JDK-backed tests are ignored like `jdk.rs`:
//! `source scripts/java_env.sh && cargo test -p a3s_dev_tools --test verify_harness -- --ignored --test-threads=1`.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;

use a3s_dev_tools::fixtures::{self, GENERATED_ENTRIES, TreeDiff, compare_trees};
use a3s_dev_tools::{DumpMode, Harness, HarnessError, Part, VerifyReport};
use common::{Dump, Json};

static LOCK: parking_lot::Mutex<()> = parking_lot::const_mutex(());

fn harness() -> Harness {
    Harness::locate().unwrap()
}

fn generate(parts: &[Part]) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    fixtures::regenerate(&harness(), &out, parts).unwrap();
    (tmp, out)
}

fn gunzip(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .unwrap();
    out
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    out.sort();
    out
}

/// Places `source` at `<dir>/.a3s/<name>` and returns the new path.
fn as_a3s(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let a3s = dir.join(".a3s");
    fs::create_dir_all(&a3s).unwrap();
    let path = a3s.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

// ------------------------------------------------------------------------------- no JDK

#[test]
fn report_parse_keeps_unicode_and_spaces_in_paths() {
    let stdout = "OK\t/r/Dossier é ü/.a3s/sync\tfr.soe.a3s.domain.repository.SyncTreeDirectory\n\
                  SKIP\t/r/Événement ü.txt\n\
                  SUMMARY\tok=1\tfail=0\tskip=1\n";
    let report = VerifyReport::parse(stdout).unwrap();
    assert_eq!(report.ok[0].path, PathBuf::from("/r/Dossier é ü/.a3s/sync"));
    assert_eq!(report.skipped[0], PathBuf::from("/r/Événement ü.txt"));
}

#[test]
fn report_parse_rejects_truncated_lines_and_bad_summary() {
    for bad in [
        "OK\t/a\nSUMMARY\tok=1\tfail=0\tskip=0\n",
        "FAIL\nSUMMARY\tok=0\tfail=1\tskip=0\n",
        "SKIP\nSUMMARY\tok=0\tfail=0\tskip=1\n",
        "SUMMARY\tok=0\tfail=0\n",
        "SUMMARY\tok=0\tfail=0\tskip=0\textra=1\n",
        "SUMMARY\tok=-1\tfail=0\tskip=0\n",
        "SUMMARY\tok=0\tfail=0\tskip=0\nOK\t/a\tX\n",
        "",
    ] {
        assert!(
            matches!(VerifyReport::parse(bad), Err(HarnessError::Protocol(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn compare_trees_reports_file_versus_directory_conflicts() {
    let tmp = tempfile::tempdir().unwrap();
    let (expected, actual) = (tmp.path().join("e"), tmp.path().join("a"));
    fs::create_dir_all(&expected).unwrap();
    fs::create_dir_all(actual.join("x")).unwrap();
    fs::write(expected.join("x"), b"file").unwrap();
    fs::write(actual.join("x/inner"), b"file").unwrap();
    // Empty file vs missing file, and a zero-length file on both sides.
    fs::write(expected.join("empty"), b"").unwrap();
    fs::write(actual.join("empty"), b"").unwrap();
    fs::write(actual.join("only_empty"), b"").unwrap();
    let diffs = compare_trees(&expected, &actual).unwrap();
    assert_eq!(
        diffs,
        vec![
            TreeDiff::Extra(PathBuf::from("only_empty")),
            TreeDiff::Missing(PathBuf::from("x")),
            TreeDiff::Extra(PathBuf::from("x/inner")),
        ]
    );
}

#[test]
fn compare_trees_on_two_missing_roots_is_empty() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(
        compare_trees(&tmp.path().join("nope1"), &tmp.path().join("nope2")).unwrap(),
        Vec::new()
    );
}

#[test]
fn install_with_an_incomplete_generation_touches_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let generated = tmp.path().join("generated");
    let fixtures_dir = tmp.path().join("fixtures");
    for entry in GENERATED_ENTRIES {
        if entry == "java_presets" {
            continue;
        }
        if entry.ends_with(".txt") {
            fs::create_dir_all(&generated).unwrap();
            fs::write(generated.join(entry), b"new").unwrap();
        } else {
            fs::create_dir_all(generated.join(entry)).unwrap();
            fs::write(generated.join(entry).join("f"), b"new").unwrap();
        }
    }
    fs::create_dir_all(fixtures_dir.join("java_local")).unwrap();
    fs::write(fixtures_dir.join("java_local/old"), b"old").unwrap();
    fs::write(fixtures_dir.join("java_descriptors.txt"), b"old").unwrap();
    assert!(fixtures::install(&generated, &fixtures_dir).is_err());
    assert_eq!(
        fs::read(fixtures_dir.join("java_local/old")).unwrap(),
        b"old"
    );
    assert_eq!(
        fs::read(fixtures_dir.join("java_descriptors.txt")).unwrap(),
        b"old"
    );
    // The generation was not consumed either.
    assert!(generated.join("java_local/f").is_file());
}

#[test]
fn install_replaces_an_entry_whose_type_changed() {
    let tmp = tempfile::tempdir().unwrap();
    let generated = tmp.path().join("generated");
    let fixtures_dir = tmp.path().join("fixtures");
    fs::create_dir_all(&generated).unwrap();
    for entry in GENERATED_ENTRIES {
        if entry.ends_with(".txt") {
            fs::write(generated.join(entry), b"new").unwrap();
        } else {
            fs::create_dir_all(generated.join(entry)).unwrap();
            fs::write(generated.join(entry).join("f"), b"new").unwrap();
        }
    }
    // Old fixtures: a directory where a file is generated, and a file where a directory is.
    fs::create_dir_all(fixtures_dir.join("java_descriptors.txt/stale")).unwrap();
    fs::write(fixtures_dir.join("java_built"), b"stale").unwrap();
    fs::create_dir_all(fixtures_dir.join("java_repos/wog_ftp")).unwrap();
    fs::write(fixtures_dir.join("java_repos/wog_ftp/sync"), b"keep").unwrap();
    fixtures::install(&generated, &fixtures_dir).unwrap();
    assert_eq!(
        fs::read(fixtures_dir.join("java_descriptors.txt")).unwrap(),
        b"new"
    );
    assert_eq!(fs::read(fixtures_dir.join("java_built/f")).unwrap(), b"new");
    assert_eq!(
        fs::read(fixtures_dir.join("java_repos/wog_ftp/sync")).unwrap(),
        b"keep"
    );
}

// ------------------------------------------------------------------------------- JDK

/// The harness claims locale independence, and verifiers will feed Rust output with modified-UTF-8
/// names (e.g. `é`) to `verify`. With `LANG=C` (or any locale that is not installed, such as
/// `de_DE.UTF-8` on this host) the JVM's `sun.jnu.encoding` is ASCII and
/// `-Dsun.jnu.encoding=UTF-8` in run.sh is ignored, so the harness crashes on such paths.
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn verify_handles_unicode_paths_in_any_locale() {
    let _guard = LOCK.lock();
    let (tmp, out) = generate(&[Part::Roundtrip]);
    let fresh = fs::read(out.join("java_roundtrip/serverinfo_hidden_2/fresh")).unwrap();
    let dir = tmp.path().join("Dossier é ü");
    let file = as_a3s(&dir, "serverinfo", &fresh);
    for locale in ["C.UTF-8", "C"] {
        let h = harness().env("LANG", locale).env("LC_ALL", locale);
        let report = h
            .verify(std::slice::from_ref(&dir), None, DumpMode::Identity)
            .unwrap_or_else(|err| panic!("LANG={locale}: {err}"));
        assert_eq!(report.ok.len(), 1, "LANG={locale}: {report:?}");
        assert_eq!(report.ok[0].path, file, "LANG={locale}");
    }
}

/// A wrong `serialVersionUID` must be a `FAIL` line with Java's message, not a crash.
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn verify_reports_a_wrong_uid_as_fail() {
    let _guard = LOCK.lock();
    let (tmp, out) = generate(&[Part::Roundtrip]);
    let mut raw = gunzip(&fs::read(out.join("java_roundtrip/serverinfo_hidden_2/fresh")).unwrap());
    let name = b"fr.soe.a3s.domain.repository.ServerInfo";
    let at = raw.windows(name.len()).position(|w| w == name).unwrap() + name.len();
    raw[at + 7] ^= 0x01;
    let file = as_a3s(&tmp.path().join("uid"), "serverinfo", &gzip(&raw));
    let report = harness()
        .verify(std::slice::from_ref(&file), None, DumpMode::Identity)
        .unwrap();
    assert_eq!(report.failed.len(), 1, "{report:?}");
    assert!(
        report.failed[0].detail.contains("InvalidClassException")
            || report.failed[0].detail.contains("local class incompatible"),
        "{report:?}"
    );
}

/// Empty and missing inputs: `FAIL` lines and exit 1, never exit 3.
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn verify_reports_empty_missing_and_orphan_files_as_fail() {
    let _guard = LOCK.lock();
    let tmp = tempfile::tempdir().unwrap();
    let empty = as_a3s(&tmp.path().join("empty"), "sync", b"");
    let missing = tmp.path().join("does-not-exist/.a3s/sync");
    let orphan_dir = tmp.path().join("orphan");
    fs::create_dir_all(&orphan_dir).unwrap();
    let orphan = orphan_dir.join("mod.pbo.zsync");
    fs::write(&orphan, b"zsync: 0.6.2\nURL: http://x/mod.pbo\n\n").unwrap();
    let report = harness()
        .verify(
            &[empty.clone(), missing.clone(), orphan.clone()],
            None,
            DumpMode::Identity,
        )
        .unwrap();
    let failed: BTreeSet<PathBuf> = report.failed.iter().map(|l| l.path.clone()).collect();
    assert_eq!(
        failed,
        BTreeSet::from([empty, missing, orphan]),
        "{report:?}"
    );
    let corrupted = report
        .failed
        .iter()
        .find(|l| l.path.ends_with("empty/.a3s/sync"))
        .unwrap();
    assert!(
        corrupted.detail.contains("The file seems to be corrupted."),
        "{corrupted:?}"
    );
}

/// The uploaded files are a Java read plus the upload serialization, so `roundtrip` of the
/// built `.a3s` file must give the same bytes (non-zip variants: no serverinfo override).
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn roundtrip_of_built_files_equals_uploaded_files() {
    let _guard = LOCK.lock();
    let (tmp, out) = generate(&[Part::Built]);
    let h = harness();
    for variant in ["http_plain", "ftp_plain"] {
        for step in ["1_initial", "3_modified"] {
            let dir = out.join("java_built").join(variant).join(step);
            for name in ["sync", "serverinfo", "changelogs", "autoconfig", "events"] {
                let rt = tmp.path().join(format!("{variant}-{step}-{name}"));
                h.roundtrip(&dir.join("repo/.a3s").join(name), &rt).unwrap();
                assert_eq!(
                    gunzip(&fs::read(&rt).unwrap()),
                    gunzip(&fs::read(dir.join("uploaded").join(name)).unwrap()),
                    "{variant}/{step}/{name}"
                );
                assert_eq!(
                    fs::read(&rt).unwrap(),
                    fs::read(dir.join("uploaded").join(name)).unwrap(),
                    "{variant}/{step}/{name} (gzip bytes)"
                );
            }
        }
    }
}

/// `RepositoryBuildProcessor.java:487` excludes `.pbo.zip` but not `.ebo.zip`: the generated
/// `mod_b.ebo.zip` becomes a leaf on the next build, is never zipped again (its extension is
/// `.zip`) and gets its own `.zsync` on HTTP. `.pbo.zip` never enters the sync.
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn ebo_zip_enters_the_sync_on_the_next_build_only() {
    let _guard = LOCK.lock();
    let (_tmp, out) = generate(&[Part::Built]);
    for variant in ["http_zip", "ftp_zip"] {
        let base = out.join("java_built").join(variant);
        for (step, expect_ebo_leaf) in [
            ("1_initial", false),
            ("2_rebuild_nochange", true),
            ("3_modified", true),
        ] {
            let text = fs::read_to_string(base.join(step).join("dumps/sync.json")).unwrap();
            assert_eq!(
                text.contains("\"str\":\"mod_b.ebo.zip\""),
                expect_ebo_leaf,
                "{variant}/{step}"
            );
            assert!(
                !text.contains(".pbo.zip\""),
                "{variant}/{step}: .pbo.zip leaf"
            );
            let repo_files = files(&base.join(step).join("repo"));
            assert!(
                !repo_files.iter().any(|f| f.ends_with(".zip.zip")),
                "{variant}/{step}: {repo_files:?}"
            );
            if variant.starts_with("http") && expect_ebo_leaf {
                assert!(
                    repo_files
                        .iter()
                        .any(|f| f == "@mod_b/Addons/mod_b.ebo.zip.zsync"),
                    "{variant}/{step}"
                );
            }
        }
        // The scenario's source-file list ignores the generated zip although it is a leaf.
        let scenario = fs::read_to_string(base.join("scenario.json")).unwrap();
        assert!(!scenario.contains("mod_b.ebo.zip"));
    }
}

/// `addEvent` goes through `transformDTO2Event`, which drops userconfig
/// (`ObjectDTOtransformer.java:361-376`); only `saveEvent` sets it.
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn events_session_add_drops_userconfig() {
    let _guard = LOCK.lock();
    let (_tmp, out) = generate(&[Part::Built]);
    let session = out.join("java_built/events_session");
    let s01 = Dump::load(&session.join("dumps/s01.json"));
    let night = s01.elements(s01.field(&s01.root, "list"))[0];
    assert_eq!(s01.entries(s01.field(night, "addonNames")).len(), 3);
    assert_eq!(s01.entries(s01.field(night, "dlcNames")).len(), 2);
    assert_eq!(
        s01.entries(s01.field(night, "userconfigFolderNames")).len(),
        0
    );
    let s02 = Dump::load(&session.join("dumps/s02.json"));
    let night = s02.elements(s02.field(&s02.root, "list"))[0];
    assert_eq!(
        s02.entries(s02.field(night, "userconfigFolderNames")).len(),
        2
    );
}

/// The manifest lists every other file once, sorted, with its SHA-256 and size.
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn manifest_matches_the_generated_tree() {
    let _guard = LOCK.lock();
    let (_tmp, out) = generate(&[Part::Local, Part::Presets, Part::Descriptors]);
    let manifest = fs::read_to_string(out.join("java_harness_manifest.txt")).unwrap();
    let mut lines = manifest.lines();
    assert_eq!(lines.next(), Some("# java_fixture_harness manifest"));
    assert!(lines.next().unwrap().starts_with("jdk 25"));
    assert!(lines.next().unwrap().starts_with("java_commit "));
    assert_eq!(lines.next(), Some("harness_version 1"));
    let entries: Vec<(String, u64, String)> = lines
        .map(|line| {
            let mut parts = line.splitn(3, "  ");
            (
                parts.next().unwrap().to_owned(),
                parts.next().unwrap().parse().unwrap(),
                parts.next().unwrap().to_owned(),
            )
        })
        .collect();
    let listed: Vec<String> = entries.iter().map(|e| e.2.clone()).collect();
    let mut sorted = listed.clone();
    sorted.sort();
    assert_eq!(listed, sorted);
    let mut on_disk = files(&out);
    on_disk.retain(|f| f != "java_harness_manifest.txt");
    assert_eq!(listed, on_disk);
    assert!(!on_disk.iter().any(|f| f.starts_with("java_built")));
    for (sha, size, rel) in &entries {
        let path = out.join(rel);
        assert_eq!(fs::metadata(&path).unwrap().len(), *size, "{rel}");
        let output = Command::new("sha256sum").arg(&path).output().unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        assert_eq!(text.split(' ').next().unwrap(), sha, "{rel}");
    }
}

/// A partial generation gives the same bytes as the full one for its parts.
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn partial_generation_equals_full_generation() {
    let _guard = LOCK.lock();
    let (_tmp1, full) = generate(&Part::ALL);
    let (_tmp2, partial) = generate(&[Part::Roundtrip, Part::Local]);
    for entry in ["java_local", "java_roundtrip"] {
        assert_eq!(
            compare_trees(&full.join(entry), &partial.join(entry)).unwrap(),
            Vec::new(),
            "{entry}"
        );
    }
    for entry in ["java_built", "java_presets", "java_descriptors.txt"] {
        assert!(!partial.join(entry).exists(), "{entry}");
    }
}

/// `generate` refuses a non-empty output directory (exit 2 → `Usage`).
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn generate_refuses_a_non_empty_output_directory() {
    let _guard = LOCK.lock();
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("keep.txt"), b"keep").unwrap();
    let err = harness()
        .generate(tmp.path(), &[Part::Descriptors])
        .unwrap_err();
    assert!(matches!(err, HarnessError::Usage(_)), "{err:?}");
    assert_eq!(fs::read(tmp.path().join("keep.txt")).unwrap(), b"keep");
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
}

/// R6: within one JVM, every empty file's SHA-1 is the literal `"0"`; the identity dump of the
/// step 1 sync must show the literal once and back-references for the other empty leaves, and
/// `scenario.json` must give `"0"` for the empty files.
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn scenario_lists_zero_sha1_for_empty_files() {
    let _guard = LOCK.lock();
    let (_tmp, out) = generate(&[Part::Built]);
    let scenario =
        Json::parse(&fs::read_to_string(out.join("java_built/http_plain/scenario.json")).unwrap());
    let steps = scenario.get("steps").unwrap().as_array();
    assert_eq!(steps.len(), 3);
    let mut empties = 0;
    for file in steps[2].get("files").unwrap().as_array() {
        if file.get("size").and_then(Json::as_i64) == Some(0) {
            empties += 1;
            assert_eq!(file.get("sha1").and_then(Json::as_str), Some("0"));
        } else {
            assert_eq!(file.get("sha1").and_then(Json::as_str).unwrap().len(), 40);
        }
    }
    assert_eq!(empties, 3);
    assert_eq!(
        scenario.get("scan_order").and_then(Json::as_str),
        Some("sorted-by-name")
    );
}
