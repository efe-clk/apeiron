//! Pack files (FORMAT.md §8.1).

use std::fs::{self, File};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::crypto::{BOX_OVERHEAD, Keys, SecretKey};
use crate::error::{Error, IoContext, Result};
use crate::fsutil;
use crate::id::{ID_LEN, ObjectId, ObjectType, to_hex};

const MAGIC: &[u8; 8] = b"APRNPACK";
const END_MAGIC: &[u8; 8] = b"APRNPEND";
const VERSION: u16 = 1;
const HEADER_LEN: u64 = 16;
const FOOTER_LEN: u64 = 12;
/// Header flag: object data and entry table are sealed (FORMAT.md §14.4).
const FLAG_ENCRYPTED: u16 = 1;
pub(crate) const ENTRY_LEN: usize = 52;
/// Bytes of an entry after its id; shared with index entries.
pub(crate) const ENTRY_TAIL_LEN: usize = ENTRY_LEN - ID_LEN;
const ZSTD_LEVEL: i32 = 3;

/// Writers start a new pack once the current one reaches this size.
pub(crate) const TARGET_SIZE: u64 = 32 * 1024 * 1024;

/// Name of a pack or index file: plain BLAKE3 of its bytes.
pub(crate) type FileHash = [u8; 32];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Compression {
    None = 0,
    Zstd = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PackEntry {
    pub(crate) id: ObjectId,
    pub(crate) ty: ObjectType,
    pub(crate) compression: Compression,
    pub(crate) offset: u64,
    pub(crate) stored_len: u32,
    pub(crate) raw_len: u32,
}

impl PackEntry {
    /// Encodes the fields after `id`: type, compression, flags, offset, lengths.
    pub(crate) fn encode_tail(&self, out: &mut Vec<u8>) {
        out.push(self.ty.as_byte());
        out.push(self.compression as u8);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&self.offset.to_le_bytes());
        out.extend_from_slice(&self.stored_len.to_le_bytes());
        out.extend_from_slice(&self.raw_len.to_le_bytes());
    }

    pub(crate) fn decode_tail(id: ObjectId, tail: &[u8; ENTRY_TAIL_LEN]) -> Option<Self> {
        let ty = ObjectType::from_byte(tail[0])?;
        let compression = match tail[1] {
            0 => Compression::None,
            1 => Compression::Zstd,
            _ => return None,
        };
        if tail[2..4] != [0, 0] {
            return None;
        }
        Some(PackEntry {
            id,
            ty,
            compression,
            offset: u64::from_le_bytes(tail[4..12].try_into().ok()?),
            stored_len: u32::from_le_bytes(tail[12..16].try_into().ok()?),
            raw_len: u32::from_le_bytes(tail[16..20].try_into().ok()?),
        })
    }
}

pub(crate) fn pack_path(packs_dir: &Path, name: &FileHash) -> PathBuf {
    let hex = to_hex(name);
    packs_dir.join(&hex[..2]).join(format!("{hex}.pack"))
}

/// Additional data that binds a sealed object to its id and type.
fn object_aad(id: &ObjectId, ty: ObjectType) -> [u8; ID_LEN + 1] {
    let mut aad = [0u8; ID_LEN + 1];
    aad[..ID_LEN].copy_from_slice(id.as_bytes());
    aad[ID_LEN] = ty.as_byte();
    aad
}

/// Streams objects into a temporary pack file.
pub(crate) struct PackWriter {
    file: BufWriter<File>,
    tmp_path: PathBuf,
    data_key: Option<SecretKey>,
    header: [u8; HEADER_LEN as usize],
    hasher: blake3::Hasher,
    entries: Vec<PackEntry>,
    len: u64,
    persisted: bool,
}

