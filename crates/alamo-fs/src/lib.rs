//! Read-only access to Alamo engine game files.
//!
//! All paths are relative to a `Data` root (e.g. `Xml/Planets.xml`), case-insensitive,
//! and may use `/` or `\` as separator.
//!
//! [`MegArchive`] reads `.meg` archives, [`LooseDir`] a plain directory, and [`GameFs::for_mod`] stacks them the way the
//! engine does (mod over Forces of Corruption over the base game, `megafiles.xml` lists, patch archives, the audio
//! archives the engine mounts itself; non-English speech is skipped). [`GameInstall::detect`] finds the Steam install
//! (env `EAW_GAME_DIR`, otherwise the registry plus `libraryfolders.vdf`). Precedence rules are at the top of `vfs.rs`;
//! the ordering of unlisted patch archives is an educated guess. Everything is read-only and `Send + Sync`; whole files
//! are read into memory.

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
