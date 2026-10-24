//! The per-user data folder, `%APPDATA%\EAW-Remake`: `settings.toml`, `saves\` and `logs\`. Every path into it goes through
//! [`data_dir`], so the name is written in one place.
//!
//! The game was developed under a working title first and kept its data in `%APPDATA%\SpacePeace`. [`init`] moves that folder to the
//! current name on the first start, before anything reads settings or saves, so existing saves, settings and logs keep working. A
//! folder that cannot be moved (a file in it is locked, a game of the old build is still running) is used where it is for that run
//! and moved by a later start.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::game::StartupLog;

/// Name of the data folder inside `%APPDATA%`.
const DIR_NAME: &str = "EAW-Remake";
/// Name of the data folder of the builds before the rename.
const LEGACY_DIR_NAME: &str = "SpacePeace";

/// The folder this run keeps its data in, settled by [`init`].
static CHOSEN: OnceLock<PathBuf> = OnceLock::new();

#[cfg(test)]
thread_local! {
    /// `%APPDATA%` of the running test, so tests never change the environment of the other tests.
    static TEST_APPDATA: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// What [`init`] did to the data folder.
#[derive(Debug, PartialEq, Eq)]
pub enum Migration {
    /// Nothing to move: the folder has its current name already, or there is no data yet.
    NotNeeded,
    /// The folder of the old name was moved to the current name.
    Moved { from: PathBuf, to: PathBuf },
    /// The folder of the old name could not be moved: this run uses it where it is, a later start tries again.
    Kept { dir: PathBuf, wanted: PathBuf, error: String },
}

impl Migration {
    /// Puts the outcome into the start-up log (the logger does not exist yet when [`init`] runs).
    pub fn record(&self, log: &mut StartupLog) {
        match self {
            Migration::NotNeeded => {}
            Migration::Moved { from, to } => log.info(format!("data folder moved from {} to {}", from.display(), to.display())),
            Migration::Kept { dir, wanted, error } => {
                log.warn(format!("cannot move the data folder {} to {}: {error}; using it where it is for this run", dir.display(), wanted.display()));
            }
        }
    }
}

/// `%APPDATA%`, `None` when it is not set.
fn appdata() -> Option<PathBuf> {
    #[cfg(test)]
    {
        if let Some(path) = TEST_APPDATA.with(|t| t.borrow().clone()) {
            return Some(path);
        }
    }
    std::env::var_os("APPDATA").map(PathBuf::from)
}

/// The data folder: `%APPDATA%\EAW-Remake`, or the folder of the old name when [`init`] could not move it; `None` when `%APPDATA%` is not
/// set. Only computes a path: until [`init`] has run (unit tests) nothing on disk is touched.
pub fn data_dir() -> Option<PathBuf> {
    CHOSEN.get().cloned().or_else(|| appdata().map(|a| a.join(DIR_NAME)))
}

/// Settles the data folder for this run: moves the folder of the old name to the current one when only the old one exists. Call once at
/// start-up, before anything reads settings, saves or logs.
pub fn init() -> Migration {
    let Some(appdata) = appdata() else { return Migration::NotNeeded };
    let (dir, outcome) = migrate_with(&appdata, |from, to| std::fs::rename(from, to));
    let _ = CHOSEN.set(dir);
    outcome
}

/// The move itself, with the way of moving a folder as a parameter so a test can make it fail. Returns the folder to use and what
/// happened. Nothing is moved when the current name exists already (it wins) or there is no folder of the old name.
fn migrate_with(appdata: &Path, rename: impl FnOnce(&Path, &Path) -> io::Result<()>) -> (PathBuf, Migration) {
    let new = appdata.join(DIR_NAME);
    let old = appdata.join(LEGACY_DIR_NAME);
    if new.exists() || !old.is_dir() {
        return (new, Migration::NotNeeded);
    }
    match rename(&old, &new) {
        Ok(()) => (new.clone(), Migration::Moved { from: old, to: new }),
        Err(e) => (old.clone(), Migration::Kept { dir: old, wanted: new, error: e.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty scratch `%APPDATA%`.
    fn scratch_appdata(name: &str) -> PathBuf {
        let appdata = std::env::temp_dir().join(format!("eaw-remake-datadir-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&appdata);
        std::fs::create_dir_all(&appdata).unwrap();
        appdata
    }

    /// A scratch `%APPDATA%` with the settings, a save and a log in the folder of the old name.
    fn appdata_with_old_data(name: &str) -> PathBuf {
        let appdata = scratch_appdata(name);
        let old = appdata.join(LEGACY_DIR_NAME);
        std::fs::create_dir_all(old.join("saves")).unwrap();
        std::fs::create_dir_all(old.join("logs")).unwrap();
        std::fs::write(old.join("settings.toml"), "master_volume = 0.25\n").unwrap();
        std::fs::write(old.join("saves").join("slot1.json"), "{\"save\":1}").unwrap();
        std::fs::write(old.join("logs").join("space-peace-1760000000.log"), "old log").unwrap();
        appdata
    }

    fn real_rename(from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    #[test]
    fn the_old_folder_moves_to_the_new_name_with_everything_in_it() {
        let appdata = appdata_with_old_data("moves");
        let (dir, outcome) = migrate_with(&appdata, real_rename);
        let new = appdata.join(DIR_NAME);
        assert_eq!(dir, new);
        assert_eq!(outcome, Migration::Moved { from: appdata.join(LEGACY_DIR_NAME), to: new.clone() });
        assert!(!appdata.join(LEGACY_DIR_NAME).exists(), "the old folder is gone");
        assert_eq!(std::fs::read_to_string(new.join("settings.toml")).unwrap(), "master_volume = 0.25\n");
        assert_eq!(std::fs::read_to_string(new.join("saves").join("slot1.json")).unwrap(), "{\"save\":1}");
        assert_eq!(std::fs::read_to_string(new.join("logs").join("space-peace-1760000000.log")).unwrap(), "old log");
        let _ = std::fs::remove_dir_all(&appdata);
    }

    #[test]
    fn a_second_start_finds_nothing_to_move() {
        let appdata = appdata_with_old_data("twice");
        let _ = migrate_with(&appdata, real_rename);
        let (dir, outcome) = migrate_with(&appdata, |_, _| panic!("nothing is left to move"));
        assert_eq!((dir, outcome), (appdata.join(DIR_NAME), Migration::NotNeeded));
        let _ = std::fs::remove_dir_all(&appdata);
    }

    #[test]
    fn a_folder_that_cannot_be_moved_is_used_where_it_is() {
        let appdata = appdata_with_old_data("refused");
        let (dir, outcome) = migrate_with(&appdata, |_, _| Err(io::Error::from(io::ErrorKind::PermissionDenied)));
        let old = appdata.join(LEGACY_DIR_NAME);
        assert_eq!(dir, old);
        assert!(matches!(&outcome, Migration::Kept { dir, wanted, .. } if *dir == old && *wanted == appdata.join(DIR_NAME)), "{outcome:?}");
        assert!(old.join("settings.toml").is_file(), "the data stays where it is");
        assert!(!appdata.join(DIR_NAME).exists());
        let _ = std::fs::remove_dir_all(&appdata);
    }

    /// The case behind the fallback, for real: a file of the old folder is held open exclusively (another process, a running game of
    /// the old build), so Windows refuses to move the folder.
    #[cfg(windows)]
    #[test]
    fn a_locked_file_in_the_old_folder_keeps_it_in_place_until_it_is_free() {
        use std::os::windows::fs::OpenOptionsExt;
        let appdata = appdata_with_old_data("locked");
        let old = appdata.join(LEGACY_DIR_NAME);
        let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(old.join("logs").join("space-peace-1760000000.log")).unwrap();
        let (dir, outcome) = migrate_with(&appdata, real_rename);
        assert_eq!(dir, old);
        assert!(matches!(outcome, Migration::Kept { .. }), "{outcome:?}");
        assert!(old.join("settings.toml").is_file());
        drop(held);
        let (dir, outcome) = migrate_with(&appdata, real_rename);
        assert_eq!(dir, appdata.join(DIR_NAME));
        assert!(matches!(outcome, Migration::Moved { .. }), "{outcome:?}");
        let _ = std::fs::remove_dir_all(&appdata);
    }

    #[test]
    fn the_new_folder_wins_when_both_exist() {
        let appdata = appdata_with_old_data("both");
        let new = appdata.join(DIR_NAME);
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("settings.toml"), "master_volume = 0.75\n").unwrap();
        let (dir, outcome) = migrate_with(&appdata, |_, _| panic!("neither folder is touched"));
        assert_eq!((dir, outcome), (new.clone(), Migration::NotNeeded));
        assert_eq!(std::fs::read_to_string(new.join("settings.toml")).unwrap(), "master_volume = 0.75\n");
        assert!(appdata.join(LEGACY_DIR_NAME).join("settings.toml").is_file());
        let _ = std::fs::remove_dir_all(&appdata);
    }

    #[test]
    fn a_fresh_install_has_nothing_to_move_and_creates_nothing() {
        let appdata = scratch_appdata("fresh");
        let (dir, outcome) = migrate_with(&appdata, |_, _| panic!("nothing to move"));
        assert_eq!((dir.clone(), outcome), (appdata.join(DIR_NAME), Migration::NotNeeded));
        assert!(!dir.exists(), "the folder is created when something is first written");
        let _ = std::fs::remove_dir_all(&appdata);
    }

    #[test]
    fn settings_saves_and_logs_live_in_the_data_folder() {
        let appdata = scratch_appdata("paths");
        TEST_APPDATA.with(|t| *t.borrow_mut() = Some(appdata.clone()));
        let data = appdata.join(DIR_NAME);
        assert_eq!(data_dir(), Some(data.clone()));
        assert_eq!(crate::settings::Settings::path(), Some(data.join("settings.toml")));
        assert_eq!(crate::logfile::logs_dir(), Some(data.join("logs")));
        if std::env::var_os("SP_SAVES_DIR").is_none() {
            assert_eq!(crate::newgame::saves_dir(), Some(data.join("saves")));
        }
        assert!(!data.exists(), "asking for the paths creates nothing");
        let _ = std::fs::remove_dir_all(&appdata);
    }
}
