# Architecture

EAW:Remake is a Rust / Bevy 0.19.1 workspace (edition 2024, Windows) of 15 crates. It reads the player's own
*Empire at War* install and the Remake 4.0 mod at run time and plays Galactic Conquest with real-time space battles.
No game file is stored in the repository.

## Crate map

Crates in dependency order; each layer depends only on the ones above it.

| Layer | Crate | Responsibility |
|---|---|---|
| Files | `alamo-fs` | Layered virtual file system (`GameFs`), `.MEG` archives, Steam path discovery |
| Parsers | `alamo-data` | Lenient XML, string tables (`.DAT`), game objects, units, campaigns, combat table |
| | `alamo-model` | `.ALO` models, `.ALA` animations, particle systems, DDS/TGA/PNG textures |
| | `alamo-map` | Tactical space maps (`.TED`) |
| | `alamo-audio` | Sound events, sample decoding (WAV, ADPCM, MP3), music director, speech, Bevy playback |
| | `alamo-lua` | Lua runtime for story scripts, AI plans and scripted events |
| Simulation (no Bevy) | `galaxy-sim` | Deterministic Galactic Conquest: economy, fleets, conquest, builds, auto-resolve |
| | `battle-sim` | Deterministic real-time space battle |
| | `galaxy-ai` | The Remake's galactic AI on an abstract `AiSim`, plus a strategy layer |
| | `galaxy-script` | Bridge: runs the Remake's Lua plots and AI on `galaxy-sim`; save/load |
| Bevy front ends | `alamo-render` | Models, Alamo shaders, particles, laser bolts, engine trails |
| | `battle-view` | The space battle scene, camera, controls, combat UI, battle audio |
| | `eaw-ui` | The original interface rebuilt from the game's own UI data |
| | `galaxy-play` | Playable galaxy map: fleet movement, build queues, notifications |
| App | `eaw-remake` | The executable: startup, menus, save/load, settings, install check |

Pipeline order, as data flows: `alamo-fs` (MEG + layered VFS) -> `alamo-data` / `alamo-model` / `alamo-map` /
`alamo-audio` / `alamo-lua` -> `galaxy-sim` / `battle-sim` / `galaxy-ai` / `galaxy-script` -> `alamo-render`,
`battle-view`, `eaw-ui`, `galaxy-play` -> `eaw-remake`.

## Simulation versus rendering

* `galaxy-sim` and `battle-sim` are headless, deterministic (seeded RNG, no global state), `Send`, and free of Bevy.
  Player and AI actions are serialisable `Command`s that produce `Event`s, so saves and replays stay consistent and
  tests run without a window. Results are identical with or without threads.
* The Bevy crates never mutate a simulation directly. `galaxy-play` runs the galaxy simulation on a worker thread and
  hands the render side immutable snapshots; `battle-view` does the same for a battle and interpolates between
  snapshots on a virtual clock, so the picture is smooth at any frame rate.
* A battle is handed over as plain data (`galaxy_sim::TacticalRequest`), played by `battle-view`, and returned as a
  `TacticalResult` that `galaxy-sim` applies like a statistical result.
* Everything that can run offscreen does: `eaw-remake --offscreen out.png` renders a scenario to a PNG with no window
  and no audio. See [CONTRIBUTING.md](../CONTRIBUTING.md) for the verification workflow.

## Where data comes from

* The Steam install (`Empire at War - Gold Pack`, app 32470) and the Workshop mod (1770851727) are found through the
  registry, `libraryfolders.vdf`, `EAW_GAME_DIR` / `EAW_MOD_DIR` or `--game-dir` / `--mod-dir`.
* `GameFs::for_mod` stacks the mod over Forces of Corruption over the base game: loose files of a stack first, then the
  MEGs named by the highest root's `megafiles.xml`, then patch archives. `--mod <workshop id>` selects other content
  (`Content::CloneWars` stacks `3229239424`, `2794270450` and `1770851727`).
* Unit stats, weapons, economy, campaigns, AI goals and Lua plans come from the XML and Lua files; the English string
  table from `MasterTextFile_ENGLISH.dat`; models, textures, maps, UI layouts and sounds from the MEG archives.
  Rules the data does not define are named estimates, see [RULES.md](RULES.md).