impl PackWriter {
    /// Starts a pack; it is encrypted when `keys` has a data key.
    pub(crate) fn new(tmp_dir: &Path, keys: &Keys) -> Result<Self> {
        let (file, tmp_path) = fsutil::create_tmp(tmp_dir)?;
        let flags = if keys.data.is_some() {
            FLAG_ENCRYPTED
        } else {
            0
        };
        let mut header = [0u8; HEADER_LEN as usize];
        header[..8].copy_from_slice(MAGIC);
        header[8..10].copy_from_slice(&VERSION.to_le_bytes());
        header[10..12].copy_from_slice(&flags.to_le_bytes());
        let mut writer = PackWriter {
            file: BufWriter::new(file),
            tmp_path,
            data_key: keys.data.clone(),
            header,
            hasher: blake3::Hasher::new(),
            entries: Vec::new(),
            len: 0,
            persisted: false,
        };
        writer.write(&header)?;
        Ok(writer)
    }

    pub(crate) fn len(&self) -> u64 {
        self.len
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        fsutil::checkpoint()
            .and_then(|()| self.file.write_all(bytes))
            .at(&self.tmp_path)?;
        self.hasher.update(bytes);
        self.len += bytes.len() as u64;
        Ok(())
    }

    /// Appends an object. `raw` is the uncompressed content that `id` was computed over.
    pub(crate) fn add(&mut self, id: ObjectId, ty: ObjectType, raw: &[u8]) -> Result<()> {
        let raw_len = u32::try_from(raw.len()).map_err(|_| Error::ObjectTooLarge(raw.len()))?;
        let compressed = zstd::bulk::compress(raw, ZSTD_LEVEL).at(&self.tmp_path)?;
        let (compression, stored) = if compressed.len() < raw.len() {
            (Compression::Zstd, compressed.as_slice())
        } else {
            (Compression::None, raw)
        };
        let sealed;
        let stored = match &self.data_key {
            Some(key) => {
                sealed = key.seal(&object_aad(&id, ty), stored)?;
                sealed.as_slice()
            }
            None => stored,
        };
        let entry = PackEntry {
            id,
            ty,
            compression,
            offset: self.len,
            stored_len: u32::try_from(stored.len())
                .map_err(|_| Error::ObjectTooLarge(stored.len()))?,
            raw_len,
        };
        self.write(stored)?;
        self.entries.push(entry);
        Ok(())
    }

    /// Appends an object exactly as another pack stored it.
    pub(crate) fn add_stored(&mut self, from: &PackEntry, stored: &[u8]) -> Result<()> {
        let entry = PackEntry {
            offset: self.len,
            ..*from
        };
        self.write(stored)?;
        self.entries.push(entry);
        Ok(())
    }

    /// Writes the entry table, syncs and moves the pack into `packs_dir`.
    pub(crate) fn finish(mut self, packs_dir: &Path) -> Result<(FileHash, Vec<PackEntry>)> {
        self.write_table()?;
        fsutil::checkpoint()
            .and_then(|()| self.file.get_ref().sync_all())
            .at(&self.tmp_path)?;

        let name = *self.hasher.finalize().as_bytes();
        fsutil::persist_new(&self.tmp_path, &pack_path(packs_dir, &name))?;
        self.persisted = true;
        Ok((name, std::mem::take(&mut self.entries)))
    }

    /// Writes the entry table and returns the pack's name, entries and
    /// bytes; the temporary file is removed.
    pub(crate) fn finish_to_bytes(mut self) -> Result<(FileHash, Vec<PackEntry>, Vec<u8>)> {
        self.write_table()?;
        let bytes = fs::read(&self.tmp_path).at(&self.tmp_path)?;
        let name = *self.hasher.finalize().as_bytes();
        Ok((name, std::mem::take(&mut self.entries), bytes))
    }

    fn write_table(&mut self) -> Result<()> {
        let mut table = Vec::with_capacity(self.entries.len() * ENTRY_LEN + FOOTER_LEN as usize);
        for entry in &self.entries {
            table.extend_from_slice(entry.id.as_bytes());
            entry.encode_tail(&mut table);
        }
        if let Some(key) = &self.data_key {
            table = key.seal(&self.header, &table)?;
        }
        let count = u32::try_from(self.entries.len())
            .map_err(|_| Error::ObjectTooLarge(self.entries.len()))?;
        table.extend_from_slice(&count.to_le_bytes());
        table.extend_from_slice(END_MAGIC);
        self.write(&table)?;
        self.file.flush().at(&self.tmp_path)
    }
}

