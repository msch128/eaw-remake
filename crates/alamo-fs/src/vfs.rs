//! The layered game file system.
//!
//! # Precedence (highest wins)
//!
//! A [`GameFs`] is built from one or more *stacks*. Each stack is an ordered list of `Data` roots,
//! highest priority first (e.g. `[mod Data, corruption/Data]`). Stacks themselves are ordered, highest
//! first; [`GameFs::for_mod`] uses `[[mod, corruption/Data], [GameData/Data]]`, so base Empire at War content
//! sits below everything else.
//!
//! Within a stack, from highest to lowest priority:
//!
//! 1. Loose files, root by root (the first root containing the file wins).
//! 2. MEG archives. The active list is the `Data/megafiles.xml` of the highest root in the stack that
//!    has one; it replaces (not extends) the lists of lower roots. Each listed path (`Data\Models.meg`) is
//!    resolved by searching the stack's roots in priority order, so a mod can list base-game archives it
//!    does not ship itself. Later list entries override earlier ones.
//!    Speech archives of languages other than English (*Speech.meg) are skipped.
//! 3. Unlisted `Patch.meg`, `Patch2.meg`, `64Patch.meg` (the engine loads these without listing them;
//!    they overlap the base archives, so they must win over them). Archives are grouped by the root they
//!    were resolved from: a root's patches override that root's listed MEGs, while any archive of a
//!    higher root overrides those of lower roots. **This ordering is an educated guess**, not
//!    verified against the engine.
//!
//! Between stacks, every file of a higher stack (loose or in a MEG) beats every file of a lower stack.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;

use crate::{FileSource, GameInstall, LooseDir, MegArchive, normalize};

/// Archives the engine loads from every root in addition to `megafiles.xml`, lowest priority first.
const PATCH_MEGS: [&str; 3] = ["Patch.meg", "Patch2.meg", "64Patch.meg"];

#[derive(Debug, Clone, Copy)]
enum Source {
    /// Index into `GameFs::loose`.
    Loose(u32),
    /// `(index into megs, entry index)`.
    Meg(u32, u32),
}

/// A layered, read-only view over loose `Data` directories and MEG archives. See the module docs for
/// the precedence rules.
pub struct GameFs {
    loose: Vec<LooseDir>,
    megs: Vec<MegArchive>,
    /// normalized path -> winning source; the loose index is queried again lazily for the real path.
    files: HashMap<String, Source>,
    /// All keys of `files`, sorted, for prefix listing.
    sorted: Vec<String>,
}

impl GameFs {
    /// Builds a single stack from `roots` (highest priority first). Missing directories are skipped.
    pub fn new(roots: &[PathBuf]) -> io::Result<Self> {
        Self::from_stacks(&[roots.to_vec()])
    }

