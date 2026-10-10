use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::Range;

use sha1::{Digest as _, Sha1};

use crate::{BlockMatches, ControlFile, Error, Result};

/// Byte ranges of the target file that must be downloaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Ascending, non-overlapping ranges, clipped to the file length.
    pub ranges: Vec<Range<u64>>,
}

impl Plan {
    /// Maximum blocks per HTTP range request (`MetaFileReader`: 100).
    pub const MAX_BLOCKS_PER_RANGE: usize = 100;

    /// Groups runs of missing blocks into ranges of at most `max_blocks` blocks.
    pub fn new(control: &ControlFile, matches: &BlockMatches, max_blocks: usize) -> Self {
        let bs = control.header.block_size as u64;
        let len = control.header.length;
        let max_blocks = max_blocks.max(1);
        let mut ranges = Vec::new();
        let mut run: Option<(usize, usize)> = None; // (first, count)
        let mut flush = |run: &mut Option<(usize, usize)>| {
            if let Some((first, count)) = run.take() {
                let start = first as u64 * bs;
                let end = ((first + count) as u64 * bs).min(len);
                ranges.push(start..end);
            }
        };
        for (block, offset) in matches.local_offsets.iter().enumerate() {
            if offset.is_some() {
                flush(&mut run);
                continue;
            }
            match &mut run {
                Some((_, count)) if *count < max_blocks => *count += 1,
                _ => {
                    flush(&mut run);
                    run = Some((block, 1));
                }
            }
        }
        flush(&mut run);
        Self { ranges }
    }

    /// Total bytes to download.
    pub fn download_size(&self) -> u64 {
        self.ranges.iter().map(|r| r.end - r.start).sum()
    }
}

/// Rebuilds the target file into `out`.
///
/// `remote` must yield the bytes of [`Plan::ranges`] concatenated in order. The output is
/// verified against the SHA-1 of the control file (Java skips this check).
pub fn assemble(
    control: &ControlFile,
    matches: &BlockMatches,
    mut local: impl Read + Seek,
    mut remote: impl Read,
    mut out: impl Write,
) -> Result<()> {
    let bs = control.header.block_size;
    let len = control.header.length;
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; bs];

    for (block, offset) in matches.local_offsets.iter().enumerate() {
        let start = block as u64 * bs as u64;
        let n = (len - start).min(bs as u64) as usize;
        let data = &mut buf[..n];
        match offset {
            Some(local_offset) => {
                local.seek(SeekFrom::Start(*local_offset))?;
                read_padded(&mut local, data)?;
            }
            None => remote.read_exact(data)?,
        }
        hasher.update(&*data);
        out.write_all(data)?;
    }
    out.flush()?;

    let actual = if len == 0 {
        "0".to_owned()
    } else {
        hex(&hasher.finalize())
    };
    let expected = &control.header.sha1;
    if !expected.is_empty() && *expected != actual {
        return Err(Error::ChecksumMismatch {
            expected: expected.clone(),
            actual,
        });
    }
    Ok(())
}

/// Reads as much as available, zero-filling past end of file.
fn read_padded(reader: &mut impl Read, buf: &mut [u8]) -> Result<()> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = reader.read(&mut buf[filled..])?;
        if n == 0 {
            buf[filled..].fill(0);
            break;
        }
        filled += n;
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(40), |mut s, b| {
        let _ignored = write!(s, "{b:02x}");
        s
    })
}
