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

/// Reads the tags of a file as the value of [`TAGS`], whichever way this
/// system keeps them.
#[cfg(target_os = "linux")]
fn read_value(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    xattr::get(path, TAGS)
}

#[cfg(target_os = "macos")]
fn read_value(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    Ok(xattr::get(path, FINDER_TAGS)?
        .as_deref()
        .and_then(from_finder))
}

/// Writes the value of [`TAGS`] the way this system keeps tags.
#[cfg(target_os = "linux")]
fn write_value(path: &Path, value: &[u8]) -> std::io::Result<()> {
    xattr::set(path, TAGS, value)
}

#[cfg(target_os = "macos")]
fn write_value(path: &Path, value: &[u8]) -> std::io::Result<()> {
    xattr::set(path, FINDER_TAGS, &to_finder(value)?)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn remove_value(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    let (name, absent) = (TAGS, 61);
    #[cfg(target_os = "macos")]
    let (name, absent) = (FINDER_TAGS, 93);
    match xattr::remove(path, name) {
        Err(e) if e.raw_os_error() != Some(absent) => Err(e),
        _ => Ok(()),
    }
}

/// Finder's attribute: a binary property list holding an array of names,
/// each with a newline and a colour number when it has a colour.
#[cfg(any(target_os = "macos", test))]
const FINDER_TAGS: &str = "com.apple.metadata:_kMDItemUserTags";

/// The value of [`TAGS`] for Finder's attribute. Colours are not kept, and a
/// comma, which a tag here cannot hold, becomes a space (FORMAT.md §6.3).
#[cfg(any(target_os = "macos", test))]
fn from_finder(bytes: &[u8]) -> Option<Vec<u8>> {
    let list: Vec<String> = plist::from_bytes(bytes).ok()?;
    let tags: Vec<String> = list
        .iter()
        .map(|t| t.split('\n').next().unwrap_or("").replace(',', " "))
        .filter(|t| !t.chars().any(char::is_control))
        .collect();
    value(&tags).ok().flatten()
}

/// Finder's attribute for a value of [`TAGS`].
#[cfg(any(target_os = "macos", test))]
fn to_finder(value: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    plist::to_writer_binary(&mut bytes, &parse(value)).map_err(std::io::Error::other)?;
    Ok(bytes)
}

/// The attributes a backup keeps of the file at `path`: its tags on Linux
/// and macOS, nothing elsewhere.
pub(crate) fn read_xattrs(path: &Path) -> Vec<(Vec<u8>, Vec<u8>)> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if let Ok(Some(value)) = read_value(path)
        && !parse(&value).is_empty()
    {
        return vec![(TAGS.as_bytes().to_vec(), value)];
    }
    let _ = path;
    Vec::new()
}

/// Writes the tags of `entry` to the file at `path`; nothing when it has
/// none, or where files have no tags.
pub(crate) fn write_xattrs(path: &Path, entry: &TreeEntry) -> std::io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if let Some((_, value)) = entry.xattrs.iter().find(|(n, _)| n == TAGS.as_bytes()) {
        return write_value(path, value);
    }
    let _ = (path, entry);
    Ok(())
}

/// The tags of the file at `path` on disk.
pub fn file_tags(path: &Path) -> Result<Vec<String>> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let value = read_value(path).map_err(|e| Error::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        Ok(value.map(|v| parse(&v)).unwrap_or_default())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = path;
        Err(Error::Unsupported(
            "tags on files are kept on Linux and macOS only".to_owned(),
        ))
    }
}

/// Sets the tags of the file at `path` on disk; none removes them.
pub fn set_file_tags(path: &Path, tags: &[String]) -> Result<()> {
    let value = value(tags)?;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let written = match value {
            Some(v) => write_value(path, &v),
            None => remove_value(path),
        };
        written.map_err(|e| Error::Io {
            path: path.to_path_buf(),
            source: e,
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (path, value);
        Err(Error::Unsupported(
            "tags on files are kept on Linux and macOS only".to_owned(),
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

    #[test]
    fn finder_tags_lose_their_colours_and_commas() {
        let mut bytes = Vec::new();
        let list = ["Red\n6", "Project X", "a,b\n2", "Red\n6"];
        plist::to_writer_binary(&mut bytes, &list).unwrap();
        let value = from_finder(&bytes).unwrap();
        assert_eq!(value, b"Red,Project X,a b");
        assert_eq!(FINDER_TAGS.split(':').count(), 2);
        // What this writes, Finder reads, and it comes back the same.
        assert_eq!(from_finder(&to_finder(&value).unwrap()).unwrap(), value);
    }

    #[test]
    fn a_damaged_finder_attribute_means_no_tags() {
        assert_eq!(from_finder(b"not a property list"), None);
        let mut bytes = Vec::new();
        plist::to_writer_binary(&mut bytes, &Vec::<String>::new()).unwrap();
        assert_eq!(from_finder(&bytes), None);
    }
}
