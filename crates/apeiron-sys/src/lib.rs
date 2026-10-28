//! Operating system calls that need `unsafe` code (ADR-014).
//!
//! Every other Apeiron crate forbids `unsafe`; this crate keeps such calls
//! small, documented and tested.

#![deny(unsafe_op_in_unsafe_fn, clippy::undocumented_unsafe_blocks)]

use std::io;
use std::path::Path;

/// Renames `from` to `to`, replacing `to` if it exists. On Windows the call
/// returns only after the rename has reached the disk (FORMAT.md §9.1).
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    imp::rename(from, to)
}

#[cfg(not(windows))]
mod imp {
    use std::io;
    use std::path::Path;

    pub(crate) fn rename(from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain([0]).collect()
    }

    pub(crate) fn rename(from: &Path, to: &Path) -> io::Result<()> {
        let from = wide(from);
        let to = wide(to);
        // SAFETY: both pointers refer to NUL-terminated UTF-16 buffers that
        // stay alive for the duration of the call.
        let ok = unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn rename_moves_and_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::write(&a, b"first").unwrap();
        rename(&a, &b).unwrap();
        assert!(!a.exists());
        assert_eq!(fs::read(&b).unwrap(), b"first");

        fs::write(&a, b"second").unwrap();
        rename(&a, &b).unwrap();
        assert_eq!(fs::read(&b).unwrap(), b"second");

        assert!(rename(&a, &b).is_err());
    }
}