    /// Builds the file system for a Forces of Corruption game, optionally with the Workshop mod
    /// `workshop_id` on top: `[[mod, corruption/Data], [GameData/Data]]`.
    pub fn for_mod(install: &GameInstall, workshop_id: Option<&str>) -> io::Result<Self> {
        let mut top = Vec::new();
        if let Some(id) = workshop_id {
            let dir = install.mod_data(id);
            if !dir.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("mod data not found: {}", dir.display()),
                ));
            }
            top.push(dir);
        }
        top.push(install.foc_data.clone());
        Self::from_stacks(&[top, vec![install.eaw_data.clone()]])
    }

    /// Builds from several stacks, highest priority first; each stack lists its roots highest first.
    pub fn from_stacks(stacks: &[Vec<PathBuf>]) -> io::Result<Self> {
        let mut loose = Vec::new();
        let mut megs = Vec::new();
        let mut files: HashMap<String, Source> = HashMap::new();

        // Lowest priority first, so later inserts overwrite earlier ones.
        for stack in stacks.iter().rev() {
            let first_loose = loose.len();
            for root in stack.iter().filter(|r| r.is_dir()) {
                loose.push(LooseDir::new(root)?);
            }
            let layers = &loose[first_loose..];

            for path in stack_meg_paths(layers) {
                let meg = MegArchive::open(&path)?;
                let mi = megs.len() as u32;
                for (ei, e) in meg.entries().iter().enumerate() {
                    files.insert(e.name.clone(), Source::Meg(mi, ei as u32));
                }
                megs.push(meg);
            }

            for (li, layer) in layers.iter().enumerate().rev() {
                for (name, _) in layer.files() {
                    files.insert(name.to_string(), Source::Loose((first_loose + li) as u32));
                }
            }
        }

        let mut sorted: Vec<String> = files.keys().cloned().collect();
        sorted.sort_unstable();
        Ok(Self {
            loose,
            megs,
            files,
            sorted,
        })
    }

    /// Number of distinct files visible.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// The MEG archives that were loaded, in load order per stack (lowest priority stack first).
    pub fn megs(&self) -> &[MegArchive] {
        &self.megs
    }

    /// Describes where `path` comes from (real file path, or `archive.meg!NAME`), for diagnostics.
    pub fn origin(&self, path: &str) -> Option<String> {
        let key = normalize(path);
        Some(match *self.files.get(&key)? {
            Source::Loose(li) => self.loose[li as usize]
                .real_path(&key)?
                .display()
                .to_string(),
            Source::Meg(mi, _) => format!("{}!{key}", self.megs[mi as usize].path().display()),
        })
    }
}

/// MEG files to load for one stack, lowest priority first. Archives are grouped by the root they were
/// resolved from (lowest root first); inside a group the listed MEGs come in list order, followed by that
/// root's patch archives.
fn stack_meg_paths(layers: &[LooseDir]) -> Vec<PathBuf> {
    let mut listed: Vec<(usize, PathBuf)> = Vec::new();
    let owner = layers.iter().find_map(|l| l.real_path("megafiles.xml"));
    if let Some(xml) = owner
        && let Ok(text) = std::fs::read(xml)
    {
        for name in parse_megafiles(&String::from_utf8_lossy(&text))
            .into_iter()
            .filter(|n| !is_foreign_speech(n))
        {
            // Resolve through the layers, highest first.
            if let Some((i, p)) = layers
                .iter()
                .enumerate()
                .find_map(|(i, l)| Some((i, l.real_path(&name)?)))
            {
                listed.push((i, p.to_path_buf()));
            }
        }
    }
    let mut paths: Vec<PathBuf> = Vec::new();
    for (i, layer) in layers.iter().enumerate().rev() {
        paths.extend(
            listed
                .iter()
                .filter(|(l, _)| *l == i)
                .map(|(_, p)| p.clone()),
        );
        for patch in PATCH_MEGS {
            if let Some(p) = layer.real_path(patch)
                && !paths.iter().any(|q| q == p)
                && !listed.iter().any(|(_, q)| q == p)
            {
                paths.push(p.to_path_buf());
            }
        }
    }
    paths
}
/// True for localized speech archives other than English (GermanSpeech.meg, ...); this project is English-only.
fn is_foreign_speech(name: &str) -> bool {
    let file = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase();
    file.ends_with("speech.meg") && file != "englishspeech.meg"
}

/// Leniently extracts the `<File>...</File>` entries of a `megafiles.xml` (BOM, comments and odd
/// whitespace tolerated).
pub(crate) fn parse_megafiles(text: &str) -> Vec<String> {
    let mut text = text.trim_start_matches('\u{feff}').to_string();
    while let Some(start) = text.find("<!--") {
        let end = text[start..]
            .find("-->")
            .map_or(text.len(), |e| start + e + 3);
        text.replace_range(start..end, "");
    }
    // ASCII lowercasing keeps byte offsets identical.
    let lower = text.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(open) = lower[pos..].find("<file>") {
        let body = pos + open + "<file>".len();
        let Some(close) = lower[body..].find("</file>") else {
            break;
        };
        let name = text[body..body + close].trim();
        if !name.is_empty() {
            out.push(name.to_string());
        }
        pos = body + close + "</file>".len();
    }
    out
}

