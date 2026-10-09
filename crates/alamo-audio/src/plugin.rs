//! Bevy plugin: SFX / speech playback with EaW-like voice rules and a music director.
//!
//! Add [`AlamoAudioPlugin`] next to Bevy's own `AudioPlugin` (part of `DefaultPlugins`), then use
//! [`SfxCommands`] to play events and [`MusicDirector`] to select the music scene. Nothing here talks to
//! the audio device directly; it only spawns `AudioPlayer` entities, so tests can run without output.
//!
//! # Rules implemented
//!
//! * **Randomization**: `Play_Sequentially` walks the sample list per event, otherwise a random sample
//!   (never the same twice in a row); volume and pitch are drawn from the event's min/max range.
//! * **Probability / predelay** are honoured. `Pre_Samples` play first, `Post_Samples` and
//!   `Chained_SFXEvent` follow when the previous sound has finished.
//! * **Max_Instances**: for ordinary sounds the oldest instance is cut; for voice-over (unit responses,
//!   HUD and ambient chatter) the new request is dropped instead so lines never cut each other off.
//! * **Overlap_Test**: events of one overlap group never play at the same time.
//! * **Voice limit** ([`AudioSettings::max_voices`]): when full, the least important voice (highest
//!   priority number, then oldest) is stolen if it is less important than the newcomer, otherwise the
//!   newcomer is dropped. Priority 1 is the most important.
//! * **Distance**: 3D events fall off as `saturation_distance / distance` and are panned by Bevy's
//!   spatial audio (add `SpatialListener` to the camera). Without a listener they play centered.
//! * **Kills_Previous_Object_SFX** stops earlier sounds of the same source entity.
//! * **Unit speech cooldown** ([`AudioSettings::speech_event_cooldown`] per event,
//!   [`AudioSettings::speech_global_cooldown`] between any two unit responses) prevents spam.
//! * **Buses**: master / music / sfx / speech / ambient in [`AudioBuses`], applied live.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::audio::{
    AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings, SpatialAudioSink,
    SpatialListener, SpatialScale, Volume,
};
use bevy::prelude::*;
use rand::Rng;
use rand::seq::SliceRandom;

use crate::music::{MusicPlanner, MusicScene};
use crate::sfx::SfxEvent;
use crate::speech::UnitResponseKind;
use crate::wav::Wav;
use crate::{MusicEvent, MusicLibrary, SampleResolver, SfxLibrary, SpeechLibrary};
use alamo_fs::FileSource;

/// Volume group a sound belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bus {
    /// Music.
    Music,
    /// Sound effects.
    Sfx,
    /// Voice-over: unit responses, HUD, advisor and campaign speech.
    Speech,
    /// Ambient loops and battle chatter.
    Ambient,
}

/// Linear volume per bus (1.0 = unchanged).
#[derive(Resource, Debug, Clone)]
pub struct AudioBuses {
    /// Applied to everything.
    pub master: f32,
    /// Music bus.
    pub music: f32,
    /// SFX bus.
    pub sfx: f32,
    /// Speech bus.
    pub speech: f32,
    /// Ambient bus.
    pub ambient: f32,
}

impl Default for AudioBuses {
    fn default() -> Self {
        Self {
            master: 1.0,
            music: 0.6,
            sfx: 1.0,
            speech: 1.0,
            ambient: 0.8,
        }
    }
}

impl AudioBuses {
    /// Combined gain of a bus including master.
    pub fn gain(&self, bus: Bus) -> f32 {
        self.master
            * match bus {
                Bus::Music => self.music,
                Bus::Sfx => self.sfx,
                Bus::Speech => self.speech,
                Bus::Ambient => self.ambient * self.sfx,
            }
    }
}

/// Tunables of the SFX engine.
#[derive(Resource, Debug, Clone)]
pub struct AudioSettings {
    /// Maximum simultaneous SFX voices.
    pub max_voices: usize,
    /// Sounds quieter than this (linear gain) at the start are not played.
    pub min_audible_gain: f32,
    /// Extra seconds before the same unit-response event may play again (0 = only the data's `Max_Instances` applies).
    pub speech_event_cooldown: f32,
    /// Extra seconds between any two unit responses (0 = a new response interrupts the one playing).
    pub speech_global_cooldown: f32,
    /// Decoded sample cache budget in bytes.
    pub cache_budget_bytes: usize,
    /// Music crossfade length used when the new event does not specify one.
    pub default_music_fade: f32,
    /// `Audio_Space_3D_Saturation_Distance_Mod` (Audio.xml): multiplies every event's `Volume_Saturation_Distance`.
    pub saturation_scale: f32,
    /// `Audio_Space_3D_Rolloff_Distance_Mod` (Audio.xml): exponent of the distance falloff (estimate: the data says only
    /// "higher = quicker rolloff"; 1 is the plain `saturation / distance`).
    pub rolloff: f32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            max_voices: 48,
            min_audible_gain: 0.01,
            speech_event_cooldown: 0.0,
            speech_global_cooldown: 0.0,
            cache_budget_bytes: 256 << 20,
            default_music_fade: 2.0,
            saturation_scale: 1.0,
            rolloff: 1.0,
        }
    }
}

/// The four sliders of an options menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VolumeCategory {
    /// Everything.
    Master,
    /// Music.
    Music,
    /// Effects (also scales the ambient bus).
    Sfx,
    /// Speech (unit responses, advisor, campaign lines).
    Speech,
}

