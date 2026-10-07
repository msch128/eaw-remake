//! Background music: a [`MusicSource`] produces tracks, the plugin plays them with crossfades.
//!
//! Track bytes are loaded (and their duration measured) on a worker thread so disk access never
//! stalls the frame loop. Playback itself uses Bevy audio with the `mp3` decoder.

use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Duration,
};

use bevy::{audio::Volume, prelude::*};
use rand::seq::SliceRandom;

use crate::cli::Args;

/// Seconds the outgoing and incoming track overlap.
const CROSSFADE_SECS: f32 = 5.0;

/// A fully loaded music track ready to be handed to the audio system.
pub struct MusicTrack {
    /// Display name (file stem).
    pub name: String,
    /// Raw encoded audio file contents.
    pub bytes: Arc<[u8]>,
    /// Playback length if it could be determined.
    pub duration: Option<Duration>,
}

/// Something that yields an endless sequence of music tracks.
///
/// Implementations may block on IO; the plugin calls [`MusicSource::next_track`] from a worker
/// thread. Later sources (e.g. the layered Empire at War file system) implement this same trait.
pub trait MusicSource: Send + 'static {
    /// Returns the next track to play, or `None` if the source has nothing (more) to offer.
    fn next_track(&mut self) -> Option<MusicTrack>;
}

/// Plays the mp3 files of a directory in shuffled order, reshuffling when the list is exhausted.
pub struct DirectoryMusicSource {
    files: Vec<PathBuf>,
    order: Vec<usize>,
    last: Option<usize>,
}

impl DirectoryMusicSource {
    /// Scans `dir` (non-recursive, read-only) for `.mp3` files, optionally filtered by file name
    /// prefix (case-insensitive). Returns `None` when no file matches.
    pub fn new(dir: &Path, prefix: Option<&str>) -> Option<Self> {
        let prefix = prefix.map(str::to_ascii_lowercase);
        let mut files: Vec<PathBuf> = fs::read_dir(dir)
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
                name.ends_with(".mp3") && prefix.as_deref().is_none_or(|pre| name.starts_with(pre))
            })
            .collect();
        files.sort();
        (!files.is_empty()).then(|| Self { files, order: Vec::new(), last: None })
    }

    /// Number of candidate files.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Picks the next file index from the shuffled order.
    fn next_index(&mut self) -> usize {
        if self.order.is_empty() {
            self.order = (0..self.files.len()).collect();
            self.order.shuffle(&mut rand::rng());
            // Avoid playing the same track twice in a row across reshuffles.
            if self.files.len() > 1 && self.order.last().copied() == self.last {
                let n = self.order.len();
                self.order.swap(0, n - 1);
            }
        }
        let i = self.order.pop().expect("order refilled above");
        self.last = Some(i);
        i
    }
}

impl MusicSource for DirectoryMusicSource {
    fn next_track(&mut self) -> Option<MusicTrack> {
        // Skip unreadable files, but give up after one full pass.
        for _ in 0..self.files.len() {
            let idx = self.next_index();
            let path = self.files[idx].clone();
            match fs::read(&path) {
                Ok(data) => {
                    let duration = mp3_duration::from_read(&mut Cursor::new(&data)).ok();
                    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?").to_owned();
                    return Some(MusicTrack { name, bytes: data.into(), duration });
                }
                Err(e) => warn!("music: cannot read {}: {e}", path.display()),
            }
        }
        None
    }
}

/// Adds shuffled, crossfaded music playback from the source configured on the command line.
pub struct MusicPlugin;

impl Plugin for MusicPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, start_music)
            .add_systems(Update, (advance_music, fade_music, skip_track).chain());
    }
}

/// Music player state (worker channel + what is currently playing).
#[derive(Resource)]
struct MusicState {
    requests: Sender<()>,
    tracks: Mutex<Receiver<MusicTrack>>,
    /// Track already loaded by the worker, waiting for its turn.
    pending: Option<MusicTrack>,
    /// A request for the next track is in flight.
    requested: bool,
    /// Currently audible (fading in / playing) track entity.
    current: Option<Current>,
    volume: f32,
}

struct Current {
    entity: Entity,
    started: f32,
    duration: Option<Duration>,
}

/// Fade state of a music entity, in seconds elapsed and direction.
#[derive(Component)]
struct MusicFade {
    t: f32,
    fading_out: bool,
}

