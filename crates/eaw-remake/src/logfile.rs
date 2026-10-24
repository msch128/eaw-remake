//! Log file for live tests: everything the app logs (and panics) is also written to
//! `%APPDATA%\EAW-Remake\logs\eaw-remake-<unix time>.log`; the 10 newest files are kept. The logs of the builds before the rename
//! (`space-peace-<unix time>.log`) came along with the data folder and count among the 10, oldest first.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use bevy::log::tracing_subscriber::{self, Layer};
use bevy::log::BoxedLayer;
use bevy::prelude::*;

/// Number of log files kept.
const KEEP: usize = 10;

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// `%APPDATA%\EAW-Remake\logs`, `None` when `%APPDATA%` is not set.
pub fn logs_dir() -> Option<PathBuf> {
    crate::datadir::data_dir().map(|d| d.join("logs"))
}

/// The unix time in the name of a log file, `eaw-remake-<time>.log` or `space-peace-<time>.log`; 0 for any other name.
fn log_stamp(path: &Path) -> u64 {
    path.file_stem().and_then(|s| s.to_str()).and_then(|s| s.rsplit('-').next()).and_then(|t| t.parse().ok()).unwrap_or(0)
}

/// Opens a new log file and removes the oldest ones.
fn open_log() -> Option<File> {
    open_log_in(&logs_dir()?)
}

/// Opens a new log file in `dir`; the oldest ones go until `KEEP` files are left, whatever their name starts with.
fn open_log_in(dir: &Path) -> Option<File> {
    std::fs::create_dir_all(dir).ok()?;
    let mut old: Vec<(u64, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "log"))
        .map(|p| (log_stamp(&p), p))
        .collect();
    old.sort();
    while old.len() >= KEEP {
        let _ = std::fs::remove_file(old.remove(0).1);
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let path = dir.join(format!("eaw-remake-{stamp}.log"));
    let file = File::create(&path).ok()?;
    let _ = LOG_PATH.set(path);
    Some(file)
}

/// `LogPlugin::custom_layer`: a plain-text copy of the log in a file.
pub fn file_layer(_app: &mut App) -> Option<BoxedLayer> {
    let file = open_log()?;
    install_panic_hook();
    Some(tracing_subscriber::fmt::layer().with_ansi(false).with_writer(Mutex::new(file)).boxed())
}

/// Panics go into the log file as well (the default hook still prints them).
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(path) = LOG_PATH.get()
            && let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(path)
        {
            let _ = writeln!(f, "PANIC: {info}\n{}", std::backtrace::Backtrace::force_capture());
        }
        default(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_logs(name: &str) -> PathBuf {
        let logs = std::env::temp_dir().join(format!("eaw-remake-log-test-{}-{name}", std::process::id())).join("logs");
        let _ = std::fs::remove_dir_all(&logs);
        std::fs::create_dir_all(&logs).unwrap();
        logs
    }

    fn names(logs: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(logs).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn opens_a_log_file_and_keeps_only_the_newest() {
        let logs = scratch_logs("keeps");
        for i in 0..15 {
            std::fs::write(logs.join(format!("eaw-remake-{i:03}.log")), "x").unwrap();
        }
        let mut file = open_log_in(&logs).expect("log file");
        writeln!(file, "hello").unwrap();
        drop(file);
        let left = names(&logs);
        assert_eq!(left.len(), KEEP, "{left:?}");
        assert!(!left.iter().any(|n| n == "eaw-remake-005.log") && left.iter().any(|n| n == "eaw-remake-006.log"), "the 6 oldest go: {left:?}");
        let newest = left.iter().max_by_key(|n| log_stamp(Path::new(n))).unwrap();
        assert!(newest.starts_with("eaw-remake-"), "{newest}");
        assert_eq!(std::fs::read_to_string(logs.join(newest)).unwrap(), "hello\n");
        let _ = std::fs::remove_dir_all(logs.parent().unwrap());
    }

    #[test]
    fn logs_of_the_old_name_count_and_go_by_age_not_by_name() {
        let logs = scratch_logs("mixed");
        // Logs of the old name from before (100 to 102) and after (111 to 113) the logs of the current name (103 to 110). Sorted by name,
        // the current name comes first and the oldest logs of the old name would stay.
        for stamp in [100, 101, 102, 111, 112, 113] {
            std::fs::write(logs.join(format!("space-peace-{stamp}.log")), "old name").unwrap();
        }
        for stamp in 103..=110 {
            std::fs::write(logs.join(format!("eaw-remake-{stamp}.log")), "current name").unwrap();
        }
        open_log_in(&logs).expect("log file");
        let left = names(&logs);
        assert_eq!(left.len(), KEEP, "{left:?}");
        // The five oldest went, whatever their name.
        for gone in ["space-peace-100.log", "space-peace-101.log", "space-peace-102.log", "eaw-remake-103.log", "eaw-remake-104.log"] {
            assert!(!left.iter().any(|n| n == gone), "{gone} is among the 10 newest: {left:?}");
        }
        for kept in ["eaw-remake-105.log", "eaw-remake-110.log", "space-peace-111.log", "space-peace-113.log"] {
            assert!(left.iter().any(|n| n == kept), "{kept} was deleted: {left:?}");
        }
        let _ = std::fs::remove_dir_all(logs.parent().unwrap());
    }

    #[test]
    fn the_time_comes_from_the_name_of_either_kind_of_log() {
        assert_eq!(log_stamp(Path::new("eaw-remake-1760000000.log")), 1_760_000_000);
        assert_eq!(log_stamp(Path::new("space-peace-1759000000.log")), 1_759_000_000);
        assert_eq!(log_stamp(Path::new("notes.log")), 0);
    }
}