impl AudioBuses {
    /// Slider value of a category in percent (0..=100).
    pub fn percent(&self, c: VolumeCategory) -> f32 {
        100.0
            * match c {
                VolumeCategory::Master => self.master,
                VolumeCategory::Music => self.music,
                VolumeCategory::Sfx => self.sfx,
                VolumeCategory::Speech => self.speech,
            }
    }

    /// Sets a category from a slider value in percent (clamped to 0..=100).
    pub fn set_percent(&mut self, c: VolumeCategory, percent: f32) {
        let v = (percent / 100.0).clamp(0.0, 1.0);
        match c {
            VolumeCategory::Master => self.master = v,
            VolumeCategory::Music => self.music = v,
            VolumeCategory::Sfx => self.sfx = v,
            VolumeCategory::Speech => self.speech = v,
        }
    }
}

/// A galactic-map happening for the advisor: resolved to the faction's HUD voice line (see
/// [`crate::galaxy`]) and played as an SFX event on the speech bus.
#[derive(Message, Debug, Clone)]
pub struct GalaxyAudio {
    /// Player faction (`"Empire"`, `"Rebel"`, `"Underworld"`).
    pub faction: String,
    /// What happened.
    pub event: crate::GalaxyEvent,
}

fn galaxy_audio_system(
    mut events: MessageReader<GalaxyAudio>,
    audio: Res<AlamoAudio>,
    mut out: MessageWriter<PlaySfx>,
) {
    for g in events.read() {
        if audio_log() {
            info!("audio advisor {:?} ({}) -> {:?}", g.event, g.faction, crate::resolve_galaxy_event(&audio.sfx, &g.faction, g.event));
        }
        if let Some(event) = crate::resolve_galaxy_event(&audio.sfx, &g.faction, g.event) {
            out.write(PlaySfx {
                event,
                position: None,
                source: None,
            });
        }
    }
}

/// Request to play an SFX event (send through [`SfxCommands`] or write the message directly).
#[derive(Message, Debug, Clone)]
pub struct PlaySfx {
    /// Event name (case-insensitive).
    pub event: String,
    /// World position; `None` plays the sound without spatialization.
    pub position: Option<Vec3>,
    /// Emitting entity, used by `Kills_Previous_Object_SFX` and to follow moving objects.
    pub source: Option<Entity>,
}

/// Plays one audio file by name (resolved like SFX samples, e.g. `REBEL_VICTORY_00.WAV`) on a bus, without an SFX event.
#[derive(Message, Debug, Clone)]
pub struct PlayFile {
    /// File name or path as the samples of SFX events are written.
    pub name: String,
    /// Volume group.
    pub bus: Bus,
}

fn process_file_requests(
    mut reader: MessageReader<PlayFile>,
    mut audio: ResMut<AlamoAudio>,
    mut assets: ResMut<Assets<AudioSource>>,
    settings: Res<AudioSettings>,
    buses: Res<AudioBuses>,
    mut commands: Commands,
) {
    for req in reader.read() {
        if audio_log() {
            info!("audio file {}", req.name);
        }
        let Some((handle, _)) = audio.sample(&req.name, settings.cache_budget_bytes, &mut assets) else {
            warn!("audio file not found: {}", req.name);
            continue;
        };
        let pb = PlaybackSettings::ONCE.with_volume(Volume::Linear(buses.gain(req.bus)));
        commands.spawn((AudioPlayer::<AudioSource>(handle), pb));
    }
}

/// Convenience wrapper around [`PlaySfx`] messages.
#[derive(bevy::ecs::system::SystemParam)]
pub struct SfxCommands<'w> {
    writer: MessageWriter<'w, PlaySfx>,
}

impl SfxCommands<'_> {
    /// Plays an event without position (GUI, HUD, music stingers ...).
    pub fn play(&mut self, event: impl Into<String>) {
        self.writer.write(PlaySfx {
            event: event.into(),
            position: None,
            source: None,
        });
    }

    /// Plays an event at a world position.
    pub fn play_at(&mut self, event: impl Into<String>, position: Vec3) {
        self.writer.write(PlaySfx {
            event: event.into(),
            position: Some(position),
            source: None,
        });
    }

    /// Plays an event for an entity (kills its previous sounds if the event says so).
    pub fn play_for(&mut self, event: impl Into<String>, source: Entity, position: Vec3) {
        self.writer.write(PlaySfx {
            event: event.into(),
            position: Some(position),
            source: Some(source),
        });
    }

    /// Unit acknowledgement, e.g. `(Select, "Tie_Fighter")` plays `Unit_Select_Tie_Fighter`.
    /// Subject to the speech cooldowns.
    pub fn unit_response(&mut self, kind: UnitResponseKind, unit: &str) {
        if let Some(name) = kind.event_name(unit) {
            self.play(name);
        }
    }
}

/// Loaded audio data and sample cache.
#[derive(Resource)]
pub struct AlamoAudio {
    /// The game file system samples are read from.
    pub fs: Arc<dyn FileSource>,
    /// SFX events.
    pub sfx: SfxLibrary,
    /// Speech lines.
    pub speech: SpeechLibrary,
    /// Music events.
    pub music: MusicLibrary,
    /// Audio constants from `Audio.xml` / `Gameconstants.xml`.
    pub constants: crate::AudioConstants,
    resolver: SampleResolver,
    cache: HashMap<String, Handle<AudioSource>>,
    cache_bytes: usize,
    failed: std::collections::HashSet<String>,
}

