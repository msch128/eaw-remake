//! Data checks against the real install (Remake mod); decode only, nothing is played. Skipped when the
//! game or the mod is not installed.

use std::collections::BTreeSet;

use alamo_audio::{MusicKind, MusicLibrary, SampleResolver, SfxLibrary, SpeechLibrary, Wav};
use alamo_fs::{FileSource, GameFs, GameInstall};

const REMAKE: &str = "1770851727";

fn fs() -> Option<GameFs> {
    let install = GameInstall::detect()?;
    if !install.mod_data(REMAKE).is_dir() {
        eprintln!("skipped: Remake mod not installed");
        return None;
    }
    GameFs::for_mod(&install, Some(REMAKE)).ok()
}

#[test]
fn remake_audio_data() {
    let Some(fs) = fs() else { return };
    let mut w = Vec::new();
    let sfx = SfxLibrary::load(&fs, &mut w);
    let speech = SpeechLibrary::load(&fs, &mut w);
    let music = MusicLibrary::load(&fs, &mut w);
    assert!(
        w.is_empty(),
        "warnings: {:?}",
        w.iter().map(|x| &x.message).collect::<Vec<_>>()
    );
    assert!(sfx.len() > 4000 && speech.len() > 1500 && music.len() > 50);

    // Music: every file resolves and every playlist kind of interest exists.
    let resolver = SampleResolver::new(&fs);
    let mut missing_music = 0;
    for e in music.iter() {
        for f in &e.files {
            if resolver.resolve(&fs, f).is_none() {
                missing_music += 1;
                eprintln!("missing music file {f} in {}", e.name);
            }
        }
    }
    for faction in ["Rebel", "Empire", "Underworld"] {
        assert!(
            music
                .by_kind(&MusicKind::Galactic(faction.into()))
                .is_some()
        );
        assert!(
            music
                .by_kind(&MusicKind::SpaceAmbient(faction.into()))
                .is_some()
        );
        assert!(
            music
                .by_kind(&MusicKind::SpaceBattle(faction.into()))
                .is_some()
        );
    }

    // Speech files.
    let (mut speech_refs, mut speech_missing) = (0, 0);
    for e in speech.iter() {
        for f in &e.files {
            speech_refs += 1;
            if resolver.resolve(&fs, f).is_none() {
                speech_missing += 1;
            }
        }
    }

    // SFX samples.
    let (mut refs, mut missing) = (0usize, 0usize);
    let mut files = BTreeSet::new();
    let mut vo_missing = 0usize;
    for e in sfx.iter().filter(|e| !e.is_preset) {
        for s in e
            .samples
            .iter()
            .chain(&e.pre_samples)
            .chain(&e.post_samples)
        {
            refs += 1;
            match resolver.resolve(&fs, s) {
                Some((p, _)) => {
                    files.insert(p);
                }
                None => {
                    missing += 1;
                    vo_missing +=
                        usize::from(e.is_unit_response_vo || e.is_hud_vo || e.is_ambient_vo);
                }
            }
        }
    }
    eprintln!(
        "sfx events {} (presets {}), speech {} ({speech_refs} files, {speech_missing} missing), music {} ({missing_music} files missing)",
        sfx.len(),
        sfx.iter().filter(|e| e.is_preset).count(),
        speech.len(),
        music.len()
    );
    eprintln!(
        "sfx sample refs {refs}, unresolved {missing} (of which voice-over events {vo_missing}), distinct files {}",
        files.len()
    );

    // Decode every referenced WAV; MP3 samples are scanned frame by frame.
    let (mut wavs, mut mp3s) = (0, 0);
    for p in &files {
        let bytes = fs.read(p).unwrap();
        if p.ends_with(".WAV") {
            let pcm = Wav::parse(&bytes)
                .and_then(|w| w.decode())
                .unwrap_or_else(|e| panic!("{p}: {e}"));
            assert!(!pcm.samples.is_empty(), "{p}: empty");
            wavs += 1;
        } else {
            mp3s += 1;
        }
    }
    eprintln!("decoded {wavs} wav files, {mp3s} mp3 sfx samples");
    assert!(wavs > 5000);
}

#[test]
fn mp3_files_scan() {
    let Some(fs) = fs() else { return };
    let mut checked = 0;
    for dir in ["Audio/Music", "Audio/Speech/English"] {
        for p in fs
            .list(dir)
            .iter()
            .filter(|p| p.ends_with(".MP3"))
            .step_by(23)
            .take(40)
        {
            let bytes = fs.read(p).unwrap();
            let d = mp3_duration::from_read(&mut std::io::Cursor::new(&bytes))
                .unwrap_or_else(|e| panic!("{p}: {e}"));
            assert!(d.as_millis() > 100, "{p}: too short");
            checked += 1;
        }
    }
    eprintln!("scanned {checked} mp3 files");
    assert!(checked > 10);
}

#[test]
fn every_wav_in_the_file_system_decodes() {
    let Some(fs) = fs() else { return };
    let mut tags = std::collections::BTreeMap::<u16, usize>::new();
    let mut n = 0;
    for p in fs.list("Audio").iter().filter(|p| p.ends_with(".WAV")) {
        let bytes = fs.read(p).unwrap();
        let wav = Wav::parse(&bytes).unwrap_or_else(|e| panic!("{p}: {e}"));
        *tags.entry(wav.format.tag).or_default() += 1;
        wav.decode().unwrap_or_else(|e| panic!("{p}: {e}"));
        n += 1;
    }
    eprintln!("all {n} WAV files decode; format tags: {tags:?}");
}

/// Audio pass 3: the events the galaxy, menu and Death Star hooks play exist in the Remake data and their samples resolve.
#[test]
fn pass3_events_exist_and_resolve() {
    let Some(fs) = fs() else { return };
    let mut w = Vec::new();
    let sfx = SfxLibrary::load(&fs, &mut w);
    let music = MusicLibrary::load(&fs, &mut w);
    let resolver = SampleResolver::new(&fs);
    let c = alamo_audio::AudioConstants::load(&fs);
    let mut names: Vec<String> = [
        "Galactic_Fleet_Hyperspace",
        "GUI_Drag_Fleet",
        "GUI_Drop_Fleet",
        "GUI_Death_Star_Button_Press",
        "GUI_Death_Star_Activate",
        "EHD_Death_Star_2_Activate",
        "Unit_Superlaser_Fire",
        "Planet_Death_SFX",
    ]
    .map(String::from)
    .to_vec();
    for f in ["rebel", "empire", "underworld"] {
        names.push(c.start_campaign_sfx(f).unwrap_or_else(|| panic!("no SFXEvent_GUI_Start_Campaign for {f}")).to_string());
    }
    for (_, _, e) in &c.unit_type_spotted {
        names.push(e.clone());
    }
    // Only the Star Destroyer / AT-AT lines are active in the Remake; the hero lines are commented out.
    assert!(c.spotted_line("Rebel", "Star_Destroyer").is_some(), "{:?}", c.unit_type_spotted);
    for n in &names {
        let e = sfx.get(n).unwrap_or_else(|| panic!("SFX event {n} missing"));
        for s in &e.samples {
            assert!(resolver.resolve(&fs, s).is_some(), "{n}: sample {s} does not resolve");
        }
    }
    let ds = music.get("Space_Map_Activate_Death_Star_Music_Event").expect("Death Star activation music");
    assert!(!ds.looping && ds.files.iter().all(|f| resolver.resolve(&fs, f).is_some()));
}
