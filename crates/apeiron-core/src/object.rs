//! Blob, tree and snapshot objects in canonical CBOR (FORMAT.md §6).

use std::convert::Infallible;

use minicbor::decode::Error as DecodeError;
use minicbor::{Decoder, Encoder};

use crate::error::{Error, Result};
use crate::id::{ID_LEN, ObjectId, ObjectType};

const KIND_FILE: u64 = 0;
const KIND_DIRECTORY: u64 = 1;
const KIND_SYMLINK: u64 = 2;
const MODE_MASK: u16 = 0o7777;
const NANOS_PER_SEC: u32 = 1_000_000_000;

/// A point in time: Unix seconds (UTC) and nanoseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Time {
    pub seconds: i64,
    pub nanos: u32,
}

/// The content of one non-empty file (FORMAT.md §6.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blob {
    pub size: u64,
    /// Id and raw length of each chunk, in file order.
    pub chunks: Vec<(ObjectId, u32)>,
}

/// One directory (FORMAT.md §6.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tree {
    /// Sorted by name bytes; names are unique.
    pub entries: Vec<TreeEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    /// File name bytes (WTF-8 on Windows).
    pub name: Vec<u8>,
    pub kind: EntryKind,
    /// Unix permission bits (lowest 12 bits).
    pub mode: Option<u16>,
    pub mtime: Option<Time>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    /// Extended attributes as (name, value), sorted by name.
    pub xattrs: Vec<(Vec<u8>, Vec<u8>)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// `blob` is `None` exactly when `size` is 0.
    File {
        size: u64,
        blob: Option<ObjectId>,
    },
    Directory {
        tree: ObjectId,
    },
    Symlink {
        target: Vec<u8>,
    },
}

/// The state of a data set at one moment (FORMAT.md §6.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub tree: ObjectId,
    pub parents: Vec<ObjectId>,
    pub time: Time,
    pub author: Option<String>,
    pub host: Option<String>,
    pub message: Option<String>,
    /// Free key-value pairs, sorted by key without duplicates.
    pub meta: Vec<(String, String)>,
}

impl Blob {
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut e = Encoder::new(Vec::new());
        ok(e.map(2))?;
        ok(e.u8(0).and_then(|e| e.u64(self.size)))?;
        ok(e.u8(1).and_then(|e| e.array(self.chunks.len() as u64)))?;
        for (id, len) in &self.chunks {
            ok(e.array(2).and_then(|e| e.bytes(id.as_bytes())))?;
            ok(e.u32(*len))?;
        }
        Ok(e.into_writer())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let blob = decode_with(ObjectType::Blob, bytes, |d| {
            let mut size = None;
            let mut chunks = None;
            map(d, |key, d| {
                match key {
                    0 => size = Some(d.u64()?),
                    1 => {
                        chunks = Some(array(d, |d| {
                            pair(d)?;
                            Ok((id(d)?, d.u32()?))
                        })?);
                    }
                    _ => return Err(unknown_key()),
                }
                Ok(())
            })?;
            Ok(Blob {
                size: size.ok_or_else(|| missing("size"))?,
                chunks: chunks.ok_or_else(|| missing("chunks"))?,
            })
        })?;
        check_canonical(ObjectType::Blob, bytes, &blob.encode()?)?;
        Ok(blob)
    }

    fn validate(&self) -> Result<()> {
        let invalid = |reason: &str| invalid(ObjectType::Blob, reason);
        if self.chunks.is_empty() {
            return Err(invalid("a blob needs at least one chunk"));
        }
        let mut total = 0u64;
        for (_, len) in &self.chunks {
            if *len == 0 {
                return Err(invalid("empty chunk"));
            }
            total = total
                .checked_add(u64::from(*len))
                .ok_or_else(|| invalid("chunk sizes overflow"))?;
        }
        if total != self.size {
            return Err(invalid("chunk sizes do not add up to the blob size"));
        }
        Ok(())
    }
}

