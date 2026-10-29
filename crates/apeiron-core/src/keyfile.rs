//! Key files of an encrypted repository (FORMAT.md §14.2).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use argon2::{Algorithm, Argon2, Params, Version};
use data_encoding::BASE32_NOPAD;
use serde::{Deserialize, Serialize};
use toml::value::Datetime;

use crate::config::utc_datetime;
use crate::crypto::{Keys, SecretKey};
use crate::error::{Error, IoContext, Result};
use crate::fsutil;
use crate::id::{IdKey, parse_hex, random_bytes, to_hex};

const FORMAT: &str = "apeiron-key";
const VERSION: u32 = 1;
const KIND_PASSWORD: &str = "password";
const KIND_RECOVERY: &str = "recovery";
const ARGON2ID: &str = "argon2id";
const BLAKE3_DERIVE: &str = "blake3-derive";
const AAD_PREFIX: &[u8] = b"apeiron-key 1";
const RECOVERY_CONTEXT: &str = "apeiron 2026 recovery key";

/// Argon2id cost of new password key files. Tests use the smallest memory
/// cost so that opening a repository stays fast.
const MEMORY_KIB: u32 = if cfg!(test) { 8 } else { 65_536 };
const ITERATIONS: u32 = 3;
const PARALLELISM: u32 = 1;
/// Key files asking for more are refused, so a planted key file cannot make
/// a reader run out of memory or time.
const MAX_MEMORY_KIB: u32 = 4 * 1024 * 1024;
const MAX_ITERATIONS: u32 = 64;
const MAX_PARALLELISM: u32 = 64;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKeyFile {
    format: String,
    version: u32,
    kind: String,
    created: Datetime,
    kdf: RawKdf,
    sealed: RawSealed,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKdf {
    algorithm: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    memory_kib: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    iterations: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parallelism: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    salt: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSealed {
    keys: String,
}

/// What a password or recovery key is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyKind {
    Password,
    Recovery,
}

/// A key file of an encrypted repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyInfo {
    /// The name of the key file without `.key`.
    pub id: String,
    pub kind: KeyKind,
    /// When the key was added, as written in the key file (RFC 3339, UTC).
    pub created: String,
    /// Whether the repository was opened with this key.
    pub current: bool,
}

/// Writes a new key file that opens with `password` and returns its id.
pub(crate) fn write_password(
    keys_dir: &Path,
    tmp_dir: &Path,
    repo_id: &[u8; 16],
    id_key: &IdKey,
    data_key: &SecretKey,
    password: &str,
) -> Result<String> {
    let salt: [u8; 16] = random_bytes()?;
    let kek = argon2id(password, &salt, MEMORY_KIB, ITERATIONS, PARALLELISM)
        .map_err(|e| Error::Encode(format!("key derivation failed: {e}")))?;
    let kdf = RawKdf {
        algorithm: ARGON2ID.to_owned(),
        memory_kib: Some(MEMORY_KIB),
        iterations: Some(ITERATIONS),
        parallelism: Some(PARALLELISM),
        salt: Some(to_hex(&salt)),
    };
    write(
        keys_dir,
        tmp_dir,
        repo_id,
        id_key,
        data_key,
        KIND_PASSWORD,
        kdf,
        &kek,
    )
}

/// Writes a new key file that opens with a new random recovery key and
/// returns its id and the recovery key as the user writes it down.
pub(crate) fn write_recovery(
    keys_dir: &Path,
    tmp_dir: &Path,
    repo_id: &[u8; 16],
    id_key: &IdKey,
    data_key: &SecretKey,
) -> Result<(String, String)> {
    let recovery: [u8; 32] = random_bytes()?;
    let kdf = RawKdf {
        algorithm: BLAKE3_DERIVE.to_owned(),
        memory_kib: None,
        iterations: None,
        parallelism: None,
        salt: None,
    };
    let kek = recovery_kek(&recovery);
    let id = write(
        keys_dir,
        tmp_dir,
        repo_id,
        id_key,
        data_key,
        KIND_RECOVERY,
        kdf,
        &kek,
    )?;
    Ok((id, format_recovery(&recovery)))
}