impl Drop for PackWriter {
    fn drop(&mut self) {
        if !self.persisted {
            let _ = fs::remove_file(&self.tmp_path);
        }
    }
}

/// Reads and validates the entry table of a pack. An encrypted repository
/// accepts only encrypted packs and a plain one only plain packs.
pub(crate) fn read_table(path: &Path, keys: &Keys) -> Result<Vec<PackEntry>> {
    let mut file = File::open(path).at(path)?;
    let file_len = file.metadata().at(path)?.len();
    if file_len < HEADER_LEN + FOOTER_LEN {
        return Err(Error::corrupt(path, "pack is too short"));
    }

    let mut header = [0u8; HEADER_LEN as usize];
    file.read_exact(&mut header).at(path)?;
    if &header[..8] != MAGIC {
        return Err(Error::corrupt(path, "bad pack magic"));
    }
    let version = u16::from_le_bytes([header[8], header[9]]);
    let flags = u16::from_le_bytes([header[10], header[11]]);
    if version != VERSION || flags & !FLAG_ENCRYPTED != 0 || header[12..16] != [0; 4] {
        return Err(Error::UnsupportedVersion {
            path: path.to_path_buf(),
            version: u32::from(version),
        });
    }
    if (flags == FLAG_ENCRYPTED) != keys.data.is_some() {
        return Err(Error::corrupt(
            path,
            "pack encryption does not match the repository",
        ));
    }

    let mut footer = [0u8; FOOTER_LEN as usize];
    file.seek(SeekFrom::Start(file_len - FOOTER_LEN)).at(path)?;
    file.read_exact(&mut footer).at(path)?;
    if &footer[4..] != END_MAGIC {
        return Err(Error::corrupt(path, "bad pack end magic"));
    }
    let count = u64::from(u32::from_le_bytes([
        footer[0], footer[1], footer[2], footer[3],
    ]));
    let mut table_len = count * ENTRY_LEN as u64;
    if keys.data.is_some() {
        table_len += BOX_OVERHEAD as u64;
    }
    let Some(table_start) = (file_len - FOOTER_LEN)
        .checked_sub(table_len)
        .filter(|&s| s >= HEADER_LEN)
    else {
        return Err(Error::corrupt(path, "entry count exceeds pack size"));
    };

    let mut table = vec![0u8; table_len as usize];
    file.seek(SeekFrom::Start(table_start)).at(path)?;
    file.read_exact(&mut table).at(path)?;
    if let Some(key) = &keys.data {
        table = key
            .open(&header, &table)
            .ok_or_else(|| Error::corrupt(path, "entry table fails authentication"))?;
    }

    let mut entries = Vec::with_capacity(count as usize);
    let mut expected_offset = HEADER_LEN;
    for raw in table.chunks_exact(ENTRY_LEN) {
        let (id, tail) = raw.split_at(ID_LEN);
        let id = ObjectId::from_bytes(id.try_into().map_err(|_| bad_entry(path))?);
        let entry = PackEntry::decode_tail(id, tail.try_into().map_err(|_| bad_entry(path))?)
            .ok_or_else(|| bad_entry(path))?;
        if entry.offset != expected_offset {
            return Err(Error::corrupt(path, "pack entries are not contiguous"));
        }
        expected_offset += u64::from(entry.stored_len);
        entries.push(entry);
    }
    if expected_offset != table_start {
        return Err(Error::corrupt(
            path,
            "pack entries do not fill the data area",
        ));
    }
    Ok(entries)
}

fn bad_entry(path: &Path) -> Error {
    Error::corrupt(path, "invalid pack entry")
}

/// Reads one object, decrypts and decompresses it and checks its id.
pub(crate) fn read_object(path: &Path, entry: &PackEntry, keys: &Keys) -> Result<Vec<u8>> {
    let stored = read_stored_unchecked(path, entry)?;
    decode_stored(path, entry, keys, &stored)
}

/// Reads the stored bytes of one object after checking that they decode to it.
pub(crate) fn read_stored(path: &Path, entry: &PackEntry, keys: &Keys) -> Result<Vec<u8>> {
    let stored = read_stored_unchecked(path, entry)?;
    decode_stored(path, entry, keys, &stored)?;
    Ok(stored)
}

