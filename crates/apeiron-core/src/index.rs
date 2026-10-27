//! Index files (FORMAT.md §8.2).

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, IoContext, Result};
use crate::fsutil;
use crate::id::{ID_LEN, ObjectId, parse_hex, to_hex};
use crate::pack::{ENTRY_TAIL_LEN, FileHash, PackEntry};

const MAGIC: &[u8; 8] = b"APRNINDX";
const VERSION: u16 = 1;
const HEADER_LEN: usize = 16;
const ENTRY_LEN: usize = ID_LEN + 4 + ENTRY_TAIL_LEN;

/// Where an object is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Location {
    pub(crate) pack: FileHash,
    pub(crate) entry: PackEntry,
}

/// Writes one index file covering `packs` and returns its path.
pub(crate) fn write_index(
    tmp_dir: &Path,
    index_dir: &Path,
    packs: &[(FileHash, Vec<PackEntry>)],
) -> Result<PathBuf> {
    let pack_count = u32::try_from(packs.len()).map_err(|_| Error::ObjectTooLarge(packs.len()))?;
    let mut entries: Vec<(u32, &PackEntry)> = Vec::new();
    for (pack_no, (_, pack_entries)) in (0u32..).zip(packs) {
        entries.extend(pack_entries.iter().map(|e| (pack_no, e)));
    }
    entries.sort_by_key(|(_, e)| e.id);
    entries.dedup_by_key(|(_, e)| e.id);

    let mut out = Vec::with_capacity(HEADER_LEN + packs.len() * 32 + 8 + entries.len() * ENTRY_LEN);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&pack_count.to_le_bytes());
    for (name, _) in packs {
        out.extend_from_slice(name);
    }
    out.extend_from_slice(&(entries.len() as u64).to_le_bytes());
    for (pack_no, entry) in entries {
        out.extend_from_slice(entry.id.as_bytes());
        out.extend_from_slice(&pack_no.to_le_bytes());
        entry.encode_tail(&mut out);
    }

    let name = to_hex(blake3::hash(&out).as_bytes());
    let dest = index_dir.join(format!("{name}.idx"));
    fsutil::write_durable_new(tmp_dir, &dest, &out)?;
    Ok(dest)
}

/// Parses the hash from an index or pack file name such as `<hex>.idx`.
pub(crate) fn hash_from_file_name(path: &Path, extension: &str) -> Option<FileHash> {
    let stem = path.file_name()?.to_str()?.strip_suffix(extension)?;
    parse_hex(stem)
}

/// Reads an index file, verifying its hash and structure.
pub(crate) fn read_index(path: &Path) -> Result<Vec<Location>> {
    let bytes = fs::read(path).at(path)?;
    let corrupt = |reason: &str| Error::corrupt(path, reason);

    let expected = hash_from_file_name(path, ".idx").ok_or_else(|| corrupt("bad file name"))?;
    if blake3::hash(&bytes).as_bytes() != &expected {
        return Err(corrupt("file hash does not match its name"));
    }
    if bytes.len() < HEADER_LEN + 8 || &bytes[..8] != MAGIC {
        return Err(corrupt("bad index header"));
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version != VERSION || bytes[10..12] != [0, 0] {
        return Err(Error::UnsupportedVersion {
            path: path.to_path_buf(),
            version: u32::from(version),
        });
    }
    let pack_count = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;

    let packs_end = HEADER_LEN + pack_count * 32;
    let count_end = packs_end + 8;
    if bytes.len() < count_end {
        return Err(corrupt("index is too short"));
    }
    let packs: Vec<FileHash> = bytes[HEADER_LEN..packs_end]
        .chunks_exact(32)
        .map(|c| c.try_into().map_err(|_| corrupt("bad pack name")))
        .collect::<Result<_>>()?;
    let entry_count = u64::from_le_bytes(
        bytes[packs_end..count_end]
            .try_into()
            .map_err(|_| corrupt("bad entry count"))?,
    );
    if (bytes.len() - count_end) as u64 != entry_count.saturating_mul(ENTRY_LEN as u64) {
        return Err(corrupt("entry count does not match index size"));
    }

    let mut locations = Vec::with_capacity(entry_count as usize);
    let mut previous: Option<ObjectId> = None;
    for raw in bytes[count_end..].chunks_exact(ENTRY_LEN) {
        let id = ObjectId::from_bytes(raw[..ID_LEN].try_into().map_err(|_| corrupt("bad id"))?);
        if previous.is_some_and(|p| p >= id) {
            return Err(corrupt("index entries are not sorted"));
        }
        previous = Some(id);
        let pack_no = u32::from_le_bytes(
            raw[ID_LEN..ID_LEN + 4]
                .try_into()
                .map_err(|_| corrupt("bad pack number"))?,
        ) as usize;
        let pack = *packs
            .get(pack_no)
            .ok_or_else(|| corrupt("pack number out of range"))?;
        let tail = raw[ID_LEN + 4..]
            .try_into()
            .map_err(|_| corrupt("bad entry"))?;
        let entry = PackEntry::decode_tail(id, tail).ok_or_else(|| corrupt("bad entry"))?;
        locations.push(Location { pack, entry });
    }
    Ok(locations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::ObjectType;
    use crate::pack::Compression;

    fn entry(id_byte: u8, offset: u64) -> PackEntry {
        PackEntry {
            id: ObjectId::from_bytes([id_byte; 32]),
            ty: ObjectType::Chunk,
            compression: Compression::None,
            offset,
            stored_len: 3,
            raw_len: 3,
        }
    }

    #[test]
    fn round_trip_sorted_across_packs() {
        let dir = tempfile::tempdir().unwrap();
        let packs = vec![
            ([1u8; 32], vec![entry(9, 16), entry(2, 19)]),
            ([2u8; 32], vec![entry(5, 16)]),
        ];
        let path = write_index(dir.path(), &dir.path().join("index"), &packs).unwrap();
        let locations = read_index(&path).unwrap();
        let ids: Vec<u8> = locations.iter().map(|l| l.entry.id.as_bytes()[0]).collect();
        assert_eq!(ids, [2, 5, 9]);
        assert_eq!(locations[1].pack, [2u8; 32]);
        assert_eq!(locations[2].entry, entry(9, 16));
    }

    #[test]
    fn detects_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let packs = vec![([1u8; 32], vec![entry(1, 16)])];
        let path = write_index(dir.path(), &dir.path().join("index"), &packs).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&path, bytes).unwrap();
        assert!(matches!(read_index(&path), Err(Error::Corrupt { .. })));
    }
}
