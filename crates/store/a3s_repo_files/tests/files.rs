use a3s_repo_files::{A3sFile, decode_a3s, encode_a3s, read_a3s, write_a3s};
use a3s_types::{ServerInfo, SyncTreeDirectory};

#[test]
fn write_then_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(A3sFile::ServerInfo.relative_path());
    let info = ServerInfo {
        revision: 7,
        ..Default::default()
    };
    write_a3s(&path, &info).unwrap();
    let back: ServerInfo = read_a3s(&path).unwrap();
    assert_eq!(back, info);
}

#[test]
fn gzip_header_matches_java() {
    let bytes = encode_a3s(&SyncTreeDirectory::new_root()).unwrap();
    assert_eq!(&bytes[..4], &[0x1f, 0x8b, 0x08, 0x00]);
    assert_eq!(&bytes[4..8], &[0, 0, 0, 0], "mtime");
    assert_eq!(bytes[9], 255, "OS");
}

#[test]
fn rejects_garbage() {
    assert!(decode_a3s::<ServerInfo>(b"not gzip").is_err());
    let wrong_type = encode_a3s(&SyncTreeDirectory::new_root()).unwrap();
    assert!(decode_a3s::<ServerInfo>(&wrong_type).is_err());
}
