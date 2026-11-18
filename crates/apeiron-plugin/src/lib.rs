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

use apeiron_services::{
    ArchiveReader, ChangeKind, ContentMerger, HookChange, Hooks, Merged, ReadFiles, TextExtractor,
    Verdict, name_matches,
};
use serde::{Deserialize, Serialize};
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder, Trap};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

mod store;

pub use store::{
    DEFAULT_STORE, MAX_DOWNLOAD, StoreEntry, StoreIndex, default_store, download, index, make_index,
};

mod bindings {
    wasmtime::component::bindgen!({ path: "wit", world: "type-driver-plugin" });
}

mod importer {
    wasmtime::component::bindgen!({ path: "wit", world: "importer-plugin" });
}

mod hook {
    wasmtime::component::bindgen!({ path: "wit", world: "hook-plugin" });
}

mod storage {
    wasmtime::component::bindgen!({ path: "wit", world: "storage-plugin" });
}

mod gate;
mod plugin_store;

pub use plugin_store::PluginStore;

use bindings::TypeDriverPlugin;
use hook::HookPlugin;
use importer::ImporterPlugin;
use storage::StoragePlugin;

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

/// What a plugin does (PLUGIN.md §2).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// Merges files and shows their differences, and reads their text.
    #[default]
    TypeDriver,
    /// Reads archive formats for `apeiron import`.
    Importer,
    /// Looks at operations before they take effect and may stop them, or
    /// tells another service after them.
    Hook,
    /// Keeps repository copies as the remote `plugin://<name>/<config>`.
    Storage,
}

/// What a plugin says about itself (PLUGIN.md §2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    /// What it does; descriptions written before there were other kinds
    /// are type drivers.
    #[serde(default)]
    pub kind: Kind,
    /// For a type driver, the files it merges; for an importer, the
    /// archives it reads.
    pub patterns: Vec<String>,
    /// The files `extract-text` reads, for search; none for most plugins.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_patterns: Vec<String>,
    /// For a hook, the events it is called for, such as `backup`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
    pub permissions: Vec<String>,
    /// The permissions the user granted when installing it (PLUGIN.md §3.1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub granted: Vec<String>,
}

/// The events a hook may be called for (PLUGIN.md §2.2).
pub const EVENTS: &[&str] = &["backup", "backup-done"];

/// How long a storage backend may take for one call: a pack over a slow
/// line.
pub const STORAGE_TIME_LIMIT: Duration = Duration::from_secs(10 * 60);

/// The host a `net:<host>[:<port>]` permission names; `None` for another
/// permission.
fn net_host(permission: &str) -> Option<&str> {
    let host = permission.strip_prefix("net:")?;
    let valid = !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'));
    valid.then_some(host)
}

impl From<bindings::exports::apeiron::plugin::type_driver::Info> for PluginInfo {
    fn from(i: bindings::exports::apeiron::plugin::type_driver::Info) -> Self {
        PluginInfo {
            name: i.name,
            version: i.version,
            description: i.description,
            kind: Kind::TypeDriver,
            patterns: i.patterns,
            text_patterns: i.text_patterns,
            events: Vec::new(),
            permissions: i.permissions,
            granted: Vec::new(),
        }
    }
}

impl From<importer::exports::apeiron::plugin::importer::Info> for PluginInfo {
    fn from(i: importer::exports::apeiron::plugin::importer::Info) -> Self {
        PluginInfo {
            name: i.name,
            version: i.version,
            description: i.description,
            kind: Kind::Importer,
            patterns: i.patterns,
            text_patterns: Vec::new(),
            events: Vec::new(),
            permissions: i.permissions,
            granted: Vec::new(),
        }
    }
}

impl From<hook::exports::apeiron::plugin::hook::Info> for PluginInfo {
    fn from(i: hook::exports::apeiron::plugin::hook::Info) -> Self {
        PluginInfo {
            name: i.name,
            version: i.version,
            description: i.description,
            kind: Kind::Hook,
            patterns: Vec::new(),
            text_patterns: Vec::new(),
            events: i.events,
            permissions: i.permissions,
            granted: Vec::new(),
        }
    }
}

