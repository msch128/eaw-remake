//! Music events named by `Factions.xml` (silent, in-memory file system): the battle load screen track of the faction and the
//! win / lose themes per opponent (`Music_Event_Tactical_*_Vs_Faction`), with the planner's naming convention as fallback
//! when a named event has no file in the install (the Remake's `Lose_To_Zann_1.MP3` is absent).

use std::collections::HashMap;
use std::io;
use std::sync::Arc;

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
        self.0.keys().filter(|k| k.starts_with(&p)).cloned().collect()
    }
}

fn app() -> App {
    let tone = Pcm { channels: 1, sample_rate: 8000, samples: (0..800).map(|i| ((i % 40) * 100) as i16).collect() }.to_wav_bytes();
    let mut m = HashMap::new();
    for f in ["load1", "load2", "lose_rebel", "lose_empire", "map"] {
        m.insert(normalize(&format!("Audio/Music/{f}.wav")), tone.clone());
    }
    let ev = |name: &str, file: &str| format!("<MusicEvent Name=\"{name}\"><Files>{file}</Files><Volume_Percent>100</Volume_Percent><Loop>No</Loop></MusicEvent>");
    m.insert(
        normalize("Xml/MusicEvents.xml"),
        format!(
            "<MusicEvents>{}{}{}{}{}{}</MusicEvents>",
            ev("Tactical_Battle_Loading_Loop_Event", "load1.wav"),
            ev("Tactical_Battle_Loading_Loop_Event_2", "load2.wav"),
            ev("Rebel_Lose_Tactical_Event", "lose_rebel.wav"),
            ev("Lose_To_Empire_Event", "lose_empire.wav"),
            ev("Lose_To_Underworld_Event", "lose_zann_not_installed.wav"),
            ev("Galactic_Map_Rebel_Music_Event", "map.wav"),
        )
        .into_bytes(),
    );
    m.insert(
        normalize("Xml/Factions.xml"),
        br#"<Factions><Faction Name="Rebel">
            <Music_Event_Battle_Load_Screen> Tactical_Battle_Loading_Loop_Event_2 </Music_Event_Battle_Load_Screen>
            <Music_Event_Tactical_Lose_Vs_Faction> Empire,	 Lose_To_Empire_Event </Music_Event_Tactical_Lose_Vs_Faction>
            <Music_Event_Tactical_Lose_Vs_Faction> Underworld, Lose_To_Underworld_Event </Music_Event_Tactical_Lose_Vs_Faction>
            <Music_Event_Tactical_Lose> Rebel_Lose_Tactical_Event </Music_Event_Tactical_Lose>
            </Faction></Factions>"#
            .to_vec(),
    );
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default())).init_asset::<AudioSource>().add_plugins(AlamoAudioPlugin::new(Arc::new(MemFs(m))));
    // Samples are read on the spot: these tests count voices frame by frame (`plugin.rs` tests the worker reads).
    app.world_mut().resource_mut::<alamo_audio::AudioSettings>().load_on_worker = false;
    app
}

fn current(app: &mut App) -> Option<String> {
    app.update();
    app.world().resource::<MusicDirector>().current_event().map(str::to_owned)
}

fn lose_vs(opponent: &str) -> Option<String> {
    let mut app = app();
    let mut d = app.world_mut().resource_mut::<MusicDirector>();
    d.set_opponent(Some(opponent.into()));
    d.set_scene(MusicScene::Result { faction: "Rebel".into(), win: false, tactical: true });
    current(&mut app)
}

#[test]
fn battle_load_screen_uses_the_music_of_the_factions_file() {
    let mut app = app();
    app.world_mut().resource_mut::<MusicDirector>().set_scene(MusicScene::Galactic("Rebel".into()));
    current(&mut app);
    app.world_mut().resource_mut::<MusicDirector>().set_scene(MusicScene::BattleLoading);
    assert_eq!(current(&mut app).as_deref(), Some("Tactical_Battle_Loading_Loop_Event_2"));
}

#[test]
fn defeat_theme_depends_on_the_opponent() {
    assert_eq!(lose_vs("Empire").as_deref(), Some("Lose_To_Empire_Event"));
    assert_eq!(lose_vs("empire").as_deref(), Some("Lose_To_Empire_Event"));
    // Named, but the file is not installed: the plain defeat theme plays instead of silence.
    assert_eq!(lose_vs("Underworld").as_deref(), Some("Rebel_Lose_Tactical_Event"));
    // Not listed: the plain defeat theme.
    assert_eq!(lose_vs("Pirates").as_deref(), Some("Rebel_Lose_Tactical_Event"));
}
