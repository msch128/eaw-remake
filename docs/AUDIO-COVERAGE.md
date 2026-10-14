# Audio coverage: Remake sound hooks vs. what the game plays

State 2026-10-14 (audio pass 4). Source of every sound: the Remake 4.0 data (workshop 1770851727) read through the layered file
system. Nothing here was listened to; every statement is either a data fact, a code fact or a silent test/ledger result and is
marked as such. Re-run the numbers with:

- `cargo run -p alamo-audio --example audio_hooks` (problems per tag) or `... -- --md` (table of all 223 hook tags),
- `cargo test -p alamo-audio --test hook_audit --test ledger --test music_data`,
- `.\tools\audio-audit.ps1 -Exe <eaw-remake.exe>` (galaxy + battle run offscreen, sound ledger, pass/fail).

## How the hooks were counted

`alamo_audio::hooks` scans every `Xml/**/*.xml` for tags whose name contains `SFX` or `Music` (39,047 references in 223 tags),
resolves each named event (SFX library or music library) and every sample through the same `SampleResolver` the game uses.
`hooks::WIRED_TAGS` lists the tags the game code reads; everything else is data the game ignores.

## Category matrix

"Plays" = the code path exists and ran in a silent ledger run or a unit test. "Data gap" = the Remake itself defines nothing
playable.

