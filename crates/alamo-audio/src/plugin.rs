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

use crate::ledger::{LedgerEntry, LedgerKind, LedgerOutcome, SoundLedger};
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
    /// Gain of a bus without the master volume (the balance between the buses, independent of `--volume`).
    pub fn gain_no_master(&self, bus: Bus) -> f32 {
        match bus {
            Bus::Music => self.music,
            Bus::Sfx => self.sfx,
            Bus::Speech => self.speech,
            Bus::Ambient => self.ambient * self.sfx,
        }
    }

    /// Combined gain of a bus including master.
    pub fn gain(&self, bus: Bus) -> f32 {
        self.master * self.gain_no_master(bus)
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
    /// What `Max_Instances` does when the limit is reached by an ordinary sound: false = the oldest instance fades out (the default; the data
    /// does not say which one the engine does), true = the new request is dropped and the playing ones finish. `SP_AUDIO_MAXINST=drop` switches it
    /// for an A/B listen.
    pub max_instances_drop_new: bool,
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
            max_instances_drop_new: std::env::var("SP_AUDIO_MAXINST").is_ok_and(|v| v.eq_ignore_ascii_case("drop")),
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

/// Plays a `SpeechEvent` of `SpeechEvents.xml` (campaign / HUD lines such as `EHD_Death_Star_Begin_Approach`) on the speech bus.
/// Unlike an SFX event it has no voice rules; an unknown event or a missing file is reported in the [`SoundLedger`].
#[derive(Message, Debug, Clone)]
pub struct PlaySpeech {
    /// Speech event name (case-insensitive).
    pub event: String,
}

fn process_speech_requests(
    mut reader: MessageReader<PlaySpeech>,
    mut ledger: ResMut<SoundLedger>,
    time: Res<Time>,
    mut audio: ResMut<AlamoAudio>,
    mut assets: ResMut<Assets<AudioSource>>,
    settings: Res<AudioSettings>,
    buses: Res<AudioBuses>,
    mut commands: Commands,
) {
    let t = time.elapsed_secs_f64();
    for req in reader.read() {
        let note = |ledger: &mut SoundLedger, sample: &str, path: Option<String>, outcome| {
            ledger.record(LedgerEntry { t, kind: LedgerKind::Speech, event: req.event.clone(), sample: sample.to_string(), path, outcome });
        };
        if audio_log() {
            info!("audio speech {}", req.event);
        }
        let Some(line) = audio.speech.get(&req.event).cloned() else {
            note(&mut ledger, "", None, LedgerOutcome::UnknownEvent);
            continue;
        };
        let usable: Vec<&String> = line.files.iter().filter(|f| audio.has_sample(f)).collect();
        for f in line.files.iter().filter(|f| !audio.has_sample(f)) {
            note(&mut ledger, f, None, LedgerOutcome::MissingFile);
        }
        if line.files.is_empty() {
            note(&mut ledger, "", None, LedgerOutcome::NoSamples);
            continue;
        }
        if usable.is_empty() {
            continue;
        }
        let file = usable[rand::rng().random_range(0..usable.len())].clone();
        match audio.sample_checked(&file, settings.cache_budget_bytes, &mut assets) {
            Ok((handle, path)) => {
                note(&mut ledger, &file, Some(path), LedgerOutcome::Played);
                let gain = line.volume_percent / 100.0 * buses.gain(Bus::Speech);
                commands.spawn((AudioPlayer::<AudioSource>(handle), PlaybackSettings::ONCE.with_volume(Volume::Linear(gain))));
            }
            Err((outcome, path)) => note(&mut ledger, &file, path, outcome),
        }
    }
}

fn process_file_requests(
    mut reader: MessageReader<PlayFile>,
    mut ledger: ResMut<SoundLedger>,
    time: Res<Time>,
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
        let (handle, path) = match audio.sample_checked(&req.name, settings.cache_budget_bytes, &mut assets) {
            Ok(v) => v,
            Err((outcome, path)) => {
                ledger.record(LedgerEntry {
                    t: time.elapsed_secs_f64(),
                    kind: LedgerKind::File,
                    event: req.name.clone(),
                    sample: req.name.clone(),
                    path,
                    outcome,
                });
                continue;
            }
        };
        ledger.record(LedgerEntry {
            t: time.elapsed_secs_f64(),
            kind: LedgerKind::File,
            event: req.name.clone(),
            sample: req.name.clone(),
            path: Some(path),
            outcome: LedgerOutcome::Played,
        });
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
    failed: HashMap<String, LedgerOutcome>,
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

    /// True if the sample resolves to a file in the file system.
    pub fn has_sample(&self, name: &str) -> bool {
        self.resolver.resolve(&*self.fs, name).is_some()
    }

    /// Resolves, reads, decodes (WAV -> PCM16) and caches a sample. `None` if it is missing or undecodable.
    pub fn sample(
        &mut self,
        name: &str,
        budget: usize,
        assets: &mut Assets<AudioSource>,
    ) -> Option<(Handle<AudioSource>, String)> {
        self.sample_checked(name, budget, assets).ok()
    }

    /// Like [`AlamoAudio::sample`], but says why a sample is unavailable: `MissingFile` (not in the file system, with the
    /// path when one was resolved but unreadable) or `BadFile` (found, but not decodable).
    pub fn sample_checked(
        &mut self,
        name: &str,
        budget: usize,
        assets: &mut Assets<AudioSource>,
    ) -> Result<(Handle<AudioSource>, String), (LedgerOutcome, Option<String>)> {
        let Some((path, _)) = self.resolver.resolve(&*self.fs, name) else {
            return Err((LedgerOutcome::MissingFile, None));
        };
        if let Some(h) = self.cache.get(&path) {
            return Ok((h.clone(), path));
        }
        if let Some(why) = self.failed.get(&path).copied() {
            return Err((why, Some(path)));
        }
        let bytes = match self.fs.read(&path) {
            Ok(b) => b,
            Err(_) => {
                self.failed.insert(path.clone(), LedgerOutcome::MissingFile);
                return Err((LedgerOutcome::MissingFile, Some(path)));
            }
        };
        let bytes = if path.ends_with(".WAV") {
            match Wav::parse(&bytes).and_then(|w| w.decode()) {
                Ok(pcm) => pcm.to_wav_bytes(),
                Err(e) => {
                    warn!("cannot decode {path}: {e}");
                    self.failed.insert(path.clone(), LedgerOutcome::BadFile);
                    return Err((LedgerOutcome::BadFile, Some(path)));
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
        Ok((handle, path))
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
            .insert_resource(SoundLedger::new(ledger_enabled()))
            .insert_resource(director)
            .add_message::<PlaySfx>()
            .add_message::<GalaxyAudio>()
            .add_message::<PlayFile>()
            .add_message::<PlaySpeech>()
            .add_systems(
                Update,
                (
                    sync_emitters,
                    galaxy_audio_system,
                    process_sfx_requests,
                    process_file_requests,
                    process_speech_requests,
                    update_voices,
                    music_system,
                    update_music_gain,
                )
                    .chain(),
            );
        if audio_log() {
            app.add_systems(Update, log_balance);
        }
        if ledger_enabled() {
            app.add_systems(Last, dump_ledger_on_exit);
        }
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
    mut ledger: ResMut<SoundLedger>,
    mut reader: MessageReader<PlaySfx>,
    mut audio: ResMut<AlamoAudio>,
    mut state: ResMut<SfxState>,
    mut assets: ResMut<Assets<AudioSource>>,
    settings: Res<AudioSettings>,
    buses: Res<AudioBuses>,
    time: Res<Time>,
    mut voices: Query<(Entity, &mut SfxVoice)>,
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
            ledger.record(LedgerEntry {
                t: now,
                kind: LedgerKind::Sfx,
                event: req.event.clone(),
                sample: String::new(),
                path: None,
                outcome: LedgerOutcome::UnknownEvent,
            });
            continue;
        };
        if audio_log() && cont.is_none() {
            info!("audio sfx {} (at {:?})", req.event, req.position.map(|p| p.round()));
        }
        if ev.is_preset {
            continue;
        }
        // A request the data rules drop is written down with the rule (it is not a problem, but a QA run wants to see it).
        macro_rules! dropped {
            ($why:expr) => {{
                ledger.record(LedgerEntry {
                    t: now,
                    kind: LedgerKind::Sfx,
                    event: req.event.clone(),
                    sample: $why.to_string(),
                    path: None,
                    outcome: LedgerOutcome::Dropped,
                });
                continue;
            }};
        }
        let first_try = cont.is_none();
        // Probability and predelay apply to fresh requests only.
        if first_try {
            if ev.probability < 100.0 && rng.random_range(0.0..100.0) >= ev.probability {
                dropped!("probability");
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
                dropped!("unit response cooldown");
            }
            if state
                .last_played
                .get(&key)
                .is_some_and(|t| now - t < f64::from(settings.speech_event_cooldown))
            {
                dropped!("event cooldown");
            }
        }
        // Overlap groups. A unit response is the exception: EaW plays the move line of an order right after the
        // select line, so the newer response cuts the older one (below) instead of being dropped.
        if !ev.is_unit_response_vo
            && let Some(g) = &ev.overlap_group
            && live.iter().any(|v| v.2.as_deref() == Some(g.as_str()))
        {
            dropped!(format!("overlap group {g}"));
        }
        // Decide audibility first: an inaudible request must not cut an audible voice (max instances, voice limit).
        {
            let sat = ev.saturation_distance.map(|s| s * settings.saturation_scale);
            let att = match (!ev.is_2d && req.position.is_some() && listener.is_some(), req.position, listener, sat) {
                (true, Some(p), Some(l), Some(s)) => distance_attenuation(p.distance(l), s, settings.rolloff),
                _ => 1.0,
            };
            if ev.volume.1 / 100.0 * att * buses.gain(bus_of(&ev)) < settings.min_audible_gain && !ev.looping && !ledger_enabled() {
                continue;
            }
        }
        // Max instances.
        if let Some(max) = ev.max_instances {
            let same: Vec<_> = live.iter().filter(|v| v.1 == key).collect();
            if same.len() as u32 >= max {
                if voice_over {
                    dropped!(format!("max instances {max} (voice over)"));
                }
                if settings.max_instances_drop_new {
                    dropped!(format!("max instances {max}"));
                }
                if let Some(oldest) = same.iter().min_by(|a, b| a.4.total_cmp(&b.4)).map(|v| v.0) {
                    cut_voice(&mut commands, &mut voices, oldest, now);
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
                cut_voice(&mut commands, &mut voices, e, now);
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
                    cut_voice(&mut commands, &mut voices, v, now);
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
            && !ledger_enabled()
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
                cut_voice(&mut commands, &mut voices, v, now);
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
            let pre_list = usable_samples(&audio, &mut ledger, now, &req.event, &ev.pre_samples);
            let main_list = usable_samples(&audio, &mut ledger, now, &req.event, &ev.samples);
            let post_list = usable_samples(&audio, &mut ledger, now, &req.event, &ev.post_samples);
            if let Some(pre) = pick(
                &pre_list,
                false,
                &format!("{key}#pre"),
                &mut state,
                &mut rng,
            ) {
                chain.push(pre);
            }
            match pick(
                &main_list,
                ev.play_sequentially,
                &key,
                &mut state,
                &mut rng,
            ) {
                Some(s) => chain.push(s),
                None if chain.is_empty() => {
                    ledger.record(LedgerEntry {
                        t: now,
                        kind: LedgerKind::Sfx,
                        event: req.event.clone(),
                        sample: String::new(),
                        path: None,
                        outcome: LedgerOutcome::NoSamples,
                    });
                    continue;
                }
                None => {}
            }
            if let Some(post) = pick(
                &post_list,
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
        let (handle, path) = match audio.sample_checked(&sample_name, settings.cache_budget_bytes, &mut assets) {
            Ok(v) => v,
            Err((outcome, path)) => {
                // Missing file: skip the rest of the chain as well, but say so.
                ledger.record(LedgerEntry {
                    t: now,
                    kind: LedgerKind::Sfx,
                    event: req.event.clone(),
                    sample: sample_name.clone(),
                    path,
                    outcome,
                });
                continue;
            }
        };
        ledger.record(LedgerEntry {
            t: now,
            kind: LedgerKind::Sfx,
            event: req.event.clone(),
            sample: sample_name.clone(),
            path: Some(path),
            outcome: LedgerOutcome::Played,
        });
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
                "audio sfx PLAY {} (unit_response={} spatial={} dist={} att={:.3} vol={:.2} bus={:?} eff={:.3})",
                req.event,
                ev.is_unit_response_vo,
                spatial,
                match (spatial, req.position, listener) {
                    (true, Some(p), Some(l)) => format!("{:.0}", p.distance(l)),
                    _ => "-".into(),
                },
                attenuation,
                base_gain,
                bus_of(&ev),
                base_gain * attenuation * buses.gain_no_master(bus_of(&ev))
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

/// Seconds a voice takes to fade out when another sound cuts it (Max_Instances, voice limit, unit-response replacement): an abrupt
/// despawn of a playing sample clicks, and at battle fire rates it happens many times a second.
const CUT_FADE_SECONDS: f32 = 0.06;

/// Cuts a playing voice with a short fade; a voice that was spawned this frame (not yet in the query) is despawned at once.
fn cut_voice(commands: &mut Commands, voices: &mut Query<(Entity, &mut SfxVoice)>, e: Entity, now: f64) {
    match voices.get_mut(e) {
        Ok((_, mut v)) => {
            if v.stopping.is_none() {
                v.stopping = Some(now);
                v.fade_out = CUT_FADE_SECONDS;
            }
        }
        Err(_) => commands.entity(e).despawn(),
    }
}

/// The samples of `list` that exist in the file system; a missing one is reported (once per sample) and never picked, so one
/// absent file of a multi-sample event does not make the event randomly silent.
fn usable_samples(audio: &AlamoAudio, ledger: &mut SoundLedger, now: f64, event: &str, list: &[String]) -> Vec<String> {
    list.iter()
        .filter(|s| {
            let ok = audio.has_sample(s);
            if !ok {
                ledger.record(LedgerEntry {
                    t: now,
                    kind: LedgerKind::Sfx,
                    event: event.to_string(),
                    sample: (*s).clone(),
                    path: None,
                    outcome: LedgerOutcome::MissingFile,
                });
            }
            ok
        })
        .cloned()
        .collect()
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

/// Marks a run without an audio output (`--no-audio`, `--volume 0`): Bevy's `AudioPlugin` is not added, so no sink ever
/// exists. Voices then end by a fixed age instead of `AudioSink::empty`, and the request / log logic runs as usual.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct NoAudioDevice;

/// Age (seconds) at which a voice counts as finished when there is no audio device.
const NO_DEVICE_VOICE_SECONDS: f32 = 2.0;

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
    no_device: Option<Res<NoAudioDevice>>,
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
        let mut finished = no_device.is_some() && plain.is_none() && spatial.is_none() && age >= NO_DEVICE_VOICE_SECONDS;
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
    let bus = if ledger_enabled() { 1.0 } else { buses.gain(bus_of(ev)) };
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

/// `Music_Event_Space_Battle_Super_Weapon` of the faction: the battle theme while a super weapon (Death Star) is in the fight and the battle
/// playlist is selected; `None` otherwise (the normal ambient / battle themes apply).
fn super_weapon_theme<'a>(c: &'a crate::constants::AudioConstants, faction: &str, in_battle: bool, super_weapon: bool) -> Option<&'a str> {
    if in_battle && super_weapon { c.super_weapon_battle_music(faction) } else { None }
}

/// The event `Xml/Factions.xml` names for the current scene (battle load screen, win / lose themes incl. the versus-faction variants),
/// when it exists and at least one of its files is in the file system; otherwise the planner's naming convention applies.
fn faction_music_event(director: &MusicDirector, audio: &AlamoAudio) -> Option<String> {
    let c = &audio.constants;
    let candidates: Vec<&str> = match director.planner.scene() {
        MusicScene::BattleLoading => c.battle_load_music(director.faction.as_deref()?).into_iter().collect(),
        MusicScene::Result { faction, win, tactical } => c.result_music(faction, director.versus.as_deref(), *win, *tactical),
        MusicScene::Space(f) => super_weapon_theme(c, f, director.planner.in_battle(), director.super_weapon).into_iter().collect(),
        _ => return None,
    };
    candidates.into_iter().find_map(|n| {
        let e = audio.music.get(n)?;
        e.files.iter().any(|f| audio.has_sample(f)).then(|| e.name.clone())
    })
}

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
    /// Faction of the last galaxy / space / result scene (the battle load screen has none of its own).
    faction: Option<String>,
    /// Faction the player fought, for the `Music_Event_*_Vs_Faction` win / lose themes.
    versus: Option<String>,
    /// A super weapon (Death Star) is in the space battle: its faction theme replaces the battle playlist.
    super_weapon: bool,
}

impl MusicDirector {
    /// Switches the scene; the change is applied (with crossfade) on the next frame.
    pub fn set_scene(&mut self, scene: MusicScene) {
        if let MusicScene::Galactic(f) | MusicScene::Space(f) | MusicScene::Result { faction: f, .. } = &scene {
            self.faction = Some(f.clone());
        }
        if !matches!(scene, MusicScene::Result { .. }) {
            self.versus = None;
        }
        if !matches!(scene, MusicScene::Space(_)) {
            self.super_weapon = false;
        }
        if self.planner.scene() != &scene {
            self.resume = None;
        }
        self.planner.set_scene(scene);
    }

    /// The faction the player fought (`None` = unknown): selects the `Music_Event_*_Vs_Faction` win / lose theme of the next result scene.
    pub fn set_opponent(&mut self, faction: Option<String>) {
        self.versus = faction;
    }

    /// Switches to `scene` once a playing win / lose stinger has finished (the stinger is not cut off by the
    /// screen change that follows a battle). Without a stinger it is the same as [`set_scene`](Self::set_scene).
    pub fn resume_after_stinger(&mut self, scene: MusicScene) {
        if matches!(self.planner.scene(), MusicScene::Result { tactical: true, .. } | MusicScene::Event(_)) {
            self.resume = Some(scene);
        } else {
            self.set_scene(scene);
        }
    }

    /// Plays a music event once (it is not looped, whatever its `Loop` says) and then goes on with `resume`, e.g. the Death Star
    /// activation theme over the galaxy map.
    pub fn play_once_then(&mut self, event: &str, resume: MusicScene) {
        self.set_scene(MusicScene::Event(event.to_owned()));
        self.resume = Some(resume);
    }

    /// True while a result stinger or a one-shot event is the scene.
    pub fn stinger_playing(&self) -> bool {
        matches!(self.planner.scene(), MusicScene::Result { .. } | MusicScene::Event(_)) && !self.oneshot_done
    }

    /// Battle intensity in 0..1 (only used in [`MusicScene::Space`]).
    pub fn set_battle_intensity(&mut self, intensity: f32) {
        self.planner.set_battle_intensity(intensity);
    }

    /// A super weapon is in the space battle (`Music_Event_Space_Battle_Super_Weapon` plays instead of the battle theme).
    pub fn set_super_weapon(&mut self, present: bool) {
        self.super_weapon = present;
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
    /// Ducking factor applied when speech is active (1.0 = normal, 0.45 = ducked).
    pub duck: f32,
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

    /// Ducking factor applied when speech is active (1.0 = normal, < 1.0 = ducked).
    pub fn duck_factor(&self) -> f32 {
        self.duck
    }
}

#[allow(clippy::too_many_arguments)]
fn start_track(
    commands: &mut Commands,
    audio: &mut AlamoAudio,
    assets: &mut Assets<AudioSource>,
    ledger: &mut SoundLedger,
    now: f64,
    budget: usize,
    event: &MusicEvent,
    file: &str,
    fade_in: f32,
    stinger: bool,
) -> bool {
    let (handle, path) = match audio.sample_checked(file, budget, assets) {
        Ok(v) => v,
        Err((outcome, path)) => {
            ledger.record(LedgerEntry { t: now, kind: LedgerKind::Music, event: event.name.clone(), sample: file.to_string(), path, outcome });
            return false;
        }
    };
    ledger.record(LedgerEntry {
        t: now,
        kind: LedgerKind::Music,
        event: event.name.clone(),
        sample: file.to_string(),
        path: Some(path),
        outcome: LedgerOutcome::Played,
    });
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
            duck: 1.0,
        },
    ));
    true
}

#[allow(clippy::too_many_arguments)]
fn music_system(
    mut commands: Commands,
    mut ledger: ResMut<SoundLedger>,
    mut director: ResMut<MusicDirector>,
    mut audio: ResMut<AlamoAudio>,
    mut assets: ResMut<Assets<AudioSource>>,
    settings: Res<AudioSettings>,
    time: Res<Time>,
    mut tracks: Query<(Entity, &mut MusicTrack, Option<&AudioSink>)>,
) {
    let dt = time.delta_secs();
    director.planner.step(dt);
    let wanted = faction_music_event(&director, &audio).or_else(|| director.planner.wanted_event(&audio.music));
    let changed = wanted != director.current;
    if changed {
        director.current = wanted.clone();
        if audio_log() {
            info!("audio music scene {:?} -> event {:?}", director.planner.scene(), wanted);
        }
        director.queue.clear();
        director.oneshot_done = false;
        let is_result_stinger = matches!(director.planner.scene(), MusicScene::Result { .. });
        let fade_out = if is_result_stinger {
            0.35 // Quick, punchy fade-out of battle music so the victory/defeat fanfare starts promptly
        } else {
            wanted
                .as_deref()
                .and_then(|n| audio.music.get(n))
                .map(|e| e.fade_out_previous)
                .filter(|f| *f > 0.0)
                .unwrap_or(settings.default_music_fade)
        };
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
    let stinger = matches!(director.planner.scene(), MusicScene::Result { .. } | MusicScene::Event(_));
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
        if changed {
            ledger.record(LedgerEntry { t: time.elapsed_secs_f64(), kind: LedgerKind::Music, event: name, sample: String::new(), path: None, outcome: LedgerOutcome::UnknownEvent });
        }
        return;
    };
    if event.files.is_empty() {
        if changed {
            ledger.record(LedgerEntry { t: time.elapsed_secs_f64(), kind: LedgerKind::Music, event: name, sample: String::new(), path: None, outcome: LedgerOutcome::NoSamples });
        }
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
    let fade_in = if changed {
        if matches!(director.planner.scene(), MusicScene::Result { .. }) {
            0.05
        } else {
            event.fade_in.max(0.05)
        }
    } else {
        0.5
    };
    while let Some(file) = director.queue.pop() {
        if start_track(
            &mut commands,
            &mut audio,
            &mut assets,
            &mut ledger,
            time.elapsed_secs_f64(),
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
    time: Res<Time>,
    buses: Res<AudioBuses>,
    voices: Query<&SfxVoice>,
    mut tracks: Query<(&mut MusicTrack, Option<&mut AudioSink>)>,
    mut duck_level: Local<f32>,
) {
    if *duck_level <= 0.0 {
        *duck_level = 1.0;
    }
    let speech_active = voices.iter().any(|v| v.bus == Bus::Speech && v.stopping.is_none());
    let target_duck = if speech_active { 0.45 } else { 1.0 };
    let dt = (time.delta_secs() as f32).min(0.1);
    let rate = if target_duck < *duck_level { 4.0 } else { 1.25 };
    *duck_level += (target_duck - *duck_level) * (dt * rate).min(1.0);

    for (mut t, sink) in &mut tracks {
        t.duck = *duck_level;
        if let Some(mut s) = sink {
            s.set_volume(Volume::Linear(t.volume * t.fade * t.duck * buses.gain(Bus::Music)));
        }
    }
}

/// `SP_AUDIO_LEDGER=<path>` (or `SP_AUDIO_LOG`) records every sound request in the [`SoundLedger`]; the TSV is written to the path
/// every 2 s and on exit.
fn ledger_enabled() -> bool {
    audio_log() || std::env::var_os("SP_AUDIO_LEDGER").is_some()
}

fn dump_ledger_on_exit(
    mut exits: MessageReader<AppExit>,
    ledger: Res<SoundLedger>,
    time: Res<Time<Real>>,
    mut last: Local<(f64, usize)>,
) {
    // The exit message can be written after this system ran in the last frame, so the file is also refreshed every 2 s.
    let exiting = exits.read().next().is_some();
    let now = time.elapsed_secs_f64();
    if !exiting && (now - last.0 < 2.0 || last.1 == ledger.entries.len()) {
        return;
    }
    *last = (now, ledger.entries.len());
    if exiting {
        info!("audio ledger summary:\n{}", ledger.summary());
    }
    if let Some(path) = std::env::var_os("SP_AUDIO_LEDGER")
        && let Err(e) = std::fs::write(&path, ledger.to_tsv())
    {
        warn!("cannot write audio ledger {path:?}: {e}");
    }
}

/// `SP_AUDIO_LOG=1` logs every sound request (SFX, files, music scene changes) for audits.
fn audio_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SP_AUDIO_LOG").is_some())
}

/// Per-bus loudness of what is playing now, as the `audio balance` log line: `music=<n>x peak avg ...`.
/// Gains are `volume x distance attenuation x bus` **without** the master volume, so the line shows the mix
/// balance even in a `--volume 0` run.
pub fn balance_line(items: &[(Bus, f32)]) -> String {
    let mut out = String::from("audio balance");
    for (name, bus) in [("music", Bus::Music), ("speech", Bus::Speech), ("sfx", Bus::Sfx), ("ambient", Bus::Ambient)] {
        let g: Vec<f32> = items.iter().filter(|i| i.0 == bus).map(|i| i.1).collect();
        let peak = g.iter().copied().fold(0.0_f32, f32::max);
        let avg = if g.is_empty() { 0.0 } else { g.iter().sum::<f32>() / g.len() as f32 };
        out.push_str(&format!(" {name}[n={} peak={peak:.2} avg={avg:.2}]", g.len()));
    }
    out
}

/// `SP_AUDIO_LOG=1`: every 5 s of game time logs the loudness of the playing music, speech, SFX and ambient sounds.
fn log_balance(
    time: Res<Time>,
    buses: Res<AudioBuses>,
    voices: Query<&SfxVoice>,
    tracks: Query<&MusicTrack>,
    mut next: Local<f64>,
) {
    let now = time.elapsed_secs_f64();
    if now < *next {
        return;
    }
    *next = now + 5.0;
    let mut items: Vec<(Bus, f32)> = voices
        .iter()
        .filter(|v| v.stopping.is_none())
        .map(|v| (v.bus, v.base_gain * v.attenuation * buses.gain_no_master(v.bus)))
        .collect();
    items.extend(tracks.iter().filter(|t| !t.outgoing).map(|t| (Bus::Music, t.volume * t.fade * buses.gain_no_master(Bus::Music))));
    if !items.is_empty() {
        info!("{}", balance_line(&items));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::AudioConstants;

    #[test]
    fn death_star_theme_only_in_the_battle_playlist_with_a_super_weapon() {
        let xml = br#"<F><Faction Name="Empire"><Music_Event_Space_Battle_Super_Weapon> Space_Map_Empire_Battle_Death_Star_Music_Event </Music_Event_Space_Battle_Super_Weapon></Faction></F>"#;
        let c = AudioConstants { faction_music: AudioConstants::parse_faction_music(xml), ..Default::default() };
        assert_eq!(super_weapon_theme(&c, "Empire", true, true), Some("Space_Map_Empire_Battle_Death_Star_Music_Event"));
        assert_eq!(super_weapon_theme(&c, "Empire", false, true), None, "ambient phase keeps the ambient theme");
        assert_eq!(super_weapon_theme(&c, "Empire", true, false), None, "no super weapon in the fight");
        assert_eq!(super_weapon_theme(&c, "Rebel", true, true), None, "faction without the tag");
        let mut d = MusicDirector::default();
        d.set_scene(MusicScene::Space("Empire".into()));
        d.set_super_weapon(true);
        d.set_scene(MusicScene::Galactic("Empire".into()));
        assert!(!d.super_weapon, "leaving the space scene clears the flag");
    }
}
