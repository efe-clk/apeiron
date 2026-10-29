//! Git ref names as Apeiron ref names (GIT.md §3).

/// Escapes each `/`-separated part of a Git name: `a-z`, `0-9`, `-` and a
/// `.` that does not start the part stay; every other byte becomes `_` and
/// two lowercase hex digits.
pub(crate) fn escape(name: &[u8]) -> String {
    let parts: Vec<String> = name
        .split(|&b| b == b'/')
        .map(|part| {
            let mut out = String::new();
            for (i, &b) in part.iter().enumerate() {
                match b {
                    b'a'..=b'z' | b'0'..=b'9' | b'-' => out.push(char::from(b)),
                    b'.' if i > 0 => out.push('.'),
                    _ => out.push_str(&format!("_{b:02x}")),
                }
            }
            out
        })
        .collect();
    parts.join("/")
}

/// Reverses [`escape`]; `None` for a name `escape` cannot have written.
pub(crate) fn unescape(name: &str) -> Option<Vec<u8>> {
    let bytes = name.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'_' {
            let hex = name.get(i + 1..i + 3)?;
            if hex.bytes().any(|b| b.is_ascii_uppercase()) {
                return None;
            }
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_escape_and_come_back() {
        let cases: [(&[u8], &str); 5] = [
            (b"main", "main"),
            (b"feature/ABC-123", "feature/_41_42_43-123"),
            (b"v1.2_rc", "v1.2_5frc"),
            (b".hidden/x", "_2ehidden/x"),
            ("çay".as_bytes(), "_c3_a7ay"),
        ];
        for (git, apeiron) in cases {
            assert_eq!(escape(git), apeiron);
            assert!(
                apeiron_core::validate_ref_name(apeiron).is_ok(),
                "{apeiron}"
            );
            assert_eq!(unescape(apeiron).unwrap(), git);
        }
        assert!(unescape("bad_4").is_none());
        assert!(unescape("bad_zz").is_none());
    }
}
