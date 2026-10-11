//! JDK-backed acceptance tests. They need JDK 25 (`scripts/java_env.sh`) and the Java
//! sources, and are ignored by default:
//!
//! ```sh
//! source scripts/java_env.sh && cargo test -p a3s_dev_tools -- --ignored --test-threads=1
//! ```
//!
//! Every test holds [`LOCK`], so they also run one at a time without `--test-threads=1`
//! (one test rebuilds the shared `build/` directory from scratch).

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use a3s_dev_tools::fixtures::{self, GENERATED_ENTRIES, compare_trees};
use a3s_dev_tools::{DumpMode, Harness, Part};
use common::{Dump, Json};

static LOCK: parking_lot::Mutex<()> = parking_lot::const_mutex(());

const BIN: &str = env!("CARGO_BIN_EXE_a3s_dev_tools");

fn harness() -> Harness {
    Harness::locate().unwrap()
}

/// Generates `parts` into `<tmp>/out`.
fn generate(parts: &[Part]) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    fixtures::regenerate(&harness(), &out, parts).unwrap();
    (tmp, out)
}

fn gunzip(path: &Path) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(fs::File::open(path).unwrap())
        .read_to_end(&mut out)
        .unwrap();
    out
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Regular files below `root`, relative, sorted.
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

#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn build_is_incremental_patched_and_minimal() {
    let _guard = LOCK.lock();
    let h = harness();
    h.clean_build().unwrap();
    let classes = h.harness_dir().join("build/classes");
    let marker = classes.join("harness/Main.class");
    let modified = |p: &Path| fs::metadata(p).unwrap().modified().unwrap();
    let (marker_before, dir_before) = (modified(&marker), modified(&classes));
    let output = h.build_output().unwrap();
    assert!(output.contains("up to date"), "{output}");
    assert_eq!(modified(&marker), marker_before);
    assert_eq!(modified(&classes), dir_before);

    let javap = Command::new("bash")
        .arg("-c")
        .arg(
            "source \"$1/scripts/java_env.sh\" && javap -c -p -cp \"$2\" \
             fr.soe.a3s.dao.repository.RepositoryBuildProcessor",
        )
        .arg("bash")
        .arg(h.repo_root())
        .arg(&classes)
        .output()
        .unwrap();
    assert!(javap.status.success());
    let text = String::from_utf8_lossy(&javap.stdout);
    assert_eq!(
        text.matches("harness/HarnessClock.now").count(),
        3,
        "{text}"
    );
    assert_eq!(
        text.matches("harness/HarnessFs.sorted").count(),
        2,
        "{text}"
    );

    assert!(!classes.join("fr/soe/a3s/ui").exists());
    for protocol in ["ftp", "sftp", "http", "webdav"] {
        assert!(
            !classes
                .join("fr/soe/a3s/dao/connection")
                .join(protocol)
                .exists()
        );
    }
    assert_eq!(
        files(&classes.join("org/apache")),
        vec!["commons/io/FileUtils.class".to_owned()]
    );
    for variant in [
        "repository_pre_connections",
        "prefs_pre_github",
        "cfg_with_aia",
        "treeleaf_pre_workshop",
    ] {
        assert_eq!(
            files(&h.harness_dir().join("build/variants").join(variant)).len(),
            1
        );
    }
}

#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn descriptors_match_measured_and_declared_uids() {
    let _guard = LOCK.lock();
    let (_tmp, out) = generate(&[Part::Descriptors]);
    let text = fs::read_to_string(out.join("java_descriptors.txt")).unwrap();
    assert!(text.starts_with("# java_fixture_harness descriptors; JDK 25"));
    let block = |class: &str| -> String {
        let start = text
            .find(&format!("\nclass {class}\n"))
            .unwrap_or_else(|| panic!("{class} missing"));
        let rest = &text[start + 1..];
        rest[..rest.find("\n\n").unwrap_or(rest.len())].to_owned()
    };
    // Computed UIDs.
    assert!(
        block("fr.soe.a3s.domain.AutoConfig").contains("\n  uid 6424431620520340750 computed\n")
    );
    assert!(
        block("fr.soe.a3s.domain.repository.FileAttributes")
            .contains("\n  uid -2500913557576506383 computed\n")
    );
    assert!(
        block("fr.soe.a3s.domain.configration.Proxy")
            .contains("\n  uid -6891013124542313138 computed\n")
    );
    // ARCHITECTURE.md descriptor table.
    for (class, uid) in [
        ("repository.SyncTreeDirectory", "-2855304993780573704"),
        ("repository.SyncTreeLeaf", "8849248143660225239"),
        ("repository.ServerInfo", "7697232677958952953"),
        ("repository.Changelogs", "7332983755625818660"),
        ("repository.Changelog", "749128678979864247"),
        ("repository.Events", "-5141643688299352462"),
        ("repository.Event", "7456226002765813117"),
        ("repository.AutoConfig", "2706280725619197755"),
        ("AbstractProtocole", "-886357032287815203"),
        ("Http", "-20"),
        ("Ftp", "-1917448655850030044"),
        ("Webdav", "-30"),
        ("Socks", "-40"),
        ("Sftp", "1"),
        ("configration.FavoriteServer", "4613236292596702132"),
    ] {
        let block = block(&format!("fr.soe.a3s.domain.{class}"));
        assert!(
            block.contains(&format!("\n  uid {uid} declared\n")),
            "{block}"
        );
    }
    let protocol = block("fr.soe.a3s.constant.ProtocolType");
    assert!(protocol.contains("\n  uid 0 "));
    assert!(protocol.contains("\n  flags 0x12 SC_SERIALIZABLE|SC_ENUM\n"));
    assert!(
        protocol.contains(
            "\n  enum FTP,FTPS,SFTP,HTTP,HTTPS,A3S,SOCKS4,SOCKS5,HTTP_WEBDAV,HTTPS_WEBDAV"
        )
    );
    let sealed = block("javax.crypto.SealedObject");
    assert!(sealed.contains("\n  field [ encodedParams [B\n"));
    assert!(sealed.contains("\n  flags 0x02 SC_SERIALIZABLE\n"));
    assert!(
        block("java.util.HashSet").contains("\n  flags 0x03 SC_WRITE_METHOD|SC_SERIALIZABLE\n")
    );
    assert!(
        block("fr.soe.a3s.domain.repository.SyncTreeLeaf")
            .contains("\n  transient complete,destinationPath,localSHA1,optional")
    );
}

