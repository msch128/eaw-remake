//! Empire at War audio: event data (SFX, speech, music), sample path resolution, WAV/ADPCM decoding
//! and (feature `bevy`) a Bevy plugin that plays everything with EaW-like rules.

pub mod music;
pub mod resolve;
pub mod sfx;
pub mod speech;
pub mod wav;

pub use music::{MusicEvent, MusicKind, MusicLibrary};
pub use resolve::SampleResolver;
pub use sfx::{SfxEvent, SfxLibrary};
pub use speech::{SpeechEvent, SpeechLibrary};
pub use wav::{AudioError, Pcm, Wav, WavFormat};

#[cfg(feature = "bevy")]
pub mod plugin;
#[cfg(feature = "bevy")]
pub use plugin::{
    AlamoAudioPlugin, AudioBuses, AudioSettings, Bus, MusicDirector, MusicScene, PlaySfx,
    SfxCommands,
};
pub use speech::UnitResponseKind;
