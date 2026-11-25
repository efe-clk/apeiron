//! Shamir's secret sharing over GF(256) (KEYS.md §3): a secret split into
//! `n` shares of which any `k` give it back, and fewer tell nothing about
//! it. Each byte of the secret is the constant of its own random
//! polynomial of degree `k - 1`; share `x` holds the polynomials' values at
//! `x`.
//!
//! A share is written as `<k>-<x>-<hex>-<check>`, where the check is the
//! start of a BLAKE3 hash of the rest, so a mistyped share is refused
//! instead of giving a wrong secret.

use crate::{Error, Result};

/// Multiplication in GF(256) with the AES polynomial; it takes the same
/// steps whatever the values.
fn mul(mut a: u8, mut b: u8) -> u8 {
    let mut product = 0u8;
    for _ in 0..8 {
        product ^= a & 0u8.wrapping_sub(b & 1);
        let carry = 0u8.wrapping_sub(a >> 7);
        a = (a << 1) ^ (0x1b & carry);
        b >>= 1;
    }
    product
}

/// The inverse of a non-zero element: `a^254`.
fn inv(a: u8) -> u8 {
    let mut result = 1;
    let mut base = a;
    let mut exp = 254u8;
    while exp > 0 {
        if exp & 1 == 1 {
            result = mul(result, base);
        }
        base = mul(base, base);
        exp >>= 1;
    }
    result
}

fn check(threshold: u8, index: u8, data: &[u8]) -> String {
    let mut h = blake3::Hasher::new();
    h.update(b"apeiron share");
    h.update(&[threshold, index]);
    h.update(data);
    h.finalize().to_hex()[..8].to_owned()
}

/// Splits `secret` into `shares` shares of which `threshold` give it back.
pub fn split(secret: &[u8], shares: u8, threshold: u8) -> Result<Vec<String>> {
    if !(2..=shares).contains(&threshold) {
        return Err(Error::Invalid(format!(
            "the threshold must be from 2 to the number of shares ({shares})"
        )));
    }
    let mut coefficients = vec![0u8; secret.len() * usize::from(threshold - 1)];
    getrandom::fill(&mut coefficients).map_err(|e| Error::Invalid(format!("random bytes: {e}")))?;
    let per = usize::from(threshold - 1);
    let mut out = Vec::new();
    for x in 1..=shares {
        let data: Vec<u8> = secret
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                // Horner's rule from the highest coefficient down to s.
                let c = &coefficients[i * per..(i + 1) * per];
                let mut y = 0u8;
                for &a in c.iter().rev() {
                    y = mul(y, x) ^ a;
                }
                mul(y, x) ^ s
            })
            .collect();
        let hex = data_encoding::HEXLOWER.encode(&data);
        out.push(format!(
            "{threshold}-{x}-{hex}-{}",
            check(threshold, x, &data)
        ));
    }
    Ok(out)
}

fn parse(text: &str) -> Result<(u8, u8, Vec<u8>)> {
    let bad = || Error::Invalid(format!("{text:?} is not a share"));
    let mut parts = text.trim().split('-');
    let (Some(k), Some(x), Some(hex), Some(sum), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return Err(bad());
    };
    let (k, x): (u8, u8) = (k.parse().map_err(|_| bad())?, x.parse().map_err(|_| bad())?);
    let data = data_encoding::HEXLOWER
        .decode(hex.to_lowercase().as_bytes())
        .map_err(|_| bad())?;
    if x == 0 || check(k, x, &data) != sum.to_lowercase() {
        return Err(Error::Invalid(format!(
            "{text:?} is mistyped or not a share"
        )));
    }
    Ok((k, x, data))
}

/// Gives back the secret from at least its threshold of shares.
pub fn join(texts: &[String]) -> Result<Vec<u8>> {
    let mut shares: Vec<(u8, u8, Vec<u8>)> = Vec::new();
    for text in texts {
        let share = parse(text)?;
        if !shares.iter().any(|s| s.1 == share.1) {
            shares.push(share);
        }
    }
    let Some(first) = shares.first() else {
        return Err(Error::Invalid("no shares".to_owned()));
    };
    let (threshold, len) = (first.0, first.2.len());
    if shares.iter().any(|s| s.0 != threshold || s.2.len() != len) {
        return Err(Error::Invalid(
            "the shares come from different splits".to_owned(),
        ));
    }
    if shares.len() < usize::from(threshold) {
        return Err(Error::Invalid(format!(
            "{threshold} shares are needed, {} were given",
            shares.len()
        )));
    }
    let shares = &shares[..usize::from(threshold)];
    // Lagrange interpolation at 0.
    let mut secret = vec![0u8; len];
    for (j, (_, xj, yj)) in shares.iter().enumerate() {
        let mut basis = 1u8;
        for (m, (_, xm, _)) in shares.iter().enumerate() {
            if m != j {
                basis = mul(basis, mul(*xm, inv(xm ^ xj)));
            }
        }
        for (s, y) in secret.iter_mut().zip(yj) {
            *s ^= mul(*y, basis);
        }
    }
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_field_works() {
        // The AES S-box's well-known pair: 0x53 · 0xCA = 1.
        assert_eq!(mul(0x53, 0xca), 1);
        for a in 1..=255u8 {
            assert_eq!(mul(a, inv(a)), 1, "{a}");
        }
    }

    #[test]
    fn any_k_of_n_shares_give_the_secret_back() {
        let secret: Vec<u8> = (0..32).map(|i| i * 7 + 3).collect();
        let shares = split(&secret, 5, 3).unwrap();
        assert_eq!(shares.len(), 5);
        for picked in [[0, 1, 2], [0, 2, 4], [4, 3, 1], [1, 2, 3]] {
            let some: Vec<String> = picked.iter().map(|&i| shares[i].clone()).collect();
            assert_eq!(join(&some).unwrap(), secret);
        }
        let two = &shares[..2];
        assert!(
            join(two)
                .unwrap_err()
                .to_string()
                .contains("3 shares are needed")
        );
        // A mistyped share is refused, not turned into a wrong secret.
        let mut typo = shares[0].clone();
        let at = typo.len() - 12;
        typo.replace_range(at..at + 1, if &typo[at..at + 1] == "a" { "b" } else { "a" });
        assert!(join(&[typo, shares[1].clone(), shares[2].clone()]).is_err());
        assert!(split(&secret, 3, 1).is_err());
        assert!(split(&secret, 3, 4).is_err());
    }
}
