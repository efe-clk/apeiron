//! Sealed boxes and repository keys (FORMAT.md §14.1, §14.3).

use std::fmt;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use crate::error::{Error, Result};
use crate::id::{IdKey, random_bytes};

const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
/// Bytes a sealed box adds to its plaintext.
pub(crate) const BOX_OVERHEAD: usize = NONCE_LEN + TAG_LEN;

/// A 32-byte secret key of XChaCha20-Poly1305.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SecretKey([u8; 32]);

impl SecretKey {
    pub(crate) fn generate() -> Result<Self> {
        Ok(SecretKey(random_bytes()?))
    }

    pub(crate) fn from_bytes(bytes: [u8; 32]) -> Self {
        SecretKey(bytes)
    }

    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Encrypts `plain` into `nonce ‖ ciphertext ‖ tag`.
    pub(crate) fn seal(&self, aad: &[u8], plain: &[u8]) -> Result<Vec<u8>> {
        let nonce: [u8; NONCE_LEN] = random_bytes()?;
        let cipher = XChaCha20Poly1305::new(&self.0.into());
        let sealed = cipher
            .encrypt(&XNonce::from(nonce), Payload { msg: plain, aad })
            .map_err(|_| Error::Encode("encryption failed".to_owned()))?;
        let mut out = Vec::with_capacity(BOX_OVERHEAD + plain.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    /// Decrypts a sealed box; `None` if it fails authentication.
    pub(crate) fn open(&self, aad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
        if sealed.len() < BOX_OVERHEAD {
            return None;
        }
        let (nonce, msg) = sealed.split_at(NONCE_LEN);
        let nonce = XNonce::try_from(nonce).ok()?;
        XChaCha20Poly1305::new(&self.0.into())
            .decrypt(&nonce, Payload { msg, aad })
            .ok()
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretKey(..)")
    }
}

/// The keys an open repository works with: `id_key` for object ids and, in
/// an encrypted repository, `data_key` for sealing stored data.
#[derive(Clone, Debug)]
pub(crate) struct Keys {
    pub(crate) id: IdKey,
    pub(crate) data: Option<SecretKey>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_tamper_detection() {
        let key = SecretKey::from_bytes([3; 32]);
        let sealed = key.seal(b"aad", b"hello").unwrap();
        assert_eq!(sealed.len(), 5 + BOX_OVERHEAD);
        assert_eq!(key.open(b"aad", &sealed).unwrap(), b"hello");

        assert!(key.open(b"other aad", &sealed).is_none());
        assert!(
            SecretKey::from_bytes([4; 32])
                .open(b"aad", &sealed)
                .is_none()
        );
        for i in 0..sealed.len() {
            let mut bad = sealed.clone();
            bad[i] ^= 1;
            assert!(key.open(b"aad", &bad).is_none(), "byte {i}");
        }
        assert!(key.open(b"aad", &sealed[..BOX_OVERHEAD - 1]).is_none());
    }

    #[test]
    fn nonces_differ() {
        let key = SecretKey::from_bytes([3; 32]);
        assert_ne!(key.seal(b"", b"x").unwrap(), key.seal(b"", b"x").unwrap());
    }
}