#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn generation_is_deterministic() {
    let _guard = LOCK.lock();
    let (_tmp1, first) = generate(&Part::ALL);
    let tmp2 = tempfile::tempdir().unwrap();
    let other_tmp = tmp2.path().join("other-tmp");
    fs::create_dir_all(&other_tmp).unwrap();
    let second = tmp2.path().join("out");
    let h = harness()
        .env("TZ", "Pacific/Kiritimati")
        .env("LANG", "de_DE.UTF-8")
        .env("TMPDIR", &other_tmp);
    fixtures::regenerate(&h, &second, &Part::ALL).unwrap();
    assert_eq!(compare_trees(&first, &second).unwrap(), Vec::new());
    for entry in GENERATED_ENTRIES {
        assert!(second.join(entry).exists(), "{entry}");
    }
    // No output file contains a work path, also inside gzip streams.
    let needles = [
        b"a3s-harness".as_slice(),
        other_tmp.to_str().unwrap().as_bytes(),
    ];
    for rel in files(&second) {
        let path = second.join(&rel);
        let mut bytes = fs::read(&path).unwrap();
        if bytes.starts_with(&[0x1f, 0x8b]) {
            bytes = gunzip(&path);
        }
        for needle in needles {
            assert!(!contains(&bytes, needle), "{rel} contains a work path");
        }
    }
}

#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn local_fixtures() {
    let _guard = LOCK.lock();
    let (_tmp, out) = generate(&[Part::Local]);
    let local = out.join("java_local");
    let all = files(&local);
    for expected in [
        "peruser/config/Arma3Sync/configuration/a3s.cfg",
        "peruser/config/Arma3Sync/configuration/a3s.prefs",
        "peruser/config/Arma3Sync/profiles/Default.a3s.profile",
        "peruser/config/Arma3Sync/profiles/Event Night.a3s.profile",
        "peruser/data/Arma3Sync/repositories/SampleFTP.a3s.repository",
        "portable/resources/configuration/a3s.cfg",
        "portable/resources/configuration/a3s.prefs",
        "portable/profiles/Default.a3s.profile",
        "portable/profiles/Event Night.a3s.profile",
        "plain/Sample_HTTP.repository.gz",
        "plain/Sample_FTP.repository.gz",
        "plain/Sample_SFTP.repository.gz",
        "variants/repository_pre_connections.a3s.repository",
        "variants/repository_pre_connections.repository.gz",
        "variants/prefs_pre_github.a3s.prefs",
        "variants/cfg_with_aia.a3s.cfg",
        "variants/treeleaf_pre_workshop.a3s.profile",
        "export/export.a3s.autoconfig",
    ] {
        assert!(
            all.iter().any(|f| f == expected),
            "{expected} missing from {all:?}"
        );
    }
    for layout in [
        "peruser/data/Arma3Sync/repositories",
        "portable/resources/ftp",
    ] {
        let names: Vec<String> = files(&local.join(layout))
            .into_iter()
            .filter(|name| name != "SampleFTP.a3s.repository")
            .collect();
        assert_eq!(names.len(), 3, "{names:?}");
        for name in &names {
            let rest = ["Sample_HTTP__", "Sample_FTP__", "Sample_SFTP__"]
                .iter()
                .find_map(|prefix| name.strip_prefix(prefix))
                .unwrap_or_else(|| panic!("{name}"));
            let hash = rest.strip_suffix(".a3s.repository").unwrap();
            assert_eq!(hash.len(), 12, "{name}");
            assert!(
                hash.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            );
        }
    }

    let report = harness()
        .verify(std::slice::from_ref(&local), None, DumpMode::Identity)
        .unwrap();
    assert!(report.is_success(), "{:?}", report.failed);
    assert!(
        report
            .skipped
            .iter()
            .all(|p| p.extension().is_some_and(|e| e == "json"))
    );
    let repository = "fr.soe.a3s.domain.repository.Repository";
    for line in &report.ok {
        if line.path.to_string_lossy().ends_with(".a3s.repository") {
            assert_eq!(line.detail, repository);
        }
    }
    let variants: Vec<_> = report
        .ok
        .iter()
        .filter(|line| line.path.starts_with(local.join("variants")))
        .collect();
    assert_eq!(variants.len(), 5);

    // The export nulls launcherOptions and clones the repository.
    let export = Dump::load(&local.join("dumps/export/export.a3s.autoconfig.json"));
    let profiles = export.elements(export.field(&export.root, "profiles"));
    assert_eq!(profiles.len(), 2);
    for profile in profiles {
        assert_eq!(export.field(profile, "launcherOptions"), &Json::Null);
    }
    let repositories = export.elements(export.field(&export.root, "repositories"));
    assert_eq!(repositories.len(), 1);
    assert_eq!(export.field(repositories[0], "revision").as_i64(), Some(0));
    assert_eq!(
        export.field(repositories[0], "noAutoDiscover").as_bool(),
        Some(true)
    );

    let aia = gunzip(&local.join("variants/cfg_with_aia.a3s.cfg"));
    assert!(contains(&aia, b"aiaOptions"));
    assert!(contains(&aia, b"fr.soe.a3s.domain.configration.AiAOptions"));
    assert!(!contains(
        &gunzip(&local.join("variants/prefs_pre_github.a3s.prefs")),
        b"preferGitHubUpdates"
    ));
    let pre_connections = gunzip(&local.join("variants/repository_pre_connections.repository.gz"));
    assert!(!contains(
        &pre_connections,
        b"numberOfRepositoryCheckConnections"
    ));
    assert!(!contains(&pre_connections, b"parallelUploadConnections"));

    // Fields missing from variant streams show their JVM zero value.
    let prefs = Dump::load(&local.join("dumps/variants/prefs_pre_github.a3s.prefs.json"));
    assert_eq!(prefs.field(&prefs.root, "preferGitHubUpdates"), &Json::Null);
    for name in [
        "repository_pre_connections.a3s.repository.json",
        "repository_pre_connections.repository.gz.json",
    ] {
        let repo = Dump::load(&local.join("dumps/variants").join(name));
        assert_eq!(
            repo.field(&repo.root, "numberOfRepositoryCheckConnections")
                .as_i64(),
            Some(0)
        );
        assert_eq!(
            repo.field(&repo.root, "parallelUploadConnections").as_i64(),
            Some(0)
        );
    }
    let current = Dump::load(&local.join("dumps/plain/Sample_FTP.repository.gz.json"));
    assert_eq!(
        current
            .field(&current.root, "parallelUploadConnections")
            .as_i64(),
        Some(6)
    );
    let leafless = Dump::load(&local.join("dumps/variants/treeleaf_pre_workshop.a3s.profile.json"));
    leafless.root.walk(&mut |value| {
        if let Some(Json::Obj(fields)) = value.get("fields") {
            for (name, field) in fields {
                if name == "workshopId" || name == "dlcAppId" {
                    assert_eq!(field, &Json::Null);
                }
            }
        }
    });
}

