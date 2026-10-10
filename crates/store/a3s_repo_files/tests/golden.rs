//! Golden tests against `.a3s` files written by the Java implementation.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use a3s_repo_files::{A3sFile, read_a3s};
use a3s_types::{
    AutoConfig, Changelogs, Events, JavaCodec, ProtocolType, ServerInfo, SyncTreeDirectory,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/java_repos")
}

fn gunzip(path: &Path) -> Vec<u8> {
    let bytes = std::fs::read(path).unwrap();
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(&bytes[..])
        .read_to_end(&mut raw)
        .unwrap();
    raw
}

/// Decodes `file`, re-encodes it and checks the bytes are identical to Java's.
fn check_identical<T: JavaCodec>(dir: &Path, file: A3sFile) -> T {
    let path = dir.join(file.file_name());
    let value: T = read_a3s(&path).unwrap();
    let raw = gunzip(&path);
    assert_eq!(
        a3s_jserial::write(&a3s_jserial::read(&raw).unwrap()).unwrap(),
        raw,
        "{file:?}: generic rewrite"
    );
    assert!(
        a3s_types::encode_stream(&value).unwrap() == raw,
        "{file:?}: typed re-encode differs from Java"
    );
    value
}

#[test]
fn wog_ftp() {
    let dir = fixtures().join("wog_ftp");
    let sync: SyncTreeDirectory = check_identical(&dir, A3sFile::Sync);
    let info: ServerInfo = check_identical(&dir, A3sFile::ServerInfo);
    let changelogs: Changelogs = check_identical(&dir, A3sFile::Changelogs);
    let auto: AutoConfig = check_identical(&dir, A3sFile::AutoConfig);
    let events: Events = check_identical(&dir, A3sFile::Events);

    assert_eq!(sync.name, "racine");
    assert_eq!(sync.leaf_count() as i64, info.number_of_files);
    assert_eq!(info.revision, 55);
    assert_eq!(changelogs.list.len(), 10);
    assert_eq!(changelogs.list.last().unwrap().revision, 55);
    let protocol = auto.protocol.unwrap();
    assert_eq!(protocol.protocol_type, ProtocolType::Ftp);
    assert_eq!(protocol.hostname(), "88.99.69.243");
    assert_eq!(protocol.remote_path(), "/Repo/WOG");
    assert!(events.list.is_empty());
}
