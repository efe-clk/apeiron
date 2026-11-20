//! `apeiron mount` (MOUNT.md): every snapshot of a repository as a
//! read-only folder, through FUSE on Linux and macOS. It uses only the
//! public `apeiron` crate.
//!
//! ```text
//! <mountpoint>/backup/documents/latest/…
//! <mountpoint>/backup/documents/2026-11-20T103000Z_3f9ae1c2/…
//! ```

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod fs;
mod tree;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use fs::{Mounted, mount};
pub use tree::{Node, Tree, snapshot_name};

/// Whether this system can mount a repository.
pub const SUPPORTED: bool = cfg!(any(target_os = "linux", target_os = "macos"));
