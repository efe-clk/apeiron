//! Git's delta format (GIT.md §7): an object written as copies from a base
//! object and new bytes, so a pack sends a changed file's changes only.

use std::collections::HashMap;

/// Matches shorter than this are written as new bytes.
const BLOCK: usize = 16;
/// The most bytes one copy instruction takes; larger copies are split.
const MAX_COPY: usize = 0xffff;
/// The most new bytes one instruction holds.
const MAX_INSERT: usize = 0x7f;

/// Where each block of `BLOCK` bytes at a multiple of `BLOCK` first occurs
/// in a base object.
pub(crate) struct Index {
    blocks: HashMap<[u8; BLOCK], u32>,
}

impl Index {
    pub(crate) fn new(base: &[u8]) -> Index {
        let mut blocks = HashMap::new();
        for (i, key) in base.as_chunks::<BLOCK>().0.iter().enumerate() {
            blocks.entry(*key).or_insert((i * BLOCK) as u32);
        }
        Index { blocks }
    }
}

/// `target` as a delta from `base`, or `None` when the delta would be
/// longer than `limit` bytes.
pub(crate) fn encode(base: &[u8], index: &Index, target: &[u8], limit: usize) -> Option<Vec<u8>> {
    if base.len() > u32::MAX as usize {
        return None;
    }
    let mut out = Vec::new();
    varint(&mut out, base.len());
    varint(&mut out, target.len());
    let mut pending = 0; // where the new bytes not yet written start
    let mut i = 0;
    while i + BLOCK <= target.len() {
        let found = <[u8; BLOCK]>::try_from(&target[i..i + BLOCK])
            .ok()
            .and_then(|key| index.blocks.get(&key));
        let Some(&at) = found else {
            i += 1;
            continue;
        };
        let (mut from, mut start) = (at as usize, i);
        // Grow the match backwards into the new bytes, then forwards.
        while start > pending && from > 0 && base[from - 1] == target[start - 1] {
            from -= 1;
            start -= 1;
        }
        let mut end = i + BLOCK;
        let mut base_end = at as usize + BLOCK;
        while end < target.len() && base_end < base.len() && base[base_end] == target[end] {
            end += 1;
            base_end += 1;
        }
        insert(&mut out, &target[pending..start]);
        copy(&mut out, from, end - start);
        if out.len() > limit {
            return None;
        }
        pending = end;
        i = end;
    }
    insert(&mut out, &target[pending..]);
    (out.len() <= limit).then_some(out)
}

/// A size as 7 bits per byte, low bits first, the top bit set while more
/// follow.
fn varint(out: &mut Vec<u8>, mut n: usize) {
    loop {
        let byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn insert(out: &mut Vec<u8>, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        let n = bytes.len().min(MAX_INSERT);
        out.push(n as u8);
        out.extend_from_slice(&bytes[..n]);
        bytes = &bytes[n..];
    }
}

/// Copy instructions: `1oooosss` and the non-zero bytes of the offset and
/// the size, low bytes first.
fn copy(out: &mut Vec<u8>, mut offset: usize, mut len: usize) {
    while len > 0 {
        let n = len.min(MAX_COPY);
        let mut op = 0x80u8;
        let mut bytes = Vec::new();
        for (bit, shift) in [0, 8, 16, 24].into_iter().enumerate() {
            let b = (offset >> shift) as u8;
            if b != 0 {
                op |= 1 << bit;
                bytes.push(b);
            }
        }
        for (bit, shift) in [0, 8].into_iter().enumerate() {
            let b = (n >> shift) as u8;
            if b != 0 {
                op |= 0x10 << bit;
                bytes.push(b);
            }
        }
        out.push(op);
        out.extend_from_slice(&bytes);
        offset += n;
        len -= n;
    }
}

#[cfg(test)]
mod tests {
    use super::{Index, encode};

    /// Git's reading of a delta, to check ours against.
    fn apply(base: &[u8], delta: &[u8]) -> Vec<u8> {
        let mut d = delta;
        let mut size = || {
            let (mut n, mut shift) = (0usize, 0);
            loop {
                let b = d[0];
                d = &d[1..];
                n |= usize::from(b & 0x7f) << shift;
                shift += 7;
                if b & 0x80 == 0 {
                    return n;
                }
            }
        };
        assert_eq!(size(), base.len());
        let want = size();
        let mut out = Vec::new();
        while let Some((&op, rest)) = d.split_first() {
            d = rest;
            if op & 0x80 == 0 {
                out.extend_from_slice(&d[..usize::from(op)]);
                d = &d[usize::from(op)..];
                continue;
            }
            let mut field = |bits: std::ops::Range<u32>| {
                let mut n = 0usize;
                for (k, bit) in bits.enumerate() {
                    if op & (1 << bit) != 0 {
                        n |= usize::from(d[0]) << (8 * k);
                        d = &d[1..];
                    }
                }
                n
            };
            let offset = field(0..4);
            let mut len = field(4..7);
            if len == 0 {
                len = 0x10000;
            }
            out.extend_from_slice(&base[offset..offset + len]);
        }
        assert_eq!(out.len(), want);
        out
    }

    fn text(seed: u64, len: usize) -> Vec<u8> {
        let mut x = seed;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                b'a' + (x % 26) as u8
            })
            .collect()
    }

    #[test]
    fn a_changed_object_is_written_as_its_changes() {
        let base = text(1, 200_000);
        let mut target = base.clone();
        target.splice(1000..1010, b"changed!!".iter().copied());
        target.extend_from_slice(b"and a new end");
        target.drain(150_000..150_100);
        let delta = encode(&base, &Index::new(&base), &target, usize::MAX).unwrap();
        assert!(delta.len() < 200, "{}", delta.len());
        assert_eq!(apply(&base, &delta), target);

        // Unrelated bytes make a delta longer than the limit.
        let other = text(2, 1000);
        assert!(encode(&base, &Index::new(&base), &other, 500).is_none());
        // Small and empty objects.
        for target in [&b""[..], b"short", &base[..5]] {
            let delta = encode(&base, &Index::new(&base), target, usize::MAX).unwrap();
            assert_eq!(apply(&base, &delta), target);
        }
    }
}