/// Ids of the `"0"` SHA-1 strings of the empty leaves of a sync dump, one entry per leaf.
fn zero_sha1_ids(dump: &Dump) -> Vec<i64> {
    let mut ids = Vec::new();
    dump.root.walk(&mut |value| {
        if value.get("class").and_then(Json::as_str)
            == Some("fr.soe.a3s.domain.repository.SyncTreeLeaf")
        {
            let raw = Dump::raw_field(value, "sha1");
            if dump.string(raw).as_deref() == Some("0") {
                let id = raw
                    .get("id")
                    .or_else(|| raw.get("ref"))
                    .and_then(Json::as_i64)
                    .unwrap();
                ids.push(id);
            }
        }
    });
    ids
}

fn dos_time_is_normalized(zip: &Path) -> bool {
    let b = fs::read(zip).unwrap();
    let eocd = b.len() - 22;
    let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as usize;
    let cen = u32_at(eocd + 16);
    let loc = u32_at(cen + 42);
    b[loc + 10..loc + 14] == [0, 0, 0x21, 0] && b[cen + 12..cen + 16] == [0, 0, 0x21, 0]
}

#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn built_fixtures() {
    let _guard = LOCK.lock();
    let (_tmp, out) = generate(&[Part::Built]);
    let h = harness();
    let steps = ["1_initial", "2_rebuild_nochange", "3_modified"];
    let dates = [1_704_067_200_000_i64, 1_704_070_800_000, 1_704_153_600_000];
    for variant in ["http_plain", "http_zip", "ftp_plain", "ftp_zip"] {
        let zip = variant.ends_with("_zip");
        // Java quirk (RepositoryBuildProcessor.java:487): a generated .ebo.zip is not excluded,
        // so the zip variants see a content change on the unchanged rebuild.
        let (revisions, updated, changelogs) = if zip {
            ([1, 2, 3], [true, true, true], [1, 2, 3])
        } else {
            ([1, 1, 2], [true, false, true], [1, 1, 2])
        };
        let base = out.join("java_built").join(variant);
        assert!(base.join("scenario.json").is_file());
        for (i, step) in steps.iter().enumerate() {
            let dir = base.join(step);
            let context = format!("{variant}/{step}");
            let info = Dump::load(&dir.join("dumps/serverinfo.json"));
            assert_eq!(
                info.field(&info.root, "revision").as_i64(),
                Some(revisions[i]),
                "{context}"
            );
            let date = info.field(&info.root, "buildDate");
            assert_eq!(
                date.get("time").and_then(Json::as_i64),
                Some(dates[i]),
                "{context}"
            );
            assert_eq!(
                info.field(&info.root, "repositoryContentUpdated").as_bool(),
                Some(updated[i]),
                "{context}"
            );
            let hidden: BTreeSet<String> = info
                .strings(info.field(&info.root, "hiddenFolderPaths"))
                .into_iter()
                .collect();
            assert_eq!(
                hidden,
                BTreeSet::from(["docs/hidden_a".to_owned(), "docs/hidden_b".to_owned()])
            );

            let logs = Dump::load(&dir.join("dumps/changelogs.json"));
            assert_eq!(
                logs.elements(logs.field(&logs.root, "list")).len(),
                changelogs[i],
                "{context}"
            );

            let sync_text = fs::read_to_string(dir.join("dumps/sync.json")).unwrap();
            for excluded in [
                "notes.a3s.txt",
                "readme.zsync.txt",
                "excluded.txt",
                "emptydir",
            ] {
                assert!(
                    !sync_text.contains(&format!("\"{excluded}\"")),
                    "{context}: {excluded}"
                );
            }

            let repo_files = files(&dir.join("repo"));
            let has_zsync = repo_files.iter().any(|f| f.ends_with(".zsync"));
            let zips: Vec<_> = repo_files
                .iter()
                .filter(|f| f.ends_with(".pbo.zip") || f.ends_with(".ebo.zip"))
                .collect();
            assert_eq!(has_zsync, variant.starts_with("http"), "{context}");
            assert_eq!(!zips.is_empty(), zip, "{context}");
            for zip_file in zips {
                assert!(
                    dos_time_is_normalized(&dir.join("repo").join(zip_file)),
                    "{zip_file}"
                );
            }

            let report = h
                .verify(&[dir.join("repo")], None, DumpMode::Identity)
                .unwrap();
            assert!(report.is_success(), "{context}: {:?}", report.failed);
            assert!(report.ok.len() >= 5, "{context}");

            let uploaded = Dump::load(&dir.join("dumps/uploaded_serverinfo.json"));
            assert_eq!(
                uploaded
                    .field(&uploaded.root, "compressedPboFilesOnly")
                    .as_bool(),
                Some(zip),
                "{context}"
            );

            // R5: autoconfig repositoryName is a back-reference to the first favorite's string.
            let auto = Dump::load(&dir.join("dumps/autoconfig.json"));
            let favorites = auto.elements(auto.field(&auto.root, "favoriteServers"));
            let favorite_name = Dump::raw_field(favorites[0], "repositoryName");
            let shared = Dump::raw_field(&auto.root, "repositoryName");
            assert_eq!(shared.get("ref"), favorite_name.get("id"), "{context}");
            assert_eq!(auto.string(shared), Some(format!("Sample {variant}")));

            // R6: "0" sharing.
            let sync = Dump::load(&dir.join("dumps/sync.json"));
            let zeros = zero_sha1_ids(&sync);
            let distinct: BTreeSet<_> = zeros.iter().collect();
            if i < 2 {
                assert_eq!((zeros.len(), distinct.len()), (2, 1), "{context}");
            } else {
                assert_eq!((zeros.len(), distinct.len()), (3, 2), "{context}");
            }
        }

        let logs = Dump::load(&base.join("3_modified/dumps/changelogs.json"));
        let list = logs.elements(logs.field(&logs.root, "list"));
        let last = list.last().unwrap();
        assert_eq!(
            logs.strings(logs.field(last, "newAddons")),
            vec!["@Mod_D".to_owned()]
        );
        assert_eq!(
            logs.strings(logs.field(last, "deletedAddons")),
            vec!["@Nested".to_owned()]
        );
        let updated: BTreeSet<String> = logs
            .strings(logs.field(last, "updatedAddons"))
            .into_iter()
            .collect();
        assert_eq!(
            updated,
            BTreeSet::from(["@mod_a".to_owned(), "@mod_b".to_owned()])
        );

        let events = Dump::load(&base.join("3_modified/dumps/events.json"));
        let list = events.elements(events.field(&events.root, "list"));
        let night = list
            .iter()
            .find(|e| events.string(events.field(e, "name")).as_deref() == Some("Night Op"))
            .unwrap();
        let addons: BTreeSet<String> = events
            .keys(events.field(night, "addonNames"))
            .into_iter()
            .collect();
        assert_eq!(
            addons,
            BTreeSet::from(["@Mod_A".to_owned(), "@mod_b".to_owned()])
        );
        let dlcs: BTreeSet<String> = events
            .keys(events.field(night, "dlcNames"))
            .into_iter()
            .collect();
        assert_eq!(dlcs, BTreeSet::from(["GM".to_owned(), "WS".to_owned()]));
        assert!(
            !fs::read_to_string(base.join("3_modified/dumps/events.json"))
                .unwrap()
                .contains("@Gone")
        );
        assert!(contains(
            &gunzip(&base.join("3_modified/input/events")),
            b"@Gone"
        ));
    }
}

