//! Locating the Steam installation of *Star Wars: Empire at War - Gold Pack* and its Workshop content.

use std::path::{Path, PathBuf};

/// Steam app id of the Empire at War Gold Pack.
pub const STEAM_APP_ID: u32 = 32470;

/// Folder name of the game below `steamapps/common`.
const GAME_DIR_NAME: &str = "Star Wars Empire at War";

/// Environment variable that overrides detection with an explicit game root.
pub const ROOT_ENV_VAR: &str = "EAW_GAME_DIR";

/// Paths of an installed game.
#[derive(Debug, Clone)]
pub struct GameInstall {
    /// `.../steamapps/common/Star Wars Empire at War`.
    pub game_root: PathBuf,
    /// Forces of Corruption data: `<root>/corruption/Data`.
    pub foc_data: PathBuf,
    /// Base Empire at War data: `<root>/GameData/Data`.
    pub eaw_data: PathBuf,
    /// `.../steamapps/workshop/content/32470` (may not exist if no mods are subscribed).
    pub workshop_root: PathBuf,
}

impl GameInstall {
    /// Detects the Steam install: `EAW_GAME_DIR` if set, otherwise the registry's `SteamPath`,
    /// `libraryfolders.vdf` and the library holding `appmanifest_32470.acf`.
    pub fn detect() -> Option<GameInstall> {
        if let Some(root) = std::env::var_os(ROOT_ENV_VAR) {
            return Self::from_root(root);
        }
        let steam = steam_path()?;
        let mut libraries = vec![steam.clone()];
        if let Ok(vdf) = std::fs::read_to_string(steam.join("steamapps/libraryfolders.vdf")) {
            libraries.extend(library_paths(&vdf));
        }
        libraries
            .into_iter()
            .find(|lib| {
                lib.join(format!("steamapps/appmanifest_{STEAM_APP_ID}.acf"))
                    .is_file()
            })
            .and_then(|lib| Self::from_library(&lib))
    }

    /// Builds an install from the game root directory, deriving the workshop folder from it
    /// (`<root>/../../workshop/content/32470`).
    pub fn from_root(root: impl Into<PathBuf>) -> Option<GameInstall> {
        let game_root: PathBuf = root.into();
        let foc_data = game_root.join("corruption").join("Data");
        let eaw_data = game_root.join("GameData").join("Data");
        if !foc_data.is_dir() && !eaw_data.is_dir() {
            return None;
        }
        let steamapps = game_root.parent().and_then(Path::parent);
        let workshop_root = steamapps
            .map(|s| s.join("workshop/content").join(STEAM_APP_ID.to_string()))
            .unwrap_or_default();
        Some(GameInstall {
            game_root,
            foc_data,
            eaw_data,
            workshop_root,
        })
    }

    fn from_library(lib: &Path) -> Option<GameInstall> {
        Self::from_root(lib.join("steamapps/common").join(GAME_DIR_NAME))
    }

    /// The `Data` directory of a Workshop mod.
    pub fn mod_data(&self, workshop_id: &str) -> PathBuf {
        self.workshop_root.join(workshop_id).join("Data")
    }
}

#[cfg(windows)]
fn steam_path() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Valve\Steam")
        .ok()?;
    let path: String = key.get_value("SteamPath").ok()?;
    Some(PathBuf::from(path))
}

#[cfg(not(windows))]
fn steam_path() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    [".steam/steam", ".local/share/Steam"]
        .iter()
        .map(|p| home.join(p))
        .find(|p| p.is_dir())
}

/// Extracts every `"path"  "..."` value from a `libraryfolders.vdf`.
fn library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| {
            let mut quoted = line.split('"').skip(1).step_by(2);
            let (key, value) = (quoted.next()?, quoted.next()?);
            key.eq_ignore_ascii_case("path")
                .then(|| PathBuf::from(value.replace("\\\\", "\\")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_libraryfolders() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t\t\"label\"\t\t\"\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"F:\\\\SteamLibrary\"\n\t}\n}\n";
        assert_eq!(
            library_paths(vdf),
            vec![
                PathBuf::from(r"C:\Program Files (x86)\Steam"),
                PathBuf::from(r"F:\SteamLibrary")
            ]
        );
    }
}