#[allow(clippy::too_many_arguments)]
fn write(
    keys_dir: &Path,
    tmp_dir: &Path,
    repo_id: &[u8; 16],
    id_key: &IdKey,
    data_key: &SecretKey,
    kind: &str,
    kdf: RawKdf,
    kek: &SecretKey,
) -> Result<String> {
    let mut plain = Vec::with_capacity(64);
    plain.extend_from_slice(id_key.as_bytes());
    plain.extend_from_slice(data_key.as_bytes());
    let raw = RawKeyFile {
        format: FORMAT.to_owned(),
        version: VERSION,
        kind: kind.to_owned(),
        created: utc_datetime(SystemTime::now()),
        kdf,
        sealed: RawSealed {
            keys: to_hex(&kek.seal(&aad(repo_id), &plain)?),
        },
    };
    // Serializing plain strings and integers cannot fail.
    let text = toml::to_string(&raw).unwrap_or_default();
    let id = fsutil::random_name()?;
    let dest = keys_dir.join(format!("{id}.key"));
    fsutil::write_durable_new(tmp_dir, &dest, text.as_bytes())?;
    Ok(id)
}

/// The key files in `keys_dir` with their ids, sorted by id.
pub(crate) fn key_files(keys_dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(keys_dir).at(keys_dir)? {
        let path = entry.at(keys_dir)?.path();
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|_| path.extension().is_some_and(|e| e == "key"));
        if let Some(id) = id {
            files.push((id.to_owned(), path.clone()));
        }
    }
    files.sort();
    Ok(files)
}

/// The key files that can be parsed, oldest first; `current` is the id of
/// the key the repository was opened with.
pub(crate) fn list(keys_dir: &Path, current: Option<&str>) -> Result<Vec<KeyInfo>> {
    let mut keys = Vec::new();
    for (id, path) in key_files(keys_dir)? {
        let Some(raw) = parse(&fs::read_to_string(&path).at(&path)?) else {
            continue;
        };
        let kind = match raw.kind.as_str() {
            KIND_PASSWORD => KeyKind::Password,
            KIND_RECOVERY => KeyKind::Recovery,
            _ => continue,
        };
        keys.push(KeyInfo {
            current: current == Some(id.as_str()),
            id,
            kind,
            created: raw.created.to_string(),
        });
    }
    keys.sort_by(|a, b| (&a.created, &a.id).cmp(&(&b.created, &b.id)));
    Ok(keys)
}

/// Deletes the key file `id`, unless it is the last one that can be parsed.
/// The caller holds the repository lock exclusive (FORMAT.md §14.2).
pub(crate) fn remove(keys_dir: &Path, id: &str) -> Result<()> {
    let files = key_files(keys_dir)?;
    let Some((_, path)) = files.iter().find(|(name, _)| name == id) else {
        return Err(Error::KeyNotFound(id.to_owned()));
    };
    let mut others = 0;
    for (name, other) in &files {
        if name != id && parse(&fs::read_to_string(other).at(other)?).is_some() {
            others += 1;
        }
    }
    if others == 0 {
        return Err(Error::LastKey);
    }
    fsutil::checkpoint()
        .and_then(|()| fs::remove_file(path))
        .at(path)?;
    fsutil::sync_dir(keys_dir)
}

/// Opens the repository keys with a password or a recovery key by trying
/// every key file in `keys_dir`. Returns the keys and the id of the key
/// file that opened.
pub(crate) fn unlock(keys_dir: &Path, repo_id: &[u8; 16], secret: &str) -> Result<(Keys, String)> {
    let recovery = parse_recovery(secret);
    let mut readable = 0;
    for (key_id, path) in key_files(keys_dir)? {
        let text = fs::read_to_string(&path).at(&path)?;
        let Some(raw) = parse(&text) else {
            continue;
        };
        readable += 1;
        let kek = match raw.kind.as_str() {
            KIND_PASSWORD => password_kek(&raw.kdf, secret),
            KIND_RECOVERY => recovery.as_ref().map(recovery_kek),
            _ => None,
        };
        let Some(kek) = kek else {
            continue;
        };
        let opened = parse_hex_vec(&raw.sealed.keys).and_then(|s| kek.open(&aad(repo_id), &s));
        if let Some(plain) = opened.filter(|p| p.len() == 64) {
            let (id, data) = plain.split_at(32);
            let (Ok(id), Ok(data)) = (id.try_into(), data.try_into()) else {
                continue;
            };
            let keys = Keys {
                id: IdKey::from_bytes(id),
                data: Some(SecretKey::from_bytes(data)),
            };
            return Ok((keys, key_id));
        }
    }
    if readable == 0 {
        return Err(Error::InvalidRepository {
            path: keys_dir.to_path_buf(),
            reason: "no readable key file".to_owned(),
        });
    }
    Err(Error::WrongKey)
}