impl AlamoAudio {
    /// Loads all event definitions from `fs`.
    pub fn load(fs: Arc<dyn FileSource>) -> Self {
        let mut warnings = Vec::new();
        let sfx = SfxLibrary::load(&*fs, &mut warnings);
        let speech = SpeechLibrary::load(&*fs, &mut warnings);
        let music = MusicLibrary::load(&*fs, &mut warnings);
        let resolver = SampleResolver::new(&*fs);
        let constants = crate::AudioConstants::load(&*fs);
        for w in warnings.iter().take(20) {
            warn!("audio data: {}: {}", w.file, w.message);
        }
        Self {
            fs,
            sfx,
            speech,
            music,
            constants,
            resolver,
            cache: HashMap::new(),
            cache_bytes: 0,
            failed: Default::default(),
        }
    }

    /// Resolves, reads, decodes (WAV -> PCM16) and caches a sample. `None` if it is missing or undecodable.
    pub fn sample(
        &mut self,
        name: &str,
        budget: usize,
        assets: &mut Assets<AudioSource>,
    ) -> Option<(Handle<AudioSource>, String)> {
        let (path, _) = self.resolver.resolve(&*self.fs, name)?;
        if let Some(h) = self.cache.get(&path) {
            return Some((h.clone(), path));
        }
        if self.failed.contains(&path) {
            return None;
        }
        let bytes = match self.fs.read(&path) {
            Ok(b) => b,
            Err(_) => {
                self.failed.insert(path);
                return None;
            }
        };
        let bytes = if path.ends_with(".WAV") {
            match Wav::parse(&bytes).and_then(|w| w.decode()) {
                Ok(pcm) => pcm.to_wav_bytes(),
                Err(e) => {
                    warn!("cannot decode {path}: {e}");
                    self.failed.insert(path);
                    return None;
                }
            }
        } else {
            bytes
        };
        if self.cache_bytes + bytes.len() > budget {
            self.cache.clear();
            self.cache_bytes = 0;
        }
        self.cache_bytes += bytes.len();
        let handle = assets.add(AudioSource {
            bytes: bytes.into(),
        });
        self.cache.insert(path.clone(), handle.clone());
        Some((handle, path))
    }
}

/// Continuation of an event after a predelay or a finished sample.
#[derive(Debug, Clone, Default)]
struct Continuation {
    /// Sample to play; empty = pick normally (predelay finished).
    sample: String,
    /// Samples still to follow after `sample`.
    rest: Vec<String>,
    /// Chained event to start after the last sample.
    chain: Option<String>,
}

/// A playing SFX voice.
#[derive(Component, Debug)]
pub struct SfxVoice {
    event: String,
    group: Option<String>,
    priority: u8,
    bus: Bus,
    base_gain: f32,
    saturation: Option<f32>,
    fade_in: f32,
    source: Option<Entity>,
    started: f64,
    follow: Vec<String>,
    chain: Option<String>,
    is_voice_over: bool,
    unit_response: bool,
    /// Falloff exponent (see [`AudioSettings::rolloff`]).
    rolloff: f32,
    /// Loops until stopped (`Play_Count -1`).
    looping: bool,
    /// Seconds of `Loop_Fade_Out_Seconds` used when the voice is stopped.
    fade_out: f32,
    /// Time the voice was told to stop; it fades out and is removed afterwards.
    stopping: Option<f64>,
    /// Distance to the listener at the last update (`None` without listener or position).
    distance: Option<f32>,
    /// Distance attenuation factor at the last update (1 = none).
    attenuation: f32,
}

impl SfxVoice {
    /// Lower-cased name of the SFX event this voice plays.
    pub fn event(&self) -> &str {
        &self.event
    }

    /// Distance to the listener at the last update (`None` for 2D sounds and without a listener).
    pub fn distance(&self) -> Option<f32> {
        self.distance
    }

    /// Distance attenuation (1 = full volume; `Volume_Saturation_Distance / distance` beyond the saturation distance).
    pub fn attenuation(&self) -> f32 {
        self.attenuation
    }

    /// True while the voice fades out before it is removed.
    pub fn is_stopping(&self) -> bool {
        self.stopping.is_some()
    }

    /// Entity the sound belongs to.
    pub fn source(&self) -> Option<Entity> {
        self.source
    }
}

/// A request waiting for its predelay.
struct Pending {
    due: f64,
    request: PlaySfx,
    cont: Continuation,
}

#[derive(Resource, Default)]
struct SfxState {
    sequence: HashMap<String, usize>,
    last_pick: HashMap<String, usize>,
    last_played: HashMap<String, f64>,
    last_unit_response: Option<f64>,
    pending: Vec<Pending>,
}

/// Registers resources and systems. `fs` is the (layered) game file system to read data and samples from.
pub struct AlamoAudioPlugin {
    fs: Arc<dyn FileSource>,
}

impl AlamoAudioPlugin {
    /// Creates the plugin; event data is loaded when the plugin is added to the app.
    pub fn new(fs: Arc<dyn FileSource>) -> Self {
        Self { fs }
    }
}

impl Plugin for AlamoAudioPlugin {
    fn build(&self, app: &mut App) {
        let audio = AlamoAudio::load(self.fs.clone());
        info!(
            "alamo-audio: {} SFX events, {} speech lines, {} music events",
            audio.sfx.len(),
            audio.speech.len(),
            audio.music.len()
        );
        // Battle music returns to the ambient playlist after the peace delay the data names (Audio.xml).
        let constants = audio.constants.clone();
        let mut director = MusicDirector::default();
        director.planner.calm_seconds = audio.constants.space_battle_to_ambient_seconds;
        app.insert_resource(audio)
            .init_resource::<AudioBuses>()
            .insert_resource(AudioSettings {
                saturation_scale: constants.space_saturation_mod,
                rolloff: constants.space_rolloff_mod,
                ..default()
            })
            .init_resource::<SfxState>()
            .insert_resource(director)
            .add_message::<PlaySfx>()
            .add_message::<GalaxyAudio>()
            .add_message::<PlayFile>()
            .add_systems(
                Update,
                (
                    sync_emitters,
                    galaxy_audio_system,
                    process_sfx_requests,
                    process_file_requests,
                    update_voices,
                    music_system,
                    update_music_gain,
                )
                    .chain(),
            );
    }
}

