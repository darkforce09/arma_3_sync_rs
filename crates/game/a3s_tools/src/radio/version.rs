//! The file version of a plugin dll, read from its Windows version resource.

use std::path::Path;

/// Signature that starts a `VS_FIXEDFILEINFO` structure, little endian.
const FIXED_FILE_INFO_SIGNATURE: [u8; 4] = 0xFEEF_04BD_u32.to_le_bytes();

/// The file version of a Windows executable or dll as `major.minor.patch.build`, from the first
/// `VS_FIXEDFILEINFO` of its version resource; `None` when it has none.
pub fn file_version(bytes: &[u8]) -> Option<String> {
    let start = bytes
        .windows(FIXED_FILE_INFO_SIGNATURE.len())
        .position(|window| window == FIXED_FILE_INFO_SIGNATURE)?;
    // The signature and the structure version come first, then the two file version words.
    let word = |offset: usize| {
        let at = start + offset;
        bytes
            .get(at..at + 4)
            .and_then(|slice| <[u8; 4]>::try_from(slice).ok())
            .map(u32::from_le_bytes)
    };
    let most = word(8)?;
    let least = word(12)?;
    Some(format!(
        "{}.{}.{}.{}",
        most >> 16,
        most & 0xFFFF,
        least >> 16,
        least & 0xFFFF
    ))
}

/// [`file_version`] of the file at `path`; `None` when it cannot be read or has no version.
pub fn plugin_version(path: &Path) -> Option<String> {
    file_version(&std::fs::read(path).ok()?)
}