impl Tree {
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut e = Encoder::new(Vec::new());
        ok(e.map(1))?;
        ok(e.u8(0).and_then(|e| e.array(self.entries.len() as u64)))?;
        for entry in &self.entries {
            entry.encode(&mut e)?;
        }
        Ok(e.into_writer())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let tree = decode_with(ObjectType::Tree, bytes, |d| {
            let mut entries = None;
            map(d, |key, d| {
                match key {
                    0 => entries = Some(array(d, TreeEntry::decode)?),
                    _ => return Err(unknown_key()),
                }
                Ok(())
            })?;
            Ok(Tree {
                entries: entries.ok_or_else(|| missing("entries"))?,
            })
        })?;
        check_canonical(ObjectType::Tree, bytes, &tree.encode()?)?;
        Ok(tree)
    }

    fn validate(&self) -> Result<()> {
        let invalid = |reason: &str| invalid(ObjectType::Tree, reason);
        for entry in &self.entries {
            entry.validate()?;
        }
        if !self.entries.windows(2).all(|w| w[0].name < w[1].name) {
            return Err(invalid("entries are not sorted by unique names"));
        }
        Ok(())
    }
}

impl TreeEntry {
    fn encode(&self, e: &mut Encoder<Vec<u8>>) -> Result<()> {
        let (size, content, target) = match &self.kind {
            EntryKind::File { size, blob } => (Some(*size), *blob, None),
            EntryKind::Directory { tree } => (None, Some(*tree), None),
            EntryKind::Symlink { target } => (None, None, Some(target)),
        };
        let kind = match self.kind {
            EntryKind::File { .. } => KIND_FILE,
            EntryKind::Directory { .. } => KIND_DIRECTORY,
            EntryKind::Symlink { .. } => KIND_SYMLINK,
        };
        let len = 2
            + u64::from(size.is_some())
            + u64::from(content.is_some())
            + u64::from(target.is_some())
            + u64::from(self.mode.is_some())
            + u64::from(self.mtime.is_some())
            + u64::from(self.uid.is_some())
            + u64::from(self.gid.is_some())
            + u64::from(!self.xattrs.is_empty());
        ok(e.map(len))?;
        ok(e.u8(0).and_then(|e| e.bytes(&self.name)))?;
        ok(e.u8(1).and_then(|e| e.u64(kind)))?;
        if let Some(size) = size {
            ok(e.u8(2).and_then(|e| e.u64(size)))?;
        }
        if let Some(content) = content {
            ok(e.u8(3).and_then(|e| e.bytes(content.as_bytes())))?;
        }
        if let Some(target) = target {
            ok(e.u8(4).and_then(|e| e.bytes(target)))?;
        }
        if let Some(mode) = self.mode {
            ok(e.u8(5).and_then(|e| e.u16(mode)))?;
        }
        if let Some(mtime) = self.mtime {
            ok(e.u8(6))?;
            encode_time(e, mtime)?;
        }
        if let Some(uid) = self.uid {
            ok(e.u8(7).and_then(|e| e.u32(uid)))?;
        }
        if let Some(gid) = self.gid {
            ok(e.u8(8).and_then(|e| e.u32(gid)))?;
        }
        if !self.xattrs.is_empty() {
            ok(e.u8(9).and_then(|e| e.array(self.xattrs.len() as u64)))?;
            for (name, value) in &self.xattrs {
                ok(e.array(2).and_then(|e| e.bytes(name)?.bytes(value)))?;
            }
        }
        Ok(())
    }

    fn decode(d: &mut Decoder<'_>) -> DecodeResult<Self> {
        let mut name = None;
        let mut kind = None;
        let mut size = None;
        let mut content = None;
        let mut target = None;
        let mut entry = TreeEntry {
            name: Vec::new(),
            kind: EntryKind::Symlink { target: Vec::new() },
            mode: None,
            mtime: None,
            uid: None,
            gid: None,
            xattrs: Vec::new(),
        };
        map(d, |key, d| {
            match key {
                0 => name = Some(d.bytes()?.to_vec()),
                1 => kind = Some(d.u64()?),
                2 => size = Some(d.u64()?),
                3 => content = Some(id(d)?),
                4 => target = Some(d.bytes()?.to_vec()),
                5 => entry.mode = Some(d.u16()?),
                6 => entry.mtime = Some(decode_time(d)?),
                7 => entry.uid = Some(d.u32()?),
                8 => entry.gid = Some(d.u32()?),
                9 => {
                    entry.xattrs = array(d, |d| {
                        pair(d)?;
                        Ok((d.bytes()?.to_vec(), d.bytes()?.to_vec()))
                    })?;
                }
                _ => return Err(unknown_key()),
            }
            Ok(())
        })?;
        let wrong = || DecodeError::message("fields do not match the entry kind");
        entry.name = name.ok_or_else(|| missing("name"))?;
        entry.kind = match (kind.ok_or_else(|| missing("kind"))?, size, content, target) {
            (KIND_FILE, Some(size), blob, None) => EntryKind::File { size, blob },
            (KIND_DIRECTORY, None, Some(tree), None) => EntryKind::Directory { tree },
            (KIND_SYMLINK, None, None, Some(target)) => EntryKind::Symlink { target },
            (KIND_FILE | KIND_DIRECTORY | KIND_SYMLINK, ..) => return Err(wrong()),
            _ => return Err(DecodeError::message("unknown entry kind")),
        };
        Ok(entry)
    }

