//! Platform differences: file names, attributes and symbolic links.

use std::ffi::{OsStr, OsString};
use std::fs::{self, Metadata};
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apeiron_core::Time;

/// Stored bytes of a file name; `None` if the platform name cannot be stored.
#[cfg(unix)]
pub(crate) fn name_bytes(name: &OsStr) -> Option<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    Some(name.as_bytes().to_vec())
}

/// Windows names are stored as UTF-8; names with unpaired surrogates are not supported.
#[cfg(not(unix))]
pub(crate) fn name_bytes(name: &OsStr) -> Option<Vec<u8>> {
    name.to_str().map(|s| s.as_bytes().to_vec())
}

/// Platform name for stored bytes; `None` if the platform cannot represent it.
#[cfg(unix)]
pub(crate) fn os_name(bytes: &[u8]) -> Option<OsString> {
    use std::os::unix::ffi::OsStrExt;
    Some(OsStr::from_bytes(bytes).to_os_string())
}

#[cfg(not(unix))]
pub(crate) fn os_name(bytes: &[u8]) -> Option<OsString> {
    std::str::from_utf8(bytes).ok().map(OsString::from)
}

pub(crate) fn to_time(t: SystemTime) -> Time {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => Time {
            seconds: i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
            nanos: d.subsec_nanos(),
        },
        Err(e) => {
            // Before 1970: whole seconds round down, nanoseconds stay positive.
            let d = e.duration();
            let secs = i64::try_from(d.as_secs()).unwrap_or(i64::MAX);
            match d.subsec_nanos() {
                0 => Time {
                    seconds: -secs,
                    nanos: 0,
                },
                n => Time {
                    seconds: -secs - 1,
                    nanos: 1_000_000_000 - n,
                },
            }
        }
    }
}

pub(crate) fn from_time(t: Time) -> Option<SystemTime> {
    let nanos = Duration::from_nanos(u64::from(t.nanos));
    if t.seconds >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(t.seconds.unsigned_abs()) + nanos)
    } else {
        UNIX_EPOCH
            .checked_sub(Duration::from_secs(t.seconds.unsigned_abs()))?
            .checked_add(nanos)
    }
}

/// Attributes stored with a tree entry.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Attrs {
    pub(crate) mode: Option<u16>,
    pub(crate) mtime: Option<Time>,
    pub(crate) uid: Option<u32>,
    pub(crate) gid: Option<u32>,
}

#[cfg(unix)]
pub(crate) fn attrs(md: &Metadata) -> Attrs {
    use std::os::unix::fs::MetadataExt;
    Attrs {
        mode: Some((md.mode() & 0o7777) as u16),
        mtime: md.modified().ok().map(to_time),
        uid: Some(md.uid()),
        gid: Some(md.gid()),
    }
}

#[cfg(not(unix))]
pub(crate) fn attrs(md: &Metadata) -> Attrs {
    Attrs {
        mtime: md.modified().ok().map(to_time),
        ..Attrs::default()
    }
}

/// Stored bytes of a symbolic link target.
#[cfg(unix)]
pub(crate) fn link_target(path: &Path) -> io::Result<Option<Vec<u8>>> {
    Ok(name_bytes(fs::read_link(path)?.as_os_str()))
}

#[cfg(not(unix))]
pub(crate) fn link_target(path: &Path) -> io::Result<Option<Vec<u8>>> {
    // Stored with `/` separators so the snapshot reads the same everywhere.
    Ok(fs::read_link(path)?
        .to_str()
        .map(|s| s.replace('\\', "/").into_bytes()))
}

#[cfg(unix)]
pub(crate) fn make_symlink(target: &[u8], path: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    std::os::unix::fs::symlink(OsStr::from_bytes(target), path)
}

/// Needs Developer Mode or administrator rights on Windows.
#[cfg(windows)]
pub(crate) fn make_symlink(target: &[u8], path: &Path) -> io::Result<()> {
    let target = std::str::from_utf8(target)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "target is not UTF-8"))?
        .replace('/', "\\");
    std::os::windows::fs::symlink_file(target, path)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn make_symlink(_target: &[u8], _path: &Path) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

/// Applies permissions and owner to a restored entry. The owner is set only
/// where the process is allowed to; otherwise it is left as is.
#[cfg(unix)]
pub(crate) fn apply_attrs(path: &Path, a: &Attrs) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if a.uid.is_some() || a.gid.is_some() {
        match std::os::unix::fs::chown(path, a.uid, a.gid) {
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {}
            other => other?,
        }
    }
    if let Some(mode) = a.mode {
        fs::set_permissions(path, fs::Permissions::from_mode(u32::from(mode)))?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn apply_attrs(_path: &Path, _a: &Attrs) -> io::Result<()> {
    Ok(())
}

/// Sets the modification time of a restored folder (not supported on Windows).
#[cfg(unix)]
pub(crate) fn set_dir_mtime(path: &Path, time: SystemTime) -> io::Result<()> {
    fs::File::open(path)?.set_modified(time)
}

#[cfg(not(unix))]
pub(crate) fn set_dir_mtime(_path: &Path, _time: SystemTime) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_conversion_round_trips() {
        for t in [
            UNIX_EPOCH,
            UNIX_EPOCH + Duration::new(1_700_000_000, 123),
            UNIX_EPOCH - Duration::new(5, 0),
            UNIX_EPOCH - Duration::new(5, 250),
        ] {
            let stored = to_time(t);
            assert!(stored.nanos < 1_000_000_000);
            assert_eq!(from_time(stored), Some(t));
        }
    }
}
