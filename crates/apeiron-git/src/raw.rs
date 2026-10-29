//! Git objects as raw bytes (GIT.md §2): parsed and written by hand, so
//! nothing is normalized and a rebuilt object gets the same id.

use std::cmp::Ordering;

/// A commit split into the parts a snapshot keeps.
pub(crate) struct Commit<'a> {
    pub(crate) tree: &'a [u8],
    pub(crate) parents: Vec<&'a [u8]>,
    /// Every header line after `tree` and `parent`, each ending in `\n`.
    pub(crate) headers: &'a [u8],
    /// `None` when the object has no blank line at all.
    pub(crate) message: Option<&'a [u8]>,
}

/// An annotated tag split the same way.
pub(crate) struct Tag<'a> {
    pub(crate) object: &'a [u8],
    /// Every header line after `object`: `type`, `tag`, `tagger` and others.
    pub(crate) headers: &'a [u8],
    pub(crate) message: Option<&'a [u8]>,
}

/// The header block, ending in `\n`, and the message after the blank line.
fn split_body(data: &[u8]) -> (&[u8], Option<&[u8]>) {
    match data.windows(2).position(|w| w == b"\n\n") {
        Some(i) => (&data[..=i], Some(&data[i + 2..])),
        None => (data, None),
    }
}

/// Removes the line `<key> <value>\n` from the start of `block`.
fn take_line<'a>(block: &mut &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let rest = block.strip_prefix(key)?.strip_prefix(b" ")?;
    let end = rest.iter().position(|&b| b == b'\n')?;
    *block = &rest[end + 1..];
    Some(&rest[..end])
}

pub(crate) fn parse_commit(data: &[u8]) -> Option<Commit<'_>> {
    let (mut block, message) = split_body(data);
    let tree = take_line(&mut block, b"tree")?;
    let mut parents = Vec::new();
    while let Some(parent) = take_line(&mut block, b"parent") {
        parents.push(parent);
    }
    Some(Commit {
        tree,
        parents,
        headers: block,
        message,
    })
}

pub(crate) fn parse_tag(data: &[u8]) -> Option<Tag<'_>> {
    let (mut block, message) = split_body(data);
    let object = take_line(&mut block, b"object")?;
    Some(Tag {
        object,
        headers: block,
        message,
    })
}

/// The value of the first header line `<key> …` in `headers`.
pub(crate) fn header<'a>(headers: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let mut block = headers;
    while !block.is_empty() {
        if let Some(value) = take_line(&mut block, key) {
            return Some(value);
        }
        let end = block.iter().position(|&b| b == b'\n')?;
        block = &block[end + 1..];
    }
    None
}

/// `Name <email>` and the Unix time of a signature `Name <email> 123 +0100`.
pub(crate) fn signature(value: &[u8]) -> (String, i64) {
    let Some(end) = value.iter().rposition(|&b| b == b'>') else {
        return (String::from_utf8_lossy(value).into_owned(), 0);
    };
    let who = String::from_utf8_lossy(&value[..=end]).into_owned();
    let seconds = std::str::from_utf8(&value[end + 1..])
        .ok()
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (who, seconds)
}

/// One entry of a tree: the mode as written, the name and the object id.
pub(crate) struct Entry {
    pub(crate) mode: Vec<u8>,
    pub(crate) name: Vec<u8>,
    pub(crate) id: Vec<u8>,
}

pub(crate) const TREE_MODE: &[u8] = b"40000";
pub(crate) const LINK_MODE: &[u8] = b"120000";
pub(crate) const GITLINK_MODE: &[u8] = b"160000";

/// `mode SP name NUL id` repeated; `hash_len` is 20 for SHA-1.
pub(crate) fn parse_tree(mut data: &[u8], hash_len: usize) -> Option<Vec<Entry>> {
    let mut entries = Vec::new();
    while !data.is_empty() {
        let space = data.iter().position(|&b| b == b' ')?;
        let nul = space + data[space..].iter().position(|&b| b == 0)?;
        let end = nul + 1 + hash_len;
        if end > data.len() {
            return None;
        }
        entries.push(Entry {
            mode: data[..space].to_vec(),
            name: data[space + 1..nul].to_vec(),
            id: data[nul + 1..end].to_vec(),
        });
        data = &data[end..];
    }
    Some(entries)
}

