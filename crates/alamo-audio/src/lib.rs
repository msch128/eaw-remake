//! Empire at War audio: event data (SFX, speech, music), sample path resolution, WAV/ADPCM decoding
//! and (feature `bevy`) a Bevy plugin that plays everything with EaW-like rules.
//!
//! The data side is plain Rust and testable without an audio device: `sfx`, `speech` and `music` parse the event
//! XML, `constants` reads the audio-related game constants, `resolve` maps sample names to files in the layered
//! file system, `wav` decodes RIFF/WAVE (including IMA and Microsoft ADPCM) and `galaxy` maps galactic events to
//! advisor and HUD sounds. With the `bevy` feature, `plugin` plays SFX and speech with the EaW voice rules
//! (random or sequential samples, probability, predelay, chained samples, volume and pitch ranges, instance limits,
//! 3D positioning) and runs a `MusicDirector` with scene playlists and fades; samples are read on worker threads.
//! The plugin only spawns `AudioPlayer` entities and never opens the device itself. `ledger` records every
//! request and its outcome, and `hooks` audits which sound tags of the game data the game code plays.

pub mod constants;
pub mod galaxy;
pub mod hooks;
pub mod music;
pub mod resolve;
pub mod sfx;
pub mod speech;
pub mod wav;

pub use constants::AudioConstants;
pub use galaxy::{GalaxyEvent, hud_prefix, resolve_galaxy_event};
pub use music::{MusicEvent, MusicKind, MusicLibrary, MusicPlanner, MusicScene};
pub use resolve::SampleResolver;
pub use sfx::{SfxEvent, SfxLibrary};
pub use speech::{SpeechEvent, SpeechLibrary};
pub use wav::{AudioError, Pcm, Wav, WavFormat};

#[cfg(feature = "bevy")]
pub mod ledger;
#[cfg(feature = "bevy")]
pub mod plugin;
#[cfg(feature = "bevy")]
pub use plugin::{
    AlamoAudioPlugin, AudioBuses, AudioSettings, Bus, GalaxyAudio, MusicDirector, MusicTrack, NoAudioDevice, PlayFile, PlaySpeech,
    PlaySfx, SfxCommands, SfxEmitter, SfxVoice, VolumeCategory,
};
pub use speech::UnitResponseKind;
