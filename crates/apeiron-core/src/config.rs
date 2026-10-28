//! The `config` file (FORMAT.md §3).

use std::path::Path;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use toml::value::{Date, Datetime, Offset, Time};

use crate::chunker::ChunkerParams;
use crate::error::{Error, Result};
use crate::id::{IdKey, parse_hex, random_bytes, to_hex};

const FORMAT: &str = "apeiron-repository";
/// Version of the repository format this crate reads and writes.
pub const FORMAT_VERSION: u32 = 1;
const VERSION: u32 = FORMAT_VERSION;
const HASH_ALGORITHM: &str = "blake3-keyed";
const CHUNKER_ALGORITHM: &str = "fastcdc-2020";
const ENCRYPTION_NONE: &str = "none";

#[derive(Clone, Debug)]
pub(crate) struct Config {
    pub(crate) id: [u8; 16],
    pub(crate) id_key: IdKey,
    pub(crate) chunker: ChunkerParams,
    created: Datetime,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    format: String,
    version: u32,
    id: String,
    created: Datetime,
    hash: RawHash,
    chunker: RawChunker,
    encryption: RawEncryption,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHash {
    algorithm: String,
    id_key: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawChunker {
    algorithm: String,
    min_size: u32,
    avg_size: u32,
    max_size: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEncryption {
    mode: String,
}

impl Config {
    pub(crate) fn generate(chunker: ChunkerParams) -> Result<Self> {
        Ok(Config {
            id: random_bytes()?,
            id_key: IdKey::generate()?,
            chunker,
            created: utc_datetime(SystemTime::now()),
        })
    }

    pub(crate) fn to_toml(&self) -> String {
        let raw = RawConfig {
            format: FORMAT.to_owned(),
            version: VERSION,
            id: to_hex(&self.id),
            created: self.created,
            hash: RawHash {
                algorithm: HASH_ALGORITHM.to_owned(),
                id_key: to_hex(self.id_key.as_bytes()),
            },
            chunker: RawChunker {
                algorithm: CHUNKER_ALGORITHM.to_owned(),
                min_size: self.chunker.min_size(),
                avg_size: self.chunker.avg_size(),
                max_size: self.chunker.max_size(),
            },
            encryption: RawEncryption {
                mode: ENCRYPTION_NONE.to_owned(),
            },
        };
        // Serializing plain strings and integers cannot fail.
        toml::to_string(&raw).unwrap_or_default()
    }

    pub(crate) fn from_toml(path: &Path, text: &str) -> Result<Self> {
        let invalid = |reason: String| Error::InvalidRepository {
            path: path.to_path_buf(),
            reason,
        };
        let raw: RawConfig = toml::from_str(text).map_err(|e| invalid(e.to_string()))?;
        if raw.format != FORMAT {
            return Err(invalid(format!("unknown format {:?}", raw.format)));
        }
        if raw.version != VERSION {
            return Err(Error::UnsupportedVersion {
                path: path.to_path_buf(),
                version: raw.version,
            });
        }
        if raw.hash.algorithm != HASH_ALGORITHM {
            return Err(invalid(format!("unknown hash {:?}", raw.hash.algorithm)));
        }
        if raw.chunker.algorithm != CHUNKER_ALGORITHM {
            return Err(invalid(format!(
                "unknown chunker {:?}",
                raw.chunker.algorithm
            )));
        }
        if raw.encryption.mode != ENCRYPTION_NONE {
            return Err(invalid(format!(
                "unsupported encryption {:?}",
                raw.encryption.mode
            )));
        }
        let id = parse_hex(&raw.id).ok_or_else(|| invalid("bad repository id".to_owned()))?;
        let id_key = parse_hex(&raw.hash.id_key).ok_or_else(|| invalid("bad id_key".to_owned()))?;
        let chunker = ChunkerParams::from_parts(
            raw.chunker.min_size,
            raw.chunker.avg_size,
            raw.chunker.max_size,
        )
        .map_err(|e| invalid(e.to_string()))?;
        Ok(Config {
            id,
            id_key: IdKey::from_bytes(id_key),
            chunker,
            created: raw.created,
        })
    }
}

/// Converts a time to a UTC TOML datetime with second precision.
fn utc_datetime(time: SystemTime) -> Datetime {
    let secs = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil-from-days (Howard Hinnant), valid for all dates after 1970.
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    Datetime {
        date: Some(Date {
            year: year as u16,
            month: month as u8,
            day: day as u8,
        }),
        time: Some(Time {
            hour: (rem / 3600) as u8,
            minute: (rem / 60 % 60) as u8,
            second: Some((rem % 60) as u8),
            nanosecond: None,
        }),
        offset: Some(Offset::Z),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_datetime_formats_known_instants() {
        let at = |secs| utc_datetime(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs));
        assert_eq!(at(0).to_string(), "1970-01-01T00:00:00Z");
        assert_eq!(at(951_782_400).to_string(), "2000-02-29T00:00:00Z");
        assert_eq!(at(1_793_102_400).to_string(), "2026-10-27T12:00:00Z");
    }

    #[test]
    fn round_trip() {
        let config = Config::generate(ChunkerParams::default()).unwrap();
        let text = config.to_toml();
        assert!(text.contains("format = \"apeiron-repository\""));
        assert!(text.contains("avg_size = 1048576"));
        let parsed = Config::from_toml(Path::new("config"), &text).unwrap();
        assert_eq!(parsed.id, config.id);
        assert_eq!(parsed.id_key, config.id_key);
        assert_eq!(parsed.chunker, config.chunker);
        assert_eq!(parsed.to_toml(), text);
    }

    #[test]
    fn rejects_newer_version_and_unknown_fields() {
        let text = Config::generate(ChunkerParams::default())
            .unwrap()
            .to_toml();
        let newer = text.replace("version = 1", "version = 2");
        assert!(matches!(
            Config::from_toml(Path::new("config"), &newer),
            Err(Error::UnsupportedVersion { version: 2, .. })
        ));
        let extra = format!("surprise = true\n{text}");
        assert!(matches!(
            Config::from_toml(Path::new("config"), &extra),
            Err(Error::InvalidRepository { .. })
        ));
    }
}
