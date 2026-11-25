//! AWS Key Management Service (KEYS.md §2.2), signed with Signature
//! Version 4. The key is a key id, an ARN or an alias; the credentials come
//! from `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and
//! `AWS_SESSION_TOKEN`, the region from the ARN or `AWS_REGION`, and
//! `AWS_ENDPOINT_URL_KMS` or `AWS_ENDPOINT_URL` names another endpoint.

use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{Result, b64, env, from_b64, post, service};

const NAME: &str = "aws-kms";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    // HMAC takes a key of any length.
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key).unwrap_or_else(|_| unreachable!());
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn region(key: &str) -> Result<String> {
    if let Some(rest) = key.strip_prefix("arn:")
        && let Some(r) = rest.split(':').nth(2).filter(|r| !r.is_empty())
    {
        return Ok(r.to_owned());
    }
    env("AWS_REGION")
        .or_else(|| env("AWS_DEFAULT_REGION"))
        .ok_or_else(|| service(NAME, "name the region in AWS_REGION or use a key ARN"))
}

/// Calls the KMS operation `target`, such as `Encrypt`.
fn call(key: &str, target: &str, body: &Value) -> Result<Value> {
    let region = region(key)?;
    let access =
        env("AWS_ACCESS_KEY_ID").ok_or_else(|| service(NAME, "AWS_ACCESS_KEY_ID is not set"))?;
    let secret = env("AWS_SECRET_ACCESS_KEY")
        .ok_or_else(|| service(NAME, "AWS_SECRET_ACCESS_KEY is not set"))?;
    let endpoint = env("AWS_ENDPOINT_URL_KMS")
        .or_else(|| env("AWS_ENDPOINT_URL"))
        .unwrap_or_else(|| format!("https://kms.{region}.amazonaws.com"));
    let endpoint = endpoint.trim_end_matches('/').to_owned();
    let host = endpoint
        .split_once("://")
        .map_or(endpoint.as_str(), |(_, h)| h)
        .to_owned();
    let now = jiff::Timestamp::now();
    let amz_date = now.strftime("%Y%m%dT%H%M%SZ").to_string();
    let date = &amz_date[..8];
    let payload = body.to_string();
    let target = format!("TrentService.{target}");
    let mut headers: Vec<(&str, String)> = vec![
        ("content-type", "application/x-amz-json-1.1".to_owned()),
        ("host", host),
        ("x-amz-date", amz_date.clone()),
        ("x-amz-target", target),
    ];
    if let Some(token) = env("AWS_SESSION_TOKEN") {
        headers.push(("x-amz-security-token", token));
    }
    headers.sort_by(|a, b| a.0.cmp(b.0));
    let canonical_headers: String = headers
        .iter()
        .map(|(k, v)| format!("{k}:{}\n", v.trim()))
        .collect();
    let signed: Vec<&str> = headers.iter().map(|(k, _)| *k).collect();
    let signed = signed.join(";");
    let canonical = format!(
        "POST\n/\n\n{canonical_headers}\n{signed}\n{}",
        hex(&Sha256::digest(payload.as_bytes()))
    );
    let scope = format!("{date}/{region}/kms/aws4_request");
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex(&Sha256::digest(canonical.as_bytes()))
    );
    let k_date = hmac(format!("AWS4{secret}").as_bytes(), date);
    let k_region = hmac(&k_date, &region);
    let k_service = hmac(&k_region, "kms");
    let k_signing = hmac(&k_service, "aws4_request");
    let signature = hex(&hmac(&k_signing, &to_sign));
    headers.push((
        "authorization",
        format!("AWS4-HMAC-SHA256 Credential={access}/{scope}, SignedHeaders={signed}, Signature={signature}"),
    ));
    // `host` is set by the client from the address.
    headers.retain(|(k, _)| *k != "host");
    post(NAME, &format!("{endpoint}/"), &headers, body)
}

pub(crate) fn wrap(key: &str, kek: &[u8; 32]) -> Result<String> {
    let answer = call(
        key,
        "Encrypt",
        &json!({ "KeyId": key, "Plaintext": b64(kek) }),
    )?;
    answer["CiphertextBlob"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| service(NAME, "no CiphertextBlob in the answer"))
}

pub(crate) fn unwrap(key: &str, wrapped: &str) -> Result<Vec<u8>> {
    let answer = call(
        key,
        "Decrypt",
        &json!({ "KeyId": key, "CiphertextBlob": wrapped }),
    )?;
    let plain = answer["Plaintext"]
        .as_str()
        .ok_or_else(|| service(NAME, "no Plaintext in the answer"))?;
    from_b64(NAME, plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key derivation of AWS's own example (Signature Version 4,
    /// "Deriving the signing key").
    #[test]
    fn the_signing_key_is_derived_as_aws_does() {
        let k = hmac(b"AWS4wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", "20120215");
        let k = hmac(&k, "us-east-1");
        let k = hmac(&k, "iam");
        let k = hmac(&k, "aws4_request");
        assert_eq!(
            hex(&k),
            "f4780e2d9f65fa895f9c67b32ce1baf0b0d8a43505a000a1a9e090d414db404d"
        );
        assert_eq!(
            region("arn:aws:kms:eu-west-1:1:key/x").unwrap(),
            "eu-west-1"
        );
    }
}
