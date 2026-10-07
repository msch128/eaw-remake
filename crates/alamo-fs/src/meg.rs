//! Reader for Petroglyph `.meg` archives.
//!
//! Supported layouts:
//! * **v1** (Empire at War / Forces of Corruption): `u32 num_filenames, u32 num_files`, a filename table
//!   of `(u16 len, bytes)`, then `num_files` records of five `u32`:
//!   `crc32, index, size, start, filename_index`.
//! * **v2 / v3, unencrypted** (later Petroglyph games; untested against real files):
//!   `u32 0xFFFFFFFF, u32 0x3F7D70A4, u32 header_size, u32 num_filenames, u32 num_files, u32 names_size`,
//!   the same filename table, then 20-byte records `u16 flags, u32 crc32, u32 index, u32 size, u32 start,
//!   u16 filename_index`. Encrypted v3 archives are rejected.
//!
//! Entries are read lazily with positional reads, so a [`MegArchive`] can be shared between threads.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::{FileSource, normalize};

const V2_FLAGS: u32 = 0xFFFF_FFFF;
const V3_ENCRYPTED_FLAGS: u32 = 0x8FFF_FFFF;
const V2_MAGIC: u32 = 0x3F7D_70A4;

/// One file inside a [`MegArchive`].
#[derive(Debug, Clone)]
pub struct MegEntry {
    /// Normalized path (see [`normalize`]).
    pub name: String,
    /// Absolute byte offset of the data inside the archive.
    pub offset: u64,
    /// Size of the data in bytes.
    pub size: u64,
}

/// An opened `.meg` archive. Only the table of contents is held in memory.
#[derive(Debug)]
pub struct MegArchive {
    path: PathBuf,
    file: File,
    entries: Vec<MegEntry>,
    /// normalized name -> index into `entries`
    index: HashMap<String, usize>,
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

fn u32_at(buf: &[u8], pos: usize) -> u32 {
    u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap())
}

fn u16_at(buf: &[u8], pos: usize) -> u16 {
    u16::from_le_bytes(buf[pos..pos + 2].try_into().unwrap())
}

#[cfg(windows)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buf, offset)
}

#[cfg(unix)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buf, offset)
}

fn read_exact_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> io::Result<()> {
    while !buf.is_empty() {
        match read_at(file, buf, offset) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => {
                buf = &mut buf[n..];
                offset += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

impl MegArchive {
    /// Opens an archive and reads its table of contents.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        Self::open_inner(&path)
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))
    }

    fn open_inner(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let file_len = file.metadata()?.len();
        let mut reader = io::BufReader::with_capacity(1 << 16, &file);

        let mut word = [0u8; 4];
        reader.read_exact(&mut word)?;
        let first = u32::from_le_bytes(word);
        let (num_names, num_files, record_size) = match first {
            V3_ENCRYPTED_FLAGS => return Err(invalid("encrypted MEG v3 is not supported")),
            V2_FLAGS => {
                let mut rest = [0u8; 20];
                reader.read_exact(&mut rest)?;
                if u32_at(&rest, 0) != V2_MAGIC {
                    return Err(invalid("bad MEG v2/v3 magic"));
                }
                (u32_at(&rest, 8), u32_at(&rest, 12), 20usize)
            }
            n => {
                reader.read_exact(&mut word)?;
                (n, u32::from_le_bytes(word), 20usize)
            }
        };
        let is_v1 = first != V2_FLAGS;
        if u64::from(num_names) > file_len || u64::from(num_files) > file_len {
            return Err(invalid("implausible MEG entry counts"));
        }

        let mut names = Vec::with_capacity(num_names as usize);
        for _ in 0..num_names {
            let mut len = [0u8; 2];
            reader.read_exact(&mut len)?;
            let mut bytes = vec![0u8; u16::from_le_bytes(len) as usize];
            reader.read_exact(&mut bytes)?;
            // Names are ASCII in practice; lossy conversion keeps odd bytes harmless.
            names.push(normalize(&String::from_utf8_lossy(&bytes)));
        }

        let mut table = vec![0u8; num_files as usize * record_size];
        reader.read_exact(&mut table)?;

        let mut entries = Vec::with_capacity(num_files as usize);
        let mut index = HashMap::with_capacity(num_files as usize);
        for rec in table.chunks_exact(record_size) {
            let (size, start, name_idx) = if is_v1 {
                (u32_at(rec, 8), u32_at(rec, 12), u32_at(rec, 16) as usize)
            } else {
                (u32_at(rec, 10), u32_at(rec, 14), u16_at(rec, 18) as usize)
            };
            let name = names
                .get(name_idx)
                .ok_or_else(|| invalid("filename index out of range"))?;
            let (offset, size) = (u64::from(start), u64::from(size));
            if offset + size > file_len {
                return Err(invalid(format!("entry {name} lies beyond end of archive")));
            }
            index.insert(name.clone(), entries.len());
            entries.push(MegEntry {
                name: name.clone(),
                offset,
                size,
            });
        }

        drop(reader);
        Ok(Self {
            path: path.to_path_buf(),
            file,
            entries,
            index,
        })
    }

    /// Path of the archive on disk.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// All entries, in file-table order.
    pub fn entries(&self) -> &[MegEntry] {
        &self.entries
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True if the archive has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Finds an entry by game path.
    pub fn find(&self, path: &str) -> Option<usize> {
        self.index.get(&normalize(path)).copied()
    }

    /// Reads the data of entry `idx` (an index into [`entries`](Self::entries)).
    pub fn read_entry(&self, idx: usize) -> io::Result<Vec<u8>> {
        let e = self
            .entries
            .get(idx)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "entry index"))?;
        let mut buf = vec![0u8; e.size as usize];
        read_exact_at(&self.file, &mut buf, e.offset)?;
        Ok(buf)
    }
}