fn bus_of(ev: &SfxEvent) -> Bus {
    if ev.is_unit_response_vo || ev.is_hud_vo {
        Bus::Speech
    } else if ev.is_ambient_vo
        || ev.looping && ev.priority >= 4
        || ev.source_file.to_ascii_lowercase().contains("ambient")
    {
        Bus::Ambient
    } else {
        Bus::Sfx
    }
}

fn is_voice_over(ev: &SfxEvent) -> bool {
    ev.is_unit_response_vo || ev.is_hud_vo || ev.is_ambient_vo
}

/// Volume factor of a 3D sound at `distance`: full volume inside the saturation distance, `(saturation / distance)^rolloff`
/// outside (estimate of the engine's curve; the data gives the saturation distance and the rolloff modifier only).
pub fn distance_attenuation(distance: f32, saturation: f32, rolloff: f32) -> f32 {
    if distance <= saturation || saturation <= 0.0 {
        1.0
    } else {
        (saturation / distance).powf(rolloff.max(0.01))
    }
}

fn listener_position(listeners: &Query<&GlobalTransform, With<SpatialListener>>) -> Option<Vec3> {
    listeners.iter().next().map(|t| t.translation())
}

#[allow(clippy::too_many_arguments)]
fn process_sfx_requests(
    mut commands: Commands,
    mut reader: MessageReader<PlaySfx>,
    mut audio: ResMut<AlamoAudio>,
    mut state: ResMut<SfxState>,
    mut assets: ResMut<Assets<AudioSource>>,
    settings: Res<AudioSettings>,
    buses: Res<AudioBuses>,
    time: Res<Time>,
    voices: Query<(Entity, &SfxVoice)>,
    listeners: Query<&GlobalTransform, With<SpatialListener>>,
) {
    let now = time.elapsed_secs_f64();
    let mut requests: Vec<(PlaySfx, Option<Continuation>)> =
        reader.read().cloned().map(|r| (r, None)).collect();
    // Predelayed requests that became due.
    let (due, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut state.pending)
        .into_iter()
        .partition(|p| p.due <= now);
    state.pending = waiting;
    requests.extend(due.into_iter().map(|p| (p.request, Some(p.cont))));
    if requests.is_empty() {
        return;
    }
    // Live view of voices, updated as we spawn / steal within this frame.
    #[allow(clippy::type_complexity)]
    let mut live: Vec<(
        Entity,
        String,
        Option<String>,
        u8,
        f64,
        Option<Entity>,
        bool,
        bool,
    )> = voices
        .iter()
        .filter(|(_, v)| v.stopping.is_none())
        .map(|(e, v)| {
            (
                e,
                v.event.clone(),
                v.group.clone(),
                v.priority,
                v.started,
                v.source,
                v.is_voice_over,
                v.unit_response,
            )
        })
        .collect();
    let listener = listener_position(&listeners);
    let mut rng = rand::rng();

    for (req, cont) in requests {
        let key = req.event.to_ascii_lowercase();
        let Some(ev) = audio.sfx.get(&req.event).cloned() else {
            if audio_log() {
                warn!("audio sfx UNKNOWN event `{}`", req.event);
            }
            continue;
        };
        if audio_log() && cont.is_none() {
            info!("audio sfx {} (at {:?})", req.event, req.position.map(|p| p.round()));
        }
        if ev.is_preset {
            continue;
        }
        let first_try = cont.is_none();
        // Probability and predelay apply to fresh requests only.
        if first_try {
            if ev.probability < 100.0 && rng.random_range(0.0..100.0) >= ev.probability {
                continue;
            }
            if ev.predelay_ms.1 > 0.0 {
                let delay = rng.random_range(ev.predelay_ms.0..=ev.predelay_ms.1) / 1000.0;
                state.pending.push(Pending {
                    due: now + f64::from(delay),
                    request: req,
                    cont: Continuation::default(),
                });
                continue;
            }
        }
        // `cont.sample` is empty after a predelay and carries the next sample when continuing a chain.
        let cont = cont.unwrap_or_default();
        let continuing = !cont.sample.is_empty();
        let voice_over = is_voice_over(&ev);

        // Unit response spam protection.
        if ev.is_unit_response_vo && first_try {
            if state
                .last_unit_response
                .is_some_and(|t| now - t < f64::from(settings.speech_global_cooldown))
            {
                continue;
            }
            if state
                .last_played
                .get(&key)
                .is_some_and(|t| now - t < f64::from(settings.speech_event_cooldown))
            {
                continue;
            }
        }
        // Overlap groups. A unit response is the exception: EaW plays the move line of an order right after the
        // select line, so the newer response cuts the older one (below) instead of being dropped.
        if !ev.is_unit_response_vo
            && let Some(g) = &ev.overlap_group
            && live.iter().any(|v| v.2.as_deref() == Some(g.as_str()))
        {
            continue;
        }
        // Max instances.
        if let Some(max) = ev.max_instances {
            let same: Vec<_> = live.iter().filter(|v| v.1 == key).collect();
            if same.len() as u32 >= max {
                if voice_over {
                    continue;
                }
                if let Some(oldest) = same.iter().min_by(|a, b| a.4.total_cmp(&b.4)).map(|v| v.0) {
                    commands.entity(oldest).despawn();
                    live.retain(|v| v.0 != oldest);
                }
            }
        }
        // Unit responses never talk over each other: the new line replaces the one still playing.
        if ev.is_unit_response_vo && first_try {
            let cut: Vec<(Entity, String)> = live.iter().filter(|v| v.7).map(|v| (v.0, v.1.clone())).collect();
            for (e, old) in cut {
                if audio_log() {
                    info!("audio sfx CUT {old} for {}", req.event);
                }
                commands.entity(e).despawn();
                live.retain(|x| x.0 != e);
            }
        }
        // Global voice limit.
        if live.len() >= settings.max_voices {
            let victim = live
                .iter()
                .filter(|v| v.3 > ev.priority || (v.3 == ev.priority && !v.6 && !voice_over))
                .max_by(|a, b| a.3.cmp(&b.3).then(b.4.total_cmp(&a.4)))
                .map(|v| v.0);
            match victim {
                Some(v) => {
                    commands.entity(v).despawn();
                    live.retain(|x| x.0 != v);
                }
                None => continue,
            }
        }
        // Distance / spatialization.
        let spatial = !ev.is_2d && req.position.is_some() && listener.is_some();
        let mut attenuation = 1.0;
        let saturation = ev.saturation_distance.map(|s| s * settings.saturation_scale);
        if let (true, Some(pos), Some(l), Some(sat)) = (spatial, req.position, listener, saturation) {
            attenuation = distance_attenuation(pos.distance(l), sat, settings.rolloff);
        }
        let volume = rng.random_range(ev.volume.0..=ev.volume.1) / 100.0;
        let base_gain = volume;
        // With `SP_AUDIO_LOG` silent (volume 0) sounds still start, so audits see the real voice rules.
        if base_gain * attenuation * buses.gain(bus_of(&ev)) < settings.min_audible_gain
            && !ev.looping
            && !audio_log()
        {
            continue;
        }
        // Kills previous.
        if ev.kills_previous
            && let Some(src) = req.source
        {
            for v in live
                .iter()
                .filter(|v| v.5 == Some(src))
                .map(|v| v.0)
                .collect::<Vec<_>>()
            {
                commands.entity(v).despawn();
                live.retain(|x| x.0 != v);
            }
        }
        // Pick the sample (pre-samples first).
        let mut follow = cont.rest.clone();
        let mut chain_event = if continuing {
            cont.chain.clone()
        } else {
            ev.chained.clone()
        };
        let sample_name = if continuing {
            cont.sample.clone()
        } else {
            let mut chain: Vec<String> = Vec::new();
            if let Some(pre) = pick(
                &ev.pre_samples,
                false,
                &format!("{key}#pre"),
                &mut state,
                &mut rng,
            ) {
                chain.push(pre);
            }
            match pick(
                &ev.samples,
                ev.play_sequentially,
                &key,
                &mut state,
                &mut rng,
            ) {
                Some(s) => chain.push(s),
                None if chain.is_empty() => continue,
                None => {}
            }
            if let Some(post) = pick(
                &ev.post_samples,
                false,
                &format!("{key}#post"),
                &mut state,
                &mut rng,
            ) {
                chain.push(post);
            }
            let first = chain.remove(0);
            follow.extend(chain);
            first
        };
        let Some((handle, _path)) =
            audio.sample(&sample_name, settings.cache_budget_bytes, &mut assets)
        else {
            // Missing file: skip the rest of the chain as well.
            continue;
        };
        let pitch = rng.random_range(ev.pitch.0..=ev.pitch.1) / 100.0;
        let mut pb = if ev.looping {
            PlaybackSettings::LOOP
        } else {
            PlaybackSettings::ONCE
        };
        pb = pb.with_speed(pitch.max(0.1));
        pb = pb.with_volume(Volume::Linear(
            if ev.loop_fade.0 > 0.0 {
                0.0
            } else {
                base_gain * attenuation
            } * buses.gain(bus_of(&ev)),
        ));
        if spatial {
            pb = pb
                .with_spatial(true)
                .with_spatial_scale(SpatialScale::new(SPATIAL_UNIT));
        }
        let voice = SfxVoice {
            event: key.clone(),
            group: ev.overlap_group.clone(),
            priority: ev.priority,
            bus: bus_of(&ev),
            base_gain,
            saturation: if spatial { saturation } else { None },
            rolloff: settings.rolloff,
            fade_in: ev.loop_fade.0,
            source: req.source,
            started: now,
            follow,
            chain: chain_event.take(),
            is_voice_over: voice_over,
            unit_response: ev.is_unit_response_vo,
            looping: ev.looping,
            fade_out: ev.loop_fade.1,
            stopping: None,
            distance: None,
            attenuation,
        };
        if audio_log() && first_try {
            info!(
                "audio sfx PLAY {} (unit_response={} spatial={} dist={} att={:.3} vol={:.2})",
                req.event,
                ev.is_unit_response_vo,
                spatial,
                match (spatial, req.position, listener) {
                    (true, Some(p), Some(l)) => format!("{:.0}", p.distance(l)),
                    _ => "-".into(),
                },
                attenuation,
                base_gain
            );
        }
        let mut ec = commands.spawn((AudioPlayer::<AudioSource>(handle), pb, voice));
        if let Some(p) = req.position {
            ec.insert(Transform::from_translation(p));
        }
        let id = ec.id();
        if ev.is_unit_response_vo {
            state.last_unit_response = Some(now);
        }
        if first_try {
            state.last_played.insert(key.clone(), now);
        }
        live.push((
            id,
            key,
            ev.overlap_group.clone(),
            ev.priority,
            now,
            req.source,
            voice_over,
            ev.is_unit_response_vo,
        ));
    }
}

