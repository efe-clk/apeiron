//! Tags on files: the `user.xdg.tags` attribute (FORMAT.md §6.3), kept in
//! a tree entry's `xattrs`.

use std::path::Path;

use apeiron_core::TreeEntry;

use crate::error::{Error, Result};

/// The attribute file managers keep tags in.
pub(crate) const TAGS: &str = "user.xdg.tags";

/// The tags of an entry, in the order stored.
pub fn entry_tags(entry: &TreeEntry) -> Vec<String> {
    entry
        .xattrs
        .iter()
        .find(|(name, _)| name == TAGS.as_bytes())
        .map(|(_, value)| parse(value))
        .unwrap_or_default()
}

/// Tags from an attribute's value: comma-separated, trimmed, none empty.
fn parse(value: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(value)
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The attribute's value for `tags`; `None` when there are none.
fn value(tags: &[String]) -> Result<Option<Vec<u8>>> {
    let mut kept: Vec<&str> = Vec::new();
    for tag in tags.iter().map(|t| t.trim()) {
        if tag.is_empty() {
            continue;
        }
        if tag.contains(',') || tag.chars().any(char::is_control) {
            return Err(Error::InvalidInput(format!(
                "{tag:?}: a tag holds no comma and no control character"
            )));
        }
        if !kept.contains(&tag) {
            kept.push(tag);
        }
    }
    Ok((!kept.is_empty()).then(|| kept.join(",").into_bytes()))
}

/// The attributes a backup keeps of the file at `path`: its tags on
/// Linux, nothing elsewhere.
pub(crate) fn read_xattrs(path: &Path) -> Vec<(Vec<u8>, Vec<u8>)> {
    #[cfg(target_os = "linux")]
    if let Ok(Some(value)) = xattr::get(path, TAGS)
        && !parse(&value).is_empty()
    {
        return vec![(TAGS.as_bytes().to_vec(), value)];
    }
    let _ = path;
    Vec::new()
}

/// Writes the tags of `entry` to the file at `path`; nothing when it has
/// none, or off Linux.
pub(crate) fn write_xattrs(path: &Path, entry: &TreeEntry) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    if let Some((_, value)) = entry.xattrs.iter().find(|(n, _)| n == TAGS.as_bytes()) {
        return xattr::set(path, TAGS, value);
    }
    let _ = (path, entry);
    Ok(())
}

/// The tags of the file at `path` on disk.
pub fn file_tags(path: &Path) -> Result<Vec<String>> {
    #[cfg(target_os = "linux")]
    {
        let value = xattr::get(path, TAGS).map_err(|e| Error::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        Ok(value.map(|v| parse(&v)).unwrap_or_default())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        Err(Error::Unsupported(
            "tags on files are kept on Linux only".to_owned(),
        ))
    }
}

/// Sets the tags of the file at `path` on disk; none removes them.
pub fn set_file_tags(path: &Path, tags: &[String]) -> Result<()> {
    let value = value(tags)?;
    #[cfg(target_os = "linux")]
    {
        let io = |e| Error::Io {
            path: path.to_path_buf(),
            source: e,
        };
        match value {
            Some(v) => xattr::set(path, TAGS, &v).map_err(io),
            None => match xattr::remove(path, TAGS) {
                Ok(()) => Ok(()),
                // No tags to remove.
                Err(e) if e.raw_os_error() == Some(61) => Ok(()),
                Err(e) => Err(io(e)),
            },
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (path, value);
        Err(Error::Unsupported(
            "tags on files are kept on Linux only".to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_parsed_and_joined() {
        assert_eq!(parse(b" a, b ,,c"), ["a", "b", "c"]);
        let tags = ["x".to_owned(), " y ".to_owned(), "x".to_owned()];
        assert_eq!(value(&tags).unwrap(), Some(b"x,y".to_vec()));
        assert_eq!(value(&[]).unwrap(), None);
        assert!(value(&["a,b".to_owned()]).is_err());
    }
}
