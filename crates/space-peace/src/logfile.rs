//! Log file for live tests: everything the app logs (and panics) is also written to
//! `%APPDATA%\SpacePeace\logs\space-peace-<unix time>.log`; the 10 newest files are kept.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use bevy::log::tracing_subscriber::{self, Layer};
use bevy::log::BoxedLayer;
use bevy::prelude::*;

/// Number of log files kept.
const KEEP: usize = 10;

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// `%APPDATA%\SpacePeace\logs`, `None` when `%APPDATA%` is not set.
pub fn logs_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("SpacePeace").join("logs"))
}

/// Opens a new log file and removes the oldest ones.
fn open_log() -> Option<File> {
    let dir = logs_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let mut old: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "log"))
        .collect();
    old.sort();
    while old.len() >= KEEP {
        let _ = std::fs::remove_file(old.remove(0));
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let path = dir.join(format!("space-peace-{stamp}.log"));
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

    #[test]
    fn opens_a_log_file_and_keeps_only_the_newest() {
        let dir = std::env::temp_dir().join(format!("space-peace-log-test-{}", std::process::id()));
        // SAFETY: the test binary runs this single test that touches APPDATA.
        unsafe { std::env::set_var("APPDATA", &dir) };
        let logs = dir.join("SpacePeace").join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        for i in 0..15 {
            std::fs::write(logs.join(format!("space-peace-{i:03}.log")), "x").unwrap();
        }
        let mut file = open_log().expect("log file");
        writeln!(file, "hello").unwrap();
        let count = std::fs::read_dir(&logs).unwrap().count();
        assert_eq!(count, KEEP);
        assert!(LOG_PATH.get().unwrap().exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
