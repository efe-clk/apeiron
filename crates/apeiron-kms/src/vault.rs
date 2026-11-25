//! HashiCorp Vault's transit secrets engine (KEYS.md §2.1). The key is
//! `<name>` of the engine at `transit/`, or `<mount>/<name>`; the address
//! and token come from `VAULT_ADDR` and `VAULT_TOKEN`, as for Vault's own
//! command, and `VAULT_NAMESPACE` when set.

use serde_json::json;

use crate::{Result, b64, env, from_b64, post, service};

const NAME: &str = "vault";

fn endpoint(key: &str, operation: &str) -> Result<(String, Vec<(&'static str, String)>)> {
    let addr = env("VAULT_ADDR").unwrap_or_else(|| "http://127.0.0.1:8200".to_owned());
    let token = env("VAULT_TOKEN").ok_or_else(|| service(NAME, "VAULT_TOKEN is not set"))?;
    let (mount, name) = key.rsplit_once('/').unwrap_or(("transit", key));
    let url = format!(
        "{}/v1/{mount}/{operation}/{name}",
        addr.trim_end_matches('/')
    );
    let mut headers = vec![("X-Vault-Token", token)];
    if let Some(ns) = env("VAULT_NAMESPACE") {
        headers.push(("X-Vault-Namespace", ns));
    }
    headers.push(("Content-Type", "application/json".to_owned()));
    Ok((url, headers))
}

pub(crate) fn wrap(key: &str, kek: &[u8; 32]) -> Result<String> {
    let (url, headers) = endpoint(key, "encrypt")?;
    let answer = post(NAME, &url, &headers, &json!({ "plaintext": b64(kek) }))?;
    answer["data"]["ciphertext"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| service(NAME, "no ciphertext in the answer"))
}

pub(crate) fn unwrap(key: &str, wrapped: &str) -> Result<Vec<u8>> {
    let (url, headers) = endpoint(key, "decrypt")?;
    let answer = post(NAME, &url, &headers, &json!({ "ciphertext": wrapped }))?;
    let plain = answer["data"]["plaintext"]
        .as_str()
        .ok_or_else(|| service(NAME, "no plaintext in the answer"))?;
    from_b64(NAME, plain)
}
