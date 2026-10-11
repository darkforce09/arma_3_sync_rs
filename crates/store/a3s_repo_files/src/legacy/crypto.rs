//! The Java client's repository-file cipher (`dao/EncryptionProvider.java`).
//!
//! `Cipher.getInstance("AES")` resolves to `AES/ECB/PKCS5Padding` in `SunJCE`: every 16-byte block
//! is processed on its own, without an IV. The key is hard-coded, so this is obfuscation, not
//! protection.

use aes::Aes128;
use aes::cipher::{Array, BlockCipherDecrypt as _, BlockCipherEncrypt as _, KeyInit as _};

/// The hard-coded AES-128 key (note the `0x00` at index 7).
// Mirrors dao/EncryptionProvider.java:13-15
pub const JAVA_AES_KEY: [u8; 16] = [
    0x01, 0x72, 0x43, 0x3E, 0x1C, 0x7A, 0x55, 0x00, 0x01, 0x72, 0x43, 0x3E, 0x1C, 0x7A, 0x55, 0x4F,
];

/// AES block size in bytes.
const BLOCK: usize = 16;

/// A ciphertext that cannot be decrypted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    /// The length is zero or not a multiple of 16 (`IllegalBlockSizeException`).
    #[error("Input length must be a non-zero multiple of 16 when decrypting, found {0} bytes")]
    BadLength(usize),

    /// The PKCS#5 padding is invalid (`BadPaddingException`), e.g. a wrong key or corrupted data.
    #[error("Given final block not properly padded")]
    BadPadding,
}

fn cipher() -> Aes128 {
    Aes128::new(&Array::from(JAVA_AES_KEY))
}

/// `AES/ECB/PKCS5Padding` encryption with [`JAVA_AES_KEY`]; always adds 1 to 16 padding bytes.
// Mirrors dao/EncryptionProvider.java:19-24
pub fn encrypt(plain: &[u8]) -> Vec<u8> {
    let cipher = cipher();
    let pad = BLOCK - plain.len() % BLOCK;
    let mut out = Vec::with_capacity(plain.len() + pad);
    out.extend_from_slice(plain);
    // `pad` is in 1..=16, so the cast is lossless.
    out.extend(std::iter::repeat_n(pad as u8, pad));
    for chunk in out.chunks_exact_mut(BLOCK) {
        let mut block = Array::from(<[u8; BLOCK]>::try_from(&*chunk).expect("16-byte chunk"));
        cipher.encrypt_block(&mut block);
        chunk.copy_from_slice(&block);
    }
    out
}

/// `AES/ECB/PKCS5Padding` decryption with [`JAVA_AES_KEY`], validating the padding.
// Mirrors dao/EncryptionProvider.java:27-32
pub fn decrypt(cipher_text: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if cipher_text.is_empty() || !cipher_text.len().is_multiple_of(BLOCK) {
        return Err(CryptoError::BadLength(cipher_text.len()));
    }
    let cipher = cipher();
    let mut out = cipher_text.to_vec();
    for chunk in out.chunks_exact_mut(BLOCK) {
        let mut block = Array::from(<[u8; BLOCK]>::try_from(&*chunk).expect("16-byte chunk"));
        cipher.decrypt_block(&mut block);
        chunk.copy_from_slice(&block);
    }
    let pad = usize::from(*out.last().expect("non-empty"));
    if !(1..=BLOCK).contains(&pad)
        || out[out.len() - pad..]
            .iter()
            .any(|&b| usize::from(b) != pad)
    {
        return Err(CryptoError::BadPadding);
    }
    out.truncate(out.len() - pad);
    Ok(out)
}