/// Event names of an events-session snapshot, in list order.
fn event_names(dump: &Dump) -> Vec<String> {
    dump.elements(dump.field(&dump.root, "list"))
        .into_iter()
        .map(|e| dump.string(dump.field(e, "name")).unwrap())
        .collect()
}

#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn events_session() {
    let _guard = LOCK.lock();
    let (_tmp, out) = generate(&[Part::Built]);
    let session = out.join("java_built/events_session");
    let snapshots: Vec<String> = files(&session)
        .into_iter()
        .filter(|f| f.ends_with("/events"))
        .collect();
    assert_eq!(snapshots.len(), 7, "{snapshots:?}");
    assert!(session.join("session.json").is_file());
    let load = |n: u32| Dump::load(&session.join(format!("dumps/s{n:02}.json")));

    assert_eq!(event_names(&load(4)), ["Night Op", "Night Op - duplicate"]);
    let s05 = load(5);
    assert_eq!(event_names(&s05), ["Night Op", "Day Op"]);
    let day = s05.elements(s05.field(&s05.root, "list"))[1];
    assert_eq!(
        s05.string(s05.field(day, "description")).as_deref(),
        Some("desc")
    );
    assert_eq!(event_names(&load(7)), ["Day Op", "Training"]);
    let s03 = load(3);
    let night = s03.elements(s03.field(&s03.root, "list"))[0];
    assert_eq!(s03.entries(s03.field(night, "addonNames")).len(), 1);
    assert_eq!(s03.entries(s03.field(night, "dlcNames")).len(), 0);
    assert_eq!(
        s03.entries(s03.field(night, "userconfigFolderNames")).len(),
        1
    );

    // HashMap block data: int buckets, int size. Training.addonNames is the only 13-key map.
    let stream = gunzip(&session.join("s06_add_second/events"));
    let capacities: Vec<i32> = stream
        .windows(10)
        .filter(|w| w[0] == 0x77 && w[1] == 8 && w[6..10] == 13_i32.to_be_bytes())
        .map(|w| i32::from_be_bytes([w[2], w[3], w[4], w[5]]))
        .collect();
    assert_eq!(capacities, vec![32]);
}

