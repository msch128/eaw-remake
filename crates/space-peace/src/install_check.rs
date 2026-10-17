//! `--check-install`: tells whether *Star Wars: Empire at War - Forces of Corruption* and the Remake mod are
//! installed, then exits, before any Bevy app, window or GPU context exists. It asks the questions the app asks when it
//! starts (`--game-dir`, `EAW_GAME_DIR`, the Steam registry and `libraryfolders.vdf`; `--mod-dir`, `EAW_MOD_DIR`, the
//! Workshop folder), so the answer is the one the game will get. The Windows installer runs it hidden and reads the exit
//! code. The installed build is a windowed app (no console): its output reaches a terminal only when redirected,
//! `eaw-remake.exe --check-install > check.txt`.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use alamo_fs::GameInstall;

use crate::cli::Args;
use crate::game;

/// Exit code: the game and the mod were found.
pub const EXIT_FOUND: i32 = 0;
/// Exit code: Forces of Corruption, part of the Gold Pack (Steam app 32470), was not found.
pub const EXIT_GAME_MISSING: i32 = 2;
/// Exit code: the Remake mod was not found.
pub const EXIT_MOD_MISSING: i32 = 3;

/// Store page of the game.
const GAME_URL: &str = "https://store.steampowered.com/app/32470/";

/// What the lookup works from: the answers of the app's own lookup and the choices that moved it.
pub struct Lookup {
    /// The install the app finds (`game::find_install`), `None` when there is none.
    pub install: Option<GameInstall>,
    /// `--game-dir` as given, named in the report when it does not hold the game.
    pub game_dir: Option<PathBuf>,
    /// `--mod-dir` or `EAW_MOD_DIR`: the mod folder, or its `Data` folder.
    pub mod_dir: Option<PathBuf>,
    /// Workshop id of the mod when there is no `mod_dir`.
    pub mod_id: String,
}

/// What the lookup found.
#[derive(Debug)]
pub enum Outcome {
    /// Forces of Corruption is not there; `why` says what was looked at.
    GameMissing { why: String },
    /// The game is there, the mod is not.
    ModMissing { game: GameInstall, mod_data: PathBuf, mod_id: String },
    /// Both are there.
    Found { game: GameInstall, mod_data: PathBuf, mod_id: String },
}

/// A folder with at least one entry: Steam creates the folders of a game or a Workshop item before it fills them.
fn has_files(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some())
}

/// A path with one kind of separator (the lookup joins `a/b` onto `C:\x`, which Windows takes but looks odd in a report).
fn show(path: &Path) -> String {
    path.components().collect::<PathBuf>().display().to_string()
}

impl Lookup {
    /// The lookup of a command line, with the app's own choice of install and mod folder.
    pub fn from_args(args: &Args) -> Self {
        Self {
            install: game::find_install(args),
            game_dir: args.game_dir.clone(),
            mod_dir: game::explicit_mod_dir(args),
            mod_id: game::default_mod_id(args.mod_id.as_deref()).to_owned(),
        }
    }

    /// Checks for Forces of Corruption first, then for the mod.
    pub fn check(&self) -> Outcome {
        let Some(game) = self.install.clone().filter(|g| has_files(&g.foc_data)) else {
            let why = match (&self.install, &self.game_dir) {
                (Some(g), _) => format!("{} has no Forces of Corruption data (corruption\\Data), the base game alone is not enough", show(&g.game_root)),
                (None, Some(dir)) => format!("--game-dir {} holds no Empire at War", show(dir)),
                (None, None) => "no Empire at War install through EAW_GAME_DIR or Steam: registry SteamPath, libraryfolders.vdf, appmanifest_32470.acf".to_owned(),
            };
            return Outcome::GameMissing { why };
        };
        let mod_data = match &self.mod_dir {
            Some(dir) => game::explicit_mod_data(dir),
            None => game.mod_data(&self.mod_id),
        };
        let mod_id = self.mod_id.clone();
        if has_files(&mod_data) { Outcome::Found { game, mod_data, mod_id } } else { Outcome::ModMissing { game, mod_data, mod_id } }
    }
}