* Tests against real data skip silently when the game is missing, so a green test run does not prove the data was
  checked: look for the skip message.

## Coordinate conventions

* Alamo space (models, particles, `.ted`) is right-handed, Z-up, with the bow at -Y. UV origin is top-left (D3D);
  front faces are counter-clockwise. Bone transforms are row-major 3x4 `[R|t]`, local to the parent.
* To Bevy / glTF (Y-up): `(x, y, z) -> (x, z, -y)`.
* To battle-sim space (X forward, Y left, Z up): `(x, y, z) -> (-y, x, z)`.
* `.ted` object yaw is counter-clockwise from +X; attackers enter at -Y facing +Y. In `battle-view`,
  `convert::SIM_TO_MAP_YAW` is 0 with `BattleConfig::arena` and PI/2 without an arena.
* More in [FORMATS.md](FORMATS.md).

## Conventions that cross crates

* Gameplay numbers come from the data; hidden engine rules are named estimates in config structs and in RULES.md.
* Parsers are lenient: warnings instead of errors. Undecoded bytes stay raw; inferred fields are marked "Inferred" or
  "(guess)" in doc comments.
* Settings live in `%APPDATA%\SpacePeace\settings.toml`, saves in `%APPDATA%\SpacePeace\saves`, logs in
  `%APPDATA%\SpacePeace\logs` (the ten newest are kept).

# Crates

## alamo-fs
Read-only file access for the Alamo engine. `MegArchive` reads `.meg` archives lazily, `LooseDir` a directory, and `GameFs::for_mod` stacks them
like the engine does: loose files of a stack first, then the MEGs named by the highest root's `megafiles.xml`, then unlisted patch archives
(order is a guess), then the audio archives. Stacks are `[mod, Forces of Corruption, base game]`. Paths are normalised (upper case, `/`, no `Data/`
prefix) so lookups are case-insensitive. `GameInstall::detect` finds Steam via env `EAW_GAME_DIR`, the registry and `libraryfolders.vdf`.
Everything is `Send + Sync` and never writes. Real-install tests skip when the game is missing.

## alamo-data
Lenient typed readers for the XML and text data. `xml.rs` never fails (tags and attributes are lower-cased, broken syntax becomes a warning),
`galaxy.rs` assembles `GameData` (campaigns, planets, trade routes, factions, starting ownership) from the index files, `objects.rs` merges all
object XML and resolves `Variant_Of_Existing_Type` only through `resolve()`, `space.rs` gives typed views (ships, squadrons, hardpoints,
projectiles, abilities, star bases, structures), `combat.rs` the damage-to-armor matrix and Gameconstants, `difficulty.rs` the tier multipliers,
`text.rs` the English string table. Unknown tags stay in `fields` maps. Ground and story data is parsed but unused.

## alamo-model
Engine-neutral parsers for `.alo` models, `.ala` animations and particle systems (`.alo` with root chunk `0x900`), plus minimal DDS/TGA/PNG decoding.
Output is exactly as stored: right-handed, Z-up, bow at -Y, row-major 3x4 bone transforms. Undecoded bytes stay raw; guessed fields are
documented as "Inferred". The whole Remake corpus (4,348 models, 7,685 animations, 1,505 particle systems) parses with 0 errors; the
`parse_all*` examples guard that. Depends only on `alamo-fs`.

## alamo-map
Parser for `.ted` map files, focused on space maps: header and names, bounds, lighting and fog, objects (asteroids, markers, spawn points)
with position and rotation, defender slots. Object types are CRC-32 hashes of the upper-cased XML names; `resolve_names` turns them into names.
Z-up in model units, yaw counter-clockwise from +X. Undecoded data stays raw. `battle-sim` reads arenas from it.

