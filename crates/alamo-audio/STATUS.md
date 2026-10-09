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

## Audio pass 2 (positional SFX, object loops, music flow, galaxy UI)
- **Listener** (was missing: nothing carried a `SpatialListener`, so every positioned sound played centered at full volume): the battle owns a
  `BattleListener` (`battle-view/src/audio.rs::battle_sources`) at the camera focus point on the battle plane, turned like the camera
  (`Audio_Space_3D_Listener_Z_Pullback_Dist` = 0 in `Audio.xml`). Pan = Bevy spatial audio; distance falloff is ours.
- **Distance**: `Volume_Saturation_Distance` x `Audio_Space_3D_Saturation_Distance_Mod` (1.5, `AudioSettings::saturation_scale`), beyond it
  `(sat / d)^rolloff` with `Audio_Space_3D_Rolloff_Distance_Mod` (1.0). The curve itself is an **estimate** (the data gives only these three numbers).
  `Max_Instances` caps simultaneous copies per event (oldest cut); the view adds 2 per name and frame.
- **Object loops**: `SfxEmitter` component (`event: Option<String>`): the plugin starts the loop at the object, follows it, fades the old one out
  (`Loop_Fade_Out_Seconds`) on a change or when the object is gone, starts nothing that would be inaudible, and gives `Max_Instances` slots to the nearest
  objects. Battle ships use `SFXEvent_Engine_Idle_Loop` / `_Moving_Loop`, else `SFXEvent_Ambient_Loop` (unit XML). Moving = route or move / attack / retreat order (estimate).
  Still no data for space ambience beds; `SFXEvent_Turret_Rotating_Loop`, `_Engine_Cinematic_Focus_Loop`, `_Ambient_Moving` are not wired.
- **Music**: result stingers play once (the data's `Loop = Yes` of the win events is ignored) and are not cut off by the return to the map:
  `MusicDirector::resume_after_stinger(scene)` (used by `flow::galaxy_music`). New track fade-in is the event's own `Fade_In_Seconds`; the previous one
  fades over `Fade_Out_Previous_Seconds`. Not defined in the data: peace / war variants of the galaxy theme, speech ducking.
- **Galaxy UI** (`space-peace/src/ui_sounds.rs`): `Click_SFX` / `Mouse_Over_SFX` per component of `CommandBarComponents.xml`, locked click and rejected command ->
  `SFXEvent_GUI_Negative_Feedback` (`GUI_Bad_Sound`), build queued/complete from the object's `SFXEvent_Build_Started/_Complete`, fleet select / move / retreat from the lead unit's
  `SFXEvent_Select` / `_Fleet_Move` / `_Move`, planet capture bonus `SFXEvent_Planet_Capture_Bonus_Reward`.
- Tests: `tests/pass2.rs` (silent), `ui_sounds::tests`; smoke scenarios `windowed-battle-audio-positional`, `windowed-ui-pointer-audio`, `windowed-galaxy-audio-build`
  (`SP_AUDIO_LOG` lines `audio sfx PLAY ... spatial=true dist=.. att=..`, `audio emitter`, `audio music track`, `audio music stinger done`).

## Audio pass 3 (galaxy fleets, Death Star, menu start sound, battle reports, balance log)
- **Galaxy fleets** (`ui_sounds.rs`): `Galactic_Fleet_Hyperspace` when one of the player's fleets leaves for hyperspace and when it drops out (the data has
  one event; the arrival use is an estimate). Its data volume is `Min/Max_Volume 1`, i.e. nearly inaudible by design. `GUI_Drag_Fleet` / `GUI_Drop_Fleet`
  are referenced by no XML (engine-side); our galaxy has no drag gesture, so pick-up = fleet selection, drop = accepted move order.
- **Superweapon** (Death Star, `--fire`): order accepted -> the unit's `TSW_SFXEvent_GUI_Bttton_Press`, `TSW_SFXEvent_Activate_Voice`, `TSW_SFXEvent_Weapon_Power_Up`
  (data predelay 2 s) and `TSW_MusicEvent_Activation_Start` (`Space_Map_Activate_Death_Star_Music_Event`, played once over the map music via
  `MusicDirector::play_once_then`, then the galaxy theme returns); planet destroyed -> `SFXEvent_Fire` (`Unit_Superlaser_Fire`) and the ability's
  `SFXEvent_Target_Affected` (`Planet_Death_SFX`, also for other factions' shots). The engine's 8 s power-up countdown / 4.75 s animation (`TSW_*`) are not
  replayed: the sim destroys at once, the sounds follow the visuals. `EHD_Death_Star_Activate` is defined but its sample (`C000_DST0102_ENG.wav`) is not in
  the install, so that voice stays silent (the Death Star II variant `EHD_Death_Star_2_Activate` resolves).
- **Menus**: `SFXEvent_GUI_Start_Campaign` of `Factions.xml` (`GUI_Select_Rebel|Empire|Underworld_SFX`) on "Start Game". Main menu music (`Main_Menu_Music_Event`)
  was already wired; no menu ambience loop and no menu-button SFX tags exist in the data (menu buttons keep `GUI_Common_Dialog_Button_*`).
- **Battle**: `SFXEvent_Hardpoint_Destroyed` is a list of `HARD_POINT_<TYPE>, event` pairs; before, the first pair (laser) played for every hardpoint. Now the pair of the
  destroyed hardpoint's `Type` plays (`Unit_Lost_Engines_<Ship>`, `_Bay_`, `_Shields_`, `_Ion_`, `_Laser_`; `SFX_NA` = silent), for the player's ships only (the unit's report).
  `SFXEvent_Unit_Type_Spotted` (`Factions.xml`) is played the first time an enemy unit type of the list is seen; only 3 entries are active in the Remake
  (`Star_Destroyer`/`AT_AT`; all hero lines are commented out). Not in the data: space turret rotation loops (`SFXEvent_Turret_Rotating_Loop` exists only for ground
  structures/props), hero-arrival or hero-ability voice lines beyond the units' own `SFXEvent_GUI_Unit_Ability_Activated` (already played), an "engine damaged" line other than the hardpoint pair.
- **Balance**: `SP_AUDIO_LOG=1` adds `bus=` / `eff=` (volume x distance x bus gain, master excluded) to each `audio sfx PLAY` line and logs
  `audio balance music[n= peak= avg=] speech[..] sfx[..] ambient[..]` every 5 s. Typical battle (Yavin -> Hoth, bus defaults music 0.6 / sfx 1 / speech 1):
  music 0.60, speech 0.7-1.0 per line, weapon shots 0.04-0.07 each (about 19 at once, sum <= 0.6), deaths 0.9-1.0. The data defines no category volumes
  (`Audio.xml` has none), so the bus defaults stay an estimate; nothing drowns the speech, no change made.
- Tests: `tests/pass3.rs` (one-shot theme resumes the map music, balance line, spotted lines), `real_audio::pass3_events_exist_and_resolve`, `constants` unit tests,
  `ui_sounds::tests`, `battle-view audio::tests`; smoke scenarios `windowed-galaxy-audio-fleet`, `windowed-galaxy-audio-superweapon` and the balance line in `windowed-battle-audio-positional`.
