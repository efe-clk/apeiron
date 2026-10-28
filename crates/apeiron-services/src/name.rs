//! Names and paths inside a snapshot (API.md §8.7).

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Bytes of a file name, or of a `/`-separated path inside a snapshot.
///
/// In JSON it is a string when the bytes are valid UTF-8 and an object
/// `{"hex": "…"}` otherwise, so no name is ever lost.
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name(pub Vec<u8>);

impl Name {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// `self/child`, or `child` when `self` is empty.
    pub fn join(&self, child: &[u8]) -> Name {
        let mut out = self.0.clone();
        if !out.is_empty() {
            out.push(b'/');
        }
        out.extend_from_slice(child);
        Name(out)
    }
}

impl From<&str> for Name {
    fn from(s: &str) -> Self {
        Name(s.as_bytes().to_vec())
    }
}

/// Shows the name, replacing invalid UTF-8.
impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&String::from_utf8_lossy(&self.0))
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", String::from_utf8_lossy(&self.0))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Repr {
    Text(String),
    Bytes { hex: String },
}

impl Serialize for Name {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match std::str::from_utf8(&self.0) {
            Ok(text) => Repr::Text(text.to_owned()),
            Err(_) => Repr::Bytes {
                hex: self.0.iter().map(|b| format!("{b:02x}")).collect(),
            },
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Name {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Repr::deserialize(deserializer)? {
            Repr::Text(text) => Ok(Name(text.into_bytes())),
            Repr::Bytes { hex } => {
                let digits = hex.as_bytes();
                if digits.len() % 2 != 0 {
                    return Err(serde::de::Error::custom("odd number of hex digits"));
                }
                digits
                    .chunks(2)
                    .map(|pair| {
                        std::str::from_utf8(pair)
                            .ok()
                            .and_then(|p| u8::from_str_radix(p, 16).ok())
                            .ok_or_else(|| serde::de::Error::custom("invalid hex digit"))
                    })
                    .collect::<Result<Vec<u8>, D::Error>>()
                    .map(Name)
            }
        }
    }
}