/// World units per Bevy spatial unit. Large so that Bevy's own distance attenuation stays at 1 and only
/// the stereo panning is used (attenuation is applied by this plugin, see [`update_voices`]).
const SPATIAL_UNIT: f32 = 1.0 / 60_000.0;

fn pick(
    list: &[String],
    sequential: bool,
    key: &str,
    state: &mut SfxState,
    rng: &mut impl Rng,
) -> Option<String> {
    match list.len() {
        0 => None,
        1 => Some(list[0].clone()),
        n => {
            let idx = if sequential {
                let c = state.sequence.entry(key.to_string()).or_insert(0);
                let i = *c % n;
                *c += 1;
                i
            } else {
                let last = state.last_pick.get(key).copied();
                let mut i = rng.random_range(0..n);
                if Some(i) == last {
                    i = (i + 1 + rng.random_range(0..n - 1)) % n;
                }
                state.last_pick.insert(key.to_string(), i);
                i
            };
            Some(list[idx].clone())
        }
    }
}

/// Updates gains and positions, starts follow-ups and removes finished voices.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update_voices(
    mut commands: Commands,
    mut voices: Query<(
        Entity,
        &mut SfxVoice,
        Option<&mut AudioSink>,
        Option<&mut SpatialAudioSink>,
        Option<&mut Transform>,
    )>,
    sources: Query<&GlobalTransform, Without<SfxVoice>>,
    listeners: Query<&GlobalTransform, With<SpatialListener>>,
    mut writer: MessageWriter<PlaySfx>,
    mut state: ResMut<SfxState>,
    buses: Res<AudioBuses>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    let listener = listener_position(&listeners);
    for (entity, mut voice, plain, spatial, mut tf) in &mut voices {
        // Follow the emitting entity while it exists; afterwards the sound finishes where it was.
        if let Some(src) = voice.source {
            match (sources.get(src), tf.as_deref_mut()) {
                (Ok(t), Some(own)) => own.translation = t.translation(),
                (Err(_), _) => {
                    voice.source = None;
                    // A loop belongs to its object: when the object is gone the loop fades out.
                    if voice.looping && voice.stopping.is_none() {
                        voice.stopping = Some(now);
                    }
                }
                _ => {}
            }
        }
        let pos = tf.as_deref().map(|t| t.translation);
        let mut gain = voice.base_gain * buses.gain(voice.bus);
        voice.attenuation = 1.0;
        voice.distance = None;
        if let (Some(sat), Some(l), Some(p)) = (voice.saturation, listener, pos) {
            let d = p.distance(l);
            voice.distance = Some(d);
            voice.attenuation = distance_attenuation(d, sat, voice.rolloff);
        }
        gain *= voice.attenuation;
        let age = (now - voice.started) as f32;
        if voice.fade_in > 0.0 && age < voice.fade_in {
            gain *= age / voice.fade_in;
        }
        let mut stopped = false;
        if let Some(t0) = voice.stopping {
            let left = 1.0 - (now - t0) as f32 / voice.fade_out.max(0.05);
            if left <= 0.0 {
                stopped = true;
            }
            gain *= left.clamp(0.0, 1.0);
        }
        let mut finished = false;
        if let Some(mut s) = plain {
            s.set_volume(Volume::Linear(gain));
            finished = s.empty();
        }
        if let Some(mut s) = spatial {
            s.set_volume(Volume::Linear(gain));
            finished = s.empty();
        }
        if stopped {
            commands.entity(entity).despawn();
            continue;
        }
        if !finished || voice.stopping.is_some() {
            continue;
        }
        let request = PlaySfx {
            event: voice.event.clone(),
            position: pos,
            source: voice.source,
        };
        if !voice.follow.is_empty() {
            let sample = voice.follow.remove(0);
            let cont = Continuation {
                sample,
                rest: std::mem::take(&mut voice.follow),
                chain: voice.chain.take(),
            };
            state.pending.push(Pending {
                due: now,
                request,
                cont,
            });
        } else if let Some(chain) = voice.chain.take() {
            writer.write(PlaySfx {
                event: chain,
                position: pos,
                source: voice.source,
            });
        }
        commands.entity(entity).despawn();
    }
}

