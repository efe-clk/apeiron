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
