//! What the plugin does with a 3D one-shot when every `Max_Instances` voice of its event plays (silent, no audio device): the nearest sounds win. A far
//! shot never cuts a near one; shots of the same loudness replace the oldest as before. In-memory file system, plus one test against the Remake's
//! real weapon events (skipped without the game).
#![cfg(feature = "bevy")]

use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use alamo_audio::plugin::SfxVoice;
use alamo_audio::{AlamoAudioPlugin, AudioSettings, Pcm, PlaySfx};
use alamo_fs::{FileSource, GameFs, GameInstall, normalize};
use bevy::asset::AssetPlugin;
use bevy::audio::{AudioSource, SpatialListener};
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;

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

/// `Gun`: a 3D one-shot with a saturation distance of 100 (150 after the 1.5 of Audio.xml, full volume inside it) and two instances; `Beep` the same
/// without position; `Hum` a loop.
fn fs() -> Arc<MemFs> {
    let mut m = HashMap::new();
    m.insert(normalize("Xml/SFXEventFiles.xml"), b"<SFXEvent_Files><File>SFXEvents.xml</File></SFXEvent_Files>".to_vec());
    m.insert(
        normalize("Xml/SFXEvents.xml"),
        br#"<SFXEvents>
        <SFXEvent Name="Gun"><Samples>a.wav</Samples><Is_3D>Yes</Is_3D><Volume_Saturation_Distance>100</Volume_Saturation_Distance><Max_Instances>2</Max_Instances><Min_Volume>50</Min_Volume><Max_Volume>50</Max_Volume></SFXEvent>
        <SFXEvent Name="Beep"><Samples>a.wav</Samples><Is_2D>Yes</Is_2D><Max_Instances>2</Max_Instances></SFXEvent>
        <SFXEvent Name="Hum"><Samples>a.wav</Samples><Is_3D>Yes</Is_3D><Play_Count>-1</Play_Count><Volume_Saturation_Distance>100</Volume_Saturation_Distance><Max_Instances>2</Max_Instances></SFXEvent>
        </SFXEvents>"#
            .to_vec(),
    );
    m.insert(normalize("Xml/Audio.xml"), b"<Audio><Audio_Space_3D_Saturation_Distance_Mod> 1.5 </Audio_Space_3D_Saturation_Distance_Mod><Audio_Space_3D_Rolloff_Distance_Mod>1.0</Audio_Space_3D_Rolloff_Distance_Mod></Audio>".to_vec());
    m.insert(normalize("Audio/SFX/a.wav"), tone());
    Arc::new(MemFs(m))
}

/// The plugin on `fs`, a listener at the origin and frames of 0.1 s (so the voices have distinct start times).
fn app_on(fs: Arc<dyn FileSource>) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default())).init_asset::<AudioSource>().add_plugins(AlamoAudioPlugin::new(fs));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(100)));
    app.world_mut().resource_mut::<AudioSettings>().load_on_worker = false;
    app.world_mut().spawn((SpatialListener::new(4.0), GlobalTransform::IDENTITY));
    app
}

fn app() -> App {
    app_on(fs())
}

/// Sends one request for `event` at `distance` from the listener (on the x axis) and runs a frame.
fn play_at(app: &mut App, event: &str, distance: f32) {
    app.world_mut().write_message(PlaySfx { event: event.into(), position: Some(Vec3::X * distance), source: None });
    app.update();
}

/// `(distance, fading out)` of the voices of `event`, nearest first.
fn voices(app: &mut App, event: &str) -> Vec<(u32, bool)> {
    let mut q = app.world_mut().query::<&SfxVoice>();
    let mut v: Vec<(u32, bool)> = q.iter(app.world()).filter(|v| v.event() == event.to_ascii_lowercase()).map(|v| (v.distance().map_or(0, |d| d.round() as u32), v.is_stopping())).collect();
    v.sort();
    v
}

fn playing(app: &mut App, event: &str) -> Vec<u32> {
    voices(app, event).into_iter().filter(|v| !v.1).map(|v| v.0).collect()
}

#[test]
fn a_far_shot_never_cuts_a_near_one() {
    let mut app = app();
    play_at(&mut app, "Gun", 10.0);
    play_at(&mut app, "Gun", 20.0);
    assert_eq!(playing(&mut app, "Gun"), [10, 20], "Gun has Max_Instances 2");
    // 3000 away: a falloff of 150 / 3000 = 0.05, 26 dB below the near ones. The old rule cut the oldest near shot for it.
    play_at(&mut app, "Gun", 3000.0);
    assert_eq!(voices(&mut app, "Gun"), [(10, false), (20, false)], "the far shot is dropped, nothing fades out");
}