// ----------------------------------------------------------------------------------------------
// Object loops (engine hum, ambient machinery)
// ----------------------------------------------------------------------------------------------

/// A world object that makes one looping sound at a time, e.g. a ship's `SFXEvent_Engine_Idle_Loop` /
/// `SFXEvent_Engine_Moving_Loop` / `SFXEvent_Ambient_Loop`. Set [`event`](Self::event) to the loop the object
/// should make now; the plugin starts it (at the object's position, following it), fades it out and starts
/// the new one when the event changes, and stops sounds that are too far away to be heard. The loop's
/// `Max_Instances` still applies, so a crowd of identical ships does not turn into a wall of hum.
#[derive(Component, Debug, Clone, Default)]
pub struct SfxEmitter {
    /// Looping SFX event this object should make now (`None` = silent).
    pub event: Option<String>,
    playing: Option<String>,
}

impl SfxEmitter {
    /// Emitter that starts out making `event`.
    pub fn new(event: Option<String>) -> Self {
        Self { event, playing: None }
    }

    /// The loop that is actually running (differs from [`event`](Self::event) for one frame, or while it is inaudible).
    pub fn playing(&self) -> Option<&str> {
        self.playing.as_deref()
    }
}

/// Estimated linear gain of an event at `distance` (mean of its volume range, distance falloff, bus gain).
fn estimated_gain(ev: &SfxEvent, distance: Option<f32>, buses: &AudioBuses, settings: &AudioSettings) -> f32 {
    // With `SP_AUDIO_LOG` silent (volume 0) sounds still start so audits see the real voice rules.
    let bus = if audio_log() { 1.0 } else { buses.gain(bus_of(ev)) };
    let mut g = (ev.volume.0 + ev.volume.1) * 0.5 / 100.0 * bus;
    if let (Some(d), Some(sat)) = (distance, ev.saturation_distance) {
        g *= distance_attenuation(d, sat * settings.saturation_scale, settings.rolloff);
    }
    g
}