fn parse(text: &str) -> Option<RawKeyFile> {
    let raw: RawKeyFile = toml::from_str(text).ok()?;
    (raw.format == FORMAT && raw.version == VERSION).then_some(raw)
}

fn aad(repo_id: &[u8; 16]) -> Vec<u8> {
    [AAD_PREFIX, repo_id.as_slice()].concat()
}

fn password_kek(kdf: &RawKdf, password: &str) -> Option<SecretKey> {
    if kdf.algorithm != ARGON2ID {
        return None;
    }
    let memory = kdf.memory_kib.filter(|&m| m <= MAX_MEMORY_KIB)?;
    let iterations = kdf.iterations.filter(|&i| i <= MAX_ITERATIONS)?;
    let parallelism = kdf.parallelism.filter(|&p| p <= MAX_PARALLELISM)?;
    let salt: [u8; 16] = parse_hex(kdf.salt.as_deref()?)?;
    argon2id(password, &salt, memory, iterations, parallelism).ok()
}

fn argon2id(
    password: &str,
    salt: &[u8; 16],
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
) -> std::result::Result<SecretKey, argon2::Error> {
    let params = Params::new(memory_kib, iterations, parallelism, Some(32))?;
    let mut out = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params).hash_password_into(
        password.as_bytes(),
        salt,
        &mut out,
    )?;
    Ok(SecretKey::from_bytes(out))
}

fn recovery_kek(recovery: &[u8; 32]) -> SecretKey {
    SecretKey::from_bytes(blake3::derive_key(RECOVERY_CONTEXT, recovery))
}

/// 52 base32 characters in groups of four, joined by `-`.
fn format_recovery(recovery: &[u8; 32]) -> String {
    let text = BASE32_NOPAD.encode(recovery);
    let groups: Vec<&str> = text
        .as_bytes()
        .chunks(4)
        .filter_map(|g| std::str::from_utf8(g).ok())
        .collect();
    groups.join("-")
}

/// Reads a recovery key, ignoring case, spaces and dashes.
fn parse_recovery(text: &str) -> Option<[u8; 32]> {
    let clean: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    BASE32_NOPAD.decode(clean.as_bytes()).ok()?.try_into().ok()
}

