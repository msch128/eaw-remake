//! Headless tests of the Bevy plugin with an in-memory file system. No audio output plugin is added, so
//! nothing is played; the tests only inspect the entities the plugin spawns.
#![cfg(feature = "bevy")]

use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use alamo_audio::plugin::{MusicTrack, SfxVoice};
use alamo_audio::{AlamoAudioPlugin, MusicDirector, MusicScene, Pcm, PlaySfx};
use alamo_fs::{FileSource, normalize};
use bevy::asset::AssetPlugin;
use bevy::audio::AudioSource;
use bevy::prelude::*;

struct MemFs(HashMap<String, Vec<u8>>);

impl FileSource for MemFs {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        self.0
            .get(&normalize(path))
            .cloned()
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }
    fn exists(&self, path: &str) -> bool {
        self.0.contains_key(&normalize(path))
    }
    fn list(&self, dir: &str) -> Vec<String> {
        let p = normalize(dir);
        let mut v: Vec<String> = self
            .0
            .keys()
            .filter(|k| k.starts_with(&p))
            .cloned()
            .collect();
        v.sort();
        v
    }
}

fn tone() -> Vec<u8> {
    Pcm {
        channels: 1,
        sample_rate: 8000,
        samples: (0..800).map(|i| ((i % 40) * 100) as i16).collect(),
    }
    .to_wav_bytes()
}

fn fs() -> Arc<MemFs> {
    let mut m = HashMap::new();
    let mut put = |p: &str, d: Vec<u8>| {
        m.insert(normalize(p), d);
    };
    put(
        "Xml/SFXEventFiles.xml",
        b"<SFXEvent_Files><File>SFXEvents.xml</File></SFXEvent_Files>".to_vec(),
    );
    put(
        "Xml/SFXEvents.xml",
        br#"<SFXEvents>
        <SFXEvent Name="Preset_VO"><Is_Preset>Yes</Is_Preset><Is_2D>Yes</Is_2D><Is_Unit_Response_VO>Yes</Is_Unit_Response_VO><Priority>1</Priority><Max_Instances>1</Max_Instances></SFXEvent>
        <SFXEvent Name="Boom"><Samples>Data\Audio\SFX\boom_a.wav Data\Audio\SFX\boom_b.wav</Samples><Max_Instances>1</Max_Instances><Is_2D>Yes</Is_2D></SFXEvent>
        <SFXEvent Name="Never"><Samples>boom_a.wav</Samples><Probability>0</Probability></SFXEvent>
        <SFXEvent Name="Missing"><Samples>nope.wav</Samples></SFXEvent>
        <SFXEvent Name="Unit_Select_A"><Use_Preset>Preset_VO</Use_Preset><Samples>boom_a.wav</Samples><Overlap_Test>va</Overlap_Test></SFXEvent>
        <SFXEvent Name="Unit_Select_B"><Use_Preset>Preset_VO</Use_Preset><Samples>boom_b.wav</Samples><Overlap_Test>vb</Overlap_Test></SFXEvent>
        <SFXEvent Name="Chain_1"><Samples>boom_a.wav</Samples><Is_2D>Yes</Is_2D><Chained_SFXEvent>Boom</Chained_SFXEvent></SFXEvent>
        </SFXEvents>"#
            .to_vec(),
    );
    put("Audio/SFX/boom_a.wav", tone());
    put("Audio/SFX/boom_b.wav", tone());
    put("Audio/Music/rebel1.wav", tone());
    put("Audio/Music/rebel2.wav", tone());
    put("Audio/Music/empire_amb.wav", tone());
    put("Audio/Music/empire_bat.wav", tone());
    put(
        "Xml/MusicEvents.xml",
        br#"<MusicEvents>
        <MusicEvent Name="Galactic_Map_Rebel_Music_Event"><Files>rebel1.wav, rebel2.wav</Files><Volume_Percent>100</Volume_Percent><Fade_In_Seconds>0</Fade_In_Seconds><Fade_Out_Previous_Seconds>1</Fade_Out_Previous_Seconds><Loop>Yes</Loop></MusicEvent>
        <MusicEvent Name="Space_Map_Empire_Ambient_Music_Event"><Files>empire_amb.wav</Files><Volume_Percent>80</Volume_Percent><Loop>Yes</Loop></MusicEvent>
        <MusicEvent Name="Space_Map_Empire_Battle_Music_Event"><Files>empire_bat.wav</Files><Volume_Percent>90</Volume_Percent><Loop>Yes</Loop></MusicEvent>
        </MusicEvents>"#
            .to_vec(),
    );
    Arc::new(MemFs(m))
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<AudioSource>()
        .add_plugins(AlamoAudioPlugin::new(fs()));
    app
}

fn play(app: &mut App, event: &str) {
    app.world_mut().write_message(PlaySfx {
        event: event.into(),
        position: None,
        source: None,
    });
}

fn voices(app: &mut App) -> Vec<String> {
    let mut q = app.world_mut().query::<&SfxVoice>();
    let n = q.iter(app.world()).count();
    (0..n).map(|i| format!("voice{i}")).collect()
}

#[test]
fn plays_and_limits() {
    let mut app = app();
    play(&mut app, "Boom");
    app.update();
    assert_eq!(voices(&mut app).len(), 1);
    // Max_Instances 1 on a normal SFX: the old instance is cut, one voice remains.
    play(&mut app, "boom");
    app.update();
    assert_eq!(voices(&mut app).len(), 1);
    // Unknown event, probability 0, and missing sample play nothing.
    for e in ["DoesNotExist", "Never", "Missing", "Preset_VO"] {
        play(&mut app, e);
    }
    app.update();
    assert_eq!(voices(&mut app).len(), 1);
}

#[test]
fn unit_responses_do_not_spam() {
    let mut app = app();
    play(&mut app, "Unit_Select_A");
    play(&mut app, "Unit_Select_B");
    app.update();
    assert_eq!(
        voices(&mut app).len(),
        1,
        "global speech cooldown must drop the second line"
    );
    play(&mut app, "Unit_Select_A");
    app.update();
    assert_eq!(
        voices(&mut app).len(),
        1,
        "same event inside the cooldown is dropped"
    );
}

#[test]
fn music_director_switches_playlists() {
    let mut app = app();
    app.world_mut()
        .resource_mut::<MusicDirector>()
        .set_scene(MusicScene::Galactic("Rebel".into()));
    app.update();
    let mut q = app.world_mut().query::<&MusicTrack>();
    let events: Vec<String> = q.iter(app.world()).map(|t| t.event.clone()).collect();
    assert_eq!(events, vec!["Galactic_Map_Rebel_Music_Event"]);

    // Space battle: starts with the ambient playlist, switches to battle when intensity rises.
    {
        let mut d = app.world_mut().resource_mut::<MusicDirector>();
        d.set_scene(MusicScene::Space("Empire".into()));
    }
    app.update();
    assert_eq!(
        app.world().resource::<MusicDirector>().current_event(),
        Some("Space_Map_Empire_Ambient_Music_Event")
    );
    app.world_mut()
        .resource_mut::<MusicDirector>()
        .set_battle_intensity(0.9);
    app.update();
    assert_eq!(
        app.world().resource::<MusicDirector>().current_event(),
        Some("Space_Map_Empire_Battle_Music_Event")
    );
}