fn read_stored_unchecked(path: &Path, entry: &PackEntry) -> Result<Vec<u8>> {
    let mut file = File::open(path).at(path)?;
    file.seek(SeekFrom::Start(entry.offset)).at(path)?;
    let mut stored = vec![0u8; entry.stored_len as usize];
    file.read_exact(&mut stored).at(path)?;
    Ok(stored)
}

/// Decrypts and decompresses the stored bytes of an object read from the
/// pack at `path` and checks its id.
pub(crate) fn decode_stored(
    path: &Path,
    entry: &PackEntry,
    keys: &Keys,
    stored: &[u8],
) -> Result<Vec<u8>> {
    let opened;
    let stored = match &keys.data {
        Some(key) => {
            opened = key
                .open(&object_aad(&entry.id, entry.ty), stored)
                .ok_or_else(|| {
                    Error::corrupt(path, format!("object {}: fails authentication", entry.id))
                })?;
            opened.as_slice()
        }
        None => stored,
    };
    let raw = match entry.compression {
        Compression::None => stored.to_vec(),
        Compression::Zstd => zstd::bulk::decompress(stored, entry.raw_len as usize)
            .map_err(|e| Error::corrupt(path, format!("object {}: {e}", entry.id)))?,
    };
    if raw.len() != entry.raw_len as usize {
        return Err(Error::corrupt(
            path,
            format!("object {}: wrong length", entry.id),
        ));
    }
    if keys.id.object_id(entry.ty, &raw) != entry.id {
        return Err(Error::corrupt(
            path,
            format!("object {}: hash mismatch", entry.id),
        ));
    }
    Ok(raw)
}