fn start_music(mut commands: Commands, args: Res<Args>) {
    let Some(dir) = &args.music_dir else {
        info!("music: no --music-dir given, music disabled");
        return;
    };
    let Some(source) = DirectoryMusicSource::new(dir, args.music_prefix.as_deref()) else {
        warn!("music: no .mp3 files found in {}", dir.display());
        return;
    };
    info!("music: {} tracks from {}", source.len(), dir.display());

    let (req_tx, req_rx) = mpsc::channel::<()>();
    let (track_tx, track_rx) = mpsc::channel::<MusicTrack>();
    thread::Builder::new()
        .name("music-loader".into())
        .spawn(move || {
            let mut source = source;
            // Ends when the app drops the request sender.
            while req_rx.recv().is_ok() {
                let Some(track) = source.next_track() else { break };
                if track_tx.send(track).is_err() {
                    break;
                }
            }
        })
        .expect("spawn music loader thread");

    let _ = req_tx.send(());
    commands.insert_resource(MusicState {
        requests: req_tx,
        tracks: Mutex::new(track_rx),
        pending: None,
        requested: true,
        current: None,
        volume: args.music_volume,
    });
}

/// Collects loaded tracks, starts the next one when due and keeps one track prefetched.
fn advance_music(
    mut commands: Commands,
    time: Res<Time>,
    state: Option<ResMut<MusicState>>,
    mut sources: ResMut<Assets<AudioSource>>,
    sinks: Query<&AudioSink>,
    mut fades: Query<&mut MusicFade>,
) {
    let Some(mut state) = state else { return };
    let state = &mut *state;
    if state.pending.is_none()
        && let Ok(track) = state.tracks.lock().expect("music channel").try_recv()
    {
        state.pending = Some(track);
        state.requested = false;
    }

    let now = time.elapsed_secs();
    let due = match &state.current {
        None => true,
        Some(cur) => {
            let finished = sinks.get(cur.entity).is_ok_and(|s| s.empty());
            let near_end = cur
                .duration
                .is_some_and(|d| now - cur.started >= d.as_secs_f32() - CROSSFADE_SECS);
            finished || near_end
        }
    };
    if !due || state.pending.is_none() {
        return;
    }

    let track = state.pending.take().expect("checked above");
    if let Some(old) = state.current.take()
        && let Ok(mut fade) = fades.get_mut(old.entity)
    {
        fade.fading_out = true;
        fade.t = 0.0;
    }
    info!(
        "music: now playing {} ({})",
        track.name,
        track.duration.map_or("unknown length".into(), |d| format!("{}:{:02}", d.as_secs() / 60, d.as_secs() % 60))
    );
    let handle = sources.add(AudioSource { bytes: track.bytes });
    let entity = commands
        .spawn((
            AudioPlayer::new(handle),
            PlaybackSettings::DESPAWN.with_volume(Volume::SILENT),
            MusicFade { t: 0.0, fading_out: false },
        ))
        .id();
    state.current = Some(Current { entity, started: now, duration: track.duration });

    // Prefetch the following track.
    if !state.requested && state.requests.send(()).is_ok() {
        state.requested = true;
    }
}

/// Applies the crossfade curves to the sinks and removes faded-out tracks.
fn fade_music(
    mut commands: Commands,
    time: Res<Time>,
    state: Option<Res<MusicState>>,
    mut q: Query<(Entity, &mut MusicFade, &mut AudioSink)>,
) {
    let Some(state) = state else { return };
    for (entity, mut fade, mut sink) in &mut q {
        fade.t += time.delta_secs();
        let k = (fade.t / CROSSFADE_SECS).clamp(0.0, 1.0);
        // Equal-power style curve.
        let gain = if fade.fading_out { (k * std::f32::consts::FRAC_PI_2).cos() } else { (k * std::f32::consts::FRAC_PI_2).sin() };
        sink.set_volume(Volume::Linear(gain * state.volume));
        if fade.fading_out && k >= 1.0 {
            commands.entity(entity).despawn();
        }
    }
}

/// `N` skips to the next track with a crossfade.
fn skip_track(keys: Res<ButtonInput<KeyCode>>, state: Option<ResMut<MusicState>>) {
    if keys.just_pressed(KeyCode::KeyN)
        && let Some(mut state) = state
        && let Some(cur) = &mut state.current
    {
        // Pretend the track is about to end so the next one is started immediately.
        cur.duration = Some(Duration::ZERO);
    }
}
