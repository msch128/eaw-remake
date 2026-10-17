//! The sound ledger (silent): every request is accounted for; unknown events, sample-less events and missing files are
//! reported instead of dropped, and a missing sample of a multi-sample event is never picked.

use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use alamo_audio::ledger::{LedgerKind, LedgerOutcome, SoundLedger};
use alamo_audio::{AlamoAudioPlugin, Pcm, PlaySfx};
use alamo_fs::{FileSource, normalize};
use bevy::asset::AssetPlugin;
use bevy::audio::AudioSource;
use bevy::prelude::*;

struct MemFs(HashMap<String, Vec<u8>>);

impl FileSource for MemFs {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        self.0.get(&normalize(path)).cloned().ok_or_else(|| io::ErrorKind::NotFound.into())
    }
    fn exists(&self, path: &str) -> bool {
        self.0.contains_key(&normalize(path))
    }
    fn list(&self, dir: &str) -> Vec<String> {
        let p = normalize(dir);
        self.0.keys().filter(|k| k.starts_with(&p)).cloned().collect()
    }
}

fn app() -> App {
    let tone = Pcm { channels: 1, sample_rate: 8000, samples: (0..800).map(|i| ((i % 40) * 100) as i16).collect() }.to_wav_bytes();
    let mut m = HashMap::new();
    let mut put = |p: &str, d: Vec<u8>| {
        m.insert(normalize(p), d);
    };
    put("Xml/SFXEventFiles.xml", b"<SFXEvent_Files><File>SFXEvents.xml</File></SFXEvent_Files>".to_vec());
    put(
        "Xml/SFXEvents.xml",
        br#"<SFXEvents>
        <SFXEvent Name="Good"><Samples>a.wav</Samples></SFXEvent>
        <SFXEvent Name="Half"><Samples>gone.wav, a.wav</Samples></SFXEvent>
        <SFXEvent Name="AllGone"><Samples>gone.wav</Samples></SFXEvent>
        <SFXEvent Name="Empty"><Samples></Samples></SFXEvent>
        </SFXEvents>"#
            .to_vec(),
    );
    put("Audio/SFX/a.wav", tone);
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<AudioSource>()
        .add_plugins(AlamoAudioPlugin::new(Arc::new(MemFs(m))));
    // Samples are read on the spot: these tests count voices frame by frame (`plugin.rs` tests the worker reads).
    app.world_mut().resource_mut::<alamo_audio::AudioSettings>().load_on_worker = false;
    app.world_mut().resource_mut::<SoundLedger>().enabled = true;
    app
}

fn play(app: &mut App, event: &str) {
    app.world_mut().write_message(PlaySfx { event: event.into(), position: None, source: None });
    app.update();
}

#[test]
fn every_outcome_is_recorded() {
    let mut app = app();
    for e in ["Good", "Nope", "Empty", "AllGone"] {
        play(&mut app, e);
    }
    let l = app.world().resource::<SoundLedger>();
    let outcome = |ev: &str| l.entries.iter().find(|e| e.event == ev && e.kind == LedgerKind::Sfx).map(|e| e.outcome);
    assert_eq!(outcome("Good"), Some(LedgerOutcome::Played));
    assert_eq!(outcome("Nope"), Some(LedgerOutcome::UnknownEvent));
    assert_eq!(outcome("Empty"), Some(LedgerOutcome::NoSamples));
    assert_eq!(outcome("AllGone"), Some(LedgerOutcome::MissingFile));
    let played = l.entries.iter().find(|e| e.event == "Good").unwrap();
    assert!(played.path.as_deref().is_some_and(|p| p.ends_with("A.WAV")), "{:?}", played.path);
    assert!(l.problems().len() >= 3);
    assert!(l.to_tsv().contains("UnknownEvent\tNope"));
}

#[test]
fn a_missing_sample_of_a_multi_sample_event_is_never_picked() {
    let mut app = app();
    for _ in 0..30 {
        play(&mut app, "Half");
    }
    let l = app.world().resource::<SoundLedger>();
    let played = l.entries.iter().filter(|e| e.event == "Half" && e.outcome == LedgerOutcome::Played).count();
    assert!(played > 0, "Half never played");
    assert!(
        l.entries.iter().filter(|e| e.event == "Half" && e.outcome == LedgerOutcome::Played).all(|e| e.sample == "a.wav"),
        "picked the missing sample"
    );
    assert!(l.entries.iter().any(|e| e.event == "Half" && e.sample == "gone.wav" && e.outcome == LedgerOutcome::MissingFile));
}