impl FileSource for GameFs {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        let key = normalize(path);
        match self.files.get(&key) {
            Some(Source::Loose(li)) => match self.loose[*li as usize].real_path(&key) {
                Some(real) => std::fs::read(real),
                None => Err(io::Error::new(io::ErrorKind::NotFound, path.to_string())),
            },
            Some(Source::Meg(mi, ei)) => self.megs[*mi as usize].read_entry(*ei as usize),
            None => Err(io::Error::new(io::ErrorKind::NotFound, path.to_string())),
        }
    }

    fn exists(&self, path: &str) -> bool {
        self.files.contains_key(&normalize(path))
    }

    fn list(&self, dir: &str) -> Vec<String> {
        let prefix = normalize(dir);
        let prefix = if prefix.is_empty() || prefix.ends_with('/') {
            prefix
        } else {
            prefix + "/"
        };
        let start = self
            .sorted
            .partition_point(|k| k.as_str() < prefix.as_str());
        self.sorted[start..]
            .iter()
            .take_while(|k| k.starts_with(&prefix))
            .cloned()
            .collect()
    }
}

impl GameFs {
    /// Reads a file and decodes it lossily as UTF-8, dropping a leading BOM.
    pub fn read_text(&self, path: &str) -> io::Result<String> {
        let bytes = self.read(path)?;
        let text = String::from_utf8_lossy(&bytes);
        Ok(text.trim_start_matches('\u{feff}').to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meg::build_v1;
    use std::path::Path;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("alamo-fs-vfs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(root: &Path, rel: &str, data: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }

    #[test]
    fn parses_lenient_megafiles() {
        let xml = "\u{feff}<?xml?><Mega_Files><!-- <File>Data\\Skip.meg</File> --><Info/>\n<FILE> Data\\A.meg </FILE>\n<File>Data\\B.meg</File></Mega_Files>";
        assert_eq!(parse_megafiles(xml), vec!["Data\\A.meg", "Data\\B.meg"]);
    }

    #[test]
    fn layering() {
        let base = scratch("base");
        let modd = scratch("mod");
        let eaw = scratch("eaw");
        put(
            &base,
            "Models.meg",
            &build_v1(&[
                ("DATA\\XML\\A.XML", b"base-meg"),
                ("DATA\\ART\\M.ALO", b"model"),
            ]),
        );
        put(
            &base,
            "Extra.meg",
            &build_v1(&[("DATA\\ART\\M.ALO", b"extra-model")]),
        );
        put(
            &base,
            "megafiles.xml",
            b"<Mega_Files><File>Data\\Models.meg</File></Mega_Files>",
        );
        put(
            &modd,
            "megafiles.xml",
            b"<Mega_Files><File>Data\\Models.meg</File><File>Data\\Extra.meg</File></Mega_Files>",
        );
        put(&modd, "Xml/A.xml", b"mod-loose");
        put(&eaw, "Xml/Old.xml", b"eaw");
        put(&eaw, "Xml/A.xml", b"eaw-a");

        let fs = GameFs::from_stacks(&[vec![modd, base], vec![eaw]]).unwrap();
        assert_eq!(fs.read("data\\xml\\a.xml").unwrap(), b"mod-loose");
        assert_eq!(fs.read("Art/m.alo").unwrap(), b"extra-model");
        assert_eq!(fs.read("xml/old.xml").unwrap(), b"eaw");
        assert!(!fs.exists("xml/nothing.xml"));
        assert_eq!(fs.list("xml"), vec!["XML/A.XML", "XML/OLD.XML"]);
        assert!(fs.origin("xml/a.xml").unwrap().ends_with("A.xml"));
    }
}