#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn collection_capacities() {
    let _guard = LOCK.lock();
    let (_tmp, out) = generate(&[Part::Roundtrip]);
    let text = fs::read_to_string(out.join("java_roundtrip/capacities.txt")).unwrap();
    let mut expected = Vec::new();
    for (n, rt) in [
        (0, 1),
        (1, 2),
        (2, 4),
        (4, 8),
        (7, 16),
        (12, 16),
        (13, 32),
        (24, 32),
        (25, 64),
    ] {
        let fresh = if n <= 12 {
            16
        } else if n <= 24 {
            32
        } else {
            64
        };
        expected.push(format!(
            "serverinfo_hidden_{n} fresh capacity={fresh} size={n}"
        ));
        expected.push(format!("serverinfo_hidden_{n} rt1 capacity={rt} size={n}"));
        expected.push(format!("serverinfo_hidden_{n} rt2 capacity={rt} size={n}"));
    }
    for (n, put, put_all) in [
        (1, 16, 2),
        (6, 16, 8),
        (12, 16, 16),
        (13, 32, 32),
        (24, 32, 32),
    ] {
        expected.push(format!("events_map_{n} fresh_put capacity={put} size={n}"));
        expected.push(format!(
            "events_map_{n} fresh_putall capacity={put_all} size={n}"
        ));
        expected.push(format!("events_map_{n} rt1 capacity={put} size={n}"));
    }
    expected.sort();
    let expected: String = expected
        .into_iter()
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(text, expected);
}

const WP18_B35_GOLDEN: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html>
  <!--Created by Arma 3 Launcher: https://arma3.com-->
  <head>
    <meta name="arma:Type" content="preset" />
    <meta name="arma:PresetName" content="Event &lt;HS13&gt;" />
    <meta name="generator" content="Arma3Sync - https://github.com/vpzbrig21/Arma3Sync" />
    <title>Arma 3</title>
    <link href="https://fonts.googleapis.com/css?family=Roboto" rel="stylesheet" type="text/css" />
    <style>
