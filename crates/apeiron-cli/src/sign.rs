//! Signing a report with an Ed25519 key, and checking the signature
//! (docs/REPORTS.md §5.1).

use std::io::Write;
use std::path::{Path, PathBuf};

use apeiron::{Error, ErrorKind};
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde::{Deserialize, Serialize};

/// The signature file beside a report, `<report>.sig`.
#[derive(Serialize, Deserialize)]
pub(crate) struct Signature {
    pub(crate) algorithm: String,
    /// The signer's public key, hex.
    pub(crate) public_key: String,
    /// The signature of the report file's bytes, hex.
    pub(crate) signature: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

fn io(path: &Path, e: std::io::Error) -> Error {
    Error::new(ErrorKind::Io, format!("{}: {e}", path.display()))
}

/// Where the signature of `report` goes.
pub(crate) fn signature_file(report: &Path) -> PathBuf {
    let mut name = report.as_os_str().to_owned();
    name.push(".sig");
    PathBuf::from(name)
}

/// Makes a new key in `file`, readable by its owner only; returns the
/// public key, hex.
pub(crate) fn new_key(file: &Path) -> Result<String, Error> {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
        .map_err(|_| Error::new(ErrorKind::Internal, "cannot make a key"))?;
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
        .map_err(|_| Error::new(ErrorKind::Internal, "cannot read the new key"))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut out = options.open(file).map_err(|e| io(file, e))?;
    out.write_all(pkcs8.as_ref())
        .and_then(|()| out.sync_all())
        .map_err(|e| io(file, e))?;
    Ok(hex(pair.public_key().as_ref()))
}

/// Signs `bytes` with the key in `key`.
pub(crate) fn sign(key: &Path, bytes: &[u8]) -> Result<Signature, Error> {
    let pkcs8 = std::fs::read(key).map_err(|e| io(key, e))?;
    let pair = Ed25519KeyPair::from_pkcs8(&pkcs8).map_err(|_| {
        Error::new(
            ErrorKind::InvalidInput,
            format!(
                "{} is not a key made by `apeiron report key`",
                key.display()
            ),
        )
    })?;
    Ok(Signature {
        algorithm: "ed25519".to_owned(),
        public_key: hex(pair.public_key().as_ref()),
        signature: hex(pair.sign(bytes).as_ref()),
    })
}

/// Checks that `signature` signs `bytes`, and, when `expected` is given,
/// with that public key. A wrong signature is `corrupt`.
pub(crate) fn check(
    bytes: &[u8],
    signature: &Signature,
    expected: Option<&str>,
) -> Result<(), Error> {
    let bad = |why: &str| Error::new(ErrorKind::Corrupt, why.to_owned());
    if signature.algorithm != "ed25519" {
        return Err(bad("the signature is not ed25519"));
    }
    if expected.is_some_and(|k| !k.eq_ignore_ascii_case(&signature.public_key)) {
        return Err(bad("the report was signed with another key"));
    }
    let key = unhex(&signature.public_key).ok_or_else(|| bad("a broken public key"))?;
    let sig = unhex(&signature.signature).ok_or_else(|| bad("a broken signature"))?;
    UnparsedPublicKey::new(&ED25519, key)
        .verify(bytes, &sig)
        .map_err(|_| bad("the signature does not match the report; it was changed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signed_report_is_checked() {
        let dir = std::env::temp_dir().join(format!("apeiron-sign-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = dir.join("key");
        let _ = std::fs::remove_file(&key);
        let public = new_key(&key).unwrap();
        assert!(new_key(&key).is_err());
        let signature = sign(&key, b"report").unwrap();
        assert_eq!(signature.public_key, public);
        check(b"report", &signature, Some(&public)).unwrap();
        assert!(check(b"changed", &signature, None).is_err());
        assert!(check(b"report", &signature, Some("00")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
