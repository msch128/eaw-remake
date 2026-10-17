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
        <SFXEvent Name="Unit_Move_A"><Use_Preset>Preset_VO</Use_Preset><Samples>boom_b.wav</Samples><Overlap_Test>va</Overlap_Test></SFXEvent>
        <SFXEvent Name="Unit_Select_B"><Use_Preset>Preset_VO</Use_Preset><Samples>boom_b.wav</Samples><Overlap_Test>vb</Overlap_Test></SFXEvent>
        <SFXEvent Name="Chain_1"><Samples>boom_a.wav</Samples><Is_2D>Yes</Is_2D><Chained_SFXEvent>Boom</Chained_SFXEvent></SFXEvent>
        </SFXEvents>"#
            .to_vec(),
    );
    put("Xml/Audio.xml", b"<Audio><Music_Space_Battle_To_Ambient_Peace_Seconds> 7 </Music_Space_Battle_To_Ambient_Peace_Seconds><!-- <Delay_Between_Space_Base_Attack_Announcement_Seconds>1</Delay_Between_Space_Base_Attack_Announcement_Seconds> --></Audio>".to_vec());
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

/// A file system on which reading `.1` takes half a second, like a cold file on a busy disk.
struct SlowFs(Arc<MemFs>, &'static str);

impl FileSource for SlowFs {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        if normalize(path) == normalize(self.1) {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        self.0.read(path)
    }
    fn exists(&self, path: &str) -> bool {
        self.0.exists(path)
    }
    fn list(&self, dir: &str) -> Vec<String> {
        self.0.list(dir)
    }
}

/// The plugin as the game runs it, on `fs`: samples are read and decoded on worker threads.
fn worker_app_on(fs: Arc<dyn FileSource>) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<AudioSource>()
        .add_plugins(AlamoAudioPlugin::new(fs));
    app
}

fn worker_app() -> App {
    worker_app_on(fs())
}

fn app() -> App {
    let mut app = worker_app();
    // Samples are read on the spot: most tests count voices frame by frame (the worker reads have tests of their own).
    app.world_mut().resource_mut::<alamo_audio::AudioSettings>().load_on_worker = false;
    app
}

