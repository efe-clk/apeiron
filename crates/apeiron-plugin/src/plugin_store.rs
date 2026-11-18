//! A remote kept by a storage backend plugin, `plugin://<name>/<config>`
//! (PLUGIN.md §2.3).

use std::path::{Path, PathBuf};

use apeiron_core::{Error as CoreError, ObjectId, Result, Store};

use crate::storage::exports::apeiron::plugin::storage::SwapFailure;
use crate::{Kind, PluginInfo, Plugins, STORAGE_TIME_LIMIT};

/// The repository copy a storage backend keeps.
pub struct PluginStore {
    plugins: Plugins,
    info: PluginInfo,
    config: String,
}

impl PluginStore {
    /// The storage backend `name` installed in `dir`, with the remote's
    /// `config` text.
    pub fn open(dir: &Path, name: &str, config: &str) -> Result<Self> {
        let plugins = Plugins::open(dir)
            .map_err(|e| CoreError::InvalidRemote(e.to_string()))?
            .with_time_limit(STORAGE_TIME_LIMIT);
        let info = plugins
            .list()
            .iter()
            .find(|i| i.name == name && i.kind == Kind::Storage)
            .cloned()
            .ok_or_else(|| {
                CoreError::InvalidRemote(format!(
                    "no storage plugin named {name} is installed; see `apeiron plugin list`"
                ))
            })?;
        Ok(PluginStore {
            plugins,
            info,
            config: config.to_owned(),
        })
    }

    fn failed(&self, path: &str, message: impl Into<String>) -> CoreError {
        CoreError::Io {
            path: self.location(path),
            source: std::io::Error::other(format!("plugin {}: {}", self.info.name, message.into())),
        }
    }

    /// Calls the plugin; `Err` when it could not run or it refused.
    fn call<T>(
        &self,
        path: &str,
        f: impl FnOnce(
            &crate::storage::exports::apeiron::plugin::storage::Guest,
            &mut wasmtime::Store<crate::State>,
            &str,
        ) -> wasmtime::Result<std::result::Result<T, String>>,
    ) -> Result<T> {
        let component = self
            .plugins
            .component(&self.info.name)
            .map_err(|e| self.failed(path, e.to_string()))?;
        let runtime = self
            .plugins
            .runtime()
            .map_err(|e| self.failed(path, e.to_string()))?;
        runtime
            .call_storage(
                &component,
                self.plugins.time_limit,
                &self.info.granted,
                |p, store| f(p.apeiron_plugin_storage(), store, &self.config),
            )
            .map_err(|e| self.failed(path, e))?
            .map_err(|e| self.failed(path, e))
    }
}

fn id_of(hex: &str, failed: impl FnOnce() -> CoreError) -> Result<ObjectId> {
    ObjectId::from_hex(hex).ok_or_else(failed)
}

impl Store for PluginStore {
    fn location(&self, path: &str) -> PathBuf {
        PathBuf::from(format!("plugin://{}/{path}", self.info.name))
    }

    fn read(&self, path: &str) -> Result<Option<Vec<u8>>> {
        self.call(path, |p, store, config| p.call_read(store, config, path))
    }

    fn read_range(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>> {
        let bytes = self.call(path, |p, store, config| {
            p.call_read_range(store, config, path, offset, len)
        })?;
        if bytes.len() as u64 != len {
            return Err(self.failed(path, "it gave fewer bytes than asked"));
        }
        Ok(bytes)
    }

    fn list(&self, dir: &str) -> Result<Vec<String>> {
        self.call(dir, |p, store, config| p.call_list_dir(store, config, dir))
    }

    fn write_new(&self, path: &str, bytes: &[u8]) -> Result<()> {
        self.call(path, |p, store, config| {
            p.call_write_new(store, config, path, bytes)
        })
    }

    fn create_dirs(&self, dirs: &[&str]) -> Result<()> {
        let dirs: Vec<String> = dirs.iter().map(|d| (*d).to_owned()).collect();
        self.call("", |p, store, config| {
            p.call_create_dirs(store, config, &dirs)
        })
    }

    fn delete(&self, path: &str) -> Result<()> {
        self.call(path, |p, store, config| p.call_delete(store, config, path))
    }

    fn read_ref(&self, name: &str) -> Result<Option<ObjectId>> {
        match self.call(name, |p, store, config| {
            p.call_read_ref(store, config, name)
        })? {
            Some(hex) => id_of(&hex, || self.failed(name, format!("not an id: {hex}"))).map(Some),
            None => Ok(None),
        }
    }

    fn list_refs(&self) -> Result<Vec<String>> {
        let mut names = self.call("refs", |p, store, config| p.call_list_refs(store, config))?;
        names.retain(|n| apeiron_core::validate_ref_name(n).is_ok());
        names.sort();
        Ok(names)
    }

    fn swap_ref(&self, name: &str, expected: Option<ObjectId>, new: ObjectId) -> Result<()> {
        let expected_hex = expected.map(|id| id.to_hex());
        let answer = self.call(name, |p, store, config| {
            p.call_swap_ref(store, config, name, expected_hex.as_deref(), &new.to_hex())
                .map(Ok)
        })?;
        match answer {
            Ok(()) => Ok(()),
            Err(SwapFailure::Conflict(actual)) => Err(CoreError::RefConflict {
                name: name.to_owned(),
                expected,
                actual: match actual {
                    Some(hex) => Some(id_of(&hex, || {
                        self.failed(name, format!("not an id: {hex}"))
                    })?),
                    None => None,
                },
            }),
            Err(SwapFailure::Failed(message)) => Err(self.failed(name, message)),
        }
    }
}