#[test]
fn a_near_shot_takes_the_farthest_voice() {
    let mut app = app();
    play_at(&mut app, "Gun", 1500.0);
    play_at(&mut app, "Gun", 3000.0);
    play_at(&mut app, "Gun", 10.0);
    assert_eq!(voices(&mut app, "Gun"), [(10, false), (1500, false), (3000, true)], "the farthest instance fades out, the older 1500 one stays");
    // A shot 2.7 dB nearer than the quietest one takes its voice.
    play_at(&mut app, "Gun", 1100.0);
    assert_eq!(playing(&mut app, "Gun"), [10, 1100], "1100 (falloff 0.136) replaces 1500 (0.1)");
}

#[test]
fn shots_of_the_same_loudness_replace_the_oldest_as_before() {
    let mut app = app();
    // All inside the saturation distance of 150: full volume, one loudness.
    for d in [10.0, 20.0, 30.0] {
        play_at(&mut app, "Gun", d);
    }
    assert_eq!(voices(&mut app, "Gun"), [(10, true), (20, false), (30, false)], "the oldest gives way");
    // Falloffs within 3 dB of each other (150 / 200 = 0.75 and 150 / 190 = 0.79) count as equal too: the oldest of them goes.
    let mut app = self::app();
    play_at(&mut app, "Gun", 200.0);
    play_at(&mut app, "Gun", 190.0);
    play_at(&mut app, "Gun", 195.0);
    assert_eq!(voices(&mut app, "Gun"), [(190, false), (195, false), (200, true)]);
}

#[test]
fn a_shot_at_most_3_db_quieter_than_the_voice_it_takes_still_gets_it() {
    let mut app = app();
    play_at(&mut app, "Gun", 100.0);
    play_at(&mut app, "Gun", 150.0);
    // 200 away: a falloff of 150 / 200 = 0.75, 2.5 dB below the two instances (both inside the saturation distance, equally loud): it replaces the oldest.
    play_at(&mut app, "Gun", 200.0);
    assert_eq!(voices(&mut app, "Gun"), [(100, true), (150, false), (200, false)]);
    // 400 away is 8.5 dB below the quietest playing one (0.75): dropped.
    play_at(&mut app, "Gun", 400.0);
    assert_eq!(playing(&mut app, "Gun"), [150, 200]);
}

#[test]
fn the_old_rule_stays_available_for_a_b_listening() {
    let mut app = app();
    app.world_mut().resource_mut::<AudioSettings>().max_instances_nearest = false;
    play_at(&mut app, "Gun", 10.0);
    play_at(&mut app, "Gun", 20.0);
    play_at(&mut app, "Gun", 3000.0);
    assert_eq!(voices(&mut app, "Gun"), [(10, true), (20, false), (3000, false)], "SP_AUDIO_MAXINST=oldest: the far shot cuts the oldest near one");
}

#[test]
fn sounds_without_a_position_and_loops_keep_their_rule() {
    let mut app = app();
    // 2D: one loudness, the oldest gives way.
    for _ in 0..3 {
        app.world_mut().write_message(PlaySfx { event: "Beep".into(), position: None, source: None });
        app.update();
    }
    let beeps = |app: &mut App| {
        let mut q = app.world_mut().query::<&SfxVoice>();
        q.iter(app.world()).filter(|v| v.event() == "beep").map(|v| v.is_stopping()).collect::<Vec<_>>().len()
    };
    assert_eq!(beeps(&mut app), 3, "two playing and the oldest fading out");
    // A loop at the cap: the old rule still applies (the emitter system hands out loop voices nearest first, the plugin does not drop them).
    play_at(&mut app, "Hum", 10.0);
    play_at(&mut app, "Hum", 20.0);
    play_at(&mut app, "Hum", 3000.0);
    assert_eq!(voices(&mut app, "Hum"), [(10, true), (20, false), (3000, false)], "the loop at the cap: the oldest fades out for the new one, near or far");
}