| Category | Plays? | Data source | Where |
|---|---|---|---|
| Menu music | yes | `MusicEvents.xml` `Main_Menu_Music_Event` | `flow.rs` |
| Menu / dialog button clicks | yes | `GUI_Common_Dialog_Button_*` (no menu tags exist in the data) | `eaw-ui` |
| Galaxy music per faction | yes (ledger: `Galactic_Map_Rebel_Music_Event`) | `Strategic_Map_Music_Event` equals the name convention | `MusicPlanner` |
| Planet selected music / ambience | data gap | none (hover: `SFXEvent_Planet_Revealed` = `GUI_Planet_Mouse_Over`, wired) | `ui_sounds.rs` |
| GUI click / hover (command bar, galaxy and battle) | yes | `Click_SFX` 267 refs, `Mouse_Over_SFX` 612 refs of `CommandBarComponents.xml`; locked click = `SFXEvent_GUI_Negative_Feedback` | `ui_sounds.rs::button_sounds` (global, also fires for the battle bar) |
| Ability button click (`EawAbilityClicked`), hardpoint click, minimap click | no | `CommandBarComponents.xml` has per-component tags, the ability buttons are not `EawButtonClicked` | open |
| Build queued / cancelled / complete | yes | `SFXEvent_Build_Started` 1790, `_Build_Cancelled` 1716, `_Build_Complete` 1647 refs | `ui_sounds.rs` |
| Death Star / Starforge completed theme | yes (new, test only) | `Build_Music_Completed` = `Galactic_Death_Star_Complete_Music_Event` | `ui_sounds.rs::production_complete_sounds` |
| Advisor lines (galaxy and battle) | yes | `<E/R/U>HD_*` HUD-VO events (`SFXEventsHUD.xml`), resolved by suffix convention | `galaxy.rs`, `GalaxyAudio` |
| Advisor lines silent in the data | data gap | `FleetArrived`, `HeroLost`, `HeroEliminated`, `CreditsGained`, `RetreatSucceeded` are commented out / sample-less | see `alamo-audio/STATUS.md` |
| Story dialog `SFX` (`Scripts/Story/Dialog_*.txt`, 52 chapters = `Button_Press`) | yes (new) | the dialog file | `ui_sounds.rs::story_dialog_sounds` (the dialog text itself has no UI consumer yet) |
| Story dialog `SPEECH` lines (`SpeechEvents.xml`, 1858 events) | partly | no dialog chapter in the Remake names a `SPEECH` line, so none plays; the path exists (first file of the event) | `ui_sounds.rs::story_dialog_sounds` |
| Lua `Play_Music` / `Play_SFX_Event` / `Stop_All_Music` | not needed | the only calls in the Remake scripts are 3 `Object.Play_SFX_Event` in `ObjectScript_Bombing_Run.lua` (ground bombing runs); no galaxy or space script plays music or sound | - |
| Unit select / move / attack / guard / stop responses | yes | `SFXEvent_Select` 1379, `_Move` 706, `_Fleet_Move` 779, `_Attack` 795, `_Guard` 609, `_Stop` 51 refs; faction fallbacks `Unit_*_<Faction>` | `battle-view/audio.rs` |
| Attack-on-hardpoint line per hardpoint type | yes | `SFXEvent_Attack_Hardpoint` 3883 refs | `battle-view/audio.rs` |
| Unit under attack / health low / critical / enemy damaged | yes | `SFXEvent_Unit_Under_Attack` 470, `_Health_*_Warning` 595, `_Enemy_Damaged_*` 232; thresholds from `Gameconstants.xml` | `battle-view/audio.rs` |
| Unit lost, hardpoint destroyed report | yes | `SFXEvent_Unit_Lost` 623, `_Hardpoint_Destroyed` 744 | `battle-view/audio.rs` |
| Ability activated voice | yes | `SFXEvent_GUI_Unit_Ability_Activated` 499 | `battle-view/audio.rs` |
| Ability deactivated / toggle on-off / target ability | yes (new, test only by the audit run: the scripted battle uses no ability) | per `Unit_Ability` block: `SFXEvent_GUI_Unit_Ability_Activated` / `_Deactivated` / `SFXEvent_Target_Ability`; faction `SFXEvent_GUI_Toggle_Non_Hero_Ability_On/Off` (`KEY, event`). Deactivation is detected when an own ability's effect time reaches 0 (the sim has no event), so a timed-out ability also plays it (estimate); the target line plays when the targeting cursor opens | `battle-view/visuals.rs`, `audio.rs`, `tactical_ui.rs` |
| Weapon fire per hardpoint / fighter | yes (ledger: 14 distinct `*_Fire_*` events in one battle) | `Fire_SFXEvent` 6564 refs, `SFXEvent_Fire` 511 | `battle-view/audio.rs` |
| Projectile impact on hull / hardpoint | yes (new: armor variant) | `Projectile_SFXEvent_Detonate` 213 refs, `_Detonate_Reduced_By_Armor` 181 refs (most laser bolts name only the armor one; before this pass those hits were silent) | `battle-view/audio.rs`, `visuals.rs` |
| Projectile impact on shields | data gap | `Projectile_SFXEvent_Detonate_Shield` -> `SFX_Shield_Absorb_Detonation`, whose `<Samples>` is empty in `SFXEVENTSDEATHS.XML`: the Remake shield hit is silent. The hook is wired (plays when samples appear) | `battle-view/audio.rs` |
| Missile shield interception | yes (new) | the intercepted projectile's `Projectile_SFXEvent_Detonate` | `battle-view/audio.rs` |
| Explosions / death | yes | `Death_SFXEvent_Start_Die` 1605 refs (86 events), `Death_Explosion_SFXEvent` 5300 refs | `battle-view/audio.rs` |
| Death end sound | yes (new) | `Death_SFXEvent_End_Die`, played 1.5 s after the start sound (estimate: no death timeline in the view); spin-away death (`Spin_Away_On_Death_SFXEvent_Start_Die`, ground) not wired | `battle-view/audio.rs` |
| Hyperspace in / out of ships | yes, with hardcoded names equal to the data | `SFXEvent_Arrive_From_Hyperspace` / `_Exit_Into_Hyperspace` = `Unit_Ship_Hyperspace_Enter/Exit` (26 refs each, one event) | `ships.rs` |
| Galaxy fleet hyperspace, drag / drop fleet | yes | `Galactic_Fleet_Hyperspace` (volume 1: nearly silent by design), `GUI_Drag_Fleet`, `GUI_Drop_Fleet` | `ui_sounds.rs` |
| Retreat started (own) | yes (now on the sim's `RetreatOrdered`, was on the button click: a refused order and the Esc menu path were wrong) | `Space_Retreat_Begin_SFXEvent` = `<F>HD_Retreat_Started_Space` | `battle-view/audio.rs` |
| Retreat started (enemy) | yes (new) | `Space_Retreat_Enemy_Begin_SFXEvent` (`Unit_Enemy_Retreat_*_Soldier`) | `battle-view/audio.rs` |
| Retreat not allowed | yes (new, countdown case only) | `Space_Retreat_Not_Allowed_SFXEvent` when the retreat button is pressed before `retreat_countdown`; the interdictor reason 1 and the Esc-menu path are not covered | `tactical_ui.rs` |
| Retreat cancel | data gap in our sim | `Space_Retreat_Cancel_SFXEvent` exists, the sim has no cancel command | - |
| Reinforcements selection / cancelled | yes (new) | `Reinforcements_Selection_SFXEvent` when the window opens, `_Cancelled_` when it is closed while a unit is picked (estimate) | `tactical_ui.rs` |
| Reinforcements requesting | no | `Reinforcements_Requesting_SFXEvent` (`<F>HD_Request_Assistance`): no clear moment (the call already plays `En_Route`) | open |
| Reinforcements available / en route / enemy wave | yes | `<F>HD_Reinforcements_Available`, `_En_Route`, `Enemy_Forces` (estimate for the enemy wave) | `battle-view/audio.rs` |
| Shields down, base under attack, base critical | yes | HUD lines `Shields_Down`, `Space_Station_Under_Attack`, `Space_Station_Damaged`; delay from `Audio.xml` | `battle-view/audio.rs` |
| Unit type spotted (Star Destroyer, AT-AT ...) | yes | `SFXEvent_Unit_Type_Spotted` of `Factions.xml` (22 refs) | `battle-view/audio.rs` |
| Engine idle / moving loops, ambient machinery loops | yes | `SFXEvent_Engine_Idle_Loop` 231, `_Engine_Moving_Loop` 251, `_Ambient_Loop` 140 | `SfxEmitter` |
| Cinematic focus engine loop, ambient moving, turret rotating loop | no | `_Engine_Cinematic_Focus_Loop` 327, `_Ambient_Moving` 224, `_Turret_Rotating_Loop` 117 refs (the turret one only on ground props) | open |
| Group move / attack, assist move / attack | yes (new, estimate) | `SFXEvent_Group_Move/_Attack` (heroes) as fallback of the fleet-move/attack line for a group order; `SFXEvent_Assist_Move/_Attack` (blank or `SFX_NA` in most units) as last resort before the faction line. The data does not say when the engine plays them | `battle-view/audio.rs` |
| Nebula / asteroid move lines, hero respawned | no | 229 / 198 / 128 refs; need terrain and respawn state the battle view does not expose | open |
| Space ambience bed | data gap | no event in `AmbientMapSounds.xml` / `SFXEventsAmbient.xml` (land weather only) | - |
| Battle music (ambient / battle, 15 s peace delay) | yes | `Space_Map_<F>_Ambient|Battle_Music_Event`, `Music_Space_Battle_To_Ambient_Peace_Seconds` | `MusicPlanner` |
| Battle load screen music | yes (new, per faction) | `Music_Event_Battle_Load_Screen` (Rebel: `..._Loop_Event_2`, Empire `_1`); before the generic event was used | `AudioConstants::battle_load_music` |
| Victory / defeat stinger, tactical | yes (new: per opponent) | `Music_Event_Tactical_Win/Lose` plus `_Vs_Faction`; a defeat by the Empire plays `Lose_To_Empire_Event`; the Underworld one points to `Lose_To_Zann_1.MP3` which is not installed, so the plain defeat theme plays | `MusicDirector::set_opponent` |
| Victory / defeat stinger, strategic (campaign end) | yes (opponent: defeat = rival with most planets, victory = rival that lost most ships) | `Music_Event_Strategic_Win/Lose` plus `_Vs_Faction` | `show_end_dialog` + `MusicDirector::set_opponent` |
| Tension music (intensity levels) | two states only | the data has ambient and battle lists, nothing else | - |
| Death Star battle theme | yes (test only) | `Music_Event_Space_Battle_Super_Weapon`; plays instead of the battle theme while a ship with a `SUPER_LASER` ability is alive (`battle_music` -> `MusicDirector::set_super_weapon`) | `plugin.rs::super_weapon_theme` |
| Land music, weather, ground units | out of scope (space only) | - | - |

## Broken data (the Remake references sounds that are not in the install)

`tests/known_audio_gaps.txt` lists the 47 events of wired tags that cannot play: 5 unknown event names (typos such as
`>Unit_Turbo_Tower_Fire`), 2 events without samples (`SFX_Shield_Absorb_Detonation`, a preset), and 40 events with a missing sample. Most missing
samples belong to other mods' voice packs (`Voiceovers\GuP`, `UnderworldSupa`, `Liberation`, ...) and to a few single files of
multi-sample events (`Wpn_Ion_Huge_3.wav`, `Sequence_Explosion_Short_1.wav`). Consequences that matter:

- `EHD_Death_Star_Activate` (`C000_DST0102_ENG.wav`) is missing: the Death Star activation voice stays silent.
- `Unit_Select_Leia` (`R2_Select_1.wav`), `Unit_Select_E_Wing` (`Ewing_Select_05.wav`) and the Bellator/Mandator lines lose one sample each.
- A missing sample of a multi-sample event used to make the event randomly silent. The plugin now picks only among samples that
  exist (`usable_samples`) and reports each missing one once.

The hook-audit test fails on any new gap, so a data change or a code change that names a sound that cannot play is caught.

## Verification without listening

- `SoundLedger` (`alamo_audio::ledger`): every SFX request, voice start, music track and file request with event, chosen sample,
  resolved path and outcome (`Played`, `UnknownEvent`, `NoSamples`, `MissingFile`, `BadFile`). `SP_AUDIO_LEDGER=<file>` records
  and writes a TSV every 2 s and at exit; problems are logged once as `audio problem:` warnings. It also keeps silent sounds
  (`--volume 0`) running so the voice rules apply.
- Offscreen runs now take part: with `SP_AUDIO_LEDGER` or `SP_AUDIO_LOG` set, `main.rs` adds the audio plugin against an empty
  device and `battle_play.rs` keeps the battle audio hooks on (without them the offscreen battle was mute).
- `tools/audio-audit.ps1` runs the galaxy and the battle (`battle-fight-orders`) scenarios this way and checks: no unknown event, no
  missing file outside `known_audio_gaps.txt`, and that music, weapon fire, impacts, deaths, unit responses and the advisor lines
  were all heard. Last run (debug build): galaxy 9 requests / 8 distinct sounds; battle about 8.6k requests / 47 distinct sounds /
  PASS.
- Tests (silent): `tests/ledger.rs`, `tests/music_data.rs`, `tests/hook_audit.rs` (3 real-data tests, skipped without the
  Remake), `hooks`, `ledger`, `constants` unit tests.

## Not verified

Nothing was heard. Volume balance, the feel of the new impact sounds (every laser hit now detonates: up to 24 sounds per
frame, 2 per event name and frame), and whether the armor-variant stand-in matches the original are unchecked by ear.
GUI clicks and the real input paths need a windowed input run (not done tonight). The retreat announcement change is covered
by constants/unit tests only; no scripted battle in the audit retreats.