## alamo-audio
Responsibility: event data, sample resolution, decoding and playback for sound effects, speech and music.
* Data layer (no audio device needed): `SfxLibrary`, `SpeechLibrary` and `MusicLibrary` parse the event XML; `SampleResolver` finds samples through the layered file system; `wav` decodes RIFF/WAVE including IMA and Microsoft ADPCM; `AudioConstants` reads the audio-related game constants; `galaxy` maps galactic events to advisor and HUD sounds.
* `AlamoAudioPlugin` (feature `bevy`, default) plays SFX with the EaW voice rules (random or sequential samples, probability, predelay, pre/post/chained samples, volume and pitch ranges, per-event `Max_Instances` where a nearer sound wins), speech with unit-response cut rules, and runs a `MusicDirector` with scene playlists and fades (`MainMenu`, `Galactic(faction)`, `Space(faction)`, stingers). It only spawns `AudioPlayer` entities and never opens a device, so tests run silently.
* Samples not yet cached are read on two worker threads; a request waits a frame or two on first use so voice rules still act as authored. MP3 and WAV are decoded on demand.
* `SoundLedger` records every request with its resolved path and outcome (`SP_AUDIO_LEDGER=<file>`); `hooks` audits which sound tags of the data the game code plays (`tests/hook_audit.rs`, with the Remake's own broken references listed in `tests/known_audio_gaps.txt`). Battle wiring lives in `battle-view/src/audio.rs`, galaxy wiring in the app.

## alamo-lua
Lua runtime for the game's scripts. The engine uses Lua 5.0; this crate embeds vendored Lua 5.1 (`mlua`) with the needed compatibility. Every
script instance has its own Lua state (`runtime.rs`), a case-insensitive `require` through the game file system, and cooperative threads
(`Create_Thread`, `Sleep`, `PumpEvents`). `api.rs` and `ai_api.rs` bind the engine functions and forward them to the `ScriptHost` trait, which
`galaxy-script` implements on top of `galaxy-sim`; `MockHost` serves tests. `story.rs` runs the plot events of `Xml/Story_*.xml` (prerequisites,
rewards, notifications). `persist.rs` snapshots a running runtime for saves. Compiled chunks are cached across script VMs, and a finished script
drops its Lua state, which keeps memory flat in long games. Unknown engine calls are logged once and return nil; presentation calls (fades,
cinematics) are ignored silently. No script text is stored in the repository.

**Lua API at a glance.** The Remake and base game ship 709 scripts: Story 119, AI-Galactic 114, AI-Land 77, AI-Space 42, GameObject 246, Library 45,
Evaluators 18, FreeStore 8, other 40. All compile under Lua 5.1 with no shims. Scripts enter the engine through XML tags (`Lua_Script`, `Goal`,
`MovementClass`, `SpaceFreeStoreScript`, `GalacticFreeStoreScript`, `Game_Scoring_Script_Name`, ...). The engine contract: a script file is loaded,
`Base_Definitions()` runs (calling `Definitions()`), then `main()` runs as a thread; `Story_Event_Trigger(name)` is called when a plot event fires;
`PumpEvents()` yields; `ServiceRate` sets the update period; `ScriptExit()` ends the script. Scripts call 446 distinct engine functions and read
10 engine-provided globals. The most used calls are object lookup (`Find_Object_Type`, `Find_All_Objects_Of_Type`, `Find_Player`, `Find_Hint`),
story plumbing (`Story_Event`, `Create_Thread`), object methods (`Get_Type`, `Get_Owner`, `Get_Position`, `Move_To`, `Despawn`, `Is_Category`),
spawning (`Spawn_Unit`, `Create_Generic_Object`) and many cinematic and camera calls. The inventory is generated, not stored: run
`cargo run -p alamo-lua --example lua_survey -- <workshop_id> --md lua-api.md` to regenerate it (per function: call count, global/story/library
use, and the areas that call it). Use it to decide which calls to implement next.

## galaxy-sim
Headless, deterministic, `Send` Galactic Conquest simulation with no Bevy dependency. `World` is the static `Arc` part (planets, hyperspace lanes,
factions, unit, structure and star base catalogs, damage matrix); `Galaxy` is the mutable state; `Sim` (`sim.rs`) steps time, applies serialisable
`Command`s and returns `Event`s. Randomness is a seeded generator; there is no global state. `production.rs` and `requirements.rs` enforce build
rules (tech, star base level, required structures and planets, locks, limits, population, credits); `combat.rs` is the statistical auto-resolver;
`tactical.rs` hands a battle to `battle-sim` as plain data and applies the result; `sight.rs` is galaxy fog of war and AI knowledge; `ground_up.rs`
is the optional "From the Ground Up" start; `api.rs` is the direct API used by `galaxy-script` (spawn units, structures, ownership, tech, alliances,
stable ship ids). `ai.rs` is a simple built-in AI kept as the "classic" option. Data values come from the Remake XML; every hidden engine rule is a
named estimate in `SimConfig` and listed in docs/RULES.md.

## battle-sim
Headless deterministic space-battle simulation (fixed 30 Hz step, rayon-parallel scans, per-entity generators seeded by `(seed, tick, id)`, so
results are identical with or without threads). `defs.rs` builds the immutable `Catalog` (ships, hardpoints positioned from `.alo` bones, turret
arcs, projectiles, damage matrix) from the data; `battle.rs` holds state, commands, events and results; `sim.rs` the step systems (movement,
weapons, projectiles, abilities, retreat, reinforcements, fog of war); `ai.rs` the side-level AI after the Remake's space scripts (formation
advance, priority target, free-store targeting, station defence, reserve, retreat rule); `formation.rs` and `nav.rs` group-move formations, A*
paths around rocks and stations, hull volumes and fighter keep-out; `arena.rs` bounds, spawn markers and obstacles from a `.ted` map; `placement.rs`
the reinforcement-zone rule shared with the view; `scripts.rs` reads the mod's object scripts (spawn lists, timings) and `spawns.rs` brings their units in. All player and AI actions are `Command`s. Data values come from the Remake XML; engine rules
that are not in the data are named estimates in `BattleConfig` or `defs.rs` and listed in docs/RULES.md. Hit points of a unit with destroyable
hardpoints are the sum of its hardpoints (a project rule, not game data).

