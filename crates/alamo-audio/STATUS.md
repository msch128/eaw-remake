# alamo-audio status

Empire at War audio: event data, sample resolution, decoding and a Bevy playback plugin (feature `bevy`, default).

## What works
- `SfxLibrary`, `SpeechLibrary` (unit responses), `MusicLibrary` (typed galactic / space / stinger events) from the XML.
- `SampleResolver` finds samples through the layered file system; `wav` decodes RIFF/WAVE incl. IMA and MS ADPCM to PCM.
- `AlamoAudioPlugin`: SFX with EaW voice rules (random/sequential, probability, predelay, pre/post/chained samples,
  volume/pitch ranges), buses, and `MusicDirector` scenes with playlists and fades.
- `tests/plugin.rs` runs without an audio device; `tests/real_audio.rs` and the `audio_census` example check real data.

## Key decisions
- The plugin only spawns `AudioPlayer` entities and never touches the device directly, so it can be tested silently.
- Disabling the `bevy` feature gives a fast data-only build.

## Gaps / next steps
- Not live-tested by ear (no audible playback in tests by project rule).
- Not yet wired into battles (weapon SFX, unit responses, battle music intensity).
- Land music kinds are parsed but unused.
