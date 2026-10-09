//! Audio pass 2, silent (no audio device): positional SFX, object loops (engine hum) and the music flow around
//! result stingers. Uses an in-memory file system like `plugin.rs`.
#![cfg(feature = "bevy")]

use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use alamo_audio::plugin::{MusicTrack, SfxVoice};
use alamo_audio::{AlamoAudioPlugin, MusicDirector, MusicScene, Pcm, PlaySfx, SfxEmitter};
use alamo_fs::{FileSource, normalize};
use bevy::asset::AssetPlugin;
use bevy::audio::{AudioSource, PlaybackSettings, SpatialListener};
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

fn tone() -> Vec<u8> {
    Pcm { channels: 1, sample_rate: 8000, samples: (0..800).map(|i| ((i % 40) * 100) as i16).collect() }.to_wav_bytes()
}

fn fs() -> Arc<MemFs> {
    let mut m = HashMap::new();
    let mut put = |p: &str, d: Vec<u8>| {
        m.insert(normalize(p), d);
    };
    put("Xml/SFXEventFiles.xml", b"<SFXEvent_Files><File>SFXEvents.xml</File></SFXEvent_Files>".to_vec());
    put(
        "Xml/SFXEvents.xml",
        br#"<SFXEvents>
        <SFXEvent Name="Gun"><Samples>a.wav</Samples><Is_3D>Yes</Is_3D><Volume_Saturation_Distance>100</Volume_Saturation_Distance><Max_Instances>2</Max_Instances><Min_Volume>50</Min_Volume><Max_Volume>50</Max_Volume></SFXEvent>
        <SFXEvent Name="Engine_Idle"><Samples>a.wav</Samples><Is_3D>Yes</Is_3D><Play_Count>-1</Play_Count><Priority>1</Priority><Volume_Saturation_Distance>100</Volume_Saturation_Distance><Loop_Fade_Out_Seconds>1.0</Loop_Fade_Out_Seconds><Min_Volume>30</Min_Volume><Max_Volume>100</Max_Volume></SFXEvent>
        <SFXEvent Name="Engine_Moving"><Samples>b.wav</Samples><Is_3D>Yes</Is_3D><Play_Count>-1</Play_Count><Priority>1</Priority><Volume_Saturation_Distance>100</Volume_Saturation_Distance><Loop_Fade_Out_Seconds>1.0</Loop_Fade_Out_Seconds><Min_Volume>30</Min_Volume><Max_Volume>100</Max_Volume></SFXEvent>
        </SFXEvents>"#
            .to_vec(),
    );
    put("Xml/Audio.xml", b"<Audio><Audio_Space_3D_Saturation_Distance_Mod> 1.5 </Audio_Space_3D_Saturation_Distance_Mod><Audio_Space_3D_Rolloff_Distance_Mod>1.0</Audio_Space_3D_Rolloff_Distance_Mod><SFXEvent_GUI_Negative_Feedback> Gun </SFXEvent_GUI_Negative_Feedback></Audio>".to_vec());
    for f in ["a", "b"] {
        put(&format!("Audio/SFX/{f}.wav"), tone());
    }
    for f in ["rebel1", "rebel2", "rebel_win"] {
        put(&format!("Audio/Music/{f}.wav"), tone());
    }
    put(
        "Xml/MusicEvents.xml",
        br#"<MusicEvents>
        <MusicEvent Name="Galactic_Map_Rebel_Music_Event"><Files>rebel1.wav, rebel2.wav</Files><Volume_Percent>100</Volume_Percent><Fade_In_Seconds>2</Fade_In_Seconds><Fade_Out_Previous_Seconds>2</Fade_Out_Previous_Seconds><Loop>Yes</Loop></MusicEvent>
        <MusicEvent Name="Rebel_Win_Tactical_Event"><Files>rebel_win.wav</Files><Volume_Percent>100</Volume_Percent><Fade_In_Seconds>0</Fade_In_Seconds><Fade_Out_Previous_Seconds>2</Fade_Out_Previous_Seconds><Loop>Yes</Loop></MusicEvent>
        <MusicEvent Name="Rebel_Lose_Tactical_Event"><Files>rebel_win.wav</Files><Volume_Percent>75</Volume_Percent><Fade_In_Seconds>0</Fade_In_Seconds><Fade_Out_Previous_Seconds>2</Fade_Out_Previous_Seconds><Loop>No</Loop></MusicEvent>
        </MusicEvents>"#
            .to_vec(),
    );
    Arc::new(MemFs(m))
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default())).init_asset::<AudioSource>().add_plugins(AlamoAudioPlugin::new(fs()));
    app
}

