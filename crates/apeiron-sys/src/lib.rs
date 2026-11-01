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

/// Lowers the CPU and disk priority, so that background work gives way to
/// what the user is doing (DAEMON.md §2). On Linux it applies to the calling
/// thread and the threads it starts afterwards, so call it before starting
/// any; on macOS and Windows it applies to the whole process. A priority that
/// is already lower is left as it is. It cannot be undone.
pub fn lower_priority() -> io::Result<()> {
    imp::lower_priority()
}

/// The niceness background work runs at: lower than normal (0), above the
/// lowest (19).
#[cfg(unix)]
const NICE: libc::c_int = 10;

#[cfg(not(windows))]
mod imp {
    use std::io;
    use std::path::Path;

    pub(crate) fn rename(from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    /// The niceness of the calling thread (Linux) or process.
    pub(crate) fn nice() -> io::Result<libc::c_int> {
        // getpriority can return -1 as a valid value; errno tells them apart.
        // SAFETY: errno is a thread-local integer the C library owns.
        unsafe { *errno() = 0 };
        // SAFETY: plain system call without pointers. The cast is needed
        // because the type of `which` differs between C libraries.
        #[allow(clippy::useless_conversion)]
        let n = unsafe { libc::getpriority(libc::PRIO_PROCESS as _, 0) };
        // SAFETY: as above.
        if n == -1 && unsafe { *errno() } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(n)
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn errno() -> *mut libc::c_int {
        // SAFETY: returns the address of the calling thread's errno.
        unsafe { libc::__errno_location() }
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn errno() -> *mut libc::c_int {
        // SAFETY: returns the address of the calling thread's errno.
        unsafe { libc::__error() }
    }

    pub(crate) fn lower_priority() -> io::Result<()> {
        if nice()? < super::NICE {
            // SAFETY: plain system call without pointers.
            #[allow(clippy::useless_conversion)]
            let r = unsafe { libc::setpriority(libc::PRIO_PROCESS as _, 0, super::NICE) };
            if r != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        lower_disk()
    }

    /// The lowest level of the best-effort disk class, which still gets its
    /// turn on a busy disk (ioprio_set(2)).
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn lower_disk() -> io::Result<()> {
        const WHO_PROCESS: libc::c_long = 1;
        const CLASS_BEST_EFFORT: libc::c_long = 2;
        const LOWEST: libc::c_long = 7;
        const CLASS_SHIFT: libc::c_long = 13;
        // SAFETY: plain system call without pointers; 0 is the calling thread.
        let r = unsafe {
            libc::syscall(
                libc::SYS_ioprio_set,
                WHO_PROCESS,
                0 as libc::c_long,
                (CLASS_BEST_EFFORT << CLASS_SHIFT) | LOWEST,
            )
        };
        if r != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Throttled disk access for the process (setiopolicy_np(3)).
    #[cfg(target_vendor = "apple")]
    fn lower_disk() -> io::Result<()> {
        const IOPOL_TYPE_DISK: libc::c_int = 0;
        const IOPOL_SCOPE_PROCESS: libc::c_int = 0;
        const IOPOL_THROTTLE: libc::c_int = 3;
        unsafe extern "C" {
            fn setiopolicy_np(
                iotype: libc::c_int,
                scope: libc::c_int,
                policy: libc::c_int,
            ) -> libc::c_int;
        }
        // SAFETY: plain library call without pointers.
        let r = unsafe { setiopolicy_np(IOPOL_TYPE_DISK, IOPOL_SCOPE_PROCESS, IOPOL_THROTTLE) };
        if r != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Other systems have no portable call for it; the CPU priority alone
    /// is lowered.
    #[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
    fn lower_disk() -> io::Result<()> {
        Ok(())
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
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, PROCESS_MODE_BACKGROUND_BEGIN, SetPriorityClass,
    };

    /// Background mode lowers the CPU, disk and memory priority of the
    /// process together.
    pub(crate) fn lower_priority() -> io::Result<()> {
        // The error a process already in background mode gets.
        const ALREADY_BACKGROUND: i32 = 402;
        // SAFETY: GetCurrentProcess returns a pseudo handle that needs no
        // closing and is valid for the call.
        let ok = unsafe { SetPriorityClass(GetCurrentProcess(), PROCESS_MODE_BACKGROUND_BEGIN) };
        if ok == 0 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() != Some(ALREADY_BACKGROUND) {
                return Err(e);
            }
        }
        Ok(())
    }

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

    #[test]
    fn the_priority_is_lowered_and_stays_low() {
        // A thread of its own, so the other tests keep their priority on
        // Linux; elsewhere the whole test process runs lower, which is
        // harmless.
        std::thread::spawn(|| {
            lower_priority().unwrap();
            #[cfg(unix)]
            assert!(imp::nice().unwrap() >= NICE);
            // A second call does not try to raise it again.
            lower_priority().unwrap();
            #[cfg(target_os = "linux")]
            {
                // Threads started afterwards inherit it.
                let child = std::thread::spawn(|| imp::nice().unwrap());
                assert!(child.join().unwrap() >= NICE);
            }
        })
        .join()
        .unwrap();
    }
}
