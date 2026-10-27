//! Object model and storage engine for Apeiron.
//!
//! The on-disk format implemented here is specified in `docs/FORMAT.md`.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod chunker;
mod config;
mod error;
mod fsutil;
mod id;
mod index;
mod pack;
mod refs;
mod repo;

pub use chunker::ChunkerParams;
pub use error::{Error, Result};
pub use id::{ID_LEN, ObjectId, ObjectType};
pub use refs::{RefOp, ReflogEntry, validate_ref_name};
pub use repo::{ObjectWriter, Repository};
