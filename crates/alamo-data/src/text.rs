//! Reader for Alamo `.dat` string tables (`MasterTextFile_ENGLISH.DAT` and friends).
//!
//! Layout (all integers little endian `u32`):
//! `count`, then `count` entries of `(crc, value_len, key_len)`, then all values as UTF-16LE
//! (`value_len` code units each), then all keys as ASCII (`key_len` bytes each).

use std::collections::HashMap;
use std::fmt;
use std::io;

use alamo_fs::FileSource;

/// Error produced when a `.dat` file is truncated or inconsistent.
#[derive(Debug)]
pub enum TextError {
    /// The file could not be read.
    Io(io::Error),
    /// The file is shorter than its header claims.
    Truncated,
}

impl fmt::Display for TextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TextError::Io(e) => write!(f, "{e}"),
            TextError::Truncated => write!(f, "string table is truncated or corrupt"),
        }
    }
}

impl std::error::Error for TextError {}

impl From<io::Error> for TextError {
    fn from(e: io::Error) -> Self {
        TextError::Io(e)
    }
}

/// A loaded string table. Keys are matched case-insensitively.
#[derive(Debug, Clone, Default)]
pub struct TextTable {
    map: HashMap<String, String>,
}

impl TextTable {
    /// Loads and parses the `.dat` file at `path` from `src`.
    pub fn load(src: &dyn FileSource, path: &str) -> Result<Self, TextError> {
        Self::parse(&src.read(path)?)
    }

    /// Parses the raw bytes of a `.dat` file.
    pub fn parse(data: &[u8]) -> Result<Self, TextError> {
        let u32_at = |off: usize| -> Result<usize, TextError> {
            data.get(off..off + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
                .ok_or(TextError::Truncated)
        };
        let count = u32_at(0)?;
        let header_end = count.checked_mul(12).and_then(|v| v.checked_add(4)).ok_or(TextError::Truncated)?;
        if header_end > data.len() {
            return Err(TextError::Truncated);
        }
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let base = 4 + i * 12;
            entries.push((u32_at(base + 4)?, u32_at(base + 8)?));
        }
        let mut vpos = header_end;
        let values_len: usize = entries.iter().map(|e| e.0 * 2).sum();
        let mut kpos = vpos.checked_add(values_len).ok_or(TextError::Truncated)?;
        let mut map = HashMap::with_capacity(count);
        for (vlen, klen) in entries {
            let vbytes = data.get(vpos..vpos + vlen * 2).ok_or(TextError::Truncated)?;
            let kbytes = data.get(kpos..kpos + klen).ok_or(TextError::Truncated)?;
            vpos += vlen * 2;
            kpos += klen;
            let units: Vec<u16> = vbytes.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect();
            let value = String::from_utf16_lossy(&units);
            let key: String = kbytes.iter().map(|&b| b as char).collect();
            map.entry(key.to_ascii_uppercase()).or_insert(value);
        }
        Ok(Self { map })
    }

    /// Looks up a string by key (case-insensitive, surrounding whitespace ignored).
    pub fn get(&self, key: &str) -> Option<&str> {
        let key = key.trim();
        self.map
            .get(key)
            .or_else(|| self.map.get(&key.to_ascii_uppercase()))
            .map(String::as_str)
    }

    /// Looks up `key`, falling back to the key itself when it is missing (useful for display).
    pub fn get_or_key<'a>(&'a self, key: &'a str) -> &'a str {
        self.get(key).unwrap_or(key.trim())
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True if the table has no entries.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Iterates over `(UPPERCASE key, value)` pairs in arbitrary order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.map.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend((entries.len() as u32).to_le_bytes());
        for (k, v) in entries {
            out.extend(0u32.to_le_bytes());
            out.extend((v.encode_utf16().count() as u32).to_le_bytes());
            out.extend((k.len() as u32).to_le_bytes());
        }
        for (_, v) in entries {
            out.extend(v.encode_utf16().flat_map(u16::to_le_bytes));
        }
        for (k, _) in entries {
            out.extend(k.bytes());
        }
        out
    }

    #[test]
    fn parses_table() {
        let t = TextTable::parse(&build(&[("TEXT_A", "Alpha"), ("Text_B", "B\u{e9}ta \u{1F600}"), ("EMPTY", "")])).unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!(t.get("text_a"), Some("Alpha"));
        assert_eq!(t.get(" TEXT_B "), Some("B\u{e9}ta \u{1F600}"));
        assert_eq!(t.get("EMPTY"), Some(""));
        assert_eq!(t.get("nope"), None);
        assert_eq!(t.get_or_key("nope"), "nope");
    }

    #[test]
    fn rejects_truncated() {
        let mut d = build(&[("K", "value")]);
        d.truncate(d.len() - 1);
        assert!(matches!(TextTable::parse(&d), Err(TextError::Truncated)));
        assert!(matches!(TextTable::parse(&[1, 0]), Err(TextError::Truncated)));
        assert!(TextTable::parse(&[0, 0, 0, 0]).unwrap().is_empty());
    }
}
