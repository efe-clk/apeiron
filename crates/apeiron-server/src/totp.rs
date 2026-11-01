//! Codes from an authenticator app as a second step of signing in with a
//! password (RFC 6238 with the usual settings: HMAC-SHA1, 30 seconds, six
//! digits; IDENTITY.md §4).

use aws_lc_rs::hmac;

use apeiron::{Error, ErrorKind};

/// Seconds one code is valid for.
const STEP: u64 = 30;
/// Codes of one step before and after are accepted too, for clocks that
/// differ and codes typed at the end of their step.
const WINDOW: u64 = 1;
const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// A new secret: 20 random bytes in base32, as authenticator apps take it.
pub fn new_secret() -> Result<String, Error> {
    let mut key = [0u8; 20];
    getrandom::fill(&mut key)
        .map_err(|e| Error::new(ErrorKind::Internal, format!("no random numbers: {e}")))?;
    Ok(base32(&key))
}

/// The address an authenticator app reads from a QR code or a link.
pub fn uri(secret: &str, user: &str, issuer: &str) -> String {
    let encode = |s: &str| form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
    format!(
        "otpauth://totp/{}:{}?secret={secret}&issuer={}&algorithm=SHA1&digits=6&period={STEP}",
        encode(issuer),
        encode(user),
        encode(issuer)
    )
}

/// The time step of the code `code` matches at `now` (seconds since 1970),
/// if it is right; `None` for a wrong or malformed code or secret.
pub(crate) fn check(secret: &str, code: &str, now: u64) -> Option<u64> {
    let code = code.trim();
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let wanted: u32 = code.parse().ok()?;
    let key = unbase32(secret)?;
    let now = now / STEP;
    (now.saturating_sub(WINDOW)..=now + WINDOW).find(|&step| code_at(&key, step) == wanted)
}

/// The code an authenticator app shows at `now` (seconds since 1970), as
/// a check that the secret was set up right; `None` for a malformed secret.
pub fn code(secret: &str, now: u64) -> Option<String> {
    Some(format!("{:06}", code_at(&unbase32(secret)?, now / STEP)))
}

/// The six-digit code of `step` (RFC 4226 §5.3).
fn code_at(key: &[u8], step: u64) -> u32 {
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, key);
    let mac = hmac::sign(&key, &step.to_be_bytes());
    let mac = mac.as_ref();
    let offset = usize::from(mac[mac.len() - 1] & 0x0f);
    let bytes = [
        mac[offset] & 0x7f,
        mac[offset + 1],
        mac[offset + 2],
        mac[offset + 3],
    ];
    u32::from_be_bytes(bytes) % 1_000_000
}

fn base32(data: &[u8]) -> String {
    let mut out = String::new();
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &byte in data {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(ALPHABET[((buffer >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(ALPHABET[((buffer << (5 - bits)) & 31) as usize]));
    }
    out
}

/// Reads base32 without padding, ignoring case and spaces.
fn unbase32(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buffer, mut bits) = (0u32, 0u32);
    for c in text.bytes().filter(|c| *c != b' ' && *c != b'=') {
        let value = ALPHABET.iter().position(|a| *a == c.to_ascii_uppercase())?;
        buffer = (buffer << 5) | value as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key of the test vectors of RFC 6238 appendix B, for SHA-1.
    const RFC_KEY: &[u8] = b"12345678901234567890";

    #[test]
    fn codes_match_the_rfc_test_vectors() {
        // The RFC gives eight digits; the last six are the six-digit code.
        for (time, code) in [
            (59u64, 94_287_082u32),
            (1_111_111_109, 7_081_804),
            (1_111_111_111, 14_050_471),
            (1_234_567_890, 89_005_924),
            (2_000_000_000, 69_279_037),
            (20_000_000_000, 65_353_130),
        ] {
            assert_eq!(code_at(RFC_KEY, time / STEP), code % 1_000_000, "{time}");
        }
    }

    #[test]
    fn a_code_is_accepted_one_step_around_and_never_further() {
        let secret = base32(RFC_KEY);
        assert_eq!(unbase32(&secret).unwrap(), RFC_KEY);
        assert_eq!(unbase32(&secret.to_lowercase()).unwrap(), RFC_KEY);
        let code = format!("{:06}", code_at(RFC_KEY, 1_234_567_890 / STEP));
        let step = 1_234_567_890 / STEP;
        for now in [1_234_567_890, 1_234_567_890 - STEP, 1_234_567_890 + STEP] {
            assert_eq!(check(&secret, &code, now), Some(step), "{now}");
        }
        assert_eq!(check(&secret, &code, 1_234_567_890 + 3 * STEP), None);
        assert_eq!(check(&secret, "12345", 1_234_567_890), None);
        assert_eq!(check(&secret, "abcdef", 1_234_567_890), None);
        assert_eq!(check("not base32!", &code, 1_234_567_890), None);
    }

    #[test]
    fn new_secrets_differ_and_make_an_address() {
        let (a, b) = (new_secret().unwrap(), new_secret().unwrap());
        assert_ne!(a, b);
        assert_eq!(unbase32(&a).unwrap().len(), 20);
        let link = uri(&a, "ayşe", "Apeiron");
        assert!(
            link.starts_with("otpauth://totp/Apeiron:ay%C5%9Fe?secret="),
            "{link}"
        );
    }
}
