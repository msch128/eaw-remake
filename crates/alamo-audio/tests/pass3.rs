//! Audio pass 3 (silent): one-shot music events over the galaxy map (Death Star activation theme), the per-bus balance
//! log line and the faction "unit type spotted" lines. Uses an in-memory file system like `pass2.rs`.

use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use alamo_audio::plugin::{Bus, MusicTrack, balance_line};
use alamo_audio::{AlamoAudioPlugin, MusicDirector, MusicScene, Pcm};
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
        let mut v: Vec<String> = self.0.keys().filter(|k| k.starts_with(&p)).cloned().collect();
        v.sort();
        v
    }
}

fn fs() -> Arc<MemFs> {
    let tone = Pcm { channels: 1, sample_rate: 8000, samples: (0..800).map(|i| ((i % 40) * 100) as i16).collect() }.to_wav_bytes();
    let mut m = HashMap::new();
    for f in ["map", "ds"] {
        m.insert(normalize(&format!("Audio/Music/{f}.wav")), tone.clone());
    }
    m.insert(
        normalize("Xml/MusicEvents.xml"),
        br#"<MusicEvents>
        <MusicEvent Name="Galactic_Map_Rebel_Music_Event"><Files>map.wav</Files><Volume_Percent>100</Volume_Percent><Fade_In_Seconds>1</Fade_In_Seconds><Fade_Out_Previous_Seconds>1</Fade_Out_Previous_Seconds><Loop>Yes</Loop></MusicEvent>
        <MusicEvent Name="Space_Map_Activate_Death_Star_Music_Event"><Files>ds.wav</Files><Volume_Percent>100</Volume_Percent><Fade_In_Seconds>0.1</Fade_In_Seconds><Fade_Out_Previous_Seconds>0.25</Fade_Out_Previous_Seconds><Loop>No</Loop></MusicEvent>
        </MusicEvents>"#
            .to_vec(),
    );
    m.insert(
        normalize("Xml/Factions.xml"),
        b"<Factions><Faction Name=\"Rebel\"><SFXEvent_Unit_Type_Spotted>Darth_Vader, Unit_Announce_Vader_Rebel_Soldier</SFXEvent_Unit_Type_Spotted></Faction></Factions>".to_vec(),
    );
    Arc::new(MemFs(m))
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default())).init_asset::<AudioSource>().add_plugins(AlamoAudioPlugin::new(fs()));
    // Samples are read on the spot: these tests count voices frame by frame (`plugin.rs` tests the worker reads).
    app.world_mut().resource_mut::<alamo_audio::AudioSettings>().load_on_worker = false;
    app
}

fn alive(app: &mut App) -> Vec<String> {
    let mut q = app.world_mut().query::<&MusicTrack>();
    let mut v: Vec<_> = q.iter(app.world()).filter(|t| !t.is_outgoing()).map(|t| t.event.clone()).collect();
    v.sort();
    v
}

#[test]
fn death_star_theme_plays_once_over_the_map_and_the_map_music_returns() {
    let mut app = app();
    app.world_mut().resource_mut::<MusicDirector>().set_scene(MusicScene::Galactic("Rebel".into()));
    app.update();
    assert_eq!(alive(&mut app), ["Galactic_Map_Rebel_Music_Event"]);
    app.world_mut().resource_mut::<MusicDirector>().play_once_then("Space_Map_Activate_Death_Star_Music_Event", MusicScene::Galactic("Rebel".into()));
    app.update();
    app.update();
    assert_eq!(alive(&mut app), ["Space_Map_Activate_Death_Star_Music_Event"]);
    let mut q = app.world_mut().query::<&mut MusicTrack>();
    for mut t in q.iter_mut(app.world_mut()) {
        if !t.is_outgoing() {
            t.end_now();
        }
    }
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(app.world().resource::<MusicDirector>().current_event(), Some("Galactic_Map_Rebel_Music_Event"));
    assert_eq!(alive(&mut app), ["Galactic_Map_Rebel_Music_Event"]);
}

#[test]
fn balance_line_reports_every_bus() {
    let line = balance_line(&[(Bus::Music, 0.6), (Bus::Speech, 0.8), (Bus::Speech, 0.4), (Bus::Sfx, 0.1)]);
    assert_eq!(
        line,
        "audio balance music[n=1 peak=0.60 avg=0.60] speech[n=2 peak=0.80 avg=0.60] sfx[n=1 peak=0.10 avg=0.10] ambient[n=0 peak=0.00 avg=0.00]"
    );
    // The pattern the smoke test uses.
    let re_ok = line.starts_with("audio balance music[n=") && line.contains(" ambient[n=");
    assert!(re_ok);
}

#[test]
fn spotted_lines_load_from_factions_xml() {
    let c = alamo_audio::AudioConstants::load(fs().as_ref());
    assert_eq!(c.spotted_line("Rebel", "Darth_Vader"), Some("Unit_Announce_Vader_Rebel_Soldier"));
}