impl FileSource for MegArchive {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        match self.find(path) {
            Some(i) => self.read_entry(i),
            None => Err(io::Error::new(io::ErrorKind::NotFound, path.to_string())),
        }
    }

    fn exists(&self, path: &str) -> bool {
        self.index.contains_key(&normalize(path))
    }

    fn list(&self, dir: &str) -> Vec<String> {
        let prefix = normalize(dir);
        let prefix = if prefix.is_empty() || prefix.ends_with('/') {
            prefix
        } else {
            prefix + "/"
        };
        let mut out: Vec<String> = self
            .entries
            .iter()
            .filter(|e| e.name.starts_with(&prefix))
            .map(|e| e.name.clone())
            .collect();
        out.sort();
        out
    }
}

/// Builds a v1 MEG in memory (used by tests, also handy for tooling).
#[cfg(test)]
pub(crate) fn build_v1(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut names = Vec::new();
    for (n, _) in files {
        names.extend((n.len() as u16).to_le_bytes());
        names.extend(n.as_bytes());
    }
    let data_start = 8 + names.len() + files.len() * 20;
    let mut out = Vec::new();
    out.extend((files.len() as u32).to_le_bytes());
    out.extend((files.len() as u32).to_le_bytes());
    out.extend(names);
    let mut pos = data_start;
    for (i, (_, d)) in files.iter().enumerate() {
        for v in [0u32, i as u32, d.len() as u32, pos as u32, i as u32] {
            out.extend(v.to_le_bytes());
        }
        pos += d.len();
    }
    for (_, d) in files {
        out.extend(*d);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn temp_file(name: &str, data: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("alamo-fs-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, data).unwrap();
        p
    }

    #[test]
    fn reads_synthetic_v1() {
        let bytes = build_v1(&[
            ("DATA\\XML\\A.XML", b"hello"),
            ("DATA\\ART\\MODELS\\X.ALO", b"world!"),
        ]);
        let meg = MegArchive::open(temp_file("v1.meg", &bytes)).unwrap();
        assert_eq!(meg.len(), 2);
        assert_eq!(meg.read("xml/a.xml").unwrap(), b"hello");
        assert_eq!(meg.read(r"Data\Art\Models\x.alo").unwrap(), b"world!");
        assert!(!meg.exists("nope"));
        assert_eq!(meg.list("art"), vec!["ART/MODELS/X.ALO"]);
        assert_eq!(meg.list("").len(), 2);
    }

    #[test]
    fn rejects_truncated() {
        let mut bytes = build_v1(&[("A", b"hello")]);
        bytes.truncate(bytes.len() - 2);
        assert!(MegArchive::open(temp_file("trunc.meg", &bytes)).is_err());
    }

    #[test]
    fn reads_synthetic_v2() {
        let names = [("DATA\\B.TXT", &b"abc"[..])];
        let mut name_tbl = Vec::new();
        for (n, _) in names {
            name_tbl.extend((n.len() as u16).to_le_bytes());
            name_tbl.extend(n.as_bytes());
        }
        let header_size = 24 + name_tbl.len() + 20;
        let mut out = Vec::new();
        for v in [
            V2_FLAGS,
            V2_MAGIC,
            header_size as u32,
            1,
            1,
            name_tbl.len() as u32,
        ] {
            out.extend(v.to_le_bytes());
        }
        out.extend(&name_tbl);
        out.extend(0u16.to_le_bytes());
        for v in [0u32, 0, 3, header_size as u32] {
            out.extend(v.to_le_bytes());
        }
        out.extend(0u16.to_le_bytes());
        out.extend(b"abc");
        let meg = MegArchive::open(temp_file("v2.meg", &out)).unwrap();
        assert_eq!(meg.read("b.txt").unwrap(), b"abc");
    }
}