impl From<storage::exports::apeiron::plugin::storage::Info> for PluginInfo {
    fn from(i: storage::exports::apeiron::plugin::storage::Info) -> Self {
        PluginInfo {
            name: i.name,
            version: i.version,
            description: i.description,
            kind: Kind::Storage,
            patterns: Vec::new(),
            text_patterns: Vec::new(),
            events: Vec::new(),
            permissions: i.permissions,
            granted: Vec::new(),
        }
    }
}

impl PluginInfo {
    /// Refuses a plugin Apeiron cannot install (PLUGIN.md §2, §3), or one
    /// that asks for permissions the user did not grant with `allow`.
    fn check_granting(&mut self, allow: &[String]) -> Result<()> {
        self.check()?;
        let missing: Vec<&String> = self
            .permissions
            .iter()
            .filter(|p| !allow.contains(p))
            .collect();
        if !missing.is_empty() {
            let list: Vec<&str> = missing.iter().map(|p| p.as_str()).collect();
            return Err(Error::Invalid(format!(
                "plugin {} asks to reach {}; install it with --allow {} if you agree",
                self.name,
                list.join(", "),
                list.join(" --allow ")
            )));
        }
        self.granted.clone_from(&self.permissions);
        Ok(())
    }

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
        match self.kind {
            Kind::TypeDriver | Kind::Importer
                if self.patterns.is_empty() && self.text_patterns.is_empty() =>
            {
                return Err(Error::Invalid(format!("plugin {n} names no file patterns")));
            }
            Kind::Hook => {
                if self.events.is_empty() {
                    return Err(Error::Invalid(format!("hook {n} names no events")));
                }
                if let Some(e) = self.events.iter().find(|e| !EVENTS.contains(&e.as_str())) {
                    return Err(Error::Invalid(format!(
                        "hook {n} asks for the event {e:?}; there are only {}",
                        EVENTS.join(", ")
                    )));
                }
            }
            _ => {}
        }
        if let Some(p) = self.permissions.iter().find(|p| net_host(p).is_none()) {
            return Err(Error::Invalid(format!(
                "plugin {n} asks for the permission {p:?}; there is only net:<host>"
            )));
        }
        if !self.permissions.is_empty() && !matches!(self.kind, Kind::Hook | Kind::Storage) {
            return Err(Error::Invalid(format!(
                "plugin {n} asks to reach the network; only hooks and storage backends may"
            )));
        }
        Ok(())
    }
}

/// The result of installing a plugin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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

/// What a plugin instance can reach: an empty WASI context, the hosts
/// granted to it and its limits.
pub(crate) struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
    http: wasmtime_wasi_http::WasiHttpCtx,
    gate: gate::Gate,
}

impl wasmtime_wasi_http::WasiHttpView for State {
    fn http(&mut self) -> wasmtime_wasi_http::WasiHttpCtxView<'_> {
        wasmtime_wasi_http::WasiHttpCtxView {
            ctx: &mut self.http,
            table: &mut self.table,
            hooks: &mut self.gate,
        }
    }
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
pub(crate) struct Runtime {
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
        // HTTP for every plugin, but every request is refused unless its host
        // was granted (PLUGIN.md §3.1).
        wasmtime_wasi_http::p2::add_only_http_to_linker_sync(&mut linker).map_err(failed)?;
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

    /// Runs `f` on a new type driver instance of `component`.
    fn call<T>(
        &self,
        component: &Component,
        time_limit: Duration,
        f: impl FnOnce(&TypeDriverPlugin, &mut Store<State>) -> wasmtime::Result<T>,
    ) -> std::result::Result<T, String> {
        self.run(time_limit, &[], |store, linker| {
            let p = TypeDriverPlugin::instantiate(&mut *store, component, linker)
                .map_err(|e| format!("does not fit version 0.2 of the plugin interface: {e}"))?;
            Ok(f(&p, store))
        })
    }

    /// Runs `f` on a new importer instance of `component`.
    fn call_importer<T>(
        &self,
        component: &Component,
        time_limit: Duration,
        f: impl FnOnce(&ImporterPlugin, &mut Store<State>) -> wasmtime::Result<T>,
    ) -> std::result::Result<T, String> {
        self.run(time_limit, &[], |store, linker| {
            let p = ImporterPlugin::instantiate(&mut *store, component, linker)
                .map_err(|e| format!("is not an importer: {e}"))?;
            Ok(f(&p, store))
        })
    }

