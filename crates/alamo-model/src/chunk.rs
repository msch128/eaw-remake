//! Generic chunk reader shared by `.alo` and `.ala` files.
//!
//! A chunk is `u32 id`, `u32 size` where bit 31 of `size` marks a container whose payload is a
//! sequence of child chunks; the low 31 bits are the payload length in bytes.

use crate::error::{Error, Result};

/// A borrowed chunk: id, container flag and payload.
#[derive(Debug, Clone, Copy)]
pub struct Chunk<'a> {
    /// Chunk id.
    pub id: u32,
    /// True if the payload is a list of child chunks.
    pub container: bool,
    /// Payload bytes (excluding the 8 byte header).
    pub data: &'a [u8],
}

impl<'a> Chunk<'a> {
    /// Iterates the child chunks of a container chunk (also works on any byte payload).
    pub fn children(&self) -> Chunks<'a> {
        Chunks { data: self.data }
    }

    /// Finds the first direct child with the given id.
    pub fn child(&self, id: u32) -> Option<Chunk<'a>> {
        self.children().filter_map(|c| c.ok()).find(|c| c.id == id)
    }

    /// A reader over the payload.
    pub fn reader(&self) -> Reader<'a> {
        Reader::new(self.data)
    }
}

/// Iterator over sibling chunks in a byte slice. Yields an error once if the data is truncated.
#[derive(Debug, Clone)]
pub struct Chunks<'a> {
    data: &'a [u8],
}

impl<'a> Chunks<'a> {
    /// Creates an iterator over the top-level chunks of `data`.
    pub fn new(data: &'a [u8]) -> Self {
        Chunks { data }
    }
}

impl<'a> Iterator for Chunks<'a> {
    type Item = Result<Chunk<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.data.is_empty() {
            return None;
        }
        if self.data.len() < 8 {
            self.data = &[];
            return Some(Err(Error::Truncated("chunk header")));
        }
        let id = u32::from_le_bytes(self.data[0..4].try_into().unwrap());
        let raw = u32::from_le_bytes(self.data[4..8].try_into().unwrap());
        let container = raw & 0x8000_0000 != 0;
        let size = (raw & 0x7fff_ffff) as usize;
        let rest = &self.data[8..];
        if size > rest.len() {
            self.data = &[];
            return Some(Err(Error::Truncated("chunk payload")));
        }
        let (data, tail) = rest.split_at(size);
        self.data = tail;
        Some(Ok(Chunk {
            id,
            container,
            data,
        }))
    }
}

/// Little-endian cursor over a byte slice; every read is bounds-checked.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Creates a reader at offset 0.
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    /// Bytes left.
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    /// Current offset.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Takes `n` bytes.
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(Error::Truncated("payload read"));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    /// Skips `n` bytes.
    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }

    /// Reads a `u8`.
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    /// Reads a `u16`.
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }

    /// Reads a `u32`.
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    /// Reads an `i32`.
    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }

    /// Reads an `f32`.
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }

    /// Reads `N` floats.
    pub fn floats<const N: usize>(&mut self) -> Result<[f32; N]> {
        let mut out = [0.0; N];
        for v in &mut out {
            *v = self.f32()?;
        }
        Ok(out)
    }
}

/// Decodes a NUL-terminated (or unterminated) Latin-1/ASCII string from `bytes`.
pub fn cstr(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    bytes[..end].iter().map(|&b| b as char).collect()
}
