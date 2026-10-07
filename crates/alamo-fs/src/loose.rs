use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use crate::{FileSource, normalize};

/// A plain directory on disk, typically a `Data` folder with loose files.
pub struct LooseDir {
    root: PathBuf,
    /// normalized path -> real path on disk
    index: HashMap<String, PathBuf>,
}

impl LooseDir {
    /// Indexes all files below `root` (the `Data` folder itself).
    pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref().to_path_buf();
        let mut index = HashMap::new();
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

    /// The directory this index was built from.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolves a game path to the real file on disk, if present.
    pub fn real_path(&self, path: &str) -> Option<&Path> {
        self.index.get(&normalize(path)).map(PathBuf::as_path)
    }

    /// Iterates (normalized path, real path) for every indexed file.
    pub fn files(&self) -> impl Iterator<Item = (&str, &Path)> {
        self.index.iter().map(|(k, v)| (k.as_str(), v.as_path()))
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
        let prefix = if prefix.is_empty() || prefix.ends_with('/') {
            prefix
        } else {
            prefix + "/"
        };
        let mut out: Vec<String> = self
            .index
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        out.sort();
        out
    }
}
