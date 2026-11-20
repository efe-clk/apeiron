//! Reading one file of a snapshot piece by piece (API.md §4, SDK.md §2.3).

use std::io;

use apeiron_core::ObjectId;

use crate::error::{Error, Result};
use crate::repo::Repository;

/// The content of one file of a snapshot, made by
/// [`Repository::open_file`]. It takes the repository lock only while it
/// fetches a chunk, so the repository stays free while the reader waits.
#[derive(Debug)]
pub struct FileReader {
    repo: Repository,
    /// Every chunk with the offset it starts at, for [`FileReader::read_at`].
    all: Vec<(u64, ObjectId, u32)>,
    /// The chunk `read_at` read last, by its place in `all`.
    cached: Option<(usize, Vec<u8>)>,
    chunks: std::vec::IntoIter<(ObjectId, u32)>,
    current: Vec<u8>,
    pos: usize,
    size: u64,
    /// Once a chunk fails, reading again gives the same error: going on
    /// would skip that chunk and hand out a file with a hole in it.
    failed: Option<Error>,
}

impl FileReader {
    pub(crate) fn new(repo: Repository, chunks: Vec<(ObjectId, u32)>) -> Self {
        let size = chunks.iter().map(|(_, len)| u64::from(*len)).sum();
        let mut start = 0;
        let all = chunks
            .iter()
            .map(|&(id, len)| {
                let at = start;
                start += u64::from(len);
                (at, id, len)
            })
            .collect();
        FileReader {
            repo,
            all,
            cached: None,
            chunks: chunks.into_iter(),
            current: Vec::new(),
            pos: 0,
            size,
            failed: None,
        }
    }

    /// The size of the whole file in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Fills `buf` with the next bytes and returns how many; 0 means the
    /// end of the file, or an empty `buf`. Each chunk is checked against
    /// its id before any of it is given.
    pub fn read_some(&mut self, buf: &mut [u8]) -> Result<usize> {
        if let Some(e) = &self.failed {
            return Err(e.clone());
        }
        while self.pos == self.current.len() {
            let Some((id, len)) = self.chunks.next() else {
                return Ok(0);
            };
            match self.fetch(&id, len) {
                Ok(chunk) => {
                    self.current = chunk;
                    self.pos = 0;
                }
                Err(e) => {
                    self.failed = Some(e.clone());
                    return Err(e);
                }
            }
        }
        let n = buf.len().min(self.current.len() - self.pos);
        buf[..n].copy_from_slice(&self.current[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }

    /// Fills `buf` with the bytes from `offset` on, independent of
    /// [`FileReader::read_some`], and returns how many; fewer than asked only
    /// at the end of the file. The last chunk read is kept, so reading a file
    /// piece by piece fetches each chunk once.
    pub fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let mut done = 0;
        while done < buf.len() {
            let at = offset + done as u64;
            if at >= self.size {
                break;
            }
            let index = self.all.partition_point(|(start, ..)| *start <= at) - 1;
            if self.cached.as_ref().is_none_or(|(i, _)| *i != index) {
                let (_, id, len) = self.all[index];
                self.cached = Some((index, self.fetch(&id, len)?));
            }
            let (start, ..) = self.all[index];
            let chunk = self.cached.as_ref().map_or(&[][..], |(_, c)| c.as_slice());
            let from = (at - start) as usize;
            let n = (buf.len() - done).min(chunk.len() - from);
            buf[done..done + n].copy_from_slice(&chunk[from..from + n]);
            done += n;
        }
        Ok(done)
    }

    fn fetch(&self, id: &ObjectId, len: u32) -> Result<Vec<u8>> {
        let core = self.repo.view()?;
        Ok(core.read_chunk(id, len)?)
    }
}

impl io::Read for FileReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.read_some(buf).map_err(io::Error::other)
    }
}