    /// Runs `f` on a new hook instance of `component`, which may reach the
    /// hosts `granted`.
    fn call_hook<T>(
        &self,
        component: &Component,
        time_limit: Duration,
        granted: &[String],
        f: impl FnOnce(&HookPlugin, &mut Store<State>) -> wasmtime::Result<T>,
    ) -> std::result::Result<T, String> {
        self.run(time_limit, granted, |store, linker| {
            let p = HookPlugin::instantiate(&mut *store, component, linker)
                .map_err(|e| format!("is not a hook: {e}"))?;
            Ok(f(&p, store))
        })
    }

    /// Runs `f` on a new storage backend instance of `component`.
    pub(crate) fn call_storage<T>(
        &self,
        component: &Component,
        time_limit: Duration,
        granted: &[String],
        f: impl FnOnce(&StoragePlugin, &mut Store<State>) -> wasmtime::Result<T>,
    ) -> std::result::Result<T, String> {
        self.run(time_limit, granted, |store, linker| {
            let p = StoragePlugin::instantiate(&mut *store, component, linker)
                .map_err(|e| format!("is not a storage backend: {e}"))?;
            Ok(f(&p, store))
        })
    }

    /// Makes a store within the limits and runs `call` on it; `call`
    /// instantiates the component and makes its calls.
    fn run<T>(
        &self,
        time_limit: Duration,
        granted: &[String],
        call: impl FnOnce(
            &mut Store<State>,
            &Linker<State>,
        ) -> std::result::Result<wasmtime::Result<T>, String>,
    ) -> std::result::Result<T, String> {
        let state = State {
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
            limits: StoreLimitsBuilder::new()
                .memory_size(MEMORY_LIMIT)
                .trap_on_grow_failure(true)
                .build(),
            http: wasmtime_wasi_http::WasiHttpCtx::new(),
            gate: gate::Gate::new(
                granted
                    .iter()
                    .filter_map(|p| net_host(p))
                    .map(str::to_owned)
                    .collect(),
            ),
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        store.set_epoch_deadline((time_limit.as_millis() / TICK.as_millis()).max(1) as u64);
        store.epoch_deadline_trap();
        call(&mut store, &self.linker)?.map_err(|e| match e.downcast_ref::<Trap>() {
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

/// What a component says about itself, as whichever kind of plugin it is.
fn describe(
    runtime: &Runtime,
    component: &Component,
    limit: Duration,
) -> std::result::Result<PluginInfo, String> {
    match runtime.call(component, limit, |p, store| {
        p.apeiron_plugin_type_driver().call_describe(store)
    }) {
        Ok(info) => Ok(info.into()),
        Err(driver) => match runtime.call_importer(component, limit, |p, store| {
            p.apeiron_plugin_importer().call_describe(store)
        }) {
            Ok(info) => Ok(info.into()),
            Err(_) => match runtime.call_hook(component, limit, &[], |p, store| {
                p.apeiron_plugin_hook().call_describe(store)
            }) {
                Ok(info) => Ok(info.into()),
                Err(_) => runtime
                    .call_storage(component, limit, &[], |p, store| {
                        p.apeiron_plugin_storage().call_describe(store)
                    })
                    .map(PluginInfo::from)
                    .map_err(|_| driver),
            },
        },
    }
}

/// The plugins installed in a folder.
pub struct Plugins {
    dir: PathBuf,
    infos: Vec<PluginInfo>,
    runtime: OnceLock<std::result::Result<Runtime, String>>,
    compiled: Mutex<HashMap<String, Component>>,
    pub(crate) time_limit: Duration,
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

    pub(crate) fn runtime(&self) -> Result<&Runtime> {
        self.runtime
            .get_or_init(|| Runtime::new().map_err(|e| e.to_string()))
            .as_ref()
            .map_err(|e| Error::Invalid(e.clone()))
    }

    /// What the plugin in `file` says about itself, after the checks of
    /// installing; nothing is installed.
    pub fn describe_file(file: &Path) -> Result<PluginInfo> {
        let wasm = fs::read(file).map_err(io_at(file))?;
        let runtime = Runtime::new()?;
        let invalid = |m: String| Error::Invalid(format!("{}: {m}", file.display()));
        let component = runtime.compile(&wasm).map_err(invalid)?;
        let info = describe(&runtime, &component, TIME_LIMIT).map_err(invalid)?;
        info.check()?;
        Ok(info)
    }

    /// Checks the component in `file` and installs it, replacing a plugin of
    /// the same name (PLUGIN.md §1).
    pub fn add(&mut self, file: &Path) -> Result<Added> {
        self.add_allowing(file, &[])
    }

    /// [`Plugins::add`], granting the permissions in `allow`; a plugin that
    /// asks for others is refused (PLUGIN.md §3.1).
    pub fn add_allowing(&mut self, file: &Path, allow: &[String]) -> Result<Added> {
        let wasm = fs::read(file).map_err(io_at(file))?;
        let runtime = self.runtime()?;
        let invalid = |m: String| Error::Invalid(format!("{}: {m}", file.display()));
        let component = runtime.compile(&wasm).map_err(invalid)?;
        let mut info = describe(runtime, &component, self.time_limit).map_err(invalid)?;
        info.check_granting(allow)?;
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

    /// The settings text of the plugin `name`, kept in `<name>.settings`;
    /// empty when there is none.
    pub fn settings(&self, name: &str) -> String {
        fs::read_to_string(self.dir.join(format!("{name}.settings"))).unwrap_or_default()
    }

    /// Sets the settings of the installed plugin `name`, such as the address
    /// a hook tells (PLUGIN.md §2.2); empty removes them.
    pub fn set_settings(&self, name: &str, text: &str) -> Result<()> {
        if !self.infos.iter().any(|i| i.name == name) {
            return Err(Error::NotFound(name.to_owned()));
        }
        let path = self.dir.join(format!("{name}.settings"));
        if text.is_empty() {
            return match fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(io_at(&path)(e)),
            };
        }
        write_atomic(&path, text.as_bytes())
    }

    /// Removes an installed plugin.
    pub fn remove(&mut self, name: &str) -> Result<PluginInfo> {
        let index = self
            .infos
            .iter()
            .position(|i| i.name == name)
            .ok_or_else(|| Error::NotFound(name.to_owned()))?;
        // The description goes first: without it the plugin is not installed.
        for ext in ["json", "wasm", "settings"] {
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

    /// The first plugin of `kind`, by name, with a pattern matching `name`.
    fn first(&self, kind: Kind, name: &[u8]) -> Option<&PluginInfo> {
        self.infos
            .iter()
            .filter(|i| i.kind == kind)
            .find(|i| i.patterns.iter().any(|p| name_matches(p, name)))
    }

    /// The first type driver, by name, with a pattern matching `name`.
    fn driver_for(&self, name: &[u8]) -> Option<&PluginInfo> {
        self.first(Kind::TypeDriver, name)
    }

    /// The first plugin, by name, that reads the text of files named `name`.
    fn extractor_for(&self, name: &[u8]) -> Option<&PluginInfo> {
        self.infos
            .iter()
            .filter(|i| i.kind == Kind::TypeDriver)
            .find(|i| i.text_patterns.iter().any(|p| name_matches(p, name)))
    }

    /// The files inside an archive named `name`, from the first importer
    /// that reads it (PLUGIN.md §2.1); `None` when none does.
    pub fn read_archive(&self, name: &[u8], content: &[u8]) -> Result<Option<(String, ReadFiles)>> {
        let Some(info) = self.first(Kind::Importer, name) else {
            return Ok(None);
        };
        let failed = |message: String| Error::Failed {
            name: info.name.clone(),
            message,
        };
        let component = self.component(&info.name)?;
        let files = self
            .runtime()?
            .call_importer(&component, self.time_limit, |p, store| {
                p.apeiron_plugin_importer().call_read(store, content)
            })
            .map_err(failed)?
            .map_err(|message| failed(format!("refused: {message}")))?;
        Ok(Some((
            info.name.clone(),
            files
                .into_iter()
                .map(|f| (f.path.into_bytes(), f.content))
                .collect(),
        )))
    }

    /// Asks every hook of `event`, by name, about `changes` (PLUGIN.md
    /// §2.2). A hook that fails, runs out of time or crashes stops the
    /// operation as a refusal would: a check that cannot run is not passed.
    pub fn run_hooks(&self, event: &str, ref_name: &str, changes: &[HookChange]) -> Verdict {
        use hook::exports::apeiron::plugin::hook as wit;
        let changes: Vec<wit::Change> = changes
            .iter()
            .map(|c| wit::Change {
                path: c.path.to_string(),
                kind: match c.kind {
                    ChangeKind::Added => wit::ChangeKind::Added,
                    ChangeKind::Removed => wit::ChangeKind::Removed,
                    ChangeKind::Modified => wit::ChangeKind::Modified,
                },
                content: c.content.clone(),
            })
            .collect();
        let mut warnings = Vec::new();
        for info in self
            .infos
            .iter()
            .filter(|i| i.kind == Kind::Hook && i.events.iter().any(|e| e == event))
        {
            let answer = self.component(&info.name).and_then(|component| {
                self.runtime()?
                    .call_hook(&component, self.time_limit, &info.granted, |p, store| {
                        p.apeiron_plugin_hook().call_check(
                            store,
                            &self.settings(&info.name),
                            event,
                            ref_name,
                            &changes,
                        )
                    })
                    .map_err(|message| Error::Failed {
                        name: info.name.clone(),
                        message,
                    })
            });
            match answer {
                Ok(Ok(said)) => {
                    warnings.extend(said.into_iter().map(|w| (info.name.clone(), w)));
                }
                Ok(Err(why)) => return Verdict::Refuse(info.name.clone(), why),
                Err(e) => return Verdict::Refuse(info.name.clone(), e.to_string()),
            }
        }
        Verdict::Allow(warnings)
    }

    /// The text inside a file, from the plugin that reads files of its name;
    /// `None` when no plugin does or the file holds none.
    pub fn extract_file(&self, name: &[u8], content: &[u8]) -> Result<Option<String>> {
        let Some(info) = self.extractor_for(name) else {
            return Ok(None);
        };
        let failed = |message: String| Error::Failed {
            name: info.name.clone(),
            message,
        };
        let component = self.component(&info.name)?;
        self.runtime()?
            .call(&component, self.time_limit, |p, store| {
                p.apeiron_plugin_type_driver()
                    .call_extract_text(store, content)
            })
            .map_err(failed)?
            .map_err(|message| failed(format!("refused: {message}")))
    }

    pub(crate) fn component(&self, name: &str) -> Result<Component> {
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

impl Hooks for Plugins {
    fn has_hooks(&self, event: &str) -> bool {
        self.infos
            .iter()
            .any(|i| i.kind == Kind::Hook && i.events.iter().any(|e| e == event))
    }

    fn check(&self, event: &str, ref_name: &str, changes: &[HookChange]) -> Verdict {
        self.run_hooks(event, ref_name, changes)
    }
}

impl ArchiveReader for Plugins {
    fn reads(&self, name: &[u8]) -> bool {
        self.first(Kind::Importer, name).is_some()
    }

    fn read(
        &self,
        name: &[u8],
        content: &[u8],
    ) -> std::result::Result<(String, ReadFiles), String> {
        match self.read_archive(name, content) {
            Ok(Some(read)) => Ok(read),
            Ok(None) => Err("no plugin reads it".to_owned()),
            Err(e) => Err(e.to_string()),
        }
    }
}

impl TextExtractor for Plugins {
    fn reads_text(&self, name: &[u8]) -> bool {
        self.extractor_for(name).is_some()
    }

    fn extract(&self, name: &[u8], content: &[u8]) -> std::result::Result<Option<String>, String> {
        self.extract_file(name, content).map_err(|e| e.to_string())
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
            kind: Kind::TypeDriver,
            patterns: vec!["*.txt".to_owned()],
            text_patterns: Vec::new(),
            events: Vec::new(),
            permissions: permissions.iter().map(|p| (*p).to_owned()).collect(),
            granted: Vec::new(),
        }
    }

    #[test]
    fn names_patterns_and_permissions_are_checked() {
        assert!(info("text-2", &[]).check().is_ok());
        for bad in ["", "Text", "-x", "a b", "a/b", &"x".repeat(65)] {
            assert!(info(bad, &[]).check().is_err(), "{bad:?}");
        }
        assert!(info("net", &["network"]).check().is_err());
        // A type driver never reaches the network; a hook may, if granted.
        assert!(info("net", &["net:example.com"]).check().is_err());
        let mut hook = info("hook", &["net:hooks.example.com"]);
        hook.kind = Kind::Hook;
        hook.events = vec!["backup-done".to_owned()];
        assert!(hook.clone().check_granting(&[]).is_err());
        assert!(
            hook.check_granting(&["net:hooks.example.com".to_owned()])
                .is_ok()
        );
        assert_eq!(hook.granted, ["net:hooks.example.com"]);
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
