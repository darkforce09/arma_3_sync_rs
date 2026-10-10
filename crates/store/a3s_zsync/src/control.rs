use std::io::Read;

use md4::{Digest as _, Md4};

use crate::{Error, Result, Rsum};

/// Block size used by Arma3Sync (fixed in `MetaFileMaker`).
pub const BLOCK_SIZE: usize = 8192;

/// The three numbers of the `Hash-Lengths` header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HashLengths {
    /// Number of consecutive blocks that must match (1 or 2).
    pub seq_matches: usize,

    /// Bytes of the rolling checksum stored per block (2–4).
    pub rsum_bytes: usize,

    /// Bytes of the MD4 checksum stored per block.
    pub checksum_bytes: usize,
}

impl HashLengths {
    /// `MetaFileMaker.analyzeFile`, reproduced including its floating-point arithmetic.
    #[expect(
        clippy::imprecise_flops,
        reason = "must match Java's `Math.log(1 + x)` exactly"
    )]
    pub fn for_file(file_len: u64, block_size: usize) -> Self {
        let len = file_len as f64;
        let bs = block_size as f64;
        let blocks = (file_len / block_size as u64) as f64; // integer division in Java
        let seq_matches = if file_len > block_size as u64 { 2 } else { 1 };

        let rsum = (((len.ln() + bs.ln()) / 2f64.ln() - 8.6) / 8.0).ceil() as i32;
        let rsum_bytes = rsum.clamp(2, 4) as usize;

        let strong = ((20.0 + (len.ln() + (1.0 + blocks).ln()) / 2f64.ln())
            / f64::from(seq_matches as u8)
            / 8.0)
            .ceil() as i32;
        let strong2 = ((7.9 + (20.0 + (1.0 + blocks).ln() / 2f64.ln())) / 8.0) as i32;
        let checksum_bytes = strong.max(strong2).max(0) as usize;

        Self {
            seq_matches,
            rsum_bytes,
            checksum_bytes,
        }
    }
}

/// Header of a `.zsync` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// `zsync:` line; Arma3Sync writes `jazsync`.
    pub version: String,
    pub filename: String,

    /// `MTime:` line, e.g. `Sat, 06 Mar 2010 09:33:36 +0000`.
    pub mtime: String,
    pub block_size: usize,
    pub length: u64,
    pub hash_lengths: HashLengths,
    pub url: String,

    /// Lowercase hex SHA-1 of the complete file (`"0"` for an empty file).
    pub sha1: String,
}

/// A parsed or generated `.zsync` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlFile {
    pub header: Header,

    /// `rsum_bytes` per block, concatenated.
    pub weak: Vec<u8>,

    /// `checksum_bytes` per block, concatenated.
    pub strong: Vec<u8>,
}

impl ControlFile {
    pub fn block_count(&self) -> usize {
        self.header.length.div_ceil(self.header.block_size as u64) as usize
    }

    pub fn weak_of(&self, block: usize) -> &[u8] {
        let n = self.header.hash_lengths.rsum_bytes;
        &self.weak[block * n..(block + 1) * n]
    }

    pub fn strong_of(&self, block: usize) -> &[u8] {
        let n = self.header.hash_lengths.checksum_bytes;
        &self.strong[block * n..(block + 1) * n]
    }

    /// Computes the checksums of `data` (the complete file content, read from `reader`).
    ///
    /// `mtime` is the formatted modification time (see [`format_mtime`]), `url` the absolute URL
    /// of the file (Arma3Sync writes `<prompt><protocol url>/<relative path>`).
    pub fn generate(
        mut reader: impl Read,
        length: u64,
        filename: &str,
        mtime: &str,
        url: &str,
        sha1: &str,
    ) -> Result<Self> {
        let hash_lengths = HashLengths::for_file(length, BLOCK_SIZE);
        let blocks = length.div_ceil(BLOCK_SIZE as u64) as usize;
        let mut weak = Vec::with_capacity(blocks * hash_lengths.rsum_bytes);
        let mut strong = Vec::with_capacity(blocks * hash_lengths.checksum_bytes);
        let mut buf = vec![0u8; BLOCK_SIZE];
        let mut remaining = length;
        while remaining > 0 {
            let n = remaining.min(BLOCK_SIZE as u64) as usize;
            reader.read_exact(&mut buf[..n])?;
            // The last block is zero-padded to the full block size before hashing.
            buf[n..].fill(0);
            weak.extend(Rsum::of(&buf).stored_bytes(hash_lengths.rsum_bytes));
            let digest = Md4::digest(&buf);
            strong.extend_from_slice(&digest[..hash_lengths.checksum_bytes.min(digest.len())]);
            remaining -= n as u64;
        }
        Ok(Self {
            header: Header {
                version: "jazsync".to_owned(),
                filename: filename.to_owned(),
                mtime: mtime.to_owned(),
                block_size: BLOCK_SIZE,
                length,
                hash_lengths,
                url: url.to_owned(),
                sha1: sha1.to_owned(),
            },
            weak,
            strong,
        })
    }

