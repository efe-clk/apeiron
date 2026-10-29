//! The plugin host of Apeiron (PLUGIN.md): installs WebAssembly components
//! and runs them on Wasmtime with no access to anything but the bytes they
//! are given, within time and memory limits.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::thread;
use std::time::Duration;

use apeiron_services::{ContentMerger, Merged, name_matches};
use serde::{Deserialize, Serialize};
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder, Trap};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

mod bindings {
    wasmtime::component::bindgen!({ path: "wit", world: "type-driver-plugin" });
}

use bindings::TypeDriverPlugin;

/// The longest a call may run (PLUGIN.md §3).
pub const TIME_LIMIT: Duration = Duration::from_secs(30);
/// The most memory a plugin may use.
pub const MEMORY_LIMIT: usize = 512 << 20;
/// How often the time of running calls is checked.
const TICK: Duration = Duration::from_millis(100);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("no plugin named {0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("plugin {name} failed: {message}")]
    Failed { name: String, message: String },
    #[error("no configuration folder for plugins; set APEIRON_PLUGINS")]
    NoFolder,
}

pub type Result<T> = std::result::Result<T, Error>;

fn io_at(path: &Path) -> impl FnOnce(io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// What a plugin says about itself (PLUGIN.md §2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub patterns: Vec<String>,
    pub permissions: Vec<String>,
}

impl From<bindings::exports::apeiron::plugin::type_driver::Info> for PluginInfo {
    fn from(i: bindings::exports::apeiron::plugin::type_driver::Info) -> Self {
        PluginInfo {
            name: i.name,
            version: i.version,
            description: i.description,
            patterns: i.patterns,
            permissions: i.permissions,
        }
    }
}

impl PluginInfo {
    /// Refuses a plugin Apeiron cannot install (PLUGIN.md §2, §3).
    fn check(&self) -> Result<()> {
        let n = &self.name;
        let valid = !n.is_empty()
            && n.len() <= 64
            && !n.starts_with('-')
            && n.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !valid {
            return Err(Error::Invalid(format!(
                "plugin name {n:?} must be lowercase letters, digits and -"
            )));
        }
        if self.patterns.is_empty() {
            return Err(Error::Invalid(format!("plugin {n} names no file patterns")));
        }
        if !self.permissions.is_empty() {
            return Err(Error::Invalid(format!(
                "plugin {n} asks for permissions ({}); this version of Apeiron grants none",
                self.permissions.join(", ")
            )));
        }
        Ok(())
    }
}

/// The result of installing a plugin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Added {
    pub info: PluginInfo,
    /// A plugin of the same name was replaced.
    pub replaced: bool,
}

/// The folder plugins are installed in: `APEIRON_PLUGINS`, or `plugins` in
/// the user's configuration folder (PLUGIN.md §1).
pub fn default_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("APEIRON_PLUGINS") {
        return Ok(PathBuf::from(dir));
    }
    dirs::config_dir()
        .map(|c| c.join("apeiron").join("plugins"))
        .ok_or(Error::NoFolder)
}

/// What a plugin instance can reach: an empty WASI context and its limits.
struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

/// The Wasmtime engine and linker, made when the first plugin runs.
struct Runtime {
    engine: Engine,
    linker: Linker<State>,
}

impl Runtime {
    fn new() -> Result<Self> {
        let failed = |e: wasmtime::Error| Error::Invalid(format!("cannot start Wasmtime: {e}"));
        let mut config = Config::new();
        config.epoch_interruption(true);
        let engine = Engine::new(&config).map_err(failed)?;
        let mut linker = Linker::new(&engine);
        // WASI with nothing granted: plugins written with a standard library
        // import it, and get no files, network, variables or arguments.
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(failed)?;
        // Moves the clock of running calls forward until the engine is gone.
        let weak = engine.weak();
        thread::Builder::new()
            .name("apeiron-plugin-clock".to_owned())
            .spawn(move || {
                loop {
                    thread::sleep(TICK);
                    match weak.upgrade() {
                        Some(engine) => engine.increment_epoch(),
                        None => return,
                    }
                }
            })
            .map_err(|e| Error::Invalid(format!("cannot start a thread: {e}")))?;
        Ok(Runtime { engine, linker })
    }

    fn compile(&self, wasm: &[u8]) -> std::result::Result<Component, String> {
        Component::new(&self.engine, wasm).map_err(|e| format!("not a WebAssembly component: {e}"))
    }