    fn validate(&self) -> Result<()> {
        let invalid = |reason: &str| invalid(ObjectType::Tree, reason);
        let name = &self.name;
        if name.is_empty()
            || name == b"."
            || name == b".."
            || name.contains(&b'/')
            || name.contains(&0)
        {
            return Err(invalid("invalid entry name"));
        }
        if let EntryKind::File { size, blob } = self.kind {
            if (size == 0) != blob.is_none() {
                return Err(invalid("a file has a blob exactly when it is not empty"));
            }
        }
        if self.mode.is_some_and(|m| m & !MODE_MASK != 0) {
            return Err(invalid("mode has bits above the lowest 12"));
        }
        if self.mtime.is_some_and(|t| t.nanos >= NANOS_PER_SEC) {
            return Err(invalid("nanoseconds out of range"));
        }
        if !self.xattrs.windows(2).all(|w| w[0].0 < w[1].0) {
            return Err(invalid(
                "extended attributes are not sorted by unique names",
            ));
        }
        Ok(())
    }
}

impl Snapshot {
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let texts = [(3, &self.author), (4, &self.host), (5, &self.message)];
        let len = 3
            + texts.iter().filter(|(_, t)| t.is_some()).count() as u64
            + u64::from(!self.meta.is_empty());
        let mut e = Encoder::new(Vec::new());
        ok(e.map(len))?;
        ok(e.u8(0).and_then(|e| e.bytes(self.tree.as_bytes())))?;
        ok(e.u8(1).and_then(|e| e.array(self.parents.len() as u64)))?;
        for parent in &self.parents {
            ok(e.bytes(parent.as_bytes()))?;
        }
        ok(e.u8(2))?;
        encode_time(&mut e, self.time)?;
        for (key, text) in texts {
            if let Some(text) = text {
                ok(e.u8(key).and_then(|e| e.str(text)))?;
            }
        }
        if !self.meta.is_empty() {
            ok(e.u8(6).and_then(|e| e.array(self.meta.len() as u64)))?;
            for (key, value) in &self.meta {
                ok(e.array(2).and_then(|e| e.str(key)?.str(value)))?;
            }
        }
        Ok(e.into_writer())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let snapshot = decode_with(ObjectType::Snapshot, bytes, |d| {
            let mut tree = None;
            let mut parents = None;
            let mut time = None;
            let mut snapshot = Snapshot {
                tree: ObjectId::from_bytes([0; ID_LEN]),
                parents: Vec::new(),
                time: Time {
                    seconds: 0,
                    nanos: 0,
                },
                author: None,
                host: None,
                message: None,
                meta: Vec::new(),
            };
            map(d, |key, d| {
                match key {
                    0 => tree = Some(id(d)?),
                    1 => parents = Some(array(d, id)?),
                    2 => time = Some(decode_time(d)?),
                    3 => snapshot.author = Some(d.str()?.to_owned()),
                    4 => snapshot.host = Some(d.str()?.to_owned()),
                    5 => snapshot.message = Some(d.str()?.to_owned()),
                    6 => {
                        snapshot.meta = array(d, |d| {
                            pair(d)?;
                            Ok((d.str()?.to_owned(), d.str()?.to_owned()))
                        })?;
                    }
                    _ => return Err(unknown_key()),
                }
                Ok(())
            })?;
            snapshot.tree = tree.ok_or_else(|| missing("tree"))?;
            snapshot.parents = parents.ok_or_else(|| missing("parents"))?;
            snapshot.time = time.ok_or_else(|| missing("time"))?;
            Ok(snapshot)
        })?;
        check_canonical(ObjectType::Snapshot, bytes, &snapshot.encode()?)?;
        Ok(snapshot)
    }

    fn validate(&self) -> Result<()> {
        let invalid = |reason: &str| invalid(ObjectType::Snapshot, reason);
        if self.time.nanos >= NANOS_PER_SEC {
            return Err(invalid("nanoseconds out of range"));
        }
        if !self.meta.windows(2).all(|w| w[0].0 < w[1].0) {
            return Err(invalid("meta is not sorted by unique keys"));
        }
        Ok(())
    }
}