impl Outcome {
    /// The process exit code of this outcome.
    pub fn exit_code(&self) -> i32 {
        match self {
            Outcome::Found { .. } => EXIT_FOUND,
            Outcome::GameMissing { .. } => EXIT_GAME_MISSING,
            Outcome::ModMissing { .. } => EXIT_MOD_MISSING,
        }
    }

    /// The text printed to stdout: the paths found, or what is missing and where to get it.
    pub fn report(&self) -> String {
        let mut out = String::from("EAW:Remake install check\n");
        // Writing to a String cannot fail.
        let _ = match self {
            Outcome::GameMissing { why } => write!(
                out,
                "game: NOT FOUND ({why})\nneeded: Star Wars: Empire at War - Gold Pack (with Forces of Corruption), Steam app 32470\n        {GAME_URL}\n"
            ),
            Outcome::ModMissing { game, mod_data, mod_id } => write!(
                out,
                "{}mod {mod_id}: NOT FOUND ({} is missing or empty)\nneeded: Empire at War Remake 4.0, subscribe on the Steam Workshop and let Steam finish the download\n        https://steamcommunity.com/sharedfiles/filedetails/?id={mod_id}\n",
                game_lines(game),
                show(mod_data),
            ),
            Outcome::Found { game, mod_data, mod_id } => writeln!(out, "{}mod {mod_id}: found {}", game_lines(game), show(mod_data)),
        };
        let _ = writeln!(out, "result: {} (exit code {})", self.verdict(), self.exit_code());
        out
    }

    fn verdict(&self) -> &'static str {
        match self {
            Outcome::Found { .. } => "game and mod found",
            Outcome::GameMissing { .. } => "game missing",
            Outcome::ModMissing { .. } => "mod missing",
        }
    }
}

fn game_lines(game: &GameInstall) -> String {
    format!("game: found {}\n      Forces of Corruption data {}\n", show(&game.game_root), show(&game.foc_data))
}

