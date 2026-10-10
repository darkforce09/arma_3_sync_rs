//! Java "modified UTF-8" as used by `DataOutput.writeUTF`.
//!
//! Differs from UTF-8 in two ways: U+0000 is encoded as `C0 80`, and supplementary characters are
//! encoded as two 3-byte surrogates instead of one 4-byte sequence.

use crate::{Error, Result};

pub fn encode(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut units = [0u16; 2];
    for c in s.chars() {
        for &unit in c.encode_utf16(&mut units).iter() {
            push_unit(&mut out, unit);
        }
    }
    out
}

fn push_unit(out: &mut Vec<u8>, unit: u16) {
    match unit {
        0x0001..=0x007F => out.push(unit as u8),
        0x0000 | 0x0080..=0x07FF => {
            out.push(0xC0 | ((unit >> 6) & 0x1F) as u8);
            out.push(0x80 | (unit & 0x3F) as u8);
        }
        _ => {
            out.push(0xE0 | ((unit >> 12) & 0x0F) as u8);
            out.push(0x80 | ((unit >> 6) & 0x3F) as u8);
            out.push(0x80 | (unit & 0x3F) as u8);
        }
    }
}

/// Decodes modified UTF-8. Unpaired surrogates are replaced with U+FFFD.
pub fn decode(bytes: &[u8]) -> Result<String> {
    let mut units = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i];
        let (unit, len) = if b0 & 0x80 == 0 {
            (u16::from(b0), 1)
        } else if b0 & 0xE0 == 0xC0 {
            let b1 = *bytes.get(i + 1).ok_or(Error::InvalidUtf8)?;
            if b1 & 0xC0 != 0x80 {
                return Err(Error::InvalidUtf8);
            }
            ((u16::from(b0 & 0x1F) << 6) | u16::from(b1 & 0x3F), 2)
        } else if b0 & 0xF0 == 0xE0 {
            let b1 = *bytes.get(i + 1).ok_or(Error::InvalidUtf8)?;
            let b2 = *bytes.get(i + 2).ok_or(Error::InvalidUtf8)?;
            if b1 & 0xC0 != 0x80 || b2 & 0xC0 != 0x80 {
                return Err(Error::InvalidUtf8);
            }
            (
                (u16::from(b0 & 0x0F) << 12) | (u16::from(b1 & 0x3F) << 6) | u16::from(b2 & 0x3F),
                3,
            )
        } else {
            return Err(Error::InvalidUtf8);
        };
        units.push(unit);
        i += len;
    }
    Ok(String::from_utf16_lossy(&units))
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    #[test]
    fn round_trip() {
        for s in ["", "abc", "racine", "\0", "é", "日本", "😀 emoji", "a\0b"] {
            assert_eq!(decode(&encode(s)).unwrap(), s);
        }
    }

    #[test]
    fn special_encodings() {
        assert_eq!(encode("\0"), [0xC0, 0x80]);
        // U+1F600 → surrogates D83D DE00, each as 3 bytes.
        assert_eq!(encode("😀"), [0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80]);
    }
}
