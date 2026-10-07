//! Generic Alamo chunk reader shared by all chunked formats.

/// Error produced while parsing chunk data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

/// One chunk: `u32 id`, `u32 size` (top bit set = container of child chunks).
#[derive(Debug, Clone)]
pub struct Chunk<'a> {
    /// Chunk id.
    pub id: u32,
    /// Payload (children for containers).
    pub data: &'a [u8],
    /// True if the size field had its top bit set, i.e. the payload is a list of child chunks.
    pub is_container: bool,
}

impl<'a> Chunk<'a> {
    /// Iterates the child chunks of a container.
    pub fn children(&self) -> Chunks<'a> {
        Chunks { rest: self.data }
    }
}

/// Iterator over sibling chunks in a byte slice; yields an error once and then stops on malformed data.
pub struct Chunks<'a> {
    rest: &'a [u8],
}

/// Iterates the top-level chunks of data.
pub fn chunks(data: &[u8]) -> Chunks<'_> {
    Chunks { rest: data }
}

impl<'a> Iterator for Chunks<'a> {
    type Item = Result<Chunk<'a>, ParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.is_empty() {
            return None;
        }
        if self.rest.len() < 8 {
            self.rest = &[];
            return Some(Err(ParseError("truncated chunk header".into())));
        }
        let id = u32::from_le_bytes(self.rest[0..4].try_into().unwrap());
        let raw = u32::from_le_bytes(self.rest[4..8].try_into().unwrap());
        let is_container = raw & 0x8000_0000 != 0;
        let size = (raw & 0x7FFF_FFFF) as usize;
        if self.rest.len() - 8 < size {
            self.rest = &[];
            return Some(Err(ParseError(format!(
                "chunk {id:#x} size {size} exceeds data"
            ))));
        }
        let data = &self.rest[8..8 + size];
        self.rest = &self.rest[8 + size..];
        Some(Ok(Chunk {
            id,
            data,
            is_container,
        }))
    }
}

/// Splits a leaf chunk into mini-chunks (`u8 id, u8 size, data`). Returns `None` if the data does not
/// tile exactly. Ids may repeat; order is preserved.
pub fn mini_chunks(mut data: &[u8]) -> Option<Vec<(u8, &[u8])>> {
    let mut out = Vec::new();
    while !data.is_empty() {
        if data.len() < 2 {
            return None;
        }
        let (id, size) = (data[0], data[1] as usize);
        if data.len() < 2 + size {
            return None;
        }
        out.push((id, &data[2..2 + size]));
        data = &data[2 + size..];
    }
    Some(out)
}

/// An owned bag of mini-chunk fields with typed accessors. Used for chunks that are only partially
/// understood so that no data is lost.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Minis(pub Vec<(u8, Vec<u8>)>);

impl Minis {
    /// Parses a leaf chunk; falls back to an empty bag if it does not tile into mini-chunks.
    pub fn parse(data: &[u8]) -> Result<Self, ParseError> {
        let m = mini_chunks(data)
            .ok_or_else(|| ParseError("leaf chunk is not a list of mini-chunks".into()))?;
        Ok(Self(m.into_iter().map(|(i, d)| (i, d.to_vec())).collect()))
    }

    /// Raw bytes of the first field with this id.
    pub fn raw(&self, id: u8) -> Option<&[u8]> {
        self.0
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, d)| d.as_slice())
    }

    /// Field as `u8`.
    pub fn u8(&self, id: u8) -> Option<u8> {
        match self.raw(id)? {
            [b] => Some(*b),
            _ => None,
        }
    }

    /// Field as little-endian `u32`.
    pub fn u32(&self, id: u8) -> Option<u32> {
        Some(u32::from_le_bytes(self.raw(id)?.try_into().ok()?))
    }

    /// Field as `f32`.
    pub fn f32(&self, id: u8) -> Option<f32> {
        self.u32(id).map(f32::from_bits)
    }

    /// Field as three `f32`s (vector or RGB colour).
    pub fn vec3(&self, id: u8) -> Option<[f32; 3]> {
        let d = self.raw(id)?;
        if d.len() != 12 {
            return None;
        }
        Some([0, 4, 8].map(|o| f32::from_le_bytes(d[o..o + 4].try_into().unwrap())))
    }

    /// Field as NUL-terminated 8-bit string (empty string if the field is just a NUL).
    pub fn str(&self, id: u8) -> Option<String> {
        let d = self.raw(id)?;
        let end = d.iter().position(|&b| b == 0).unwrap_or(d.len());
        Some(String::from_utf8_lossy(&d[..end]).into_owned())
    }

    /// Field as NUL-terminated UTF-16LE string.
    pub fn wstr(&self, id: u8) -> Option<String> {
        let d = self.raw(id)?;
        let units: Vec<u16> = d
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .take_while(|&u| u != 0)
            .collect();
        Some(String::from_utf16_lossy(&units))
    }
}