    /// Runs `f` on a new instance of `component`, within the limits.
    fn call<T>(
        &self,
        component: &Component,
        time_limit: Duration,
        f: impl FnOnce(&TypeDriverPlugin, &mut Store<State>) -> wasmtime::Result<T>,
    ) -> std::result::Result<T, String> {
        let state = State {
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
            limits: StoreLimitsBuilder::new()
                .memory_size(MEMORY_LIMIT)
                .trap_on_grow_failure(true)
                .build(),
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        store.set_epoch_deadline((time_limit.as_millis() / TICK.as_millis()).max(1) as u64);
        store.epoch_deadline_trap();
        let run = TypeDriverPlugin::instantiate(&mut store, component, &self.linker)
            .map_err(|e| format!("does not fit version 0.1 of the plugin interface: {e}"))?;
        f(&run, &mut store).map_err(|e| match e.downcast_ref::<Trap>() {
            Some(Trap::Interrupt) => format!("took longer than {} s", time_limit.as_secs()),
            Some(trap) => format!("crashed: {trap}"),
            // Growing memory past the limit is not a trap of WebAssembly.
            None if e.root_cause().to_string().contains("growing memory") => format!(
                "crashed: it needed more than {} MiB of memory",
                MEMORY_LIMIT >> 20
            ),
            None => format!("crashed: {}", e.root_cause()),
        })
    }
}

/// The plugins installed in a folder.
pub struct Plugins {
    dir: PathBuf,
    infos: Vec<PluginInfo>,
    runtime: OnceLock<std::result::Result<Runtime, String>>,
    compiled: Mutex<HashMap<String, Component>>,
    time_limit: Duration,
}

impl Plugins {
    /// Reads what is installed in `dir`; a missing folder holds no plugins.
    /// Nothing is compiled until a plugin is needed.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        let mut infos = Vec::new();
        match fs::read_dir(&dir) {
            Ok(entries) => {
                for entry in entries {
                    let path = entry.map_err(io_at(&dir))?.path();
                    if path.extension().is_some_and(|e| e == "json") {
                        let text = fs::read_to_string(&path).map_err(io_at(&path))?;
                        let info: PluginInfo = serde_json::from_str(&text)
                            .map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))?;
                        infos.push(info);
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_at(&dir)(e)),
        }
        infos.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Plugins {
            dir,
            infos,
            runtime: OnceLock::new(),
            compiled: Mutex::new(HashMap::new()),
            time_limit: TIME_LIMIT,
        })
    }

    /// Stops calls after `limit` instead of [`TIME_LIMIT`].
    pub fn with_time_limit(mut self, limit: Duration) -> Self {
        self.time_limit = limit;
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The installed plugins, by name.
    pub fn list(&self) -> &[PluginInfo] {
        &self.infos
    }

    fn runtime(&self) -> Result<&Runtime> {
        self.runtime
            .get_or_init(|| Runtime::new().map_err(|e| e.to_string()))
            .as_ref()
            .map_err(|e| Error::Invalid(e.clone()))
    }

    /// Checks the component in `file` and installs it, replacing a plugin of
    /// the same name (PLUGIN.md §1).
    pub fn add(&mut self, file: &Path) -> Result<Added> {
        let wasm = fs::read(file).map_err(io_at(file))?;
        let runtime = self.runtime()?;
        let invalid = |m: String| Error::Invalid(format!("{}: {m}", file.display()));
        let component = runtime.compile(&wasm).map_err(invalid)?;
        let info: PluginInfo = runtime
            .call(&component, self.time_limit, |p, store| {
                p.apeiron_plugin_type_driver().call_describe(store)
            })
            .map_err(invalid)?
            .into();
        info.check()?;
        fs::create_dir_all(&self.dir).map_err(io_at(&self.dir))?;
        write_atomic(&self.dir.join(format!("{}.wasm", info.name)), &wasm)?;
        let json = serde_json::to_vec_pretty(&info)
            .map_err(|e| Error::Invalid(format!("cannot write the plugin's description: {e}")))?;
        write_atomic(&self.dir.join(format!("{}.json", info.name)), &json)?;
        self.lock().insert(info.name.clone(), component);
        let replaced = match self.infos.iter_mut().find(|i| i.name == info.name) {
            Some(old) => {
                *old = info.clone();
                true
            }
            None => {
                self.infos.push(info.clone());
                self.infos.sort_by(|a, b| a.name.cmp(&b.name));
                false
            }
        };
        Ok(Added { info, replaced })
    }

    /// Removes an installed plugin.
    pub fn remove(&mut self, name: &str) -> Result<PluginInfo> {
        let index = self
            .infos
            .iter()
            .position(|i| i.name == name)
            .ok_or_else(|| Error::NotFound(name.to_owned()))?;
        // The description goes first: without it the plugin is not installed.
        for ext in ["json", "wasm"] {
            let path = self.dir.join(format!("{name}.{ext}"));
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(io_at(&path)(e)),
            }
        }
        self.lock().remove(name);
        Ok(self.infos.remove(index))
    }

    /// The first plugin, by name, with a pattern matching `name`.
    fn driver_for(&self, name: &[u8]) -> Option<&PluginInfo> {
        self.infos
            .iter()
            .find(|i| i.patterns.iter().any(|p| name_matches(p, name)))
    }

    fn component(&self, name: &str) -> Result<Component> {
        if let Some(c) = self.lock().get(name) {
            return Ok(c.clone());
        }
        let path = self.dir.join(format!("{name}.wasm"));
        let wasm = fs::read(&path).map_err(io_at(&path))?;
        let component = self
            .runtime()?
            .compile(&wasm)
            .map_err(|message| Error::Failed {
                name: name.to_owned(),
                message,
            })?;
        self.lock().insert(name.to_owned(), component.clone());
        Ok(component)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Component>> {
        self.compiled.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Merges a file with the plugin that handles its name (PLUGIN.md §4):
    /// `None` when no plugin does or the changes conflict.
    pub fn merge_file(
        &self,
        name: &[u8],
        base: Option<&[u8]>,
        ours: &[u8],
        theirs: &[u8],
    ) -> Result<Option<Merged>> {
        let Some(info) = self.driver_for(name) else {
            return Ok(None);
        };
        let failed = |message: String| Error::Failed {
            name: info.name.clone(),
            message,
        };
        let component = self.component(&info.name)?;
        let result = self
            .runtime()?
            .call(&component, self.time_limit, |p, store| {
                p.apeiron_plugin_type_driver()
                    .call_merge(store, base, ours, theirs)
            })
            .map_err(failed)?;
        match result {
            Ok(Some(data)) => Ok(Some(Merged {
                data,
                plugin: info.name.clone(),
            })),
            Ok(None) => Ok(None),
            Err(message) => Err(failed(format!("refused: {message}"))),
        }
    }

    /// A difference for people to read between two versions of a file, from
    /// the plugin that handles its name; `None` when there is none.
    pub fn diff_file(&self, name: &[u8], old: &[u8], new: &[u8]) -> Result<Option<String>> {
        let Some(info) = self.driver_for(name) else {
            return Ok(None);
        };
        let failed = |message: String| Error::Failed {
            name: info.name.clone(),
            message,
        };
        let component = self.component(&info.name)?;
        self.runtime()?
            .call(&component, self.time_limit, |p, store| {
                p.apeiron_plugin_type_driver().call_diff(store, old, new)
            })
            .map_err(failed)?
            .map_err(|message| failed(format!("refused: {message}")))
    }
}

impl std::fmt::Debug for Plugins {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plugins")
            .field("dir", &self.dir)
            .field("infos", &self.infos)
            .finish_non_exhaustive()
    }
}