    /// Serializes like `HeaderMaker.getFullHeader` followed by the block checksums.
    pub fn to_bytes(&self) -> Vec<u8> {
        let h = &self.header;
        let header = format!(
            "zsync: {}\nFilename: {}\nMTime: {}\nBlocksize: {}\nLength: {}\nHash-Lengths: {},{},{}\nURL: {}\nSHA-1: {}\n\n",
            h.version,
            h.filename,
            h.mtime,
            h.block_size,
            h.length,
            h.hash_lengths.seq_matches,
            h.hash_lengths.rsum_bytes,
            h.hash_lengths.checksum_bytes,
            h.url,
            h.sha1,
        );
        let mut out = header.into_bytes();
        let (rn, sn) = (h.hash_lengths.rsum_bytes, h.hash_lengths.checksum_bytes);
        for block in 0..self.block_count() {
            out.extend_from_slice(&self.weak[block * rn..(block + 1) * rn]);
            out.extend_from_slice(&self.strong[block * sn..(block + 1) * sn]);
        }
        out
    }

    /// Parses a `.zsync` file (jazsync or standard zsync without `Z-Map2`).
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let invalid = |msg: &str| Error::InvalidControlFile(msg.to_owned());
        let split = bytes
            .windows(2)
            .position(|w| w == b"\n\n")
            .ok_or_else(|| invalid("missing end of header"))?;
        let header_text =
            std::str::from_utf8(&bytes[..split]).map_err(|_err| invalid("header is not UTF-8"))?;
        let body = &bytes[split + 2..];

        let mut fields = std::collections::HashMap::new();
        for line in header_text.lines() {
            let line = line.trim_end_matches('\r');
            if let Some((key, value)) = line.split_once(':') {
                fields.insert(key.trim(), value.strip_prefix(' ').unwrap_or(value));
            }
        }
        let get = |key: &str| {
            fields
                .get(key)
                .copied()
                .ok_or_else(|| Error::InvalidControlFile(format!("missing {key}")))
        };
        let block_size: usize = get("Blocksize")?
            .trim()
            .parse()
            .map_err(|_err| invalid("bad Blocksize"))?;
        if block_size == 0 || !block_size.is_power_of_two() || block_size > 1 << 24 {
            return Err(invalid("bad Blocksize"));
        }
        let length: u64 = get("Length")?
            .trim()
            .parse()
            .map_err(|_err| invalid("bad Length"))?;
        let lengths: Vec<usize> = get("Hash-Lengths")?
            .split(',')
            .map(|s| s.trim().parse())
            .collect::<Result<_, _>>()
            .map_err(|_err| invalid("bad Hash-Lengths"))?;
        let [seq_matches, rsum_bytes, checksum_bytes] = lengths[..] else {
            return Err(invalid("bad Hash-Lengths"));
        };
        if !(1..=2).contains(&seq_matches)
            || !(1..=4).contains(&rsum_bytes)
            || !(3..=16).contains(&checksum_bytes)
        {
            return Err(invalid("Hash-Lengths out of range"));
        }

        let blocks = length.div_ceil(block_size as u64) as usize;
        let per_block = rsum_bytes + checksum_bytes;
        if body.len() < blocks.saturating_mul(per_block) {
            return Err(invalid("truncated block checksums"));
        }
        let mut weak = Vec::with_capacity(blocks * rsum_bytes);
        let mut strong = Vec::with_capacity(blocks * checksum_bytes);
        for chunk in body.chunks_exact(per_block).take(blocks) {
            weak.extend_from_slice(&chunk[..rsum_bytes]);
            strong.extend_from_slice(&chunk[rsum_bytes..]);
        }

        Ok(Self {
            header: Header {
                version: get("zsync")?.trim().to_owned(),
                filename: fields.get("Filename").copied().unwrap_or("").to_owned(),
                mtime: fields.get("MTime").copied().unwrap_or("").to_owned(),
                block_size,
                length,
                hash_lengths: HashLengths {
                    seq_matches,
                    rsum_bytes,
                    checksum_bytes,
                },
                url: fields.get("URL").copied().unwrap_or("").to_owned(),
                sha1: fields
                    .get("SHA-1")
                    .copied()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase(),
            },
            weak,
            strong,
        })
    }
}

/// Formats a modification time like Java's `SimpleDateFormat("EEE, dd MMM yyyy HH:mm:ss Z")`
/// in the local time zone.
pub fn format_mtime(millis: i64) -> String {
    use chrono::TimeZone as _;
    chrono::Local
        .timestamp_millis_opt(millis)
        .single()
        .map(|t| t.format("%a, %d %b %Y %H:%M:%S %z").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{BLOCK_SIZE, HashLengths};

    fn hl(len: u64) -> (usize, usize, usize) {
        let h = HashLengths::for_file(len, BLOCK_SIZE);
        (h.seq_matches, h.rsum_bytes, h.checksum_bytes)
    }

    #[test]
    // Expected values computed independently from the Java formula.
    fn hash_lengths_like_java() {
        assert_eq!(hl(0), (1, 2, 3));
        assert_eq!(hl(1), (1, 2, 3));
        assert_eq!(hl(8192), (1, 3, 5));
        assert_eq!(hl(8193), (2, 3, 3));
        assert_eq!(hl(1_000_000), (2, 4, 4));
        assert_eq!(hl(500_000_000), (2, 4, 5));
    }
}
