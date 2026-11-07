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
        FileReader {
            repo,
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