/// Git's order: a folder sorts as if its name ended in `/`.
fn git_order(a: &Entry, b: &Entry) -> Ordering {
    let key = |e: &Entry| {
        let mut k = e.name.clone();
        if e.mode == TREE_MODE {
            k.push(b'/');
        }
        k
    };
    key(a).cmp(&key(b))
}

pub(crate) fn write_tree(mut entries: Vec<Entry>) -> Vec<u8> {
    entries.sort_by(git_order);
    let mut out = Vec::new();
    for e in entries {
        out.extend_from_slice(&e.mode);
        out.push(b' ');
        out.extend_from_slice(&e.name);
        out.push(0);
        out.extend_from_slice(&e.id);
    }
    out
}

fn body(out: &mut Vec<u8>, headers: &[u8], message: Option<&[u8]>) {
    out.extend_from_slice(headers);
    if let Some(message) = message {
        out.push(b'\n');
        out.extend_from_slice(message);
    }
}

pub(crate) fn write_commit(
    tree: &str,
    parents: &[String],
    headers: &[u8],
    message: Option<&[u8]>,
) -> Vec<u8> {
    let mut out = format!("tree {tree}\n").into_bytes();
    for parent in parents {
        out.extend_from_slice(format!("parent {parent}\n").as_bytes());
    }
    body(&mut out, headers, message);
    out
}

pub(crate) fn write_tag(object: &str, headers: &[u8], message: Option<&[u8]>) -> Vec<u8> {
    let mut out = format!("object {object}\n").into_bytes();
    body(&mut out, headers, message);
    out
}

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commits_and_tags_split_and_join_exactly() {
        let raw = b"tree 1111\nparent 2222\nparent 3333\nauthor A <a@x> 1700000000 +0100\ncommitter C <c@x> 1700000100 -0230\ngpgsig -----BEGIN-----\n \n line\n -----END-----\n\nsubject\n\nbody\n";
        let c = parse_commit(raw).unwrap();
        assert_eq!(c.tree, b"1111");
        assert_eq!(c.parents, [&b"2222"[..], b"3333"]);
        assert!(c.headers.starts_with(b"author ") && c.headers.ends_with(b"END-----\n"));
        assert_eq!(c.message.unwrap(), b"subject\n\nbody\n");
        let parents = ["2222".to_owned(), "3333".to_owned()];
        assert_eq!(write_commit("1111", &parents, c.headers, c.message), raw);
        let committer = header(c.headers, b"committer").unwrap();
        assert_eq!(signature(committer), ("C <c@x>".to_owned(), 1_700_000_100));

        let bare = b"tree 1111\nauthor A <a@x> 1 +0000\ncommitter A <a@x> 1 +0000\n";
        let c = parse_commit(bare).unwrap();
        assert!(c.message.is_none() && c.parents.is_empty());
        assert_eq!(write_commit("1111", &[], c.headers, c.message), bare);

        let tag = b"object 4444\ntype commit\ntag v1\ntagger T <t@x> 5 +0000\n\nrelease\n";
        let t = parse_tag(tag).unwrap();
        assert_eq!(t.object, b"4444");
        assert_eq!(write_tag("4444", t.headers, t.message), tag);
    }

    #[test]
    fn trees_sort_folders_as_git_does() {
        let entry = |mode: &[u8], name: &[u8]| Entry {
            mode: mode.to_vec(),
            name: name.to_vec(),
            id: vec![7; 20],
        };
        // "a.txt" < "a/" < "a0" in Git's order.
        let bytes = write_tree(vec![
            entry(b"100644", b"a0"),
            entry(TREE_MODE, b"a"),
            entry(b"100644", b"a.txt"),
        ]);
        let names: Vec<Vec<u8>> = parse_tree(&bytes, 20)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, [&b"a.txt"[..], b"a", b"a0"]);
        assert!(parse_tree(&bytes[..bytes.len() - 1], 20).is_none());
        assert_eq!(from_hex(&to_hex(b"\x00\xff")).unwrap(), b"\x00\xff");
    }
}
