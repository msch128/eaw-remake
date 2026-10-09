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
- Battle wiring lives in `battle-view/src/audio.rs` (unit responses, advisor lines, music intensity); see "Audio feel pass" below.
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

## Audio feel pass (event-log verified)
- `SP_AUDIO_LOG=1` logs `audio sfx <name>` (request), `audio sfx PLAY` (voice started), `audio sfx CUT a for b` (unit response replaced),
  `audio advisor <Event> -> <HUD event>` and `audio music scene ... -> event ...`. With the log on, silent (volume 0) sounds still start so the voice rules run.
- Unit responses (`Is_Unit_Response_VO`): one line at a time, a new response cuts the playing one (a move order right after a select
  plays the move line); the same event is never stacked (`Max_Instances 1` of `Preset_UR`). The invented 0.6 s / 2.5 s cooldowns are now 0
  (`AudioSettings`, still tunable).
- `AudioConstants` (`constants.rs`): `Music_Space_Battle_To_Ambient_Peace_Seconds` = 15 and `Delay_Between_Space_Base_Attack_Announcement_Seconds` = 60
  from `Audio.xml`, `Health_Low/Critical_Percent_Threshold` = 0.33 / 0.10 from `Gameconstants.xml`. The music planner uses the 15 s peace delay.
- Battle music: ambient until a hostile shot/hit, battle while shots keep coming (3 s window, estimate), ambient again after the peace delay.
  The data defines no hero-arrival, winning/losing or other intensity rules besides the win/lose stingers.
- Data has none: speech ducking of music (no tag in Audio.xml / Gameconstants.xml), space ambience loops (`AmbientMapSounds.xml` / `SFXEventsAmbient.xml`
  are land weather only), HUD lines for FleetArrived, HeroLost/Eliminated, EnemyUnit/StructureDestroyed, EnemyFleeing, EnemyHeroDetected, RetreatSucceeded
  (commented out or sample-less for all three factions).