/// The Remake's own weapon events (skipped without the game files): a fighter's laser has a handful of instances; 60 far shots arriving while the
/// nearest ones play cut none of them, and the nearest shot of the whole volley is among the ones that play at the end.
#[test]
fn the_remake_fighter_lasers_keep_their_nearest_shots() {
    let Some(install) = GameInstall::detect().filter(|i| i.mod_data("1770851727").is_dir()) else { return };
    let fs = Arc::new(GameFs::for_mod(&install, Some("1770851727")).expect("game files"));
    let mut app = app_on(fs);
    let (event, cap, saturation) = {
        let audio = app.world().resource::<alamo_audio::plugin::AlamoAudio>();
        let e = audio.sfx.get("Unit_Laser_Fire_Empire_Fighter_1").expect("event");
        (e.name.clone(), e.max_instances.expect("Max_Instances"), e.saturation_distance.expect("saturation") * 1.5)
    };
    assert!((2..=8).contains(&cap), "{event}: Max_Instances {cap}");
    // The near ones inside the saturation distance, then a volley from 2000 to 9000 units away, nearest last.
    let near: Vec<f32> = (0..cap).map(|i| 10.0 + 5.0 * i as f32).collect();
    for d in &near {
        play_at(&mut app, &event, *d);
    }
    for i in 0..60 {
        play_at(&mut app, &event, 9000.0 - 100.0 * i as f32);
    }
    let live = voices(&mut app, &event);
    assert_eq!(live.iter().filter(|v| !v.1).count() as u32, cap, "{live:?}");
    assert!(live.iter().filter(|v| !v.1).all(|v| (v.0 as f32) < saturation), "only the near shots play: {live:?}");
    assert!(live.iter().all(|v| !v.1), "no near shot fades out: {live:?}");
}

/// Timing, not a check (`cargo test -p alamo-audio --test max_instances -- --ignored --nocapture`): 40 requests a frame at scattered distances against an
/// event with `Max_Instances` 4 (a fighter laser) and one with 40, with the old rule and the new one.
#[test]
#[ignore = "timing"]
fn what_the_nearest_rule_costs_per_request() {
    let mut files = HashMap::new();
    files.insert(normalize("Xml/SFXEventFiles.xml"), b"<SFXEvent_Files><File>SFXEvents.xml</File></SFXEvent_Files>".to_vec());
    files.insert(
        normalize("Xml/SFXEvents.xml"),
        br#"<SFXEvents>
        <SFXEvent Name="Laser"><Samples>a.wav</Samples><Is_3D>Yes</Is_3D><Volume_Saturation_Distance>100</Volume_Saturation_Distance><Max_Instances>4</Max_Instances></SFXEvent>
        <SFXEvent Name="Crowd"><Samples>a.wav</Samples><Is_3D>Yes</Is_3D><Volume_Saturation_Distance>100</Volume_Saturation_Distance><Max_Instances>40</Max_Instances></SFXEvent>
        </SFXEvents>"#
            .to_vec(),
    );
    files.insert(normalize("Xml/Audio.xml"), b"<Audio/>".to_vec());
    files.insert(normalize("Audio/SFX/a.wav"), tone());
    let fs = Arc::new(MemFs(files));
    for event in ["Laser", "Crowd"] {
        for nearest in [false, true] {
            let mut app = app_on(fs.clone());
            app.world_mut().resource_mut::<AudioSettings>().max_instances_nearest = nearest;
            let frames = 600;
            let per_frame = 40;
            let mut seed = 12345u32;
            let mut rand = move || {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1u32 << 24) as f32
            };
            // Warm up so that the sample is decoded and the voices stand at their limit.
            for _ in 0..20 {
                app.world_mut().write_message(PlaySfx { event: event.into(), position: Some(Vec3::X * (10.0 + 4000.0 * rand())), source: None });
                app.update();
            }
            let started = std::time::Instant::now();
            for _ in 0..frames {
                for _ in 0..per_frame {
                    app.world_mut().write_message(PlaySfx { event: event.into(), position: Some(Vec3::X * (10.0 + 4000.0 * rand())), source: None });
                }
                app.update();
            }
            let total = started.elapsed();
            eprintln!(
                "{event:<6} Max_Instances {}: {} rule: {:.1} us per frame of {per_frame} requests ({:.0} ns per request)",
                if event == "Laser" { 4 } else { 40 },
                if nearest { "nearest" } else { "oldest " },
                total.as_secs_f64() * 1e6 / f64::from(frames),
                total.as_secs_f64() * 1e9 / f64::from(frames * per_frame)
            );
        }
    }
}