## galaxy-ai
The Remake's galactic AI on an abstract `AiSim` (implemented in `galaxy-script`). `data.rs` loads `Data/Xml/AI`; `equation.rs` and `perception.rs`
evaluate the perceptual equations (variable definitions are reconstructed from usage); `engine.rs` scores goals and targets, keeps per-category
budgets and starts plan scripts; `taskforce.rs` implements the space-relevant task force calls (`Produce_Force`, `Move_To`, `Release_Forces`).
Cost is about 75 to 90 ms of AI per game week for 12 factions.

**Strategic layer (`strategy.rs`).** The Remake's goal functions rate a target by importance and priority and ignore distance, supply, nearby
fleets and the enemy's main stack. This layer multiplies each offensive or defensive desire by a utility factor (0.2 to 3) and changes force sizing.
Deterministic, `Send`, no Bevy.
- Inputs: a snapshot every 4 s of game time with fleet strength per planet and player (square law, `sqrt(durability x damage rate)`, the measure the
  auto-resolver uses) and fixed defences of each planet.
- Influence maps: strength x 0.5^hops over the route graph (3 hops); hops to the nearest owned planet give supply and distance.
- Utility: value x aggression, minus risk x caution, minus a distance term; bonuses for the frontier, for neutral-planet expansion (fading with
  share of the map, with a brake on the leading faction), flanking a target held below half of the owner's main stack, and pockets that isolate
  owner planets; own planets use a defend term. A consolidation phase after recent losses damps offence. All weights are estimates.
- Information: the data's `AIUsesFogOfWarGalactic` False lets the AI of every difficulty see everything; with fog-bound data it sees its own
  planets and fleets plus two lanes around them and remembers the last seen strength.
- Force sizing: attack forces are sized to the believed defence times a margin and contrast factor; free-store ships are reused first; when a
  target needs 50 to 90 percent of everything the player owns, two plans attack together and rally so they arrive at the same time.
- Difficulty: Easy, Normal and Hard come from `Xml/DifficultyAdjustments.xml` (credit, build time, battle multipliers, contrast, goal sleep).
  Brutal (Hard plus 4x credits, 0.8x build time, full map knowledge) and Fair (no multipliers) are project tiers and estimates. Personalities
  (aggression, expansion, caution) and a per-match opening (balanced, expand, turtle, raid) add variety; heroes stay at home.