impl ContentMerger for Plugins {
    fn handles(&self, name: &[u8]) -> bool {
        self.driver_for(name).is_some()
    }

    fn merge(
        &self,
        name: &[u8],
        base: Option<&[u8]>,
        ours: &[u8],
        theirs: &[u8],
    ) -> std::result::Result<Option<Merged>, String> {
        self.merge_file(name, base, ours, theirs)
            .map_err(|e| e.to_string())
    }
}

/// Writes a file through a temporary file and a rename, so a crash leaves
/// either the old file or the new one.
fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data).map_err(io_at(&tmp))?;
    fs::rename(&tmp, path).map_err(io_at(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str, permissions: &[&str]) -> PluginInfo {
        PluginInfo {
            name: name.to_owned(),
            version: "1".to_owned(),
            description: String::new(),
            patterns: vec!["*.txt".to_owned()],
            permissions: permissions.iter().map(|p| (*p).to_owned()).collect(),
        }
    }

    #[test]
    fn names_patterns_and_permissions_are_checked() {
        assert!(info("text-2", &[]).check().is_ok());
        for bad in ["", "Text", "-x", "a b", "a/b", &"x".repeat(65)] {
            assert!(info(bad, &[]).check().is_err(), "{bad:?}");
        }
        assert!(info("net", &["network"]).check().is_err());
        let mut none = info("x", &[]);
        none.patterns.clear();
        assert!(none.check().is_err());
    }

    #[test]
    fn a_missing_folder_holds_no_plugins() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = Plugins::open(dir.path().join("none")).unwrap();
        assert!(plugins.list().is_empty());
        assert!(!plugins.handles(b"a.txt"));
        assert_eq!(
            plugins.merge_file(b"a.txt", None, b"a", b"b").unwrap(),
            None
        );
    }
}