fn listener(app: &mut App, at: Vec3) {
    app.world_mut().spawn((SpatialListener::new(4.0), GlobalTransform::from_translation(at)));
}

fn play_at(app: &mut App, event: &str, at: Vec3) {
    app.world_mut().write_message(PlaySfx { event: event.into(), position: Some(at), source: None });
}

/// (attenuation, distance, spatialized) of the first voice of `event`.
fn first_voice(app: &mut App, event: &str) -> Option<(f32, Option<f32>, bool)> {
    let mut q = app.world_mut().query::<(&SfxVoice, &PlaybackSettings)>();
    q.iter(app.world()).find(|(v, _)| v.event() == event.to_ascii_lowercase()).map(|(v, pb)| (v.attenuation(), v.distance(), pb.spatial))
}

#[test]
fn positional_sfx_fall_off_with_distance_to_the_listener() {
    // `Gun` has Volume_Saturation_Distance 100 and Audio.xml scales space saturation distances by 1.5: full volume inside
    // 150, (150 / distance) outside.
    let mut near = app();
    listener(&mut near, Vec3::ZERO);
    play_at(&mut near, "Gun", Vec3::new(50.0, 0.0, 0.0));
    near.update();
    let (att, dist, spatial) = first_voice(&mut near, "Gun").expect("near gun plays");
    assert!(spatial, "positioned 3D events are spatialized (left / right pan)");
    assert_eq!(att, 1.0);
    assert!((dist.unwrap() - 50.0).abs() < 0.01);

    let mut far = app();
    listener(&mut far, Vec3::ZERO);
    play_at(&mut far, "Gun", Vec3::new(0.0, 1000.0, 0.0));
    far.update();
    let (att, dist, _) = first_voice(&mut far, "Gun").expect("far gun still plays, quietly");
    assert!((att - 0.15).abs() < 1e-4, "150 / 1000 = 0.15, got {att}");
    assert!((dist.unwrap() - 1000.0).abs() < 0.01);
}

#[test]
fn without_a_listener_positional_sfx_are_not_spatialized() {
    // This is what the battle did before its camera carried a `SpatialListener`: no attenuation, no pan.
    let mut app = app();
    play_at(&mut app, "Gun", Vec3::new(0.0, 1000.0, 0.0));
    app.update();
    let (att, dist, spatial) = first_voice(&mut app, "Gun").unwrap();
    assert!(!spatial && att == 1.0 && dist.is_none());
}

#[test]
fn simultaneous_instances_are_capped_by_max_instances() {
    let mut app = app();
    listener(&mut app, Vec3::ZERO);
    for i in 0..6 {
        play_at(&mut app, "Gun", Vec3::new(10.0 * i as f32, 0.0, 0.0));
    }
    app.update();
    let mut q = app.world_mut().query::<&SfxVoice>();
    assert_eq!(q.iter(app.world()).filter(|v| v.event() == "gun").count(), 2, "Gun has Max_Instances 2");
}

fn emitter_voices(app: &mut App, e: Entity) -> Vec<bool> {
    let mut q = app.world_mut().query::<&SfxVoice>();
    q.iter(app.world()).filter(|v| v.source() == Some(e)).map(|v| v.is_stopping()).collect()
}

#[test]
fn engine_loops_follow_the_emitter_and_switch_with_it() {
    let mut app = app();
    listener(&mut app, Vec3::ZERO);
    let ship = app.world_mut().spawn((GlobalTransform::from_translation(Vec3::new(30.0, 0.0, 0.0)), SfxEmitter::new(Some("Engine_Idle".into())))).id();
    app.update();
    app.update();
    assert_eq!(emitter_voices(&mut app, ship), vec![false], "one idle loop for the ship");
    assert_eq!(app.world().get::<SfxEmitter>(ship).unwrap().playing(), Some("Engine_Idle"));
    // The ship starts to move: the idle loop fades out, the moving loop starts.
    app.world_mut().get_mut::<SfxEmitter>(ship).unwrap().event = Some("Engine_Moving".into());
    app.update();
    app.update();
    let mut v = emitter_voices(&mut app, ship);
    v.sort();
    assert_eq!(v, vec![false, true], "moving loop plays, idle loop is fading out");
    // The ship is destroyed: its loops fade out.
    app.world_mut().despawn(ship);
    app.update();
    app.update();
    let mut q = app.world_mut().query::<&SfxVoice>();
    assert!(q.iter(app.world()).all(|v| v.is_stopping()), "loops of a vanished object stop");
}