fn parse_hex_vec(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Setup {
        dir: tempfile::TempDir,
        id_key: IdKey,
        data_key: SecretKey,
    }

    fn setup() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("keys")).unwrap();
        fs::create_dir(dir.path().join("tmp")).unwrap();
        Setup {
            dir,
            id_key: IdKey::from_bytes([1; 32]),
            data_key: SecretKey::from_bytes([2; 32]),
        }
    }

    impl Setup {
        fn keys(&self) -> PathBuf {
            self.dir.path().join("keys")
        }

        fn add_password(&self, password: &str) -> PathBuf {
            let tmp = self.dir.path().join("tmp");
            let id = write_password(
                &self.keys(),
                &tmp,
                &[9; 16],
                &self.id_key,
                &self.data_key,
                password,
            )
            .unwrap();
            self.keys().join(format!("{id}.key"))
        }

        fn add_recovery(&self) -> String {
            let tmp = self.dir.path().join("tmp");
            write_recovery(&self.keys(), &tmp, &[9; 16], &self.id_key, &self.data_key)
                .unwrap()
                .1
        }
    }

    #[test]
    fn opens_with_any_password_or_the_recovery_key() {
        let s = setup();
        s.add_password("first");
        s.add_password("second");
        let recovery = s.add_recovery();
        assert_eq!(recovery.len(), 52 + 12);

        let lower = recovery.to_lowercase().replace('-', " ");
        for secret in ["first", "second", recovery.as_str(), lower.as_str()] {
            let (keys, _) = unlock(&s.keys(), &[9; 16], secret).unwrap();
            assert_eq!(keys.id, s.id_key);
            assert_eq!(keys.data, Some(s.data_key.clone()));
        }
        assert!(matches!(
            unlock(&s.keys(), &[9; 16], "third"),
            Err(Error::WrongKey)
        ));
    }

    #[test]
    fn key_file_is_bound_to_its_repository() {
        let s = setup();
        s.add_password("pw");
        assert!(matches!(
            unlock(&s.keys(), &[8; 16], "pw"),
            Err(Error::WrongKey)
        ));
    }

    #[test]
    fn damaged_key_file_is_skipped_and_never_opens() {
        let s = setup();
        let damaged = s.add_password("pw");
        let text = fs::read_to_string(&damaged).unwrap();
        // Flip one hex digit of the sealed keys.
        let at = text.find("keys = \"").unwrap() + 20;
        let mut bytes = text.into_bytes();
        bytes[at] = if bytes[at] == b'0' { b'1' } else { b'0' };
        fs::write(&damaged, bytes).unwrap();
        assert!(matches!(
            unlock(&s.keys(), &[9; 16], "pw"),
            Err(Error::WrongKey)
        ));

        fs::write(&damaged, "not toml").unwrap();
        assert!(matches!(
            unlock(&s.keys(), &[9; 16], "pw"),
            Err(Error::InvalidRepository { .. })
        ));
        s.add_password("pw");
        unlock(&s.keys(), &[9; 16], "pw").unwrap();
    }

    #[test]
    fn refuses_excessive_kdf_cost() {
        let s = setup();
        let path = s.add_password("pw");
        let text = fs::read_to_string(&path).unwrap();
        let costly = text.replace("memory_kib = 8", "memory_kib = 4294967295");
        assert_ne!(costly, text);
        fs::write(&path, costly).unwrap();
        assert!(matches!(
            unlock(&s.keys(), &[9; 16], "pw"),
            Err(Error::WrongKey)
        ));
    }

    #[test]
    fn list_and_remove_keep_one_key() {
        let s = setup();
        let first = s.add_password("first");
        let first_id = first.file_stem().unwrap().to_str().unwrap().to_owned();
        s.add_recovery();
        fs::write(s.keys().join("ffff.key"), "not toml").unwrap();

        let keys = list(&s.keys(), Some(&first_id)).unwrap();
        assert_eq!(keys.len(), 2);
        let password = keys.iter().find(|k| k.id == first_id).unwrap();
        assert_eq!(password.kind, KeyKind::Password);
        assert!(password.current);
        let recovery = keys.iter().find(|k| k.id != first_id).unwrap();
        assert_eq!(recovery.kind, KeyKind::Recovery);
        assert!(!recovery.current);

        assert!(matches!(
            remove(&s.keys(), "nope"),
            Err(Error::KeyNotFound(_))
        ));
        remove(&s.keys(), &first_id).unwrap();
        assert!(!first.exists());
        assert!(matches!(
            unlock(&s.keys(), &[9; 16], "first"),
            Err(Error::WrongKey)
        ));
        // The damaged file does not count as a key that still opens.
        assert!(matches!(
            remove(&s.keys(), &recovery.id),
            Err(Error::LastKey)
        ));
        remove(&s.keys(), "ffff").unwrap();
        assert_eq!(list(&s.keys(), None).unwrap().len(), 1);
    }

    #[test]
    fn recovery_key_format() {
        let key = [0xA5; 32];
        let text = format_recovery(&key);
        assert_eq!(text.split('-').count(), 13);
        assert_eq!(parse_recovery(&text), Some(key));
        assert_eq!(parse_recovery("not a key"), None);
        assert_eq!(parse_recovery(&text[..text.len() - 1]), None);
    }
}
