//! Remotes reached over the network (FORMAT.md §15, ADR-018). Each one is
//! an [`apeiron_core::Store`]; the network libraries run on Tokio behind
//! that blocking interface.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod s3;
mod sftp;

pub use s3::S3Store;
pub use sftp::{SftpStore, SshOptions};
