use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};

use md4::{Digest as _, Md4};

use crate::{ControlFile, Error, Result, Rsum};

/// Which target blocks were found in the local file, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockMatches {
    /// For each target block, the offset of identical data in the local file.
    pub local_offsets: Vec<Option<u64>>,
}

impl BlockMatches {
    pub fn matched(&self) -> usize {
        self.local_offsets.iter().filter(|o| o.is_some()).count()
    }

    /// Percentage of blocks already available locally (`FileMaker.mapMatcher` result).
    pub fn completion_percent(&self) -> u8 {
        if self.local_offsets.is_empty() {
            return 0;
        }
        (self.matched() * 100 / self.local_offsets.len()) as u8
    }
}

/// How far ahead of the scan position the buffer is kept filled.
const LOOKAHEAD_BLOCKS: usize = 2;

/// Drop consumed bytes from the front of the buffer once this many accumulate.
const DRAIN_THRESHOLD: usize = 4 * 1024 * 1024;

/// Scans `local` (an older version of the file) for blocks of the target described by
/// `control`.
///
/// Like zsync, a block is accepted when its rolling checksum and truncated MD4 match, and — when
/// the control file requires 2 sequential matches — the following block matches too. The data
/// past the end of the local file is treated as zeros, mirroring the zero-padded last block.
pub fn match_blocks(
    control: &ControlFile,
    mut local: impl Read,
    cancel: Option<&AtomicBool>,
) -> Result<BlockMatches> {
    let block_count = control.block_count();
    let mut offsets = vec![None; block_count];
    if block_count == 0 {
        return Ok(BlockMatches {
            local_offsets: offsets,
        });
    }

    let bs = control.header.block_size;
    let lengths = control.header.hash_lengths;
    let mut index: HashMap<u32, Vec<u32>> = HashMap::with_capacity(block_count);
    for block in 0..block_count {
        let key = Rsum::key_from_stored(control.weak_of(block));
        index.entry(key).or_default().push(block as u32);
    }

    let mut buf: Vec<u8> = Vec::with_capacity(DRAIN_THRESHOLD + LOOKAHEAD_BLOCKS * bs);
    let mut base: u64 = 0; // file offset of buf[0]
    let mut local_len: u64 = 0; // bytes read from `local` so far
    let mut eof = false;
    let mut pos: u64 = 0;
    let mut rsum: Option<Rsum> = None;
    let mut since_cancel_check = 0usize;

    let mut chunk = vec![0u8; 1024 * 1024];
    loop {
        // Ensure buf covers [pos, pos + LOOKAHEAD_BLOCKS * bs).
        let need = (pos - base) as usize + LOOKAHEAD_BLOCKS * bs;
        while buf.len() < need && !eof {
            let n = local.read(&mut chunk)?;
            if n == 0 {
                eof = true;
            } else {
                buf.extend_from_slice(&chunk[..n]);
                local_len += n as u64;
            }
        }
        if eof && pos >= local_len {
            break;
        }
        if buf.len() < need {
            buf.resize(need, 0); // zero padding past EOF
        }

        let at = (pos - base) as usize;
        let window = &buf[at..at + bs];
        let sum = *rsum.get_or_insert_with(|| Rsum::of(window));

        let mut matched_here = false;
        if let Some(candidates) = index.get(&sum.key(lengths.rsum_bytes)) {
            let strong = &Md4::digest(window)[..lengths.checksum_bytes];
            let mut next_strong = None;
            for &block in candidates {
                let block = block as usize;
                if offsets[block].is_some() || control.strong_of(block) != strong {
                    continue;
                }
                let needs_next = lengths.seq_matches == 2 && block + 1 < block_count;
                if needs_next {
                    let next = next_strong.get_or_insert_with(|| {
                        Md4::digest(&buf[at + bs..at + 2 * bs])[..lengths.checksum_bytes].to_vec()
                    });
                    if control.strong_of(block + 1) != next.as_slice() {
                        continue;
                    }
                    if offsets[block + 1].is_none() {
                        offsets[block + 1] = Some(pos + bs as u64);
                    }
                }
                offsets[block] = Some(pos);
                matched_here = true;
            }
        }

        if matched_here {
            pos += bs as u64;
            rsum = None;
        } else {
            let old = buf[at];
            let new = buf[at + bs];
            if let Some(r) = rsum.as_mut() {
                r.roll(old, new, bs);
            }
            pos += 1;
        }

        since_cancel_check += 1;
        if since_cancel_check >= 1 << 20 {
            since_cancel_check = 0;
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err(Error::Cancelled);
            }
        }

        let consumed = (pos - base) as usize;
        if consumed >= DRAIN_THRESHOLD {
            buf.drain(..consumed);
            base = pos;
        }
    }

    Ok(BlockMatches {
        local_offsets: offsets,
    })
}
