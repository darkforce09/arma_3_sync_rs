/// The rsync/zsync rolling checksum, as implemented by `jazsync.Rsum`.
///
/// `a` is the sum of the bytes, `b` the sum of `(len - i) * byte`; both wrap at 16 bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rsum {
    pub a: u16,
    pub b: u16,
}

impl Rsum {
    /// Checksum of a whole block.
    pub fn of(block: &[u8]) -> Self {
        let mut a: u16 = 0;
        let mut b: u16 = 0;
        let len = block.len();
        for (i, &byte) in block.iter().enumerate() {
            let byte = u16::from(byte);
            a = a.wrapping_add(byte);
            b = b.wrapping_add(((len - i) as u16).wrapping_mul(byte));
        }
        Self { a, b }
    }

    /// Slides the window by one byte: removes `old` (the first byte), appends `new`.
    #[inline]
    pub fn roll(&mut self, old: u8, new: u8, block_len: usize) {
        let old = u16::from(old);
        self.a = self.a.wrapping_sub(old).wrapping_add(u16::from(new));
        self.b = self
            .b
            .wrapping_sub((block_len as u16).wrapping_mul(old))
            .wrapping_add(self.a);
    }

    /// `jazsync.Rsum.getValue()`: `a | b << 16`.
    pub fn value(self) -> u32 {
        u32::from(self.a) | (u32::from(self.b) << 16)
    }

    /// The `rsum_bytes` stored in a `.zsync` file (`MetaFileMaker.intToBytes`): the tail of
    /// big-endian `a` followed by big-endian `b`.
    pub fn stored_bytes(self, rsum_bytes: usize) -> Vec<u8> {
        let [a_hi, a_lo] = self.a.to_be_bytes();
        let [b_hi, b_lo] = self.b.to_be_bytes();
        let all = [a_hi, a_lo, b_hi, b_lo];
        all[4 - rsum_bytes.clamp(1, 4)..].to_vec()
    }

    /// Lookup key for the `rsum_bytes` stored bytes.
    pub fn key(self, rsum_bytes: usize) -> u32 {
        let mask = if rsum_bytes >= 4 {
            u32::MAX
        } else {
            (1u32 << (8 * rsum_bytes)) - 1
        };
        (u32::from(self.a) << 16 | u32::from(self.b)) & mask
    }

    /// Lookup key from stored bytes, consistent with [`Self::key`].
    pub fn key_from_stored(stored: &[u8]) -> u32 {
        stored.iter().fold(0u32, |acc, &b| acc << 8 | u32::from(b))
    }
}

#[cfg(test)]
mod tests {
    use super::Rsum;

    #[test]
    fn rolling_matches_fresh() {
        let data: Vec<u8> = (0..200u32).map(|i| (i * 7 + 3) as u8).collect();
        let len = 32;
        let mut r = Rsum::of(&data[..len]);
        for start in 1..(data.len() - len) {
            r.roll(data[start - 1], data[start + len - 1], len);
            assert_eq!(r, Rsum::of(&data[start..start + len]), "start {start}");
        }
    }

    #[test]
    fn stored_byte_order() {
        let r = Rsum {
            a: 0x1234,
            b: 0xabcd,
        };
        assert_eq!(r.stored_bytes(4), [0x12, 0x34, 0xab, 0xcd]);
        assert_eq!(r.stored_bytes(3), [0x34, 0xab, 0xcd]);
        assert_eq!(r.stored_bytes(2), [0xab, 0xcd]);
        for n in 2..=4 {
            assert_eq!(r.key(n), Rsum::key_from_stored(&r.stored_bytes(n)));
        }
    }
}