/// Updates until `done` holds (a worker thread needs a moment), at most five seconds.
fn update_until(app: &mut App, mut done: impl FnMut(&mut App) -> bool) -> bool {
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(5) {
        app.update();
        if done(app) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    false
}

fn play(app: &mut App, event: &str) {
    app.world_mut().write_message(PlaySfx {
        event: event.into(),
        position: None,
        source: None,
    });
}

fn voices(app: &mut App) -> Vec<String> {
    // A cut voice fades out for a moment before it is despawned: only voices that are not stopping count.
    let mut q = app.world_mut().query::<&SfxVoice>();
    let n = q.iter(app.world()).filter(|v| !v.is_stopping()).count();
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
    // The cut voice is still there, fading out (no click), and is flagged as stopping.
    let mut all = app.world_mut().query::<&SfxVoice>();
    assert_eq!(all.iter(app.world()).count(), 2);
    // Unknown event, probability 0, and missing sample play nothing.
    for e in ["DoesNotExist", "Never", "Missing", "Preset_VO"] {
        play(&mut app, e);
    }
    app.update();
    assert_eq!(voices(&mut app).len(), 1);
}

fn voice_events(app: &mut App) -> Vec<String> {
    let mut q = app.world_mut().query::<&SfxVoice>();
    let mut v: Vec<String> = q.iter(app.world()).filter(|v| !v.is_stopping()).map(|v| v.event().to_string()).collect();
    v.sort();
    v
}

#[test]
fn move_line_plays_right_after_select_line() {
    // EaW answers a move order with the move line even while the select line is still playing: the newer
    // unit response cuts the older one (this used to be dropped by a 0.6 s global cooldown).
    let mut app = app();
    play(&mut app, "Unit_Select_A");
    app.update();
    assert_eq!(voice_events(&mut app), vec!["unit_select_a"]);
    play(&mut app, "Unit_Move_A");
    app.update();
    assert_eq!(voice_events(&mut app), vec!["unit_move_a"]);
}

#[test]
fn unit_responses_do_not_talk_over_each_other() {
    let mut app = app();
    play(&mut app, "Unit_Select_A");
    play(&mut app, "Unit_Select_B");
    app.update();
    assert_eq!(voice_events(&mut app), vec!["unit_select_b"], "one unit response at a time");
}

#[test]
fn same_response_is_not_stacked() {
    // Preset_UR has Max_Instances 1: a repeated select while it plays is dropped, not layered.
    let mut app = app();
    play(&mut app, "Unit_Select_A");
    app.update();
    play(&mut app, "Unit_Select_A");
    app.update();
    assert_eq!(voice_events(&mut app), vec!["unit_select_a"]);
}

#[test]
fn battle_music_peace_delay_comes_from_audio_xml() {
    let app = app();
    assert_eq!(app.world().resource::<MusicDirector>().planner.calm_seconds, 7.0);
    let c = &app.world().resource::<alamo_audio::plugin::AlamoAudio>().constants;
    assert_eq!(c.space_battle_to_ambient_seconds, 7.0);
    // The commented-out tag is ignored.
    assert_eq!(c.space_base_attack_announcement_delay, 60.0);
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

#[test]
fn volume_categories_map_to_percent() {
    use alamo_audio::{AudioBuses, Bus, VolumeCategory};
    let mut b = AudioBuses::default();
    b.set_percent(VolumeCategory::Master, 50.0);
    b.set_percent(VolumeCategory::Sfx, 200.0);
    assert_eq!(b.percent(VolumeCategory::Sfx), 100.0);
    assert!((b.gain(Bus::Sfx) - 0.5).abs() < 1e-6);
    assert!((b.gain(Bus::Ambient) - 0.5 * b.ambient).abs() < 1e-6);
}

#[test]
fn speech_ducks_music_gain() {
    let mut app = app();
    {
        let mut d = app.world_mut().resource_mut::<MusicDirector>();
        d.set_scene(MusicScene::Galactic("Rebel".into()));
    }
    app.update();
    let initial_duck = {
        let mut q = app.world_mut().query::<&MusicTrack>();
        q.iter(app.world()).next().map(|t| t.duck_factor()).unwrap_or(1.0)
    };
    assert!((initial_duck - 1.0).abs() < 1e-4, "music starts at full gain");

    // Play a voice-over speech sound.
    play(&mut app, "Unit_Select_A");
    std::thread::sleep(std::time::Duration::from_millis(80));
    app.update();
    let ducked = {
        let mut q = app.world_mut().query::<&MusicTrack>();
        q.iter(app.world()).next().map(|t| t.duck_factor()).unwrap_or(1.0)
    };
    assert!(ducked < 0.9, "speech ducks music gain while playing: {ducked}");

    // Despawn speech voice to simulate the speech line finishing.
    let speech_entities: Vec<Entity> = app.world_mut().query_filtered::<Entity, With<SfxVoice>>().iter(app.world()).collect();
    for e in speech_entities {
        app.world_mut().despawn(e);
    }
    std::thread::sleep(std::time::Duration::from_millis(150));
    app.update();
    std::thread::sleep(std::time::Duration::from_millis(200));
    app.update();
    std::thread::sleep(std::time::Duration::from_millis(200));
    app.update();
    let recovered = {
        let mut q = app.world_mut().query::<&MusicTrack>();
        q.iter(app.world()).next().map(|t| t.duck_factor()).unwrap_or(1.0)
    };
    assert!(recovered > ducked, "music recovers after speech finishes: {recovered} > {ducked}");
}

fn all_voices(app: &mut App) -> usize {
    app.world_mut().query::<&SfxVoice>().iter(app.world()).count()
}

fn music_events(app: &mut App) -> Vec<String> {
    let mut q = app.world_mut().query::<&MusicTrack>();
    q.iter(app.world()).map(|t| t.event.clone()).collect()
}

#[test]
fn a_new_sample_is_read_on_a_worker_and_its_sound_follows() {
    let mut app = worker_app();
    play(&mut app, "Boom");
    app.update();
    // The frame that asked for an uncached sample does not read it: the sound waits for the worker.
    assert_eq!(all_voices(&mut app), 0);
    assert!(update_until(&mut app, |a| voices(a).len() == 1), "the sound never started");
    // Cached now: the next request plays on its own frame (Max_Instances 1 fades the first one out).
    play(&mut app, "Boom");
    app.update();
    assert_eq!(all_voices(&mut app), 2);
    assert_eq!(voices(&mut app).len(), 1);
}

#[test]
fn a_music_track_starts_once_a_worker_has_read_its_file() {
    let mut app = worker_app();
    app.world_mut().resource_mut::<MusicDirector>().set_scene(MusicScene::Galactic("Rebel".into()));
    app.update();
    assert!(music_events(&mut app).is_empty(), "the music file is not read on the frame the scene changes");
    assert!(update_until(&mut app, |a| !music_events(a).is_empty()), "the music never started");
    assert_eq!(music_events(&mut app), vec!["Galactic_Map_Rebel_Music_Event"]);
}

#[test]
fn a_file_request_plays_once_a_worker_has_read_it() {
    let mut app = worker_app();
    app.world_mut().write_message(alamo_audio::PlayFile { name: "boom_b.wav".into(), bus: alamo_audio::Bus::Sfx });
    let players = |a: &mut App| a.world_mut().query::<&AudioPlayer>().iter(a.world()).count();
    app.update();
    assert_eq!(players(&mut app), 0);
    assert!(update_until(&mut app, |a| players(a) == 1), "the file never played");
}

/// Asks the plugin to get `sample` ready, as a sound request does; true once it is.
fn prepare(app: &mut App, sample: &str) -> bool {
    app.world_mut().resource_scope(|world, mut audio: Mut<alamo_audio::plugin::AlamoAudio>| {
        audio.prepare(sample, 256 << 20, &mut world.resource_mut::<Assets<AudioSource>>())
    })
}

#[test]
fn a_newer_unit_response_waits_for_an_older_one_whose_sample_is_still_read() {
    // The select line's sample is slow to read, the move line's is ready: the move line still comes second and replaces the
    // select line, as it does when both are cached.
    let mut app = worker_app_on(Arc::new(SlowFs(fs(), "Audio/SFX/boom_a.wav")));
    assert!(update_until(&mut app, |a| prepare(a, "boom_b.wav")));
    play(&mut app, "Unit_Select_A");
    app.update();
    play(&mut app, "Unit_Move_A");
    app.update();
    assert!(voice_events(&mut app).is_empty(), "the move line waits for the select line");
    assert!(update_until(&mut app, |a| !voice_events(a).is_empty()), "no line played");
    assert_eq!(voice_events(&mut app), vec!["unit_move_a"]);
}
