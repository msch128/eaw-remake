//! Turns sample names from the XML into paths in the game file system.
//!
//! Samples are written either as full paths (`Data\Audio\SFX\Weapons\x.wav`) or as bare file names
//! (`U000_TFP0101_ENG.wav`, `GC_Rebel_1.mp3`). Bare names are looked up in `Audio/Speech/English`,
//! `Audio/SFX`, `Audio/Music` and finally in an index of every file below `Audio` by file name.

use std::collections::HashMap;

use alamo_fs::{FileSource, normalize};

const SEARCH_DIRS: [&str; 4] = [
    "Audio/Speech/English",
    "Audio/SFX",
    "Audio/Music",
    "Audio/Voiceovers",
];

/// How a sample name was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// The given path exists as written.
    Exact,
    /// A bare name found in one of the standard audio directories.
    SearchDir,
    /// A bare name or wrong directory found through the file-name index.
    Index,
}

/// Look-up helper holding a file-name index of the `Audio` tree.
#[derive(Debug, Default)]
pub struct SampleResolver {
    /// upper-case file name -> normalized path (first one wins; shortest path preferred)
    by_name: HashMap<String, String>,
}

impl SampleResolver {
    /// Indexes everything below `Audio`.
    pub fn new(src: &dyn FileSource) -> Self {
        let mut by_name: HashMap<String, String> = HashMap::new();
        for path in src.list("Audio") {
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            by_name
                .entry(name)
                .and_modify(|old| {
                    if path.len() < old.len() {
                        *old = path.clone();
                    }
                })
                .or_insert(path);
        }
        Self { by_name }
    }

    /// Resolves a sample to a normalized path, or `None` if the file does not exist anywhere.
    pub fn resolve(&self, src: &dyn FileSource, sample: &str) -> Option<(String, Resolution)> {
        let norm = normalize(sample);
        if src.exists(&norm) {
            return Some((norm, Resolution::Exact));
        }
        let name = norm.rsplit('/').next().unwrap_or(&norm);
        if !norm.contains('/') {
            for dir in SEARCH_DIRS {
                let cand = format!("{}/{name}", normalize(dir));
                if src.exists(&cand) {
                    return Some((cand, Resolution::SearchDir));
                }
            }
        }
        self.by_name
            .get(name)
            .map(|p| (p.clone(), Resolution::Index))
    }

    /// Number of indexed audio files.
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// True if nothing is indexed.
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}
