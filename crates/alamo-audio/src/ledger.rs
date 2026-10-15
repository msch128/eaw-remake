//! Sound ledger: every sound the plugin was asked to play, with what became of it.
//!
//! Silent runs (`--volume 0`, offscreen, tests) can not be listened to, so the plugin writes down each SFX request, voice
//! start, music track and file request together with the resolved file and whether it exists in the game file system.
//! Problems (unknown event, event without samples, missing or undecodable file) are never dropped silently: they are
//! counted here and logged once per name. `SP_AUDIO_LEDGER=<path>` writes the whole ledger as TSV when the app exits.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Which subsystem produced an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LedgerKind {
    /// An SFX event request (`PlaySfx`).
    Sfx,
    /// A music track of a `MusicEvent`.
    Music,
    /// A direct file request (`PlayFile`).
    File,
    /// A speech event (`PlaySpeech`).
    Speech,
}

/// What became of a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LedgerOutcome {
    /// A voice / track was started from an existing file.
    Played,
    /// The event name is not defined in the data.
    UnknownEvent,
    /// The event is defined but names no sample / file.
    NoSamples,
    /// The sample is named but not found in the file system.
    MissingFile,
    /// The file exists but could not be decoded.
    BadFile,
    /// A rule of the data dropped the request (probability, overlap group, max instances of a voice-over, cooldown); the reason is in `sample`.
    Dropped,
}

impl LedgerOutcome {
    /// True for outcomes that mean the data promised a sound that did not play.
    pub fn is_problem(self) -> bool {
        !matches!(self, Self::Played | Self::Dropped)
    }
}

/// One ledger row.
#[derive(Debug, Clone)]
pub struct LedgerEntry {
    /// App time in seconds.
    pub t: f64,
    /// Source of the request.
    pub kind: LedgerKind,
    /// SFX / music event name (the file name for `LedgerKind::File`).
    pub event: String,
    /// Sample as named in the data (empty when none was chosen).
    pub sample: String,
    /// Resolved path in the game file system, when found.
    pub path: Option<String>,
    /// Result.
    pub outcome: LedgerOutcome,
}

/// All entries of a run. Recording is off unless `SP_AUDIO_LEDGER` or `SP_AUDIO_LOG` is set (or [`SoundLedger::enabled`] is set by a test).
#[derive(Debug, Default, bevy::prelude::Resource)]
pub struct SoundLedger {
    /// Whether [`SoundLedger::record`] stores entries.
    pub enabled: bool,
    /// Entries in request order.
    pub entries: Vec<LedgerEntry>,
    /// Problems already logged (each distinct one is logged once, recording or not).
    warned: std::collections::HashSet<(LedgerKind, String, String, LedgerOutcome)>,
}

/// Cap on stored entries; a long soak keeps the first ones and counts nothing more (problems are logged when they happen).
const MAX_ENTRIES: usize = 400_000;

impl SoundLedger {
    /// A ledger that records when `enabled`.
    pub fn new(enabled: bool) -> Self {
        Self { enabled, ..Self::default() }
    }

    /// A ledger that records.
    pub fn recording() -> Self {
        Self { enabled: true, ..Self::default() }
    }

    /// Adds an entry (stored only when recording). A problem is logged as a warning the first time it happens.
    pub fn record(&mut self, entry: LedgerEntry) {
        if entry.outcome.is_problem()
            && self.warned.insert((entry.kind, entry.event.to_ascii_lowercase(), entry.sample.clone(), entry.outcome))
        {
            bevy::log::warn!(
                "audio problem: {:?} {:?} event `{}` sample `{}` path {:?}",
                entry.kind,
                entry.outcome,
                entry.event,
                entry.sample,
                entry.path
            );
        }
        if self.enabled && self.entries.len() < MAX_ENTRIES {
            self.entries.push(entry);
        }
    }

    /// Entries that are problems, de-duplicated by (kind, event, sample, outcome) with a count.
    pub fn problems(&self) -> Vec<(LedgerEntry, usize)> {
        let mut map: BTreeMap<(LedgerKind, String, String, LedgerOutcome), (LedgerEntry, usize)> = BTreeMap::new();
        for e in self.entries.iter().filter(|e| e.outcome.is_problem()) {
            map.entry((e.kind, e.event.to_ascii_lowercase(), e.sample.clone(), e.outcome))
                .and_modify(|v| v.1 += 1)
                .or_insert((e.clone(), 1));
        }
        map.into_values().collect()
    }

    /// Distinct events that played at least once, with their play count, by kind.
    pub fn played(&self, kind: LedgerKind) -> BTreeMap<String, usize> {
        let mut m = BTreeMap::new();
        for e in self.entries.iter().filter(|e| e.kind == kind && e.outcome == LedgerOutcome::Played) {
            *m.entry(e.event.clone()).or_default() += 1;
        }
        m
    }

    /// TSV dump: `t kind outcome event sample path`.
    pub fn to_tsv(&self) -> String {
        let mut s = String::from("t\tkind\toutcome\tevent\tsample\tpath\n");
        for e in &self.entries {
            let _ = writeln!(
                s,
                "{:.2}\t{:?}\t{:?}\t{}\t{}\t{}",
                e.t,
                e.kind,
                e.outcome,
                e.event,
                e.sample,
                e.path.as_deref().unwrap_or("-")
            );
        }
        s
    }

    /// Human summary: played counts per kind and every distinct problem.
    pub fn summary(&self) -> String {
        let mut s = String::new();
        for kind in [LedgerKind::Sfx, LedgerKind::Music, LedgerKind::File, LedgerKind::Speech] {
            let p = self.played(kind);
            let _ = writeln!(s, "{kind:?}: {} distinct events played, {} plays", p.len(), p.values().sum::<usize>());
        }
        let dropped = self.entries.iter().filter(|e| e.outcome == LedgerOutcome::Dropped).count();
        let _ = writeln!(s, "dropped by data rules (probability, overlap group, voice-over instance limit): {dropped}");
        let problems = self.problems();
        let _ = writeln!(s, "problems: {}", problems.len());
        for (e, n) in problems {
            let _ = writeln!(s, "  {:?} {:?} event `{}` sample `{}` x{n}", e.kind, e.outcome, e.event, e.sample);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(outcome: LedgerOutcome, event: &str) -> LedgerEntry {
        LedgerEntry { t: 0.0, kind: LedgerKind::Sfx, event: event.into(), sample: "A.WAV".into(), path: None, outcome }
    }

    #[test]
    fn disabled_ledger_records_nothing() {
        let mut l = SoundLedger::default();
        l.record(entry(LedgerOutcome::Played, "x"));
        assert!(l.entries.is_empty());
    }

    #[test]
    fn problems_are_deduplicated_and_counted() {
        let mut l = SoundLedger::recording();
        l.record(entry(LedgerOutcome::MissingFile, "Boom"));
        l.record(entry(LedgerOutcome::MissingFile, "boom"));
        l.record(entry(LedgerOutcome::Played, "ok"));
        let p = l.problems();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].1, 2);
        assert_eq!(l.played(LedgerKind::Sfx).get("ok"), Some(&1));
        assert!(l.summary().contains("MissingFile"));
    }
}
