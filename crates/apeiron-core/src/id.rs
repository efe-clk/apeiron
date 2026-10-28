//! Object identifiers (FORMAT.md §4, §5).

use std::fmt;

use crate::error::{Error, Result};

/// Length of an object identifier in bytes.
pub const ID_LEN: usize = 32;

/// Identifier of an object: its keyed BLAKE3 hash.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId([u8; ID_LEN]);

impl ObjectId {
    pub const fn from_bytes(bytes: [u8; ID_LEN]) -> Self {
        ObjectId(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; ID_LEN] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        to_hex(&self.0)
    }

    /// Parses 64 lowercase hex characters.
    pub fn from_hex(s: &str) -> Option<Self> {
        parse_hex(s).map(ObjectId)
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl serde::Serialize for ObjectId {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> serde::Deserialize<'de> for ObjectId {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let hex = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        ObjectId::from_hex(&hex)
            .ok_or_else(|| serde::de::Error::custom("expected 64 lowercase hex characters"))
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ObjectId({self})")
    }
}

/// Kind of a stored object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ObjectType {
    Chunk = 0x01,
    Blob = 0x02,
    Tree = 0x03,
    Snapshot = 0x04,
}

impl ObjectType {
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x01 => Some(ObjectType::Chunk),
            0x02 => Some(ObjectType::Blob),
            0x03 => Some(ObjectType::Tree),
            0x04 => Some(ObjectType::Snapshot),
            _ => None,
        }
    }

    pub fn as_byte(self) -> u8 {
        self as u8
    }
}

/// Per-repository key used to derive object identifiers.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct IdKey([u8; 32]);

impl IdKey {
    pub(crate) fn generate() -> Result<Self> {
        Ok(IdKey(random_bytes()?))
    }

    pub(crate) fn from_bytes(bytes: [u8; 32]) -> Self {
        IdKey(bytes)
    }

    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// `ID = BLAKE3_keyed(id_key, type_byte ‖ content)`.
    pub(crate) fn object_id(&self, ty: ObjectType, content: &[u8]) -> ObjectId {
        let mut hasher = blake3::Hasher::new_keyed(&self.0);
        hasher.update(&[ty.as_byte()]);
        hasher.update(content);
        ObjectId(*hasher.finalize().as_bytes())
    }
}

impl fmt::Debug for IdKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("IdKey(..)")
    }
}

pub(crate) fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).map_err(Error::Random)?;
    Ok(buf)
}

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    out
}

/// Parses exactly `2 * N` lowercase hex characters.
pub(crate) fn parse_hex<const N: usize>(s: &str) -> Option<[u8; N]> {
    fn digit(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        }
    }
    let s = s.as_bytes();
    if s.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (i, pair) in s.chunks_exact(2).enumerate() {
        out[i] = (digit(pair[0])? << 4) | digit(pair[1])?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_id_is_keyed_blake3_over_type_and_content() {
        let key = IdKey::from_bytes([7; 32]);
        let id = key.object_id(ObjectType::Chunk, b"hello");
        let expected = blake3::keyed_hash(&[7; 32], b"\x01hello");
        assert_eq!(id.as_bytes(), expected.as_bytes());
    }

    #[test]
    fn type_and_key_change_the_id() {
        let key = IdKey::from_bytes([7; 32]);
        let other_key = IdKey::from_bytes([8; 32]);
        let chunk = key.object_id(ObjectType::Chunk, b"x");
        assert_ne!(chunk, key.object_id(ObjectType::Blob, b"x"));
        assert_ne!(chunk, other_key.object_id(ObjectType::Chunk, b"x"));
    }

    #[test]
    fn hex_round_trip() {
        let id = ObjectId::from_bytes([0xab; 32]);
        let hex = id.to_hex();
        assert_eq!(hex, "ab".repeat(32));
        assert_eq!(ObjectId::from_hex(&hex), Some(id));
    }

    #[test]
    fn hex_rejects_uppercase_and_wrong_length() {
        assert_eq!(ObjectId::from_hex(&"AB".repeat(32)), None);
        assert_eq!(ObjectId::from_hex("abcd"), None);
        assert_eq!(ObjectId::from_hex(&"zz".repeat(32)), None);
    }

    #[test]
    fn object_type_bytes() {
        for ty in [
            ObjectType::Chunk,
            ObjectType::Blob,
            ObjectType::Tree,
            ObjectType::Snapshot,
        ] {
            assert_eq!(ObjectType::from_byte(ty.as_byte()), Some(ty));
        }
        assert_eq!(ObjectType::from_byte(0), None);
        assert_eq!(ObjectType::from_byte(5), None);
    }
}
