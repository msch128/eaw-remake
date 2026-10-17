//! Debug aid for frame spikes (`SP_FRAME_LOG=<ms>`): one-off work on the main thread (model builds, texture
//! decodes, particle and animation parses) leaves a note here, and the battle view prints the notes of every
//! frame that took longer than the limit. Without the variable nothing is recorded.

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

static NOTES: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Notes kept between two [`take`] calls.
const MAX_NOTES: usize = 256;

/// True when `SP_FRAME_LOG` is set.
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SP_FRAME_LOG").is_some())
}

/// Records a note for the current frame (only when [`enabled`]).
pub fn note(text: impl FnOnce() -> String) {
    if enabled() {
        let mut notes = NOTES.lock().unwrap_or_else(|e| e.into_inner());
        // Nobody may be taking them (no battle view in the app): keep the most recent ones only.
        if notes.len() >= MAX_NOTES {
            notes.remove(0);
        }
        notes.push(text());
    }
}

/// Records `what` with its duration if it took at least `min`.
pub fn note_slow(what: &str, took: Duration, min: Duration) {
    if took >= min {
        note(|| format!("{what} {took:.1?}"));
    }
}

/// Takes the notes recorded since the last call.
pub fn take() -> Vec<String> {
    std::mem::take(&mut *NOTES.lock().unwrap_or_else(|e| e.into_inner()))
}