#[allow(clippy::too_many_arguments)]
fn sync_emitters(
    mut emitters: Query<(Entity, &mut SfxEmitter, &GlobalTransform)>,
    mut voices: Query<&mut SfxVoice>,
    listeners: Query<&GlobalTransform, With<SpatialListener>>,
    audio: Res<AlamoAudio>,
    buses: Res<AudioBuses>,
    settings: Res<AudioSettings>,
    time: Res<Time>,
    mut out: MessageWriter<PlaySfx>,
) {
    let now = time.elapsed_secs_f64();
    let listener = listener_position(&listeners);
    // Nearest first: when an event has a `Max_Instances` cap, the objects closest to the listener get the voices.
    let mut order: Vec<(Entity, Vec3, Option<f32>)> = emitters
        .iter()
        .map(|(e, _, tf)| (e, tf.translation(), listener.map(|l| tf.translation().distance(l))))
        .collect();
    order.sort_by(|a, b| a.2.unwrap_or(0.0).total_cmp(&b.2.unwrap_or(0.0)));
    let mut taken: HashMap<String, u32> = HashMap::new();
    for (entity, pos, distance) in order {
        let Ok((_, mut em, _)) = emitters.get_mut(entity) else { continue };
        // Hysteresis: a loop starts at twice the audible gain and stops below it.
        let desired = em.event.clone().filter(|name| {
            let Some(ev) = audio.sfx.get(name) else { return false };
            let g = estimated_gain(ev, distance, &buses, &settings);
            let floor = settings.min_audible_gain;
            let audible = if em.playing.as_deref() == Some(name.as_str()) { g >= floor } else { g >= floor * 2.0 };
            let n = taken.entry(name.to_ascii_lowercase()).or_default();
            let free = ev.max_instances.is_none_or(|m| *n < m);
            if audible && free {
                *n += 1;
            }
            audible && free
        });
        if desired == em.playing {
            continue;
        }
        if let Some(old) = em.playing.take() {
            for mut v in &mut voices {
                if v.source == Some(entity) && v.looping && v.event.eq_ignore_ascii_case(&old) && v.stopping.is_none() {
                    v.stopping = Some(now);
                }
            }
        }
        if let Some(new) = desired {
            if audio_log() {
                info!("audio emitter {entity:?} -> {new}");
            }
            out.write(PlaySfx { event: new.clone(), position: Some(pos), source: Some(entity) });
            em.playing = Some(new);
        }
    }
}

// ----------------------------------------------------------------------------------------------
// Music
// ----------------------------------------------------------------------------------------------

/// Selects music. Set the [`scene`](MusicScene) and, during battles, the intensity (0..1, e.g. the share
/// of ships in combat); the [`MusicPlanner`] inside switches between ambient and battle playlists with
/// hysteresis and this director crossfades.
#[derive(Resource, Debug, Default)]
pub struct MusicDirector {
    /// Data-driven track selection (tunable thresholds live here).
    pub planner: MusicPlanner,
    current: Option<String>,
    queue: Vec<String>,
    oneshot_done: bool,
    /// Scene to switch to once the current result stinger has played to its end.
    resume: Option<MusicScene>,
}

impl MusicDirector {
    /// Switches the scene; the change is applied (with crossfade) on the next frame.
    pub fn set_scene(&mut self, scene: MusicScene) {
        if self.planner.scene() != &scene {
            self.resume = None;
        }
        self.planner.set_scene(scene);
    }

    /// Switches to `scene` once a playing win / lose stinger has finished (the stinger is not cut off by the
    /// screen change that follows a battle). Without a stinger it is the same as [`set_scene`](Self::set_scene).
    pub fn resume_after_stinger(&mut self, scene: MusicScene) {
        if matches!(self.planner.scene(), MusicScene::Result { tactical: true, .. }) {
            self.resume = Some(scene);
        } else {
            self.set_scene(scene);
        }
    }

    /// True while a result stinger is the scene.
    pub fn stinger_playing(&self) -> bool {
        matches!(self.planner.scene(), MusicScene::Result { .. }) && !self.oneshot_done
    }

    /// Battle intensity in 0..1 (only used in [`MusicScene::Space`]).
    pub fn set_battle_intensity(&mut self, intensity: f32) {
        self.planner.set_battle_intensity(intensity);
    }

    /// The scene currently set.
    pub fn scene(&self) -> &MusicScene {
        self.planner.scene()
    }

    /// Name of the music event that is playing (or about to).
    pub fn current_event(&self) -> Option<&str> {
        self.current.as_deref()
    }
}

/// A playing music track.
#[derive(Component, Debug)]
pub struct MusicTrack {
    /// Music event the track belongs to.
    pub event: String,
    volume: f32,
    /// Current fade factor 0..1.
    fade: f32,
    /// Fade speed per second (positive in, negative out).
    fade_rate: f32,
    outgoing: bool,
    forced_end: bool,
}