## galaxy-script
Runs the Remake's Lua plot scripts and `GameScoring` on top of `galaxy-sim`. `ScriptedSim` steps the sim and the script scheduler together, turns
sim events into story notifications (planet captured, construction complete, tech level, battles won or lost, units destroyed) and services
scripts at their `ServiceRate` in game time. `SimHost` implements `alamo_lua::ScriptHost`; scripts change the sim only through galaxy-sim's public
API, so the sim stays script-agnostic. `ai.rs` implements `galaxy_ai::AiSim` so the Remake AI drives the AI factions. Defaults: Remake AI plus
strategic layer at Normal difficulty. `save.rs` writes snapshot saves (sim state, story and Lua globals, AI memory; format-versioned, with the
name tables of the world); Lua coroutines cannot be serialised, so a loaded game restarts its scripts and AI plans and is not bit-identical to
the original run. All player input goes through `ScriptedSim` so saves stay consistent.

Saves stay loadable by later builds. A field added to a saved struct (`galaxy_sim::state`, `SimSnapshot`, `SimConfig`, `ScriptedConfig`,
`alamo_lua::persist`, `galaxy_ai::AiMemory`) carries `#[serde(default)]`, so a file without it loads with the default. The checkpoint of a save
is `Galaxy::state_hash`; a load compares it with the state as the file holds it, hashed the way the writing build hashed
(`galaxy_sim::HashEra`). A new field is therefore left out of the hash, hashed only when it differs from its default, or it starts a new era.
`crates/galaxy-sim/tests/data/galaxy_<date>.json` are small made-up states with the field sets of earlier builds; their checkpoints are pinned.

Planet, faction, unit, structure and star base ids are indices into catalogs that are rebuilt from the game data at every start, so they move
when a build reads more of the data (a new kind of build item) or the mod changes. A save therefore carries the name behind every id
(`galaxy_sim::NameTables` in the header, guarded by counts and a fingerprint). A load compares them with the tables of the freshly built match:
equal tables mean the state goes in untouched; otherwise `galaxy_sim::IdMap` maps saved ids to current ones by name and
`Sim::remap_snapshot` rewrites every id of the snapshot (`remap.rs` takes every saved struct apart field by field without `..`, so a new field
does not compile until someone decides whether it holds an id). `save.rs` does the same for the planet, unit and structure ids inside the
object handles of saved Lua globals and for the alliances. Names the save does not know are new: such planets and factions keep the state of a
fresh match. A name the save uses and the game data lacks refuses the load with a message that lists it. The lane of a travelling fleet is
looked up again between its two planets. The AI memory is indexed by planet and player and is dropped (with a load warning) when either list
changed. `SAVE_VERSION` is raised only for a change a reader cannot follow by defaults and names.

The files of the builds before the name tables hold only the counts and the fingerprint. `legacy.rs` rebuilds the unit table of the writing
build from today's catalog: the build items the converter derives (hero upgrades, phase repairs, purchases without a ship, agents, structure
swaps) each arrived as one block, so an earlier catalog is today's without some blocks, and the first builds listed squadrons by object kind
only. The combination whose count and fingerprint are those of the file is the catalog of the file (the fingerprint proves the planet,
faction, unit and structure names); the ids are then translated as above. A file that matches none is refused with that reason. A new kind
of derived build item gets a `Block` there only if saves without name tables from before it still matter; saves with tables need nothing.

## alamo-render
Responsibility: spawns original Alamo models in Bevy with an EaW-style look. Depends on `alamo-fs` and `alamo-model`.
* `AlamoRenderPlugin::new(Arc<GameFs>)`; `AlamoModel { path, team_color, lod, bow_forward, unit, attached }` spawns bones, rigid and skinned meshes, proxies and hardpoints. Mesh, material and texture caches are shared (500 TIEs in six team colours are 18 draw calls). DDS stays block-compressed (BC1/2/3); TGA and PNG are decoded with mips. `preload` loads models and textures on worker threads.
* One custom material (`alamo.wgsl`, reconstructed because no HLSL source exists): bump, gloss, team colour, planet (clouds, atmosphere, city lights), additive, alpha, shield (local hit flashes), nebula, emissive. Shading is in gamma space: use `Tonemapping::None`. The sun is the first `DirectionalLight` (`spawn_sun`, `spawn_sun_shadowed`); up to two further directional lights add a diffuse-only fill term.
* Particles are simulated on the CPU, one dynamic mesh per (texture, blend mode). Emitter space is Z-up like the models. Projectile bolts are batched (`ProjectileVisual`); impacts arrive as `ProjectileImpact`, `SpawnParticleEffect` and `ShieldHit`, with definitions read from the XML (`xmldata`).
* Also here: `.ala` playback by `<MODEL>_<KIND>_<NN>.ALA` lookup, turret aiming limited by the hardpoint XML, hardpoint destruction, hyperspace-arrival and death sequences, distance culling (`LodSettings`), and a frame-spike debug aid (`SP_FRAME_LOG`).
* Run offscreen only: `cargo run -p alamo-render --example render_scene -- <model|-> out.png`, plus `showcase`, `battle`, `xml_grep`, `tex_dump`.