body { margin: 0; padding: 0; color: #fff; background: #000; }
body, th, td { font: 95%/1.3 Roboto, Segoe UI, Tahoma, Arial, Helvetica, sans-serif; }
td { padding: 3px 30px 3px 0; }
h1 { padding: 20px 20px 0 20px; color: white; font-weight: 200; font-family: segoe ui; font-size: 3em; margin: 0; }
em { font-variant: italic; color: silver; }
.before-list { padding: 5px 20px 10px 20px; }
.mod-list, .dlc-list { background: #222222; padding: 20px; }
.footer { padding: 20px; color: gray; }
.whups { color: gray; }
a { color: #D18F21; text-decoration: underline; }
a:hover { color: #F1AF41; text-decoration: none; }
.from-steam { color: #449EBD; }
.from-local { color: gray; }
    </style>
  </head>
  <body>
    <h1>Arma 3 - Preset <strong>Event &lt;HS13&gt;</strong></h1>
    <p class="before-list"><em>To import this preset, drag this file onto the Launcher window. Or click the MODS tab, then PRESET in the top right, then IMPORT at the bottom, and finally select this file.</em></p>
    <div class="mod-list">
      <table>
        <tr data-type="ModContainer">
          <td data-type="DisplayName">GM Enhancement &amp; Intern</td>
          <td><span class="from-steam">Steam</span></td>
          <td><a href="https://steamcommunity.com/sharedfiles/filedetails/?id=3147580028" data-type="Link">https://steamcommunity.com/sharedfiles/filedetails/?id=3147580028</a></td>
        </tr>
      </table>
    </div>
    <div class="dlc-list">
      <table>
        <tr data-type="DlcContainer">
          <td data-type="DisplayName">Western Sahara</td>
          <td><a href="https://store.steampowered.com/app/1681170" data-type="Link">https://store.steampowered.com/app/1681170</a></td>
        </tr>
      </table>
    </div>
    <div class="footer"><span>Created by Arma 3 Launcher by Bohemia Interactive. Exported by Arma3Sync.</span></div>
  </body>
</html>
"#;

#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn presets() {
    let _guard = LOCK.lock();
    let (_tmp, out) = generate(&[Part::Presets]);
    let presets = out.join("java_presets");
    let read = |case: &str, file: &str| fs::read_to_string(presets.join(case).join(file)).unwrap();
    assert_eq!(read("p1_golden", "preset.html"), WP18_B35_GOLDEN);
    assert!(!read("p2_no_cdlc_escaped", "preset.html").contains("<div class=\"dlc-list\">"));
    assert!(read("p2_no_cdlc_escaped", "preset.html").contains("Mod &quot;Quoted&quot; &amp; Co"));
    assert!(
        read("p3_duplicate_nested", "result.txt")
            .contains("warning: Duplicate Workshop ID omitted: ")
    );
    assert!(
        read("p4_raw_lf_name", "preset.html")
            .contains("<td data-type=\"DisplayName\">First line\nSecond line</td>")
    );
    for case in ["p5_missing_meta", "p7_zero_id"] {
        let result = read(case, "result.txt");
        assert!(result.starts_with("valid=false\n"), "{result}");
        assert_eq!(
            result.lines().filter(|l| l.starts_with("issue: ")).count(),
            1,
            "{result}"
        );
        assert!(!presets.join(case).join("preset.html").exists());
    }
    assert!(
        read("p6_name_fallback", "preset.html")
            .contains("<td data-type=\"DisplayName\">@nameless</td>")
    );
    for case in files(&presets) {
        assert!(!case.contains("a3s-harness"));
    }
}

#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn verify_reports_java_failures() {
    let _guard = LOCK.lock();
    let (tmp, out) = generate(&[Part::Built]);
    let h = harness();
    let source = out.join("java_built/http_zip/1_initial/repo");
    // Copies keep file mtimes: the zsync header carries the source mtime.
    let copy = |name: &str| -> PathBuf {
        let target = tmp.path().join(name);
        fs::create_dir_all(&target).unwrap();
        for rel in files(&source) {
            let to = target.join(&rel);
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            fs::copy(source.join(&rel), &to).unwrap();
            let mtime = fs::metadata(source.join(&rel)).unwrap().modified().unwrap();
            fs::File::options()
                .write(true)
                .open(&to)
                .unwrap()
                .set_modified(mtime)
                .unwrap();
        }
        target
    };

    let intact = copy("intact");
    let report = h.verify(&[intact], None, DumpMode::Identity).unwrap();
    assert!(report.is_success(), "{report:?}");

    let truncated = copy("truncated");
    let sync = truncated.join(".a3s/sync");
    let bytes = fs::read(&sync).unwrap();
    fs::write(&sync, &bytes[..bytes.len() / 2]).unwrap();
    let report = h
        .verify(std::slice::from_ref(&sync), None, DumpMode::Identity)
        .unwrap();
    assert!(!report.is_success());
    assert!(
        report.failed[0]
            .detail
            .contains("The file seems to be corrupted."),
        "{report:?}"
    );

    let swapped = copy("swapped");
    fs::copy(swapped.join(".a3s/serverinfo"), swapped.join(".a3s/sync")).unwrap();
    let report = h
        .verify(&[swapped.join(".a3s/sync")], None, DumpMode::Identity)
        .unwrap();
    assert!(
        report.failed[0]
            .detail
            .contains("expected fr.soe.a3s.domain.repository.SyncTreeDirectory"),
        "{report:?}"
    );

    let flipped = copy("flipped");
    let zsync = flipped.join("@Mod_A/addons/mod_a.pbo.zsync");
    let mut bytes = fs::read(&zsync).unwrap();
    let body = bytes.windows(2).position(|w| w == b"\n\n").unwrap() + 2;
    bytes[body + 3] ^= 0x55;
    fs::write(&zsync, bytes).unwrap();
    let report = h.verify(&[zsync], None, DumpMode::Identity).unwrap();
    assert_eq!(report.failed.len(), 1, "{report:?}");

    let replaced = copy("replaced");
    let zip = replaced.join("@Mod_A/addons/mod_a.pbo.zip");
    fs::write(&zip, stored_zip("mod_a.pbo", b"different content")).unwrap();
    let report = h.verify(&[zip], None, DumpMode::Identity).unwrap();
    assert!(report.failed[0].detail.contains("sha1"), "{report:?}");

    // Exit code 1 through the CLI.
    let output = Command::new(BIN)
        .arg("java-verify")
        .arg(&truncated)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
}

/// A one-entry STORED zip.
fn stored_zip(name: &str, data: &[u8]) -> Vec<u8> {
    let mut crc = flate2::Crc::new();
    crc.update(data);
    let crc = crc.sum();
    let size = u32::try_from(data.len()).unwrap();
    let name_len = u16::try_from(name.len()).unwrap();
    let mut out = Vec::new();
    let header = |out: &mut Vec<u8>, central: bool| {
        out.extend_from_slice(if central {
            &[0x50, 0x4b, 1, 2, 20, 0]
        } else {
            &[0x50, 0x4b, 3, 4]
        });
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        if central {
            // Comment length, disk, internal and external attributes, local header offset 0.
            out.extend_from_slice(&[0; 14]);
        }
    };
    header(&mut out, false);
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(data);
    let central_offset = u32::try_from(out.len()).unwrap();
    header(&mut out, true);
    out.extend_from_slice(name.as_bytes());
    let central_size = u32::try_from(out.len()).unwrap() - central_offset;
    out.extend_from_slice(&[0x50, 0x4b, 5, 6, 0, 0, 0, 0, 1, 0, 1, 0]);
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn selftest() {
    let _guard = LOCK.lock();
    let report = harness().selftest().unwrap();
    assert!(report.is_success(), "{report:?}");
    assert_eq!(report.ok.len(), 3);
}

#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn roundtrip_and_dump() {
    let _guard = LOCK.lock();
    let (tmp, out) = generate(&[Part::Roundtrip]);
    let h = harness();
    let dir = out.join("java_roundtrip/serverinfo_hidden_2");
    let rt = tmp.path().join("rt");
    h.roundtrip(&dir.join("fresh"), &rt).unwrap();
    assert_eq!(fs::read(&rt).unwrap(), fs::read(dir.join("rt1")).unwrap());
    // The harness classifies by name: dump through a `.a3s/serverinfo` path.
    let a3s = tmp.path().join("a/.a3s");
    let b3s = tmp.path().join("b/.a3s");
    fs::create_dir_all(&a3s).unwrap();
    fs::create_dir_all(&b3s).unwrap();
    fs::copy(dir.join("fresh"), a3s.join("serverinfo")).unwrap();
    fs::copy(dir.join("rt1"), b3s.join("serverinfo")).unwrap();
    let semantic_fresh = h.dump(&a3s.join("serverinfo"), DumpMode::Semantic).unwrap();
    let semantic_rt1 = h.dump(&b3s.join("serverinfo"), DumpMode::Semantic).unwrap();
    assert_eq!(semantic_fresh, semantic_rt1);
    assert!(!semantic_fresh.contains("\"id\""));
    let identity_fresh = h.dump(&a3s.join("serverinfo"), DumpMode::Identity).unwrap();
    assert_eq!(
        identity_fresh,
        h.dump(&b3s.join("serverinfo"), DumpMode::Identity).unwrap()
    );
    assert!(identity_fresh.contains("\"id\":1"));
}

#[cfg(unix)]
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn cli_regen_and_check() {
    let _guard = LOCK.lock();
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("generated");
    let status = Command::new(BIN)
        .args(["regen-fixtures", "--out"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());
    let mut entries: Vec<String> = fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    assert_eq!(entries, GENERATED_ENTRIES);

    // A repository layout whose tests/fixtures holds the generated tree.
    let real_root = harness().repo_root().to_path_buf();
    let root = tmp.path().join("root");
    fs::create_dir_all(root.join("tests/fixtures")).unwrap();
    std::os::unix::fs::symlink(real_root.join("tools"), root.join("tools")).unwrap();
    std::os::unix::fs::symlink(real_root.join("scripts"), root.join("scripts")).unwrap();
    fs::write(root.join("tests/fixtures/README.md"), "kept").unwrap();
    fixtures::install(&out, &root.join("tests/fixtures")).unwrap();
    let check = || {
        Command::new(BIN)
            .env("A3S_REPO_ROOT", &root)
            .args(["regen-fixtures", "--check"])
            .output()
            .unwrap()
    };
    let output = check();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );

    fs::write(
        root.join("tests/fixtures/java_presets/p1_golden/result.txt"),
        "changed\n",
    )
    .unwrap();
    let output = check();
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("java_presets/p1_golden/result.txt"),
        "{stdout}"
    );
}

/// Copies `source` to `target` the way a git checkout does: contents only, mtimes = now.
fn checkout_copy(source: &Path, target: &Path) {
    for rel in files(source) {
        let to = target.join(&rel);
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::copy(source.join(&rel), &to).unwrap();
        fs::File::options()
            .write(true)
            .open(&to)
            .unwrap()
            .set_modified(std::time::SystemTime::now())
            .unwrap();
    }
}

/// Sets the mtimes recorded in `scenario.json` and `generated_files.json` on a checkout.
fn restore_mtimes(variant_dir: &Path) {
    let load = |name: &str| Json::parse(&fs::read_to_string(variant_dir.join(name)).unwrap());
    for list in [load("scenario.json"), load("generated_files.json")] {
        for step in list.get("steps").unwrap().as_array() {
            let repo = variant_dir
                .join(step.get("name").and_then(Json::as_str).unwrap())
                .join("repo");
            for file in step.get("files").unwrap().as_array() {
                let Some(ms) = file.get("mtime_ms").and_then(Json::as_i64) else {
                    continue;
                };
                let path = repo.join(file.get("path").and_then(Json::as_str).unwrap());
                let size = file.get("size").and_then(Json::as_i64).unwrap();
                assert_eq!(
                    fs::metadata(&path).unwrap().len(),
                    u64::try_from(size).unwrap(),
                    "{}",
                    path.display()
                );
                let mtime = std::time::UNIX_EPOCH
                    + std::time::Duration::from_millis(u64::try_from(ms).unwrap());
                fs::File::options()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_modified(mtime)
                    .unwrap();
            }
        }
    }
}

/// After a git checkout (mtimes lost), restoring the mtimes recorded in `scenario.json` and
/// `generated_files.json` makes every snapshot, `.zsync` files included, verify again.
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn snapshots_verify_after_checkout_with_restored_mtimes() {
    let _guard = LOCK.lock();
    let (tmp, out) = generate(&[Part::Built]);
    let h = harness();
    for variant in ["http_plain", "http_zip"] {
        let checkout = tmp.path().join("checkout").join(variant);
        checkout_copy(&out.join("java_built").join(variant), &checkout);
        let before = h
            .verify(&[checkout.join("1_initial/repo")], None, DumpMode::Identity)
            .unwrap();
        assert!(
            !before.is_success(),
            "{variant}: zsync must depend on mtimes"
        );
        restore_mtimes(&checkout);
        for step in ["1_initial", "2_rebuild_nochange", "3_modified"] {
            let report = h
                .verify(
                    &[checkout.join(step).join("repo")],
                    None,
                    DumpMode::Identity,
                )
                .unwrap();
            assert!(report.is_success(), "{variant}/{step}: {:?}", report.failed);
        }
    }
    // The .ebo.zip the next build hashes is recorded as a sync leaf with its pinned mtime.
    let generated = Json::parse(
        &fs::read_to_string(out.join("java_built/http_zip/generated_files.json")).unwrap(),
    );
    let step1 = &generated.get("steps").unwrap().as_array()[0];
    let ebo = step1
        .get("files")
        .unwrap()
        .as_array()
        .iter()
        .find(|f| f.get("path").and_then(Json::as_str) == Some("@mod_b/Addons/mod_b.ebo.zip"))
        .unwrap();
    assert_eq!(ebo.get("sync_leaf"), Some(&Json::Bool(true)));
    assert_eq!(
        ebo.get("mtime_ms").and_then(Json::as_i64),
        Some(1_704_067_200_000)
    );
}

/// Non-ASCII work and output paths with a C locale (`sun.jnu.encoding` follows the locale only).
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn generate_with_non_ascii_tmpdir_in_c_locale() {
    let _guard = LOCK.lock();
    let tmp = tempfile::tempdir().unwrap();
    let tmpdir = tmp.path().join("Dossier é ü");
    fs::create_dir_all(&tmpdir).unwrap();
    let out = tmp.path().join("sortie é");
    let h = harness()
        .env("TMPDIR", &tmpdir)
        .env("LANG", "C")
        .env("LC_ALL", "C");
    fixtures::regenerate(&h, &out, &[Part::Local, Part::Presets]).unwrap();
    assert!(out.join("java_local/dumps").is_dir());
    assert!(out.join("java_presets/p1_golden/preset.html").is_file());
}

/// `verify` on a path with nothing Arma3Sync reads is a failure, not a silent all-SKIP.
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn verify_fails_when_nothing_is_readable() {
    let _guard = LOCK.lock();
    let (_tmp, out) = generate(&[Part::Built]);
    let uploaded = out.join("java_built/http_plain/1_initial/uploaded");
    let report = harness()
        .verify(std::slice::from_ref(&uploaded), None, DumpMode::Identity)
        .unwrap();
    assert_eq!(report.failed.len(), 1, "{report:?}");
    assert_eq!(report.failed[0].path, uploaded);
    assert!(report.failed[0].detail.contains(".a3s directory"));
    assert_eq!(report.skipped.len(), 5);
}

/// `generate` refuses `--work` and `--out` inside each other (exit 2).
#[cfg(unix)]
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn generate_refuses_nested_work_and_out() {
    let _guard = LOCK.lock();
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let run = |work: &Path| {
        Command::new("bash")
            .arg(harness().harness_dir().join("run.sh"))
            .args(["generate", "--only", "descriptors", "--out"])
            .arg(&out)
            .arg("--work")
            .arg(work)
            .output()
            .unwrap()
    };
    assert_eq!(run(&out.join("work")).status.code(), Some(2));
    assert_eq!(run(tmp.path()).status.code(), Some(2));
    assert_eq!(run(&out).status.code(), Some(2));
    assert!(!out.join("java_descriptors.txt").exists());

    // Symbolic links are resolved: --work reaching --out through a link is nested too.
    let link = tmp.path().join("link-to-out");
    fs::create_dir_all(&out).unwrap();
    std::os::unix::fs::symlink(&out, &link).unwrap();
    assert_eq!(run(&link.join("work")).status.code(), Some(2));
}

/// A source root that is not a git checkout (e.g. an extracted tarball) builds with Java
/// commit `unknown`, and a change to any of its `.java` files invalidates the stamp.
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn build_from_a_non_git_copy_of_the_java_sources() {
    let _guard = LOCK.lock();
    let real = harness();
    real.build().unwrap();
    // The same resolution order as build.sh.
    let java_src = std::env::var_os("A3S_JAVA_SRC")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            real.repo_root()
                .join("java_version/Arma3Sync/src/main/java")
        })
        .canonicalize()
        .or_else(|_err| {
            // Agent worktrees: the sources live next to the main checkout.
            let common = Command::new("git")
                .arg("-C")
                .arg(real.repo_root())
                .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
                .output()
                .unwrap();
            let common = PathBuf::from(String::from_utf8(common.stdout).unwrap().trim());
            common
                .parent()
                .unwrap()
                .join("java_version/Arma3Sync/src/main/java")
                .canonicalize()
        })
        .unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let copy = tmp.path().join("Arma3Sync/src/main/java");
    for rel in files(&java_src) {
        let to = copy.join(&rel);
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::copy(java_src.join(&rel), &to).unwrap();
    }
    let h = harness().env("A3S_JAVA_SRC", &copy);
    let output = h.build_output().unwrap();
    assert!(output.contains("Java commit unknown"), "{output}");
    let stamp_file = h.harness_dir().join("build/stamp");
    let stamp = fs::read_to_string(&stamp_file).unwrap();
    assert!(h.build_output().unwrap().contains("up to date"));

    // Any .java file of the root is part of the stamp, also one nothing compiles explicitly.
    let changed = copy.join("fr/soe/a3s/jazsync/MD4.java");
    let mut text = fs::read_to_string(&changed).unwrap();
    text.push_str("\n// harness stamp test\n");
    fs::write(&changed, text).unwrap();
    let output = h.build_output().unwrap();
    assert!(!output.contains("up to date"), "{output}");
    assert_ne!(fs::read_to_string(&stamp_file).unwrap(), stamp);

    let (_gen, out) = {
        let gen_tmp = tempfile::tempdir().unwrap();
        let out = gen_tmp.path().join("out");
        h.generate(&out, &[Part::Descriptors]).unwrap();
        (gen_tmp, out)
    };
    let descriptors = fs::read_to_string(out.join("java_descriptors.txt")).unwrap();
    assert!(
        descriptors
            .lines()
            .next()
            .unwrap()
            .ends_with("; Java commit unknown")
    );

    // Back to the real sources for the other tests.
    real.build().unwrap();
}
