//! The plugin store (PLUGIN.md §6): an index of plugins to download, made
//! by the release workflow beside the plugins it builds.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Error, Kind, PluginInfo, Plugins, Result, io_at};

/// The index of the latest release of Apeiron.
pub const DEFAULT_STORE: &str =
    "https://github.com/efe-clk/apeiron/releases/latest/download/plugins.json";
/// The largest plugin downloaded.
pub const MAX_DOWNLOAD: u64 = 256 << 20;

/// One plugin of a store's index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct StoreEntry {
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub kind: Kind,
    /// Where the `.wasm` file is: an address, or a name beside the index.
    pub file: String,
    /// SHA-256 of the file, hex; a download that differs is refused.
    pub sha256: String,
    pub size: u64,
}

/// A store's index, `plugins.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct StoreIndex {
    pub plugins: Vec<StoreEntry>,
}

/// The store to use: `APEIRON_PLUGIN_STORE`, or the latest release.
pub fn default_store() -> String {
    std::env::var("APEIRON_PLUGIN_STORE")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_STORE.to_owned())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn is_web(address: &str) -> bool {
    address.starts_with("https://") || address.starts_with("http://")
}

/// Reads an address over HTTP(S), or a file; at most `limit` bytes.
fn fetch(address: &str, limit: u64) -> Result<Vec<u8>> {
    let failed = |message: String| Error::Invalid(format!("{address}: {message}"));
    if !is_web(address) {
        let path = PathBuf::from(address.strip_prefix("file://").unwrap_or(address));
        let data = std::fs::read(&path).map_err(io_at(&path))?;
        if data.len() as u64 > limit {
            return Err(failed("too large".to_owned()));
        }
        return Ok(data);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| failed(e.to_string()))?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| failed(e.to_string()))?;
        let mut response = client
            .get(address)
            .send()
            .await
            .map_err(|e| failed(e.to_string()))?;
        if !response.status().is_success() {
            return Err(failed(format!("the store answered {}", response.status())));
        }
        let mut data = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| failed(e.to_string()))? {
            data.extend_from_slice(&chunk);
            if data.len() as u64 > limit {
                return Err(failed("too large".to_owned()));
            }
        }
        Ok(data)
    })
}

/// Where `file` of an index at `store` is: itself when it is an address,
/// else beside the index.
fn beside(store: &str, file: &str) -> Result<String> {
    if is_web(file) {
        return Ok(file.to_owned());
    }
    if file.contains("..") || file.starts_with('/') || file.contains('\\') {
        return Err(Error::Invalid(format!(
            "the store names a file outside it: {file}"
        )));
    }
    match store.rfind('/') {
        Some(end) => Ok(format!("{}/{file}", &store[..end])),
        None => Ok(file.to_owned()),
    }
}

/// The plugins a store offers.
pub fn index(store: &str) -> Result<Vec<StoreEntry>> {
    let data = fetch(store, 16 << 20)?;
    let index: StoreIndex = serde_json::from_slice(&data)
        .map_err(|e| Error::Invalid(format!("{store} is not a plugin index: {e}")))?;
    Ok(index.plugins)
}

/// Downloads `entry` of `store` into `to` after checking its size and its
/// SHA-256 against the index.
pub fn download(store: &str, entry: &StoreEntry, to: &Path) -> Result<()> {
    let address = beside(store, &entry.file)?;
    let data = fetch(&address, MAX_DOWNLOAD)?;
    let sum = hex(&Sha256::digest(&data));
    if data.len() as u64 != entry.size || !sum.eq_ignore_ascii_case(&entry.sha256) {
        return Err(Error::Invalid(format!(
            "{address} is not the file the store lists (SHA-256 {sum}); nothing was installed"
        )));
    }
    std::fs::write(to, &data).map_err(io_at(to))
}

/// An index of the plugin files `files`, each named beside it by its file
/// name, for the release workflow.
pub fn make_index(files: &[PathBuf]) -> Result<StoreIndex> {
    let mut plugins = Vec::new();
    for file in files {
        let data = std::fs::read(file).map_err(io_at(file))?;
        let info: PluginInfo = Plugins::describe_file(file)?;
        plugins.push(StoreEntry {
            name: info.name,
            version: info.version,
            description: info.description,
            kind: info.kind,
            file: file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            sha256: hex(&Sha256::digest(&data)),
            size: data.len() as u64,
        });
    }
    plugins.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(StoreIndex { plugins })
}

#[cfg(test)]
mod tests {
    use super::beside;

    #[test]
    fn files_are_found_beside_the_index() {
        assert_eq!(
            beside(
                "https://x.example/releases/latest/download/plugins.json",
                "a.wasm"
            )
            .unwrap(),
            "https://x.example/releases/latest/download/a.wasm"
        );
        assert_eq!(
            beside("/srv/store/plugins.json", "a.wasm").unwrap(),
            "/srv/store/a.wasm"
        );
        assert_eq!(
            beside("https://a/i.json", "https://b/c.wasm").unwrap(),
            "https://b/c.wasm"
        );
        assert!(beside("https://a/i.json", "../etc/x").is_err());
        assert!(beside("https://a/i.json", "/etc/x").is_err());
    }
}