type DecodeResult<T> = std::result::Result<T, DecodeError>;

fn invalid(ty: ObjectType, reason: impl Into<String>) -> Error {
    Error::InvalidObject {
        ty,
        reason: reason.into(),
    }
}

/// Writing into a `Vec` cannot fail.
fn ok<T>(r: std::result::Result<T, minicbor::encode::Error<Infallible>>) -> Result<()> {
    r.map(|_| ()).map_err(|e| Error::Encode(e.to_string()))
}

fn decode_with<T>(
    ty: ObjectType,
    bytes: &[u8],
    f: impl FnOnce(&mut Decoder<'_>) -> DecodeResult<T>,
) -> Result<T> {
    let mut d = Decoder::new(bytes);
    let value = f(&mut d).map_err(|e| invalid(ty, e.to_string()))?;
    if d.position() != bytes.len() {
        return Err(invalid(ty, "trailing bytes"));
    }
    Ok(value)
}

/// A re-encoded object must give back the exact bytes it was decoded from.
fn check_canonical(ty: ObjectType, bytes: &[u8], encoded: &[u8]) -> Result<()> {
    if bytes != encoded {
        return Err(invalid(ty, "not in canonical encoding"));
    }
    Ok(())
}

/// Reads a definite-length map whose integer keys are in ascending order,
/// calling `field` with each key and the decoder positioned at its value.
fn map<'b>(
    d: &mut Decoder<'b>,
    mut field: impl FnMut(u64, &mut Decoder<'b>) -> DecodeResult<()>,
) -> DecodeResult<()> {
    let len = d
        .map()?
        .ok_or_else(|| DecodeError::message("indefinite-length map"))?;
    let mut last = None;
    for _ in 0..len {
        let key = d.u64()?;
        if last.is_some_and(|last| key <= last) {
            return Err(DecodeError::message("map keys are not in ascending order"));
        }
        last = Some(key);
        field(key, d)?;
    }
    Ok(())
}

/// Reads a definite-length array, decoding each element with `item`.
fn array<'b, T>(
    d: &mut Decoder<'b>,
    mut item: impl FnMut(&mut Decoder<'b>) -> DecodeResult<T>,
) -> DecodeResult<Vec<T>> {
    let len = d
        .array()?
        .ok_or_else(|| DecodeError::message("indefinite-length array"))?;
    // Every element takes at least one byte; do not trust `len` for allocation.
    let mut out = Vec::with_capacity(len.min(d.input().len() as u64) as usize);
    for _ in 0..len {
        out.push(item(d)?);
    }
    Ok(out)
}

/// Reads the header of a two-element array.
fn pair(d: &mut Decoder<'_>) -> DecodeResult<()> {
    match d.array()? {
        Some(2) => Ok(()),
        _ => Err(DecodeError::message("expected a pair")),
    }
}

fn id(d: &mut Decoder<'_>) -> DecodeResult<ObjectId> {
    let bytes: [u8; ID_LEN] = d
        .bytes()?
        .try_into()
        .map_err(|_| DecodeError::message("id is not 32 bytes"))?;
    Ok(ObjectId::from_bytes(bytes))
}

fn encode_time(e: &mut Encoder<Vec<u8>>, time: Time) -> Result<()> {
    ok(e.array(2)
        .and_then(|e| e.i64(time.seconds))
        .and_then(|e| e.u32(time.nanos)))
}

fn decode_time(d: &mut Decoder<'_>) -> DecodeResult<Time> {
    pair(d)?;
    Ok(Time {
        seconds: d.i64()?,
        nanos: d.u32()?,
    })
}

fn missing(field: &'static str) -> DecodeError {
    DecodeError::message(format!("missing field {field}"))
}

fn unknown_key() -> DecodeError {
    DecodeError::message("unknown key")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> ObjectId {
        ObjectId::from_bytes([byte; ID_LEN])
    }

    fn time() -> Time {
        Time {
            seconds: -5,
            nanos: 999_999_999,
        }
    }

    fn full_tree() -> Tree {
        Tree {
            entries: vec![
                TreeEntry {
                    name: b"dir".to_vec(),
                    kind: EntryKind::Directory { tree: id(1) },
                    mode: Some(0o755),
                    mtime: Some(time()),
                    uid: Some(1000),
                    gid: Some(1000),
                    xattrs: vec![
                        (b"user.a".to_vec(), b"1".to_vec()),
                        (b"user.b".to_vec(), vec![]),
                    ],
                },
                TreeEntry {
                    name: b"empty".to_vec(),
                    kind: EntryKind::File {
                        size: 0,
                        blob: None,
                    },
                    mode: None,
                    mtime: None,
                    uid: None,
                    gid: None,
                    xattrs: vec![],
                },
                TreeEntry {
                    name: b"file\xff".to_vec(),
                    kind: EntryKind::File {
                        size: 70_000,
                        blob: Some(id(2)),
                    },
                    mode: Some(0o644),
                    mtime: None,
                    uid: None,
                    gid: None,
                    xattrs: vec![],
                },
                TreeEntry {
                    name: b"link".to_vec(),
                    kind: EntryKind::Symlink {
                        target: b"../x".to_vec(),
                    },
                    mode: None,
                    mtime: None,
                    uid: None,
                    gid: None,
                    xattrs: vec![],
                },
            ],
        }
    }

    fn full_snapshot() -> Snapshot {
        Snapshot {
            tree: id(3),
            parents: vec![id(4), id(5)],
            time: time(),
            author: Some("Ada <ada@example.org>".to_owned()),
            host: Some("laptop".to_owned()),
            message: Some("nightly".to_owned()),
            meta: vec![
                ("a".to_owned(), "1".to_owned()),
                ("source-path".to_owned(), "/home".to_owned()),
            ],
        }
    }

    fn minimal_snapshot() -> Snapshot {
        Snapshot {
            tree: id(3),
            parents: vec![],
            time: Time {
                seconds: 1_700_000_000,
                nanos: 0,
            },
            author: None,
            host: None,
            message: None,
            meta: vec![],
        }
    }

    fn is_invalid(r: Result<impl std::fmt::Debug>) -> bool {
        matches!(r, Err(Error::InvalidObject { .. }))
    }

    #[test]
    fn objects_round_trip() {
        let blob = Blob {
            size: 300,
            chunks: vec![(id(1), 100), (id(2), 200)],
        };
        assert_eq!(Blob::decode(&blob.encode().unwrap()).unwrap(), blob);
        let tree = full_tree();
        assert_eq!(Tree::decode(&tree.encode().unwrap()).unwrap(), tree);
        let empty = Tree { entries: vec![] };
        assert_eq!(Tree::decode(&empty.encode().unwrap()).unwrap(), empty);
        for snapshot in [full_snapshot(), minimal_snapshot()] {
            assert_eq!(
                Snapshot::decode(&snapshot.encode().unwrap()).unwrap(),
                snapshot
            );
        }
    }

    #[test]
    fn blob_bytes_match_spec() {
        let blob = Blob {
            size: 500,
            chunks: vec![(id(7), 500)],
        };
        let mut expected = vec![0xa2, 0x00, 0x19, 0x01, 0xf4, 0x01, 0x81, 0x82, 0x58, 0x20];
        expected.extend_from_slice(&[7; 32]);
        expected.extend_from_slice(&[0x19, 0x01, 0xf4]);
        assert_eq!(blob.encode().unwrap(), expected);
    }

    #[test]
    fn snapshot_bytes_match_spec() {
        let mut expected = vec![0xa3, 0x00, 0x58, 0x20];
        expected.extend_from_slice(&[3; 32]);
        // parents: [], time: [1700000000, 0]
        expected.extend_from_slice(&[0x01, 0x80, 0x02, 0x82, 0x1a, 0x65, 0x53, 0xf1, 0x00, 0x00]);
        assert_eq!(minimal_snapshot().encode().unwrap(), expected);
    }

    #[test]
    fn non_canonical_encodings_are_rejected() {
        let canonical = minimal_snapshot().encode().unwrap();
        let tail = &canonical[36..];

        // Integer not in its shortest form: key 0 as 0x18 0x00.
        let mut long_key = vec![0xa3, 0x18, 0x00, 0x58, 0x20];
        long_key.extend_from_slice(&[3; 32]);
        long_key.extend_from_slice(tail);
        // Indefinite-length map.
        let mut indefinite = canonical.clone();
        indefinite[0] = 0xbf;
        indefinite.push(0xff);
        // Trailing bytes.
        let mut trailing = canonical.clone();
        trailing.push(0x00);
        // An absent optional field written as an empty array (meta: []).
        let mut empty_meta = canonical.clone();
        empty_meta[0] = 0xa4;
        empty_meta.extend_from_slice(&[0x06, 0x80]);
        // Unknown key 7.
        let mut unknown = canonical.clone();
        unknown[0] = 0xa4;
        unknown.extend_from_slice(&[0x07, 0x00]);
        // Keys out of order: time (2) before parents (1).
        let mut swapped = canonical[..36].to_vec();
        swapped.extend_from_slice(&tail[2..]);
        swapped.extend_from_slice(&tail[..2]);

        for bytes in [long_key, indefinite, trailing, empty_meta, unknown, swapped] {
            assert!(is_invalid(Snapshot::decode(&bytes)), "{bytes:02x?}");
        }
    }

    #[test]
    fn invalid_objects_are_rejected() {
        let blobs = [
            Blob {
                size: 10,
                chunks: vec![(id(1), 9)],
            },
            Blob {
                size: 0,
                chunks: vec![],
            },
            Blob {
                size: 0,
                chunks: vec![(id(1), 0)],
            },
        ];
        for blob in blobs {
            assert!(is_invalid(blob.encode()), "{blob:?}");
        }

        let mut trees = Vec::new();
        for name in [&b""[..], b".", b"..", b"a/b", b"a\0b"] {
            let mut tree = full_tree();
            tree.entries[0].name = name.to_vec();
            trees.push(tree);
        }
        let mut unsorted = full_tree();
        unsorted.entries.swap(0, 1);
        let mut duplicate = full_tree();
        duplicate.entries[1].name = b"dir".to_vec();
        let mut empty_with_blob = full_tree();
        empty_with_blob.entries[1].kind = EntryKind::File {
            size: 0,
            blob: Some(id(9)),
        };
        let mut data_without_blob = full_tree();
        data_without_blob.entries[2].kind = EntryKind::File {
            size: 1,
            blob: None,
        };
        let mut high_mode = full_tree();
        high_mode.entries[0].mode = Some(0o10000);
        let mut unsorted_xattrs = full_tree();
        unsorted_xattrs.entries[0].xattrs.reverse();
        trees.extend([
            unsorted,
            duplicate,
            empty_with_blob,
            data_without_blob,
            high_mode,
            unsorted_xattrs,
        ]);
        for tree in trees {
            assert!(is_invalid(tree.encode()), "{tree:?}");
        }

        let mut bad_nanos = full_snapshot();
        bad_nanos.time.nanos = 1_000_000_000;
        let mut unsorted_meta = full_snapshot();
        unsorted_meta.meta.reverse();
        let mut duplicate_meta = full_snapshot();
        duplicate_meta.meta[1].0 = "a".to_owned();
        for snapshot in [bad_nanos, unsorted_meta, duplicate_meta] {
            assert!(is_invalid(snapshot.encode()), "{snapshot:?}");
        }
    }

    #[test]
    fn fields_must_match_entry_kind() {
        // A directory entry with a `size` field: {0: "d", 1: 1, 2: 0, 3: id}.
        let mut bytes = vec![
            0xa1, 0x00, 0x81, 0xa4, 0x00, 0x41, b'd', 0x01, 0x01, 0x02, 0x00,
        ];
        bytes.extend_from_slice(&[0x03, 0x58, 0x20]);
        bytes.extend_from_slice(&[1; 32]);
        assert!(is_invalid(Tree::decode(&bytes)));
    }

    #[test]
    fn damaged_bytes_never_panic_or_decode_differently() {
        let bytes = full_tree().encode().unwrap();
        for len in 0..bytes.len() {
            assert!(is_invalid(Tree::decode(&bytes[..len])));
        }
        for i in 0..bytes.len() {
            for bit in 0..8 {
                let mut damaged = bytes.clone();
                damaged[i] ^= 1 << bit;
                // A flip may still form a valid tree, but then it must re-encode to the damaged bytes.
                if let Ok(tree) = Tree::decode(&damaged) {
                    assert_eq!(tree.encode().unwrap(), damaged);
                }
            }
        }
    }
}
