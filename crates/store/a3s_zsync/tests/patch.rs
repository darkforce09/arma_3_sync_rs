use std::io::Cursor;

use a3s_zsync::{BLOCK_SIZE, ControlFile, Plan, assemble, match_blocks};
use sha1::{Digest as _, Sha1};

fn sha1_hex(data: &[u8]) -> String {
    if data.is_empty() {
        return "0".to_owned();
    }
    Sha1::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn pseudo_random(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

fn control_for(data: &[u8]) -> ControlFile {
    ControlFile::generate(
        data,
        data.len() as u64,
        "test.pbo",
        "Thu, 01 Jan 1970 00:00:00 +0000",
        "http://example.org/repo/@mod/addons/test.pbo",
        &sha1_hex(data),
    )
    .unwrap()
}

/// Runs a full client-side patch of `old` into `new`; returns bytes downloaded.
fn patch(old: &[u8], new: &[u8]) -> u64 {
    // Round-trip the control file through its serialized form, like a real download.
    let control = ControlFile::parse(&control_for(new).to_bytes()).unwrap();
    let matches = match_blocks(&control, old, None).unwrap();
    let plan = Plan::new(&control, &matches, Plan::MAX_BLOCKS_PER_RANGE);
    let mut remote = Vec::new();
    for range in &plan.ranges {
        remote.extend_from_slice(&new[range.start as usize..range.end as usize]);
    }
    let mut out = Vec::new();
    assemble(&control, &matches, Cursor::new(old), &remote[..], &mut out).unwrap();
    assert_eq!(out, new);
    plan.download_size()
}

#[test]
fn control_file_round_trips() {
    for len in [0, 1, 100, BLOCK_SIZE, BLOCK_SIZE + 1, 5 * BLOCK_SIZE + 17] {
        let data = pseudo_random(len, len as u32);
        let control = control_for(&data);
        assert_eq!(control.block_count(), len.div_ceil(BLOCK_SIZE));
        let parsed = ControlFile::parse(&control.to_bytes()).unwrap();
        assert_eq!(parsed, control);
    }
}

#[test]
fn header_format_matches_jazsync() {
    let data = pseudo_random(10, 1);
    let bytes = control_for(&data).to_bytes();
    let header_end = bytes.windows(2).position(|w| w == b"\n\n").unwrap();
    let header = std::str::from_utf8(&bytes[..header_end + 2]).unwrap();
    assert_eq!(
        header,
        format!(
            "zsync: jazsync\nFilename: test.pbo\nMTime: Thu, 01 Jan 1970 00:00:00 +0000\n\
             Blocksize: 8192\nLength: 10\nHash-Lengths: 1,2,3\n\
             URL: http://example.org/repo/@mod/addons/test.pbo\nSHA-1: {}\n\n",
            sha1_hex(&data)
        )
    );
    // One block: 2 rsum bytes + 3 MD4 bytes.
    assert_eq!(bytes.len(), header.len() + 5);
}

#[test]
fn identical_file_needs_nothing() {
    let data = pseudo_random(20 * BLOCK_SIZE + 123, 7);
    assert_eq!(patch(&data, &data), 0);
}

#[test]
fn empty_local_downloads_everything() {
    let data = pseudo_random(3 * BLOCK_SIZE + 5, 9);
    assert_eq!(patch(&[], &data), data.len() as u64);
}

#[test]
fn shifted_and_modified_content() {
    let old = pseudo_random(40 * BLOCK_SIZE + 999, 3);
    let mut new = Vec::new();
    new.extend_from_slice(&old[..10 * BLOCK_SIZE]);
    new.extend_from_slice(b"inserted bytes shift everything after them");
    new.extend_from_slice(&old[10 * BLOCK_SIZE..30 * BLOCK_SIZE]);
    new.extend_from_slice(&pseudo_random(2 * BLOCK_SIZE, 99)); // replaced region
    new.extend_from_slice(&old[32 * BLOCK_SIZE..]);
    let downloaded = patch(&old, &new);
    assert!(
        downloaded < 6 * BLOCK_SIZE as u64,
        "downloaded {downloaded} bytes"
    );
}

#[test]
fn truncated_and_grown_files() {
    let old = pseudo_random(10 * BLOCK_SIZE, 5);
    patch(&old, &old[..7 * BLOCK_SIZE + 100]);
    let mut grown = old.clone();
    grown.extend_from_slice(&pseudo_random(BLOCK_SIZE / 2, 6));
    patch(&old, &grown);
    patch(&old, &[]);
}

#[test]
fn rejects_wrong_sha1() {
    let data = pseudo_random(BLOCK_SIZE * 2, 1);
    let mut control = control_for(&data);
    control.header.sha1 = "deadbeef".to_owned();
    let matches = match_blocks(&control, &data[..], None).unwrap();
    let mut out = Vec::new();
    let err = assemble(&control, &matches, Cursor::new(&data), &[][..], &mut out);
    assert!(matches!(
        err,
        Err(a3s_zsync::Error::ChecksumMismatch { .. })
    ));
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(24))]
    #[test]
    fn random_edits(
        len in 0usize..(6 * BLOCK_SIZE),
        seed in 0u32..1000,
        cut in 0usize..(6 * BLOCK_SIZE),
        insert_len in 0usize..3000,
    ) {
        let old = pseudo_random(len, seed);
        let cut = cut.min(old.len());
        let mut new = old[..cut].to_vec();
        new.extend(pseudo_random(insert_len, seed + 1));
        new.extend_from_slice(&old[cut..]);
        patch(&old, &new);
    }
}
