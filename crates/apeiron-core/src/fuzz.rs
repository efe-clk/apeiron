//! Entry points for fuzzing the format (FORMAT.md §1, `fuzz/`): each takes
//! bytes from anywhere and must return, never panic or run out of memory.
//! Checksums and file names are made to fit, so the fuzzer reaches the
//! decoders behind them.

use std::path::Path;

use crate::crypto::Keys;
use crate::id::{IdKey, ObjectType};
use crate::object::{Blob, Snapshot, Tree};
use crate::pack::{Compression, PackEntry};

/// Blobs, trees and snapshots (FORMAT.md §6).
pub fn objects(data: &[u8]) {
    let _ = Blob::decode(data);
    let _ = Tree::decode(data);
    let _ = Snapshot::decode(data);
}

/// An index file named by its hash, as it must be (FORMAT.md §8.2).
pub fn index(data: &[u8]) {
    let name = format!("{}.idx", blake3::hash(data).to_hex());
    let keys = Keys {
        id: IdKey::from_bytes([7; 32]),
        data: None,
    };
    let _ = crate::index::decode_index(Path::new(&name), data, &keys);
}

/// The stored bytes of a pack entry (FORMAT.md §8.1): the first byte picks
/// compression and type, the next four the raw length.
pub fn pack_entry(data: &[u8]) {
    let (Some(&head), Some(len)) = (data.first(), data.get(1..5)) else {
        return;
    };
    let raw_len = u32::from_le_bytes([len[0], len[1], len[2], len[3]]) % (8 << 20);
    let stored = &data[5..];
    let entry = PackEntry {
        id: crate::ObjectId::from_bytes([0; 32]),
        ty: ObjectType::from_byte(head & 0x07).unwrap_or(ObjectType::Chunk),
        compression: if head & 0x80 == 0 {
            Compression::None
        } else {
            Compression::Zstd
        },
        offset: 0,
        stored_len: u32::try_from(stored.len()).unwrap_or(u32::MAX),
        raw_len,
    };
    let keys = Keys {
        id: IdKey::from_bytes([7; 32]),
        data: None,
    };
    let _ = crate::pack::decode_stored(Path::new("x.pack"), &entry, &keys, stored);
}

/// The text files: `config`, ref files, and reflog and oplog lines with
/// their checks (FORMAT.md §3, §9.3–§9.5), and remote addresses.
pub fn text(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let _ = crate::config::Config::from_toml(Path::new("config"), text);
    let _ = crate::refs::decode_ref(Path::new("ref"), text);
    let _ = crate::remote::RemoteUrl::parse(text);
    for line in text.lines() {
        let checked = format!("{line} {}", crate::refs::short_check(line));
        let _ = crate::refs::ReflogEntry::decode(&checked);
        let checked = format!("{line} {}", crate::oplog::short_check(line));
        let _ = crate::oplog::Operation::decode(&checked);
    }
}