impl MusicTrack {
    /// Ends the track as if its sample had played out. For tests that run without an audio device (no sink).
    pub fn end_now(&mut self) {
        self.forced_end = true;
    }

    /// True while the track fades out because another music event took over.
    pub fn is_outgoing(&self) -> bool {
        self.outgoing
    }
}

#[allow(clippy::too_many_arguments)]
fn start_track(
    commands: &mut Commands,
    audio: &mut AlamoAudio,
    assets: &mut Assets<AudioSource>,
    budget: usize,
    event: &MusicEvent,
    file: &str,
    fade_in: f32,
    stinger: bool,
) -> bool {
    let Some((handle, _)) = audio.sample(file, budget, assets) else {
        return false;
    };
    let single_loop = event.files.len() == 1 && event.looping && !stinger;
    if audio_log() {
        info!("audio music track {} (event {}, fade_in {:.2}s, loop {})", file, event.name, fade_in, single_loop);
    }
    let pb = if single_loop {
        PlaybackSettings::LOOP
    } else {
        PlaybackSettings::ONCE
    }
    .with_volume(Volume::Linear(0.0));
    commands.spawn((
        AudioPlayer::<AudioSource>(handle),
        pb,
        MusicTrack {
            event: event.name.clone(),
            volume: event.volume_percent / 100.0,
            fade: if fade_in > 0.0 { 0.0 } else { 1.0 },
            fade_rate: if fade_in > 0.0 { 1.0 / fade_in } else { 0.0 },
            outgoing: false,
            forced_end: false,
        },
    ));
    true
}

#[allow(clippy::too_many_arguments)]
fn music_system(
    mut commands: Commands,
    mut director: ResMut<MusicDirector>,
    mut audio: ResMut<AlamoAudio>,
    mut assets: ResMut<Assets<AudioSource>>,
    settings: Res<AudioSettings>,
    time: Res<Time>,
    mut tracks: Query<(Entity, &mut MusicTrack, Option<&AudioSink>)>,
) {
    let dt = time.delta_secs();
    director.planner.step(dt);
    let wanted = director.planner.wanted_event(&audio.music);
    let changed = wanted != director.current;
    if changed {
        director.current = wanted.clone();
        if audio_log() {
            info!("audio music scene {:?} -> event {:?}", director.planner.scene(), wanted);
        }
        director.queue.clear();
        director.oneshot_done = false;
        let fade_out = wanted
            .as_deref()
            .and_then(|n| audio.music.get(n))
            .map(|e| e.fade_out_previous)
            .filter(|f| *f > 0.0)
            .unwrap_or(settings.default_music_fade);
        for (_, mut t, _) in &mut tracks {
            if !t.outgoing {
                t.outgoing = true;
                t.fade_rate = -1.0 / fade_out.max(0.05);
            }
        }
    }
    // Finished / faded-out tracks.
    let mut incoming_alive = false;
    let mut need_next = false;
    for (entity, mut t, sink) in &mut tracks {
        t.fade = (t.fade + t.fade_rate * dt).clamp(0.0, 1.0);
        if t.outgoing && t.fade <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        if !t.outgoing {
            if t.fade >= 1.0 {
                t.fade_rate = 0.0;
            }
            incoming_alive = true;
            if t.forced_end || sink.is_some_and(|s| s.empty()) {
                commands.entity(entity).despawn();
                incoming_alive = false;
                need_next = true;
            }
        }
    }
    let stinger = matches!(director.planner.scene(), MusicScene::Result { .. });
    // A result stinger without a playable event must not hold the scene back.
    let playable = director.current.as_deref().and_then(|n| audio.music.get(n)).is_some_and(|e| !e.files.is_empty());
    if stinger && !playable {
        director.oneshot_done = true;
    }
    if stinger && director.oneshot_done && let Some(next) = director.resume.take() {
        if audio_log() {
            info!("audio music stinger done -> resume {:?}", next);
        }
        director.planner.set_scene(next);
        return;
    }
    let Some(name) = director.current.clone() else {
        return;
    };
    if incoming_alive || director.oneshot_done {
        return;
    }
    let Some(event) = audio.music.get(&name).cloned() else {
        return;
    };
    if event.files.is_empty() {
        return;
    }
    if director.queue.is_empty() {
        if need_next && (!event.looping || stinger) {
            director.oneshot_done = true; // stinger or non-looping playlist finished
            return;
        }
        let mut order = event.files.clone();
        order.shuffle(&mut rand::rng());
        director.queue = order;
    }
    // The event's own `Fade_In_Seconds` (the previous music fades out over its `Fade_Out_Previous_Seconds`); the
    // floor only avoids a click. Following tracks of a playlist ease in over half a second.
    let fade_in = if changed { event.fade_in.max(0.05) } else { 0.5 };
    while let Some(file) = director.queue.pop() {
        if start_track(
            &mut commands,
            &mut audio,
            &mut assets,
            settings.cache_budget_bytes,
            &event,
            &file,
            fade_in,
            stinger,
        ) {
            break;
        }
    }
}
fn update_music_gain(
    buses: Res<AudioBuses>,
    mut tracks: Query<(&MusicTrack, Option<&mut AudioSink>)>,
) {
    for (t, sink) in &mut tracks {
        if let Some(mut s) = sink {
            s.set_volume(Volume::Linear(t.volume * t.fade * buses.gain(Bus::Music)));
        }
    }
}

/// `SP_AUDIO_LOG=1` logs every sound request (SFX, files, music scene changes) for audits.
fn audio_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SP_AUDIO_LOG").is_some())
}