/// Checks that the file's BLAKE3 hash equals its name.
pub(crate) fn verify_file_hash(path: &Path, expected: &FileHash) -> Result<()> {
    let mut hasher = blake3::Hasher::new();
    let file = File::open(path).at(path)?;
    hasher.update_reader(file).at(path)?;
    if hasher.finalize().as_bytes() != expected {
        return Err(Error::corrupt(path, "file hash does not match its name"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunker::tests::noise;
    use crate::id::IdKey;

    fn plain_keys() -> Keys {
        Keys {
            id: IdKey::from_bytes([1; 32]),
            data: None,
        }
    }

    fn encrypted_keys() -> Keys {
        Keys {
            id: IdKey::from_bytes([1; 32]),
            data: Some(SecretKey::from_bytes([2; 32])),
        }
    }

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join("tmp");
        let packs = dir.path().join("packs");
        fs::create_dir(&tmp).unwrap();
        (dir, tmp, packs)
    }

    fn write_pack(
        tmp: &Path,
        packs: &Path,
        key: &Keys,
        objects: &[&[u8]],
    ) -> (FileHash, Vec<PackEntry>) {
        let mut writer = PackWriter::new(tmp, key).unwrap();
        for data in objects {
            writer
                .add(
                    key.id.object_id(ObjectType::Chunk, data),
                    ObjectType::Chunk,
                    data,
                )
                .unwrap();
        }
        writer.finish(packs).unwrap()
    }

    #[test]
    fn round_trip_with_and_without_compression() {
        let (_dir, tmp, packs) = setup();
        let key = plain_keys();
        let compressible = vec![b'a'; 10_000];
        let random = noise(10_000, 3);
        let (name, entries) = write_pack(&tmp, &packs, &key, &[&compressible, &random, b""]);
        assert_eq!(entries[0].compression, Compression::Zstd);
        assert_eq!(entries[1].compression, Compression::None);

        let path = pack_path(&packs, &name);
        verify_file_hash(&path, &name).unwrap();
        assert_eq!(read_table(&path, &key).unwrap(), entries);
        assert_eq!(read_object(&path, &entries[0], &key).unwrap(), compressible);
        assert_eq!(read_object(&path, &entries[1], &key).unwrap(), random);
        assert_eq!(read_object(&path, &entries[2], &key).unwrap(), b"");
        assert_eq!(fs::read_dir(&tmp).unwrap().count(), 0);
    }

    #[test]
    fn detects_flipped_byte() {
        let (_dir, tmp, packs) = setup();
        let key = plain_keys();
        let random = noise(1000, 4);
        let (name, entries) = write_pack(&tmp, &packs, &key, &[&random]);
        let path = pack_path(&packs, &name);
        let mut bytes = fs::read(&path).unwrap();
        bytes[HEADER_LEN as usize + 10] ^= 1;
        fs::write(&path, bytes).unwrap();

        assert!(matches!(
            verify_file_hash(&path, &name),
            Err(Error::Corrupt { .. })
        ));
        assert!(matches!(
            read_object(&path, &entries[0], &key),
            Err(Error::Corrupt { .. })
        ));
    }

    #[test]
    fn rejects_truncated_pack() {
        let (_dir, tmp, packs) = setup();
        let key = plain_keys();
        let (name, _) = write_pack(&tmp, &packs, &key, &[b"abc", b"def"]);
        let path = pack_path(&packs, &name);
        let bytes = fs::read(&path).unwrap();
        fs::write(&path, &bytes[..bytes.len() - 5]).unwrap();
        assert!(matches!(
            read_table(&path, &key),
            Err(Error::Corrupt { .. })
        ));
    }

    #[test]
    fn dropped_writer_removes_tmp_file() {
        let (_dir, tmp, _packs) = setup();
        let mut writer = PackWriter::new(&tmp, &plain_keys()).unwrap();
        writer
            .add(ObjectId::from_bytes([0; 32]), ObjectType::Chunk, b"x")
            .unwrap();
        drop(writer);
        assert_eq!(fs::read_dir(&tmp).unwrap().count(), 0);
    }

    #[test]
    fn encrypted_pack_round_trips_and_hides_content() {
        let (_dir, tmp, packs) = setup();
        let key = encrypted_keys();
        let text = b"secret file content ".repeat(100);
        let (name, entries) = write_pack(&tmp, &packs, &key, &[&text, b""]);
        let path = pack_path(&packs, &name);
        let bytes = fs::read(&path).unwrap();
        assert_eq!(&bytes[10..12], &[1, 0]);
        assert!(!bytes.windows(6).any(|w| w == b"secret"));
        assert!(!bytes.windows(32).any(|w| w == entries[0].id.as_bytes()));
        assert_eq!(entries[1].stored_len as usize, BOX_OVERHEAD);

        verify_file_hash(&path, &name).unwrap();
        assert_eq!(read_table(&path, &key).unwrap(), entries);
        assert_eq!(read_object(&path, &entries[0], &key).unwrap(), text);
        assert_eq!(read_object(&path, &entries[1], &key).unwrap(), b"");

        // The wrong data key, or none, never reads the pack.
        let other = Keys {
            data: Some(SecretKey::from_bytes([3; 32])),
            ..encrypted_keys()
        };
        assert!(matches!(
            read_table(&path, &other),
            Err(Error::Corrupt { .. })
        ));
        assert!(matches!(
            read_table(&path, &plain_keys()),
            Err(Error::Corrupt { .. })
        ));
        assert!(matches!(
            read_object(&path, &entries[0], &other),
            Err(Error::Corrupt { .. })
        ));
    }

    #[test]
    fn plain_pack_is_refused_in_an_encrypted_repository() {
        let (_dir, tmp, packs) = setup();
        let (name, _) = write_pack(&tmp, &packs, &plain_keys(), &[b"abc"]);
        let path = pack_path(&packs, &name);
        assert!(matches!(
            read_table(&path, &encrypted_keys()),
            Err(Error::Corrupt { .. })
        ));
    }

    #[test]
    fn swapped_encrypted_objects_fail_authentication() {
        let (_dir, tmp, packs) = setup();
        let key = encrypted_keys();
        let (name, entries) = write_pack(&tmp, &packs, &key, &[b"first", b"other"]);
        let path = pack_path(&packs, &name);
        // Each entry pointed at the other object's sealed bytes.
        let mut swapped = entries[0];
        swapped.offset = entries[1].offset;
        assert!(matches!(
            read_object(&path, &swapped, &key),
            Err(Error::Corrupt { .. })
        ));
    }
}
