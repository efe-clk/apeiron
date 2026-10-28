//! Object model and storage engine for Apeiron.
//!
//! The on-disk format implemented here is specified in `docs/FORMAT.md`.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod chunker;
mod config;
mod error;
#[cfg(test)]
mod fault;
mod fsutil;
mod id;
mod index;
mod lock;
mod object;
mod pack;
mod refs;
mod repo;

pub use chunker::ChunkerParams;
pub use config::FORMAT_VERSION;
pub use error::{Error, Result};
pub use id::{ID_LEN, ObjectId, ObjectType};
pub use lock::RepoLock;
pub use object::{Blob, EntryKind, Snapshot, Time, Tree, TreeEntry};
pub use refs::{RefOp, ReflogEntry, RepairReport, validate_ref_name};
pub use repo::{GcOptions, GcReport, ObjectWriter, Repository, VerifyReport};
