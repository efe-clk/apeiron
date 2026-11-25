//! Key management for Apeiron's key files (KEYS.md): services that wrap a
//! key-encryption key (FORMAT.md §14.2), and splitting a recovery key into
//! shares. The repository's own keys never leave the computer; a service
//! only ever sees the 32 random bytes of a key-encryption key.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod aws;
mod gcp;
mod pkcs11;
pub mod shamir;
mod vault;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("{provider}: {message}")]
    Service { provider: String, message: String },
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn service(provider: &str, message: impl std::fmt::Display) -> Error {
    Error::Service {
        provider: provider.to_owned(),
        message: message.to_string(),
    }
}

/// The services a key can be held by.
pub const PROVIDERS: [&str; 4] = ["vault", "aws-kms", "gcp-kms", "pkcs11"];

/// Splits `<provider>:<key>`, such as `vault:apeiron` or
/// `aws-kms:arn:aws:kms:eu-west-1:111122223333:key/…`.
pub fn parse(spec: &str) -> Result<(String, String)> {
    let (provider, key) = spec
        .split_once(':')
        .filter(|(p, k)| PROVIDERS.contains(p) && !k.is_empty())
        .ok_or_else(|| {
            Error::Invalid(format!(
                "{spec:?} is not <provider>:<key>; the providers are {}",
                PROVIDERS.join(", ")
            ))
        })?;
    Ok((provider.to_owned(), key.to_owned()))
}

/// 32 random bytes for a new key-encryption key.
pub fn new_kek() -> Result<[u8; 32]> {
    let mut kek = [0u8; 32];
    getrandom::fill(&mut kek).map_err(|e| Error::Invalid(format!("random bytes: {e}")))?;
    Ok(kek)
}

/// Has `provider` wrap `kek` with its key `key`; the text it gives back is
/// what the key file keeps.
pub fn wrap(provider: &str, key: &str, kek: &[u8; 32]) -> Result<String> {
    match provider {
        "vault" => vault::wrap(key, kek),
        "aws-kms" => aws::wrap(key, kek),
        "gcp-kms" => gcp::wrap(key, kek),
        "pkcs11" => pkcs11::wrap(key, kek),
        _ => Err(Error::Invalid(format!("unknown provider {provider:?}"))),
    }
}

/// Has `provider` unwrap what [`wrap`] gave.
pub fn unwrap(provider: &str, key: &str, wrapped: &str) -> Result<[u8; 32]> {
    let plain = match provider {
        "vault" => vault::unwrap(key, wrapped)?,
        "aws-kms" => aws::unwrap(key, wrapped)?,
        "gcp-kms" => gcp::unwrap(key, wrapped)?,
        "pkcs11" => pkcs11::unwrap(key, wrapped)?,
        _ => return Err(Error::Invalid(format!("unknown provider {provider:?}"))),
    };
    plain
        .try_into()
        .map_err(|_| service(provider, "the unwrapped key is not 32 bytes"))
}

/// The 32 bytes of a recovery key as the user writes it (FORMAT.md §14.2):
/// any letter case, with or without spaces and dashes.
pub fn recovery_bytes(text: &str) -> Result<[u8; 32]> {
    let clean: String = text
        .chars()
        .filter(|c| !matches!(c, '-' | ' '))
        .map(|c| c.to_ascii_uppercase())
        .collect();
    data_encoding::BASE32_NOPAD
        .decode(clean.as_bytes())
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b).ok())
        .ok_or_else(|| Error::Invalid("not a recovery key".to_owned()))
}

/// A recovery key as it is shown: 52 base32 characters in groups of four
/// joined by `-`.
pub fn recovery_text(bytes: &[u8; 32]) -> String {
    let text = data_encoding::BASE32_NOPAD.encode(bytes);
    let groups: Vec<&str> = text
        .as_bytes()
        .chunks(4)
        .filter_map(|c| std::str::from_utf8(c).ok())
        .collect();
    groups.join("-")
}

pub(crate) fn b64(bytes: &[u8]) -> String {
    data_encoding::BASE64.encode(bytes)
}

pub(crate) fn from_b64(provider: &str, text: &str) -> Result<Vec<u8>> {
    data_encoding::BASE64
        .decode(text.as_bytes())
        .map_err(|e| service(provider, format!("not base64: {e}")))
}

/// Posts JSON and reads the JSON answer; an answer that is not a success
/// is an error with its text.
pub(crate) fn post(
    provider: &str,
    url: &str,
    headers: &[(&str, String)],
    body: &serde_json::Value,
) -> Result<serde_json::Value> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| service(provider, e))?;
    let mut request = client.post(url).body(body.to_string());
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let response = request.send().map_err(|e| service(provider, e))?;
    let status = response.status();
    let text = response.text().map_err(|e| service(provider, e))?;
    if !status.is_success() {
        return Err(service(provider, format!("{status}: {}", text.trim())));
    }
    serde_json::from_str(&text).map_err(|e| service(provider, format!("not JSON: {e}")))
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_name_a_provider_and_a_key() {
        assert_eq!(
            parse("vault:apeiron").unwrap(),
            ("vault".to_owned(), "apeiron".to_owned())
        );
        let arn = "aws-kms:arn:aws:kms:eu-west-1:111122223333:key/1234";
        assert_eq!(
            parse(arn).unwrap().1,
            "arn:aws:kms:eu-west-1:111122223333:key/1234"
        );
        for bad in ["vault", "vault:", "kms:x", "apeiron"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert_ne!(new_kek().unwrap(), new_kek().unwrap());
        let key = [9u8; 32];
        let text = recovery_text(&key);
        assert_eq!(text.len(), 52 + 12);
        assert_eq!(
            recovery_bytes(&text.to_lowercase().replace('-', " ")).unwrap(),
            key
        );
    }
}