## battle-view
Responsibility: renders and drives a `battle-sim` space battle with the EaW look and controls.
* `BattleViewPlugin` (needs `AlamoRenderPlugin` first); a battle starts when the host inserts a `BattleSession` (from a `galaxy_sim::TacticalRequest` via `BattleSession::from_request`, or built by hand in examples). The battle must use `BattleConfig::arena` (map space); `convert::SIM_TO_MAP_YAW` is 0 for that, and PI/2 without an arena.
* The simulation runs on a worker thread (`sim.rs`) and publishes one snapshot per step; the render side keeps a virtual clock and interpolates, so the picture is smooth at any frame rate. `blocking: true` makes offscreen runs reproducible. Commands go back through a channel (`BattleRuntime::command`).
* `scene` builds the static map from a `.ted` (skydome, backdrop planet, props, sun and fill lights, bounds); `ships`, `flight`, `turrets`, `effects` and `missiles` draw and animate units; `control` and `camera` handle selection, orders and the camera; `tactical_ui`, `hud`, `retreat_prompt` and `cursor` connect `eaw-ui` and the game's mouse pointers; `audio` sends `PlaySfx` and music messages (budgeted per frame); `fog` draws the fog shroud.
* `warmup` spawns one instance of every unit type a side can field behind the loading screen so no model, pipeline or file load happens mid-battle. The tactical camera draws each squadron as its leader alone at a fixed screen size (`ships::small_craft_boost`), the battle cam every craft at its model scale; every squadron carries one bar and flag under that craft, own and enemy, and a picked squadron a see-through box round it.
* The result leaves as the `BattleFinished` message with the sim's `TacticalResult`. Quality presets (`QualityPreset`: MSAA, bloom, shadows, particle density) are applied by `BattleViewSettings::apply_quality`. Profiling: `--gpu-profile` in `battle_demo`, `--features profile` with `TRACE_CHROME`.

## eaw-ui
Responsibility: the original interface rebuilt in Bevy UI from the game's data (galactic and space-tactical only; ground and story UI are dropped on purpose).
* `EawUiPlugin::new(fs).with_game_data(game)`; spawn a bar with `spawn_eaw_command_bar(CommandBarSpec)` or any other shell with `spawn_shell`; `spawn_dialog` draws GC message boxes. The host writes plain data resources (`CommandBarData`, `PanelData`, `MinimapData`, `TacticalData`, `TacticalOverlays`, `MemberBars`, `SquadIcons`) and reads Bevy messages back (`EawButtonClicked`, `EawAbilityClicked`, `EawSelectionClicked`, `EawHardpointClicked`, `EawSlotDropped`, `EawMinimapClicked`, `EawDialogResult`). `UiTexturesLoaded` fires when the mega texture is decoded (on the async compute pool); bars requested earlier wait for it.
* Modules: `textures` (mega texture and loose UI textures), `components` (`CommandBarComponents.xml`), `shell` (placement and frame art), `widgets` (buttons, slots, bars, tooltips), `panels` (fleet organizer, planet screen, reinforcement and hero frames), `tactical` (abilities, selection grid, retreat and reinforcement labels, ship brackets and hardpoint reticles), `member_bars` (squadron bars and flags drawn as one batched UI item) and `squad_icons` (an icon layer the battle view no longer fills).
* Every shell root carries `ShellOpaque`, an alpha mask of its frame art in 4 design-px cells; use `hit` and `overlaps` for pointer tests, never the root node, which is the bounding box of all art.
* Set `EawUiSettings::camera` when rendering to an image. Fonts come from the operating system (`system-fonts` feature: Bahnschrift for the bar, Arial for tooltips); nothing is bundled.

