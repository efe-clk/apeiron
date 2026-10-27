//! Content-defined chunking (FORMAT.md §7).

use std::io::Read;

use fastcdc::v2020::{Normalization, StreamCDC};

use crate::error::{Error, Result};

const MIN_AVG_SIZE: u32 = 64 * 1024;
const MAX_AVG_SIZE: u32 = 4 * 1024 * 1024;

/// FastCDC size parameters. Only the writer uses them; readers do not need them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkerParams {
    min_size: u32,
    avg_size: u32,
    max_size: u32,
}

impl ChunkerParams {
    /// Builds parameters from an average size: a power of two between 64 KiB and 4 MiB.
    /// The minimum is a quarter of it and the maximum four times it.
    pub fn new(avg_size: u32) -> Result<Self> {
        if !avg_size.is_power_of_two() || !(MIN_AVG_SIZE..=MAX_AVG_SIZE).contains(&avg_size) {
            return Err(Error::InvalidChunkerParams(format!(
                "average size {avg_size} must be a power of two between {MIN_AVG_SIZE} and {MAX_AVG_SIZE}"
            )));
        }
        Ok(ChunkerParams {
            min_size: avg_size / 4,
            avg_size,
            max_size: avg_size * 4,
        })
    }

    pub(crate) fn from_parts(min_size: u32, avg_size: u32, max_size: u32) -> Result<Self> {
        let params = ChunkerParams::new(avg_size)?;
        if params.min_size != min_size || params.max_size != max_size {
            return Err(Error::InvalidChunkerParams(format!(
                "min/max must be {}/{} for average {avg_size}",
                params.min_size, params.max_size
            )));
        }
        Ok(params)
    }

    pub fn min_size(&self) -> u32 {
        self.min_size
    }

    pub fn avg_size(&self) -> u32 {
        self.avg_size
    }

    pub fn max_size(&self) -> u32 {
        self.max_size
    }

    /// Splits `reader` into chunks, holding at most `max_size` bytes of input in memory.
    pub(crate) fn chunks<R: Read>(&self, reader: R) -> Chunks<R> {
        Chunks(StreamCDC::with_level(
            reader,
            self.min_size as usize,
            self.avg_size as usize,
            self.max_size as usize,
            Normalization::Level1,
        ))
    }
}

impl Default for ChunkerParams {
    fn default() -> Self {
        ChunkerParams {
            min_size: 256 * 1024,
            avg_size: 1024 * 1024,
            max_size: 4 * 1024 * 1024,
        }
    }
}

pub(crate) struct Chunks<R: Read>(StreamCDC<R>);

impl<R: Read> Iterator for Chunks<R> {
    type Item = Result<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|chunk| {
            chunk
                .map(|c| c.data)
                .map_err(|e| Error::Input(std::io::Error::from(e)))
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Deterministic pseudo-random bytes (xorshift64).
    pub(crate) fn noise(len: usize, seed: u64) -> Vec<u8> {
        let mut state = seed | 1;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 24) as u8
            })
            .collect()
    }

    fn split(params: ChunkerParams, data: &[u8]) -> Vec<Vec<u8>> {
        params.chunks(data).collect::<Result<Vec<_>>>().unwrap()
    }

    #[test]
    fn default_matches_spec() {
        let params = ChunkerParams::default();
        assert_eq!(params, ChunkerParams::new(1024 * 1024).unwrap());
        assert_eq!(params.min_size(), 262_144);
        assert_eq!(params.max_size(), 4_194_304);
    }

    #[test]
    fn rejects_out_of_range_sizes() {
        assert!(ChunkerParams::new(32 * 1024).is_err());
        assert!(ChunkerParams::new(8 * 1024 * 1024).is_err());
        assert!(ChunkerParams::new(100_000).is_err());
        assert!(ChunkerParams::from_parts(1, 1024 * 1024, 4 * 1024 * 1024).is_err());
    }

    #[test]
    fn chunks_reassemble_and_respect_bounds() {
        let params = ChunkerParams::new(64 * 1024).unwrap();
        let data = noise(3 * 1024 * 1024, 1);
        let chunks = split(params, &data);
        assert!(chunks.len() > 10);
        assert_eq!(chunks.concat(), data);
        let (last, rest) = chunks.split_last().unwrap();
        for c in rest {
            assert!(c.len() >= params.min_size() as usize);
            assert!(c.len() <= params.max_size() as usize);
        }
        assert!(!last.is_empty() && last.len() <= params.max_size() as usize);
    }

    #[test]
    fn insertion_only_changes_nearby_chunks() {
        let params = ChunkerParams::new(64 * 1024).unwrap();
        let data = noise(3 * 1024 * 1024, 2);
        let mut edited = b"inserted".to_vec();
        edited.extend_from_slice(&data);
        let before = split(params, &data);
        let after = split(params, &edited);
        let shared = after.iter().filter(|c| before.contains(c)).count();
        assert!(shared + 3 >= before.len(), "{shared} of {}", before.len());
    }

    #[test]
    fn empty_input_has_no_chunks() {
        assert!(split(ChunkerParams::default(), &[]).is_empty());
    }
}
