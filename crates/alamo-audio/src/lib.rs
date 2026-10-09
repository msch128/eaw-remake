//! Empire at War audio: event data (SFX, speech, music), sample path resolution, WAV/ADPCM decoding
//! and (feature `bevy`) a Bevy plugin that plays everything with EaW-like rules.

pub mod constants;
pub mod galaxy;
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
pub mod plugin;
#[cfg(feature = "bevy")]
pub use plugin::{
    AlamoAudioPlugin, AudioBuses, AudioSettings, Bus, GalaxyAudio, MusicDirector, MusicTrack, PlayFile,
    PlaySfx, SfxCommands, SfxEmitter, SfxVoice, VolumeCategory,
};
pub use speech::UnitResponseKind;
