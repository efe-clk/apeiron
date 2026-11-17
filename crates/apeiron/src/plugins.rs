//! Installing, listing and removing plugins (PLUGIN.md §1). Plugins belong
//! to the user, not to a repository; `dir` is the plugin folder, the
//! user's by default.

use std::path::{Path, PathBuf};

use apeiron_plugin::Plugins;

use crate::error::Result;
use crate::{PluginAdded, PluginInfo};

fn open(dir: Option<&Path>) -> Result<Plugins> {
    let dir = match dir {
        Some(dir) => dir.to_path_buf(),
        None => apeiron_plugin::default_dir()?,
    };
    Ok(Plugins::open(dir)?)
}

/// The installed plugins, by name.
pub fn plugins(dir: Option<&Path>) -> Result<Vec<PluginInfo>> {
    Ok(open(dir)?.list().to_vec())
}

/// Checks the WebAssembly component in `file` and installs it, replacing a
/// plugin of the same name.
pub fn add_plugin(file: &Path, dir: Option<&Path>) -> Result<PluginAdded> {
    Ok(open(dir)?.add(file)?)
}

/// Removes an installed plugin.
pub fn remove_plugin(name: &str, dir: Option<&Path>) -> Result<PluginInfo> {
    Ok(open(dir)?.remove(name)?)
}

/// The plugin folder used when none is given.
pub fn plugin_dir() -> Result<PathBuf> {
    Ok(apeiron_plugin::default_dir()?)
}

/// The plugins a store offers (PLUGIN.md §6); `store` is the address or
/// file of its index, `APEIRON_PLUGIN_STORE` or the latest release when
/// absent.
pub fn store_plugins(store: Option<&str>) -> Result<Vec<crate::StoreEntry>> {
    let store = store.map_or_else(apeiron_plugin::default_store, str::to_owned);
    Ok(apeiron_plugin::index(&store)?)
}

/// Downloads the plugin `name` from a store, checks its SHA-256 against the
/// store's index, and installs it after the checks of [`add_plugin`].
pub fn install_plugin(name: &str, store: Option<&str>, dir: Option<&Path>) -> Result<PluginAdded> {
    use crate::error::{Error, ErrorKind};
    let store = store.map_or_else(apeiron_plugin::default_store, str::to_owned);
    let entry = apeiron_plugin::index(&store)?
        .into_iter()
        .find(|e| e.name == name)
        .ok_or_else(|| {
            Error::new(
                ErrorKind::NotFound,
                format!("the store {store} offers no plugin named {name}"),
            )
        })?;
    let mut plugins = open(dir)?;
    let folder = plugins.dir().to_path_buf();
    std::fs::create_dir_all(&folder).map_err(|e| Error::io(folder.clone(), &e))?;
    let file = folder.join(format!(".download-{}-{name}.wasm", std::process::id()));
    let added = apeiron_plugin::download(&store, &entry, &file)
        .map_err(Error::from)
        .and_then(|()| Ok(plugins.add(&file)?));
    let _ = std::fs::remove_file(&file);
    let added = added?;
    if added.info.name != name {
        let _ = plugins.remove(&added.info.name);
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "the store's {name} calls itself {}; it was not installed",
                added.info.name
            ),
        ));
    }
    Ok(added)
}

/// The index of a store holding the plugin files `files`, each beside it by
/// its file name; the release workflow makes `plugins.json` with it.
pub fn plugin_index(files: &[PathBuf]) -> Result<crate::StoreIndex> {
    Ok(apeiron_plugin::make_index(files)?)
}
