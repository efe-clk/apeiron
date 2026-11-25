//! Google Cloud Key Management Service (KEYS.md §2.3). The key is
//! `projects/<p>/locations/<l>/keyRings/<r>/cryptoKeys/<k>`; the access
//! token comes from `GOOGLE_OAUTH_ACCESS_TOKEN`, or else from
//! `gcloud auth print-access-token`.

use std::process::{Command, Stdio};

use serde_json::json;

use crate::{Result, b64, env, from_b64, post, service};

const NAME: &str = "gcp-kms";

fn token() -> Result<String> {
    if let Some(token) = env("GOOGLE_OAUTH_ACCESS_TOKEN") {
        return Ok(token);
    }
    let output = Command::new("gcloud")
        .args(["auth", "print-access-token"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| {
            service(
                NAME,
                format!("set GOOGLE_OAUTH_ACCESS_TOKEN or install gcloud: {e}"),
            )
        })?;
    if !output.status.success() {
        return Err(service(
            NAME,
            String::from_utf8_lossy(&output.stderr).trim(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn call(key: &str, operation: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
    let endpoint = env("APEIRON_GCP_KMS_ENDPOINT")
        .unwrap_or_else(|| "https://cloudkms.googleapis.com".to_owned());
    let url = format!("{}/v1/{key}:{operation}", endpoint.trim_end_matches('/'));
    let headers = [
        ("Authorization", format!("Bearer {}", token()?)),
        ("Content-Type", "application/json".to_owned()),
    ];
    post(NAME, &url, &headers, body)
}

pub(crate) fn wrap(key: &str, kek: &[u8; 32]) -> Result<String> {
    let answer = call(key, "encrypt", &json!({ "plaintext": b64(kek) }))?;
    answer["ciphertext"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| service(NAME, "no ciphertext in the answer"))
}

pub(crate) fn unwrap(key: &str, wrapped: &str) -> Result<Vec<u8>> {
    let answer = call(key, "decrypt", &json!({ "ciphertext": wrapped }))?;
    let plain = answer["plaintext"]
        .as_str()
        .ok_or_else(|| service(NAME, "no plaintext in the answer"))?;
    from_b64(NAME, plain)
}
