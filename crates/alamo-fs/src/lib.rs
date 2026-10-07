//! Read-only access to Alamo engine game files.
//!
//! All paths are relative to a `Data` root (e.g. `Xml/Planets.xml`), case-insensitive,
//! and may use `/` or `\` as separator.

mod install;
mod loose;
mod meg;
mod vfs;

pub use install::GameInstall;
pub use loose::LooseDir;
pub use meg::{MegArchive, MegEntry};
pub use vfs::GameFs;

use std::io;

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