/// Runs the check for `--check-install`: prints the report and returns the exit code. A closed stdout (the installer
/// reads the code only) must not change the answer, so write errors are ignored.
pub fn run(args: &Args) -> i32 {
    let outcome = Lookup::from_args(args).check();
    let _ = std::io::stdout().write_all(outcome.report().as_bytes());
    outcome.exit_code()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const REMAKE: &str = "1770851727";

    /// A scratch Steam library below the temp folder, removed again when it goes out of scope.
    struct Library(PathBuf);

    impl Library {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("eaw-install-check-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        /// `steamapps/common/Star Wars Empire at War`, where the game lives in a library.
        fn game_root(&self) -> PathBuf {
            self.0.join("steamapps/common/Star Wars Empire at War")
        }

        /// `steamapps/workshop/content/32470/<id>/Data`, where Steam puts a Workshop item.
        fn workshop_data(&self, id: &str) -> PathBuf {
            self.0.join("steamapps/workshop/content/32470").join(id).join("Data")
        }

        /// Makes `dir` a folder that holds something (one placeholder file of our own).
        fn fill(&self, dir: &Path) {
            fs::create_dir_all(dir).unwrap();
            fs::write(dir.join("placeholder.txt"), "x").unwrap();
        }

        /// Forces of Corruption data in the game folder.
        fn install_foc(&self) {
            self.fill(&self.game_root().join("corruption/Data"));
        }

        /// The lookup `--game-dir <game folder>` makes (no `--mod-dir`).
        fn lookup(&self) -> Lookup {
            Lookup { install: GameInstall::from_root(self.game_root()), game_dir: Some(self.game_root()), mod_dir: None, mod_id: REMAKE.into() }
        }
    }

    impl Drop for Library {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn exit_codes_are_the_ones_the_installer_reads() {
        assert_eq!((EXIT_FOUND, EXIT_GAME_MISSING, EXIT_MOD_MISSING), (0, 2, 3));
    }

    #[test]
    fn an_empty_game_dir_is_a_missing_game() {
        let lib = Library::new("empty-game");
        fs::create_dir_all(lib.game_root()).unwrap();
        let outcome = lib.lookup().check();
        assert_eq!(outcome.exit_code(), EXIT_GAME_MISSING);
        let report = outcome.report();
        assert!(report.contains("--game-dir") && report.contains("Steam app 32470") && report.contains("exit code 2"), "{report}");
    }

    #[test]
    fn no_install_at_all_is_a_missing_game() {
        let lookup = Lookup { install: None, game_dir: None, mod_dir: None, mod_id: REMAKE.into() };
        let outcome = lookup.check();
        assert_eq!(outcome.exit_code(), EXIT_GAME_MISSING);
        assert!(outcome.report().contains("libraryfolders.vdf"));
    }

    #[test]
    fn the_base_game_without_forces_of_corruption_is_a_missing_game() {
        let lib = Library::new("base-only");
        lib.fill(&lib.game_root().join("GameData/Data"));
        lib.fill(&lib.workshop_data(REMAKE));
        let outcome = lib.lookup().check();
        assert_eq!(outcome.exit_code(), EXIT_GAME_MISSING);
        assert!(outcome.report().contains("no Forces of Corruption data"));
    }

    #[test]
    fn a_game_without_the_workshop_mod_is_a_missing_mod() {
        let lib = Library::new("no-mod");
        lib.install_foc();
        let outcome = lib.lookup().check();
        assert_eq!(outcome.exit_code(), EXIT_MOD_MISSING);
        let report = outcome.report();
        assert!(report.contains("game: found") && report.contains("mod 1770851727: NOT FOUND") && report.contains("?id=1770851727"), "{report}");
    }

    #[test]
    fn a_workshop_folder_with_nothing_in_it_is_a_missing_mod() {
        let lib = Library::new("empty-mod");
        lib.install_foc();
        fs::create_dir_all(lib.workshop_data(REMAKE)).unwrap();
        assert_eq!(lib.lookup().check().exit_code(), EXIT_MOD_MISSING);
    }

    #[test]
    fn the_game_and_the_mod_in_the_workshop_folder_are_found() {
        let lib = Library::new("both");
        lib.install_foc();
        lib.fill(&lib.workshop_data(REMAKE));
        let outcome = lib.lookup().check();
        assert_eq!(outcome.exit_code(), EXIT_FOUND);
        let report = outcome.report();
        assert!(report.contains(&show(&lib.game_root())) && report.contains(&show(&lib.workshop_data(REMAKE))), "{report}");
        assert!(report.contains("exit code 0"), "{report}");
    }

    #[test]
    fn another_mod_id_looks_in_that_workshop_folder() {
        let lib = Library::new("other-id");
        lib.install_foc();
        lib.fill(&lib.workshop_data(REMAKE));
        let lookup = Lookup { mod_id: "123".into(), ..lib.lookup() };
        assert_eq!(lookup.check().exit_code(), EXIT_MOD_MISSING);
    }

    #[test]
    fn an_explicit_mod_dir_replaces_the_workshop_folder() {
        let lib = Library::new("mod-dir");
        lib.install_foc();
        lib.fill(&lib.workshop_data(REMAKE));
        let missing = Lookup { mod_dir: Some(lib.0.join("no-such-mod")), ..lib.lookup() };
        assert_eq!(missing.check().exit_code(), EXIT_MOD_MISSING, "the Workshop copy does not count when --mod-dir names another folder");
        let elsewhere = lib.0.join("my-mod");
        lib.fill(&elsewhere.join("Data"));
        let with_data = Lookup { mod_dir: Some(elsewhere.clone()), ..lib.lookup() }.check();
        assert_eq!(with_data.exit_code(), EXIT_FOUND);
        assert!(with_data.report().contains(&show(&elsewhere.join("Data"))));
    }

    #[test]
    fn a_mod_dir_may_be_the_data_folder_itself() {
        let lib = Library::new("mod-data");
        lib.install_foc();
        let data = lib.0.join("loose-mod-data");
        lib.fill(&data);
        let lookup = Lookup { mod_dir: Some(data), ..lib.lookup() };
        assert_eq!(lookup.check().exit_code(), EXIT_FOUND);
    }
}
