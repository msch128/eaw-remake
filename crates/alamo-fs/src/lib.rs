//! Read-only access to Alamo engine game files.
//!
//! All paths are relative to a `Data` root (e.g. `Xml/Planets.xml`), case-insensitive,
//! and may use `/` or `\` as separator.

use std::io;
use std::path::{Path, PathBuf};

/// Something game files can be read from: a loose directory, a MEG archive or a layered stack of both.
pub trait FileSource: Send + Sync {
    /// Reads a whole file.
    fn read(&self, path: &str) -> io::Result<Vec<u8>>;

    /// Returns true if the file exists in this source.
    fn exists(&self, path: &str) -> bool;

    /// Lists all files (recursively) below `dir`, as normalized paths (see [`normalize`]).
    /// An empty `dir` lists everything.
    fn list(&self, dir: &str) -> Vec<String>;
}

/// Normalizes a game path: uppercase, `/` as separator, no leading `./`, `/` or `DATA/` prefix.
pub fn normalize(path: &str) -> String {
    let mut p = path.replace('\\', "/").to_uppercase();
    while let Some(rest) = p.strip_prefix("./").or_else(|| p.strip_prefix('/')) {
        p = rest.to_string();
    }
    if let Some(rest) = p.strip_prefix("DATA/") {
        p = rest.to_string();
    }
    p
}

/// A plain directory on disk, typically a `Data` folder with loose files.
pub struct LooseDir {
    root: PathBuf,
    /// normalized path -> real path on disk
    index: std::collections::HashMap<String, PathBuf>,
}

impl LooseDir {
    /// Indexes all files below `root` (the `Data` folder itself).
    pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref().to_path_buf();
        let mut index = std::collections::HashMap::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                if entry.file_type()?.is_dir() {
                    stack.push(path);
                } else if let Ok(rel) = path.strip_prefix(&root) {
                    index.insert(normalize(&rel.to_string_lossy()), path);
                }
            }
        }
        Ok(Self { root, index })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl FileSource for LooseDir {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        match self.index.get(&normalize(path)) {
            Some(real) => std::fs::read(real),
            None => Err(io::Error::new(io::ErrorKind::NotFound, path.to_string())),
        }
    }

    fn exists(&self, path: &str) -> bool {
        self.index.contains_key(&normalize(path))
    }

    fn list(&self, dir: &str) -> Vec<String> {
        let prefix = normalize(dir);
        let prefix = if prefix.is_empty() || prefix.ends_with('/') { prefix } else { prefix + "/" };
        let mut out: Vec<String> = self.index.keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
        out.sort();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_paths() {
        assert_eq!(normalize(r"Data\Xml\Planets.xml"), "XML/PLANETS.XML");
        assert_eq!(normalize("./xml/planets.xml"), "XML/PLANETS.XML");
        assert_eq!(normalize("/Art/Models/x.alo"), "ART/MODELS/X.ALO");
    }
}