#[test]
fn engine_loops_too_far_away_are_not_started() {
    let mut app = app();
    listener(&mut app, Vec3::ZERO);
    let ship = app.world_mut().spawn((GlobalTransform::from_translation(Vec3::new(100_000.0, 0.0, 0.0)), SfxEmitter::new(Some("Engine_Idle".into())))).id();
    app.update();
    app.update();
    assert!(emitter_voices(&mut app, ship).is_empty(), "inaudible loops do not take a voice");
    *app.world_mut().get_mut::<GlobalTransform>(ship).unwrap() = GlobalTransform::from_translation(Vec3::new(80.0, 0.0, 0.0));
    app.update();
    app.update();
    assert_eq!(emitter_voices(&mut app, ship), vec![false]);
}

fn music_tracks(app: &mut App) -> Vec<(String, bool)> {
    let mut q = app.world_mut().query::<&MusicTrack>();
    let mut v: Vec<_> = q.iter(app.world()).map(|t| (t.event.clone(), t.is_outgoing())).collect();
    v.sort();
    v
}

fn set_scene(app: &mut App, s: MusicScene) {
    app.world_mut().resource_mut::<MusicDirector>().set_scene(s);
}

#[test]
fn result_stinger_plays_through_before_the_galaxy_music_returns() {
    let mut app = app();
    set_scene(&mut app, MusicScene::Galactic("Rebel".into()));
    app.update();
    set_scene(&mut app, MusicScene::Result { faction: "Rebel".into(), win: true, tactical: true });
    app.update();
    // Back on the map the flow asks for the galaxy music: it must wait for the stinger.
    app.world_mut().resource_mut::<MusicDirector>().resume_after_stinger(MusicScene::Galactic("Rebel".into()));
    app.update();
    app.update();
    assert_eq!(app.world().resource::<MusicDirector>().current_event(), Some("Rebel_Win_Tactical_Event"));
    let alive: Vec<_> = music_tracks(&mut app).into_iter().filter(|(_, out)| !out).collect();
    assert_eq!(alive.len(), 1, "exactly one non-fading track: {alive:?}");
    // The stinger ends (Loop = Yes in the data, but a result stinger plays once): the galaxy music resumes.
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
    let alive: Vec<_> = music_tracks(&mut app).into_iter().filter(|(_, out)| !out).collect();
    assert_eq!(alive.len(), 1);
    assert_eq!(alive[0].0, "Galactic_Map_Rebel_Music_Event");
}

#[test]
fn a_new_scene_cancels_the_pending_resume() {
    let mut app = app();
    set_scene(&mut app, MusicScene::Result { faction: "Rebel".into(), win: false, tactical: true });
    app.world_mut().resource_mut::<MusicDirector>().resume_after_stinger(MusicScene::Galactic("Rebel".into()));
    set_scene(&mut app, MusicScene::BattleLoading);
    for _ in 0..3 {
        app.update();
    }
    assert!(matches!(app.world().resource::<MusicDirector>().scene(), MusicScene::BattleLoading));
}

#[test]
fn same_scene_does_not_restart_the_music() {
    let mut app = app();
    for _ in 0..3 {
        set_scene(&mut app, MusicScene::Galactic("Rebel".into()));
        app.update();
    }
    assert_eq!(music_tracks(&mut app).len(), 1, "setting the same scene again keeps the one track");
}

#[test]
fn distance_constants_come_from_audio_xml() {
    let app = app();
    let c = &app.world().resource::<alamo_audio::plugin::AlamoAudio>().constants;
    assert_eq!(c.space_saturation_mod, 1.5);
    assert_eq!(c.negative_feedback.as_deref(), Some("Gun"));
    let s = app.world().resource::<alamo_audio::AudioSettings>();
    assert_eq!((s.saturation_scale, s.rolloff), (1.5, 1.0));
}
