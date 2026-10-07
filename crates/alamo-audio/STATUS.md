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

## Music and galaxy audio (data-driven)
- Music data: `Xml/MusicEvents.xml` (Remake == FoC layout). `MusicPlanner` (pure, no Bevy) maps `MusicScene` + battle
  intensity to an event name: `Main_Menu_Music_Event`, `Galactic_Map_<F>_Music_Event`,
  `Tactical_Battle_Loading_Loop_Event` (battle load; the older `Galactic_Battle_Pending_*` events are commented out in the
  Remake), `Space_Map_<F>_Ambient|Battle_Music_Event` (hysteresis: battle >= 0.35, back to ambient after 25 s below 0.1),
  `<F>_Win|Lose_Tactical|Strategic_Event` stingers. Unknown factions fall back to Empire. `MusicDirector` wraps the planner
  (`director.planner` holds the tunables) and does playlists/crossfades using each event's `Fade_*` values.
- Galaxy advisor lines: EaW plays them as HUD-VO SFX events `<E|R|U>HD_<Suffix>` (not SpeechEvents). `GalaxyEvent` ->
  `resolve_galaxy_event(sfx, faction, event)`. Present in the data for all three factions: UnitBuilt (New_Units),
  StructureBuilt, PlanetCaptured/Lost, BattlePending, BattleEnded, ResearchComplete, TechLevelGained, UnitCapReached,
  Defeated. Commented out in the data (silent): FleetArrived, HeroLost/Eliminated, CreditsGained, RetreatSucceeded.
- Volume: `AudioBuses` + `VolumeCategory` (Master/Music/Sfx/Speech; `percent`/`set_percent`; ambient follows Sfx).
- Tests (silent): `tests/director_flow.rs` steps the planner menu -> galactic -> loading -> space low/high intensity -> win/lose
  for Empire and Rebel and checks every file resolves; galaxy events resolve to existing samples for all factions.

## Integration for space-peace
1. Menu: `MusicDirector::set_scene(MusicScene::MainMenu)`; galaxy: `MusicScene::Galactic(player_faction)`.
2. Battle load screen: `MusicScene::BattleLoading`; battle start: `MusicScene::Space(player_faction)` and every frame
   `set_battle_intensity(ships_firing / ships_total)` (0..1).
3. Battle end: `MusicScene::Result { faction, win, tactical: true }`; campaign end: `tactical: false`; then back to Galactic.
4. Galaxy events: `MessageWriter<GalaxyAudio>`: `GalaxyAudio { faction: "Rebel".into(), event: GalaxyEvent::PlanetCaptured }`
   (unit built -> `UnitBuilt`, battle pending -> `BattlePending`, research done -> `ResearchComplete`, ...).
5. Options menu: `ResMut<AudioBuses>`: `set_percent(VolumeCategory::Music, v)` etc., `percent()` to read sliders.
