//! Object model and storage engine for Apeiron.
//!
//! The on-disk format implemented here is specified in `docs/FORMAT.md`.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