## galaxy-play
Responsibility: makes the galactic map playable on top of `galaxy-sim`.
* `driver` runs the simulation (plain `galaxy-sim` or the scripted `galaxy_script::ScriptedSim`) on a worker thread: the render side sends requests (commands, "advance N seconds", battle resolution, save, load) and polls immutable snapshots. It is testable without Bevy.
* `GalaxyPlayPlugin::deferred()` plus `start_play(world, PlayInit)` / `stop_play(world)` let the host start a game when a campaign opens. All host interaction goes through `PlayAction` and `PlayEvent` messages; the host never calls `Sim` mutators.
* Pure data models: `geometry` (planet layout, fleet placement on lanes, ETAs, route planning with blockades), `build_menu` (availability and reasons), `notify`, `catalog` (display names and map models). `state` holds the resources, `PlayUiModel` (the UI seam) and `GameClock` (pause, speed levels, a `hold` flag for the host).
* `input`, `carry` and `visuals` handle picking (a planet's globe beats fleet icons around it, decorations listed as planets are never picked), drag-and-drop with fleet pads as drop targets, and fleet models, rings and labels. New fleet models spawn hidden and show once fitted.
* A "Fight" asks the worker for the tactical setup (`PlayEvent::TacticalReady`); the host plays it and answers `ApplyTactical` or `CancelTactical`. `soak` example and ignored tests run 100 to 120 game weeks headless and check money, script errors, frozen fleets and determinism.

## eaw-remake (the executable)
Responsibility: the application shell that wires every crate together.
* `main` and `cli` parse flags and decide between `--check-install` (exits before any Bevy app exists), an offscreen render (`offscreen`: scripted scenarios to PNG, 3840x2160 by default, no window, no audio) and the windowed app.
* `flow` owns the screens (Loading, MainMenu, CampaignSelect, Galaxy, SpaceBattle); `loading` parses game data on a worker thread and warms up the galaxy scene; `game` maps parsed campaigns to what the map shows (exact ownership, routes, props, the 50 degree galaxy camera from `Gameconstants.xml`).
* The galaxy is drawn by `galaxy_map`, `planet_visual`, `planet_view`, `camera`, `lane.wgsl` (lanes add in display-space values like the original); `play` and `play_ui` connect `galaxy-play` to the `eaw-ui` command bar; `battle_seam` and `battle_play` open a `battle-view` session for a pending battle and return the result.
* `settings` (TOML under `%APPDATA%`, CLI flags override), `options`, `menu`, `logfile` (ten newest logs kept, panics included). The `dist` cargo feature builds a console-less executable for the installer; opt-in self tests (`input_test`, `drag_demo`, `label_probe`) inject window events or probe labels and are run by `tools/smoke.ps1`.

## Interface conventions (eaw-ui)

* All UI data comes from the mod and base game at run time; nothing is copied into the repository.
* **Look** of each element: `Data/Xml/CommandBarComponents.xml` (870 components in the mod: `TextButton`, `Button`, `Bar`, `Shell`, `Icon`). Textures per state (`Mouse_Over_`, `Selected_`, `Disabled_`, `Build_`, `Flash_`, `Overlay`), `Scale` (multiplier of the texture's natural size), `Icon_Offset`, text offsets and fonts, tooltip ids, `Toggle`, `Drag_And_Drop`.
* **Placement**: shell models `Data/Art/Models/I_*.ALO` are 2D scenes of hidden quads. A bone gives a component's centre, its bounding box its size. Units are design pixels of the original 1366 x 768 layout, x right, y up, origin at the bar's bottom-left. The UI scales with screen height / 768. Frame art (`Empire_Faceplate_ALT0`, `Rebel_Faceplate_ALT1`, `Underworld_Faceplate_ALT2`) uses mapped regions of the 2048x512 dashboard sheets with slanted edges, so it is rasterized from the mesh, not drawn as sprites.
* **Textures**: small UI textures are packed in `Art/Textures/MT_COMMANDBAR.TGA` (4096x4096) with a directory in `MT_COMMANDBAR.MTD` (`u32 count`, then 81-byte records `name[64] x y w h flag`). Unit icons are `Icon_Name` in the unit XML (`I_BUTTON_UNIT_*`); strings come from `MasterTextFile_ENGLISH.DAT`.
* **Shells**: `i_main_commandbar` (galactic bar), `i_main_skirmish` (space battle bar, 1017 px wide; the galactic bar is 1379 x 287), `i_main_organize` (fleet organizer), `map_shell` (planet screen), `i_main_reinforce`, hero frames (frame for n+1 slots per `_ALTn` mesh; both factions use the Rebel-named meshes), plus dialogs and market/corruption frames (not built).
* **Component families**: `b_*` buttons, `Text_*` labels, `queue00..09` production queue (left five space, right five ground), `b_Create_00..39` build pads, `s_select_NN` + `s_health_NN` + `s_shield_NN` tactical selection slots (24), `special_button_NN` abilities, `c_button00..05` command buttons, `r_NNNN` reinforcement slots.
* **Factions**: texture lists in the XML are indexed Rebel 0, Empire 1, Underworld 2, Pirates 3, Hutts 4; frame meshes use their own `_ALTn` suffix (Empire 0, Rebel 1, Underworld 2). Skins: Empire blue, Rebel red, Underworld amber.
* **Bars**: a `Bar` has `Max_Bar_Level` (10) flat-colour textures (`i_bar_health_small0..10`: black 0, red 1-3, orange 4-5, yellow 6-7, yellow-green 8-9, green 10) over a track (`i_bar_control_wide`). The engine picks `round(fraction * 10)`. World bars use the small, medium or large set (34, 68, 102 px wide, 2 px tall); the shield uses the full `i_bar_shield_*10` clipped to the fraction.
* **Fonts**: the game names `EmpireAtWar-Medium`/`-Bold` and Arial; no font files exist in the data. Bahnschrift (bar) and Arial (tooltips) come from Windows.
* **Tactical abilities**: each `Unit_Ability` type maps by convention to icon `I_SA_<TYPE>.TGA` and strings `TEXT_TOOLTIP_ABILITY_<TYPE>_NAME` / `_DESCRIPTION`; `HUNT` and `SPOILER_LOCK` are toggles; types without icon are skipped. The shell's 24 `special_button` rectangles are small (24x25 design px); buttons are placed per unit card, two per card, centred over it, at `<Scale>` 0.8. An outline animation (`sa_ami_outline_NN`, 8 fps) runs while an ability is on.
* **Hardpoint reticles**: `I_HARD_POINT_RETICLE_<KIND>[_TRACKED].DDS`, the art `Gameconstants.xml` names per hardpoint `Type` (`HardPoint_Target_Reticle_Enemy_Texture` / `_Friendly_Texture`: WEAPONS_TURBO for lasers, WEAPONS_MISSILE, WEAPONS_TORPEDO, WEAPONS_ION, WEAPONS_MASS_DRIVER, ENGINES, SHIELD_GEN, DOCKING_BAY, GRAV_WELL, GENERIC for tractor beams), `HardPoint_Target_Reticle_*_Screen_Size` (0.022) of the screen height, white art tinted by the hardpoint's health; shown on the ship under the cursor (own or enemy) and on a selected enemy. No bracket texture exists for ship selection, so brackets are drawn corners.
* **Tooltips**: rich panel (icon and title row, divider, body, two-column stat rows, red warning). Text format: first line title, `@Label<TAB>value` stat row, `!text` warning, anything else body. Build pads, queue slots and `WorldTooltip` anchor beside the advisor droid; other buttons anchor at the cursor. `Encyclopedia_Text` ids (with `TEXT_LINE` as line break) are the long descriptions.
* **Planet view**: the structure pads fill only the bottom `b_create_NN` row (the engine lays the pads out in code; the shell bones for the 80 pad/unit slots all sit at the origin); `PadRemap` maps slot to entry.
* **Per-craft bars and flags**: one bar (0.84 of the craft's drawn sphere, 20.6 design px long) and a garrison flag per craft, batched into the UI render phase as two glyph runs (one item per rectangle cost 3-4 ms per frame with 2000 craft). Unpicked squadrons are one 11.4 design px icon on a faint plate in a layer below the command bar.
