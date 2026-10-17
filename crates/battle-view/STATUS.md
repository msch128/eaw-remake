# battle-view status

Bevy plugin that renders and drives a `battle-sim` space battle with the EaW look and controls.

## What works
- Scene from a space `.ted` (`SpaceScene`): skydome, backdrop planet, props, sun + ambient, bounds.
- Sim on a worker thread (`sim.rs`), one snapshot per step; render interpolates (smooth at any FPS).
  `blocking: true` makes offscreen runs reproducible.
- Ships as `AlamoModel` (team colour, bank into turns, pitch with climbs, small craft weave: `flight.rs`), hardpoint death (hide meshes, damage fires,
  death particle), capital death sequence, fighters despawn with a small explosion.
- Bolts: pooled `ProjectileVisual`; impacts via `ProjectileImpact` / `ShieldHit` (budgeted per frame).
- Camera: wheel zoom, WASD/edge pan, Q/E or MMB rotate, Ctrl+MMB grab, clamped to map bounds.
- Control: click/box select, Ctrl+A, Ctrl+digit groups, RMB move / Shift+RMB attack-move / attack /
  attack hardpoint (markers on hovered or clicked enemy), S stop. Gizmo circles, hull/shield bars.
- Tests: `cargo test -p battle-view` (3 coordinate tests). Clippy clean.

## API
- `BattleViewPlugin` (needs `AlamoRenderPlugin` first); start with
  `commands.insert_resource(BattleSession::new(setup, VisualCatalog::build(..), SpaceScene::load(..)))`.
- Resources: `BattleViewSettings`, `BattleStatus`, `BattleRuntime` (`command(side, cmd)`),
  `Selection`, `Hover`, `Pointer`, `RenderedShips`; component `BattleCamera`.
- Sim must use `BattleConfig::arena` (map space). `convert::SIM_TO_MAP_YAW` is 0 for that; without
  an arena the sim places attackers at -X and the constant must be PI/2.

## Run
```
$env:CARGO_TARGET_DIR="...\target\battle"
cargo run --release -p battle-view --example battle_demo -- --offscreen out.png --size 3840x2160 --times 5,30,90 --auto
```
Without `--offscreen` it opens a window. Other flags: `--map`, `--cam yaw,pitch,dist[,x,z]`,
`--overlay`, `--speed`, `--msaa`, `--no-bloom`, `--scale`, `--small`. `--features profile` +
`TRACE_CHROME=trace.json` writes a Chrome trace.

## Known gaps / bugs
- No Death_Clone animation; death particle files run ~30 s.
- CPU-bound: ~5 ms (Coruscant) to ~9 ms (Kuat, 41 heavy props) per frame at 4K release, measured while
  another game ran. Props are not merged or culled by distance.
- Demo `--auto` camera zooms to max distance in the new large arena; `PROPS`/`NOPROPS` env switches
  in the demo are debug leftovers.
- No move-destination lines or muzzle flashes; sound hooks exist but were never heard in a live run. Pointer/box select untested with a real mouse.
- Impact particles are one entity each (alamo-render `AutoDespawn`, default 10 s); capped by
  `max_effects`.

## Next steps
1. Test real-mouse selection/orders in a window; tune pick radii and `ui_scale` for 4K.
2. Merge or LOD static props; check Bevy extraction cost (transform propagation, batching).
3. Death-clone models or shorter death effects; engine glow tied to speed.
4. Shield bubble visibility from sim state; hull-damage smoke below 50% health.
5. Live-test the round trip in the app (see "Integration for space-peace"); reinforcements in the request.

## Open questions

- Is the huge Kuat prop clutter (Executor etc. under the plane) wanted in battles?

## Galaxy <-> tactical loop (new)
- `BattleSession::from_request(&GameData, &dyn FileSource, &TacticalRequest, player: Option<FactionId>)` builds catalog,
  arena, `.ted` scene (`Art/Maps/<request.map>`), visuals and team colours from a `galaxy_sim::TacticalRequest`.
  The player's side gets no built-in AI (units idle until ordered); `None` = AI plays all sides.
- `BattleRuntime::outcome() -> Option<TacticalResult>`: available once the sim reports the battle over (one side
  destroyed or retreated, or the time limit); survivors/retreated per request group.
- Message `BattleFinished { battle_id, result }` is written once when the displayed picture reaches the end.
- `Snapshot::result` carries the final `BattleResult`; `ShipSnap::abilities` carries (active, cooldown) per ability.
- `tactical_ui::TacticalUiPlugin` (needs `eaw_ui::EawUiPlugin::new(fs).with_game_data(data)`): spawns the EaW space tactical
  bar with each battle, fills `TacticalData` (selection grouped per unit type with hull/shield, ability buttons with cooldown/active,
  retreat countdown/ready, population) and turns clicks into commands (`b_retreat` -> `Command::Retreat`, or with `BattleViewSettings::retreat_prompt` the retreat question of `retreat_prompt.rs` (Retreat / Auto-Resolve / Cancel), during the jump countdown -> `Command::CancelRetreat`, ability button ->
  `UseAbility` for the focused group, selection slot -> sub-select, shift removes). Not wired: reinforcement buttons, eaw-ui
  world-space overlays (gizmo bars stay), `Hardpoint` markers.
- `audio` module: `PlaySfx` messages for weapon fire (`Fire_SFXEvent`), non-shield hits (`Projectile_SFXEvent_Detonate`), ship death
  (`Death_Explosion_SFXEvent`), hardpoint destroyed, ability activation, unit select/move/attack responses (`SFXEvent_Select/_Move/_Attack`),
  budgeted per frame (24 total, 2 per event name). Silent without `AlamoAudioPlugin` (the view registers the message queue itself);
  `BattleViewSettings::audio = false` switches the hooks off; `BattleAudioStats` counts what was sent. `MusicDirector` (if present) gets
  `MusicScene::Space(player faction)` and an intensity from the share of ships still alive.
- Map fill lights: `scene.lights[1..3]` are spawned as shadowless directional lights; `alamo.wgsl::fill_light` adds their diffuse term
  (directional lights after the first; the sun stays the first one).
- Example `tactical_roundtrip --offscreen out.png [--at 25] [--speed 30] [--ui]`: pending battle in galaxy-sim -> `tactical_setup` ->
  `from_request` -> AI vs AI at 30x -> `BattleFinished` -> `apply_tactical_result`, prints the new galaxy state.

## Integration for space-peace
Add `battle-view`, `galaxy-sim` (already there through galaxy-play) and `eaw-ui` to the app; add `AlamoRenderPlugin`, `BattleViewPlugin`,
`TacticalUiPlugin` and `EawUiPlugin` once at startup (they are idle without a `BattleSession`).

1. On `PlayEvent::OpenTacticalBattle(PendingBattle)` (sim thread or wherever the `Sim` lives): `let req = sim.tactical_setup(battle.id)?;`
   and hand `req` (plain data, `Clone + Send`) to the app thread. Pause the galaxy clock for that battle.
2. App thread, entering `AppScreen::SpaceBattle`: hide the galaxy scene/camera, then
   `commands.insert_resource(BattleSession::from_request(&data, &*game_fs, &req, Some(player_faction_id))?)`.
   Optional: `BattleViewSettings { audio: true, ui_scale, msaa_samples, .. }` and `EawUiSettings::camera = None` (the UI uses the default camera).
   Tip: building the catalog takes a moment (only the units in the request are loaded); do it on a worker thread if the loading stall matters.
3. Each frame in `SpaceBattle`: `MessageReader<BattleFinished>`; on a message send `f.battle_id`/`f.result` (it is `Clone + Send`) to the sim
   side and call `sim.apply_tactical_result(f.battle_id, &f.result)` -> `Result<Vec<Event>, TacticalError>` (events: `UnitsDestroyed`,
   `BattleWon/Lost`, `BattleResolved`; planet capture follows from the normal capture rule on the next `step`). Forward the events like any
   other sim events.
4. Leaving the screen: `commands.remove_resource::<BattleSession>()` (despawns scene, camera, bar, worker thread) and show the galaxy again.
   Watch for the `BattleRuntime` resource to disappear before showing the galaxy camera. If the player quits the battle early, call
   `sim.cancel_tactical(id)` and then `resolve_battle_auto(id)`, or let the battle run with `BattleViewSettings::speed`.
5. Auto-resolve is unchanged: `Sim::resolve_battle_auto` / `resolve_battle_tactical` still work (the latter now goes through the same request
   path as the front end).

## Quality presets and shadows
- `BattleViewSettings::{shadows, particle_density}` and `QualityPreset {Low, Medium, High}` (`apply_quality`): Low = no MSAA / bloom / shadows, 0.5 particles;
  Medium = 2x MSAA, bloom, 0.75; High = 4x MSAA, bloom, 3-cascade sun shadows to 8000 units, full density. `start_battle` sets `AlamoRenderSettings::shadows`
  and `ParticleSettings::density` before the models spawn; `spawn_scene` uses `spawn_sun_shadowed` when shadows are on. `battle_demo --shadows | --quality low|medium|high`.
- space-peace `Settings`: `shadows`, `particle_density`, options rows "Graphics quality" (cycles the presets, "Custom" when the values match none) and "Shadows".
- 4K, ~290 ships, `battle_demo --seed 7`: default 10.3 ms mean, `--shadows` 10.1, Low 8.6, High 10.1 (p99 12-14 ms). Shadows are visible (`target/vis/shadow_cmp.png`).

## GPU profiling
- `battle_view::gpu_profile` (`timestamp_render_plugin()` replaces the `RenderPlugin`, `GpuProfilePlugin` collects Bevy's `RenderDiagnosticsPlugin` timestamps, `GpuProfile::summary()` prints
  per-pass medians/p95, GPU and CPU). `battle_demo --gpu-profile` prints it at the end. Needs Vulkan/DX12 timestamp queries; per-system CPU time is not collected here (use `--features profile`
  with `TRACE_CHROME=1`). To use it in `space-peace --offscreen`: build `DefaultPlugins` with `.set(timestamp_render_plugin())`, add `GpuProfilePlugin` after it, print `GpuProfile::summary()` on exit.
- 4K, 290-ship battle, High (shadows): GPU medians per frame: bin_unpacking 0.96 ms (GPU mesh preprocessing), opaque pass 0.47, bloom 0.19, transparent pass (all particles) 0.07, clustering 0.07,
  upscaling 0.03: about 1.8 ms in total against a 9.4 ms wall mean, so the demo is CPU / pacing bound, not GPU bound. Hitches of 20-40 ms line up with sim steps of 4-12 ms (blocking offscreen mode waits for the sim).

## Sim threading and step cost (checked, nothing to change)
- `sim.rs` already runs `battle-sim` on a worker thread: the render side keeps a virtual clock, fetches the snapshots for `tick` and `tick + 1` and interpolates; commands go through a channel;
  `blocking` (offscreen/tests) waits for the snapshots so output stays deterministic. The windowed app uses the non-blocking path, so sim steps never run on the render thread.
- `battle_demo` now prints sim step statistics (`sim step over N ticks`). 4K, ~290 ships, three runs: step mean 0.44-0.59 ms, p50 0.38-0.41, p99 1.1-1.9; one outlier of ~40 ms in two of the
  runs (coincides with the screenshot readback). `battle_bench` (headless, 1254 ships incl. fighters): 0.45 ms/step parallel, 1.73 ms sequential. The wall frame mean is 7.7-8.2 ms, so the render
  thread (main-thread systems, extraction, draw submission), not the sim, is the CPU cost; the GPU is ~1.8 ms (see GPU profiling). Next candidate: a Chrome trace of the render-side systems.

## Render-side CPU trace (4K, ~290 ships, steady state after the first 8 s)
- Chrome trace (`--features profile`, `TRACE_CHROME=...`, ~2.5 GB for 30 s; aggregate with grep + awk on the `system: name=` spans) with tracing overhead, per frame: render world `run_render_schedule` 8.4 ms
  (`render_system` 3.6, of which `camera_driver` 2.7 but its passes only ~0.6; `write_binned_instance_buffers` 1.1; `allocate_and_free_meshes` 1.0 (our per-frame particle/bolt batch meshes); `submit_pending_command_buffers` 0.8;
  `prepare_preprocess_bind_groups` 0.7). Main world is small: `propagate_parent_transforms` 0.29, `simulate_particles` 0.22, `animate_models` 0.16, `update_particle_batches` 0.15, `update_ships` 0.13, everything else < 0.12 ms.
- Scene scaling: `battle_demo --small` (1214 mesh entities, ~650 visible) 7.3 ms vs 8.4 ms for the full battle (2728 entities, ~850-1150 visible): the 290 ships add only ~1 ms; ~6.5 ms is a floor of the offscreen
  harness (1 ms `ScheduleRunnerPlugin` wait, Bevy render-graph/submit overhead, MSAA/HDR 4K target) that does not scale with resolution (960x540 is not faster). No game-side hotspot to optimize; the windowed app does not have the 1 ms wait.

## Entering a battle without a stall
- `start_battle` now requests `BattleSession::asset_paths()` (ship models, sky, nebula, props) from `alamo_render::ModelPreloader` the first frame the session exists and waits (`is_idle`) before it spawns the
  scene, so file reads, `.alo` parsing and texture decoding run on worker threads and the render thread only builds meshes. The system runs while the `BattleSession` resource exists (no longer only on the frame
  it is added); it does nothing once the battle has started. No app change needed: the loading screen just stays up a few frames longer. Hardpoint/turret models are not in the list and still load on first use (preloaded since 2026-10-17, see "Reinforcements without a stall").
- `battle_demo` prints `longest of the first 400 frames`. Release, 1920x1080, demo battle: 107-118 ms (frame 2) before, 62 ms after; the remaining 62 ms is the demo's own `Battle::new` (the app builds the battle on a worker thread in `from_request`). A later 58-61 ms frame in one of three runs is the Executor's mesh build (18 ms) plus other spawns.
- Preload now also covers the battle's particle textures: `BattleSession::effect_names()` (death effects of ships and hardpoints, the class explosions, the particle objects of every projectile in the catalog)
  goes to `ModelPreloader::request_particles`. Hardpoint/turret models are not part of the list because battle-view does not spawn them as models (no `with_unit`); the log line `load_model` (>= 20 ms) shows
  any model that still loads late.
- Measured on the real app path (`space-peace --offscreen --screen galaxy --campaign GC_Mini_Web_Way_Rebel --weeks 250 --stop-on-battle --battle-choice fight`, the integrator's "longest frame while loading"):
  release 71-125 ms with preload vs 191-232 ms without (`SP_NO_PRELOAD=1` switches it off); dev profile 144 ms vs 627 ms. Run-to-run noise on this shared machine is about +-30 ms.
  Tried and dropped (no measurable change): a per-frame spawn budget for `spawn_models` (`AlamoRenderSettings::spawn_budget`, still available, default off) and `RenderAssetBytesPerFrame` (16 MiB). The remaining ~70-100 ms
  frame is not in `start_battle` (it takes ~0.1 ms) nor in model loading; it is app-side (tactical UI / session start) and needs a Chrome trace of that frame.
- Chrome trace of the battle entry on the app path (`cargo build --release -p space-peace --features battle-view/profile`, `TRACE_CHROME=...`; ~27 GB for 100 s, so analyse with grep + awk for spans > 10 ms and
  delete it): the remaining hitches are three separate one-frame costs, none of them model or texture loading (traced times, release is faster): `OnEnter(AppScreen::SpaceBattle)` 55 ms (`battle_play::spawn_loading_art`
  / `suspend_galaxy_look`, `hide_galaxy`, `hide_play_ui`), the command flush that spawns the ships' bone hierarchies 31 ms (+10 ms in `spawn_models`), and `tactical_ui::spawn_bar` -> `spawn_eaw_command_bar` 40 ms
  (the whole command bar tree in one command). Bevy's own `run_render_schedule` shows 30-40 ms spikes (and the systems that wait for it) a few times per run. Not fixed: each needs structural work
  (async loading-art decode, chunked UI spawn).
- Battle-entry hitches, round 2 (all behind the loading screen): (1) the loading art is decoded on a worker thread (`loading::decode_art`, `battle_play::attach_loading_art`): `OnEnter(SpaceBattle)` no longer shows up
  in the trace (was 55 ms) and the art fades in a few frames later; (2) `tactical_ui::spawn_bar` runs as soon as the `BattleSession` exists (resource `TacticalBarSpawned`, removed with the runtime or the session), a
  frame of its own before the ships spawn (still ~44 ms traced, it is one command inside eaw-ui); (3) `start_battle` sets `AlamoRenderSettings::spawn_budget` to 8 ms (reset in `stop_battle`) so ships spawn over several
  frames, and `battle_play` keeps the loading screen until no `AlamoModel` is waiting for its `AlamoModelLoaded`. The ship spawn flush went from 31 + 10 ms to 25 ms per frame.
  Longest frame while loading, release: Yavin_WW scenario 71-125 ms before, Hoth (`--campaign GC_Full_Web_Way_Rebel --select "The Yavin System" --weeks 8 --move Hoth --battle-choice fight`, deterministic) 56-70 ms now,
  one random-battle run 57/57/91 ms. The rest is Bevy's render world (`run_render_schedule` 30-40 ms spikes with `propagate_parent_transforms`/instance-buffer writes waiting on it), not our systems.
  `tools\smoke.ps1 -Only battle-fight-orders` passes.

## Reinforcements without a stall (2026-10-17)
- `SP_FRAME_LOG=<ms>` (`frame_log.rs`) logs every battle frame whose main-thread work or duration exceeds the limit with what it did: new ships / models / particle systems, render pipelines created or
  still compiling (render world), the longest render-world frame, the last sim step and every main-thread load noted in `alamo_render::frame_trace` (model builds, texture decodes, particle / animation / bolt
  parses, time spent waiting for the simulation). `alamo-audio` logs its first-play reads and decodes with the same variable.
- Measured (release, `--offscreen --size 1920x1080`, Yavin -> Hoth with `--battle-reserve 3`, `--grant-unit MC-80_Star_Cruiser,MC-75_Cruiser,Endurance_Fleet_Carrier,Dauntless_Cruiser_R,Bothan_Assault_Cruiser_R
  --at "The Yavin System"` and `--battle-click b_reinforcement,r_0000,...` calling in one unit of each of the 10 reserve types): every arrival of a type that was not on the field read and parsed files on the main
  thread: the particle systems its proxies name (engine glows, blink lights: 7-14 ms each from a cold disk; MC-75 frame 92 ms, Bothan 38 ms), its hardpoint models with their textures (MC-80: 40 turrets in
  31 + 21 + 15 ms frames, Endurance bridge / tower 12 + 10 ms), the bolt texture of a new weapon on its first shot (`w_turbolaser.dds` 12.5-13 ms), explosion particle systems on their first use.
- Fixes: the preload list adds the models the hardpoints attach (`BattleSession::attached_models`, also missile models), the `Damage_Particles` objects and the IDLE / DIE clips; `alamo-render`'s preload parses
  proxy particle systems with each model, keeps the parsed effect systems and builds the bolts (see its STATUS). The warm-up (`warmup.rs`, `BattleViewSettings::warm_up`, set by space-peace) spawns one instance
  of every (unit type, side) the request can field (groups, reserve, squadron members, hangar fighters) plus the arrival flash and marker in front of the battle camera while the loading screen covers it; meshes,
  team-colour materials, turrets, proxy particles and render pipelines are built then, and the instances are removed once every model is built and no pipeline is compiling (at most 3 s more, 10 s in all). The
  battle clock stands until then, and `battle_play` keeps the loading screen until `BattleWarmup::done`. The arrival / departure flashes share one sphere mesh.
- Result (same scenario): main-thread file loads during the battle 40 (174 ms, worst frame 80 ms) -> 0; worst arrival frame 95 ms -> under 17 ms (no logged frame at the arrivals). Warm-up 0.1-0.9 s with 27-44
  instances. Still open: the battle music start / track change reads the whole MP3 on the main thread (13-20 ms, `alamo-audio` `start_track`); frames of 30-50 ms without any load note come from the shared
  machine (other builds on the same cores).

## Hyperspace flash without a sphere (2026-10-17)
- User report: units that jump out of hyperspace briefly have a white sphere around them (a Star Destroyer inside a translucent
  grey-white sphere). Cause: the arrival flash (d15ea16; the component `ships::ArrivalFlash` since U59) was an additive `Sphere::new(1.0).mesh().uv(24, 16)` (425
  vertices) of one even brightness, alpha 0.75 fading to 0 over 0.6 s, radius 0.18 to 0.9 times the ship's radius, switched on 1.275 s after
  the ship appeared (when it has settled): the ship stood inside a ball with a hard rim. The departure flash of a retreating ship was the same
  sphere. The warm-up (ec712e6), a tint and the shield meshes are not the source: in the frame burst below the only thing round the ships is
  that sphere (it shows from 9.55 s, once it has grown past the hull; the ships look the same without it), and the shield meshes stay hidden.
- Now: a camera-facing quad (two triangles) with a glow image (`flash_glow`: tight core, thin rays along the screen axes and diagonals,
  smooth to zero before the edge of the image: no rim at any angle), additive and unlit, 0.4 s (`FLASH_SECONDS`), half side 0.8 to 4 times
  the flash radius (0.36 to 1.8 ship radii at an arrival, about a quarter of that is bright), brightness `FLASH_PEAK` 1.0 times (1 - t)^2.
  `spawn_flash` / `flash_look` / `apply_flash` are small pure functions and tested; the flash entity goes with the battle (`BattleEntity`,
  the departure flash had none). `warmup.rs` warms the same textured material (its pipeline differs from the old untextured one), so a
  reinforcement still drops in without a stall.
- Tests (ships.rs): `a_flash_is_a_soft_glow_without_a_rim` (no step larger than 0.05 of the brightness per hundredth of the radius at 9
  angles, 0 at the edge and in the corners, the old sphere stepped 0.75), `a_flash_is_one_flat_quad_not_a_ball` (4 vertices, the sphere had
  425), `a_spawned_flash_waits_hidden_and_adds_its_glow_to_what_is_behind_it`, `a_flash_blooms_and_fades_without_outgrowing_its_ship` (the
  glow adds under 0.05 at the ship's own radius), `a_flash_turns_its_glow_to_the_camera_and_leaves_when_over`.
- Frame burst (offscreen, `battle_demo --small --size 1280x720 --cam 30,35,3000 --times 9.20,9.25,...,10.00` with `SP_DEMO_REINFORCE=3
  SP_DEMO_ARRIVAL=1`: three Star Destroyers called in at 3 s arrive at 8.1 s): before, three hard-edged translucent spheres from 9.55 to
  10.0 s; after, a soft flare from 9.55 to about 9.75 s and nothing from 9.85 s. `SP_DEMO_ARRIVAL=1` frames the player's newest capital ship;
  `SP_DEMO_RETREAT=61` (the sim allows a retreat from 60 s, the jump follows 10 s later) shows the departure flash: while the battle is over
  the sim clock stands still, so a departure flash of the last ships stays at its first look (a small star at the ship's old place) until
  `FLASH_WALL_SECONDS` (3 s) ran out, as the sphere did.
- Not done: no Remake frame of an arrival to compare with (the video's battle chapter has none; the engine draws the jump itself, the XML has
  only sounds and speed factors), so the size, length and brightness of the flare are look estimates. Everything is in the constants
  `FLASH_PEAK` / `FLASH_SECONDS` / `FLASH_REACH`; a flash that is not wanted at all is one `spawn_flash` call to remove.

## Tactical screen fidelity round (2026-10-08, tactical agent)
- **World overlays are eaw-ui's now:** `tactical_ui::sync_overlays` fills `eaw_ui::TacticalOverlays` (own resource; `TacticalData` no longer carries overlays):
  green corner brackets + the game's `i_bar_*` health/shield textures for selected and hovered ships, a health line under every own ship
  (`ShipOverlay::always`, cap 200), brackets/bars on the hovered or selected enemy, hardpoint reticles (`I_HARD_POINT_RETICLE_*`) of the targeted enemy;
  a click on a reticle sends `AttackHardpoint`. Names float only over hovered ships and a lone selected ship. The gizmo rings/bars of
  `control::overlays` run only when `TacticalUiPlugin` is absent (`control::OverlaysActive`). Order flashes and the drag box are still gizmos.
- **`hud.rs`** (positions measured from the real-game reference screenshot, design pixels): R2 advisor bubble (`TEXT_DROID_ADVISOR_<FACTION>_SPACE_NN`,
  first hint at 1 s, then one every 75 s for 14 s: estimates), "Mission Objectives" box (`TEXT_TEST_OBJECTIVE_HEADER`, `TEXT_TACTICAL_PRIMARY_SPACE_ATTACK`
  or `..._DEFEND_<ATTACKER>` from the request: attacker = any side before the last), hero portraits top right (`is_named_hero` ships of the player, `Icon_Name`,
  health line, ring turns orange when selected, click selects, double click focuses). Not done: the cyan tail of the advisor bubble, hero ability buttons.
- **Camera:** edge scroll 2 px region, speeds 1000 (close) to 4000 (far) units/s and Shift x2 straight from `Gameconstants.xml` (`Tactical_*`, `Push_Scroll_Speed_Modifier`);
  Space or a double click on a ship (`control::FocusCamera`) centres on the selection; Page Up/Down and middle-drag tilt (limits are estimates). Default pitch/zoom limits
  are still estimates (not in the data).
- **Input blocking:** `control::ui_block` sets `UiBlock` while the cursor is on a visible UI node with a picture (alpha tested, the bar's frame art is mostly transparent),
  a background or an `Interaction`; the overlay layer (ship bars, reticles) does not count. `selection`/`orders` ignore clicks and hover then.
  **Flicker rule (2026-10-16):** UI that follows the pointer or appears because of the hover (cursor labels, hardpoint tooltip,
  unit panel, hero billboards) must carry `control::PointerTransparent`. Without it the node blocks the world under itself, the hover clears,
  the node hides, the hover returns: a toggle every frame (the order cursor and hardpoint flicker of a7e667e). The hover pick keeps last
  frame's ship / hardpoint while the cursor is still on it (`pick`, `keep_hovered_hardpoint`) so overlapping moving targets do not alternate.
  Tests: `ui_block_tests`, `pick_tests`.
  **Pitfall found:** node transforms are physical pixels of the window; `ComputedNode::inverse_scale_factor` includes `UiScale`, so cursor / inverse_scale_factor is wrong
  (use `camera.target_scaling_factor()`).
- **Bug fixed in eaw-ui:** `b_droid_help_tactical` (R2) is a 93x120 Block button that covered pause / fast-forward, so those clicks went to the droid. It is now built first
  (underneath). Camera/pause/option buttons are fine.
- **Real-input self test** `input_test.rs`: `SP_INPUT_TEST=1|exit` with a windowed run (`space-peace --hidden ...`, battle for the player) injects `CursorMoved`,
  `MouseButtonInput` and the window's cursor position before Bevy's input systems, so it exercises the real pipeline (`ButtonInput`, `ui_focus_system`, `read_pointer`):
  select ship, right-click order, plain `Button` control experiment, a command bar button click, bar area keeps the selection, empty click deselects. One line
  `input-test: N checks PASS|FAIL`; smoke scenario `windowed-battle-input`. Failures print the nodes under the cursor.

## Tactical round 2 (2026-10-08, tactical agent)
- **Overlays:** one bracket + bar pair per unit (a squadron is one unit: `ShipSnap::squad`), only for selected/hovered units (no always-on lines); shield bar sits above the hull bar. Since 2026-10-17 fighters, bombers and squadron members have no bracket / bar pair of their own: each craft carries its own bar and flag (last section).
  Hardpoint reticles with health on the hovered ship, the selected enemy or a lone selected ship (clickable only on enemies with own ships selected).
  Everything battle-scoped is cleared on exit (`remove_bar` empties `TacticalOverlays`, the HUD, fog, armed order, battle cam, selection, pause) and `check_teardown`
  logs an ERROR (smoke fails) if tactical UI entities are left 8 frames later. All despawns in this area are `try_despawn`.
- **Buttons (real input, covered by `input_test`):** `b_play_pause_t` pause, `b_fast_forward_t` 1x/2x, `b_camera_t` battle cam (C: follows the selected unit or the fleet, circles it;
  tooltip rewritten because `TEXT_MOVIE_MODE` reads `BATTLECAM "!`), `c_button00..02` arm Attack / Attack-move / Move for the next left click (right click or Esc cancels),
  `c_button04` stop, `c_button05` guard (stop + defensive stance), `b_droid_help_tactical` next hint, `b_story_arc_t` hides/shows the objectives. Waypoint mode (`c_button03`) acts as Move.
- **Right click on an enemy = attack:** also on the outer part of big models (ground point within 1.3 x radius of an enemy footprint; picking radius is the full bounding sphere).
- **Fog of war:** `ShipSnap::seen_by` (from `Battle::is_visible_to`) -> `ShipRender::visible`; unseen enemies are hidden, not pickable, not on the minimap; the minimap is dark outside the
  sensor discs (`Space_FOW_Reveal_Range`) of own ships (`eaw_ui::MinimapFog`).
- **Reinforcements:** a click on a reserve slot arms placement; the cursor shows a ring (green/red + reason, rule = `Battle::reinforcement_zone_check` evaluated on the snapshot with
  `BattleConfig::reinforce_zone`), a left click sends `Command::ReinforceAt` (Shift = whole stack). Scripted offscreen runs (`synthetic_pointer`) place automatically.
  Targeted abilities (`reveal_radius > 0`, FOW_REVEAL_PING) arm a target click -> `UseAbilityAt`. Unit tooltip (name, hull, shield) after 0.35 s of hovering.
- Not done: ability hotkeys (the data has none; digits are control groups), autocast toggles, unit description text in the tooltip (key not found yet), waypoint paths.

## Tactical round 3 (2026-10-08): video reference alignment
- **Camera (user spec):** hold the middle button: left / right rotates (yaw), up / down tilts (pitch, limits 0.55..1.30 rad are estimates); Page Up / Page Down rotate, Q / E too; wheel zooms.
  Covered by `input_test` (`middle-drag-rotates-and-tilts`, `page-up-rotates`; 20 checks in all).
- **Overlay look (video frames v0700/v0710):** small units are marked by a little icon box (`ShipOverlay::icon`, the unit's `Icon_Name`) with two bars under it, squadrons only while selected or hovered (unselected, own ones show their fighters with the bars and the flag: user's video b0703s, b0732s); capitals and stations
  keep corner brackets and carry the bars above (shield above hull) (a picked squadron's box has no bars under it any more: its craft carry their own). Hardpoint reticles show health; hovering one shows "<Text_ID of the hardpoint> - <projectile damage> DMG - <health>%".
- **Unit panel:** hovering a ship (0.35 s) shows the cyan glass panel above R2 (icon, name, `Encyclopedia_Unit_Class`, first `Encyclopedia_Text` string, hull / shield); the advisor bubble hides meanwhile.
  The "No AI / Destination" lines of the video are not reproduced (the snapshot has no order state).
- **Heroes:** portraits top right with orange rings (white when selected), plus a small face box floating next to each hero ship (`HeroBillboard`).
- Seen in the offscreen render: the integrator's galaxy planet tooltip ("The Yavin System ...") stays on screen during the battle.

## Tactical round 4: ability targeting and hotkeys
- Targeted abilities (`reveal_radius > 0`, `zone` = lightning storm, `super_laser` = Death Star) arm a target click: the cursor shows the area (ping radius, storm radius plus the placing range
  `zone.max_range` around the caster, super laser aim circle of 600 units = the sim's target search), green / violet / orange when valid, red with the reason when the sim would refuse
  ("Not ready", "Out of range", "No target": a capital, frigate or structure of the enemy within 600 of the point). Valid click = `Command::UseAbilityAt`; the mode stays on after a refusal.
  The sim's refusal itself is not visible to the view (commands are fire and forget), so the checks are replicated on the snapshot.
- Hotkeys: Shift + 1..6 press the ability buttons of the focused unit (the data has none; plain digits stay control groups). Autocast toggles are not implemented (no state for it in battle-sim).
- Tests: `ability-hotkey` in `input_test` only runs for real when the selected unit has an ability (it is vacuous in the Hoth smoke scenario); the targeting cursor has no automated test yet.
- Esc (host decision, `space-peace::battle_play::battle_menu`): cancels an armed order / targeting first; with nothing armed it opens the Resume / Retreat menu, which pauses the battle clock while open (the player's own pause is restored on close). `control::orders` no longer reads Esc. `input_test` steps `escape ArmedCleared / Paused(true) / Paused(false)` (25 checks).

## Tactical round 5: move lines
- `ShipSnap::order` / `ShipRender::order` (sim `Order`) feed `control::overlays`: every selected ship of the player, and every ship ordered in the last 3 s (`RecentOrders`), draws a thin line to its destination
  (Move / Guard green, AttackMove yellow, Attack red to the target), fading along its length (`line_gradient`, 16 segments), plus a double ring at the destination (30 px radius at any zoom, one per destination).
  A squadron shows one line (its first fighter). The line is straight until battle-sim exposes the planned path (`Ship::path_*` exist but are not in the snapshot yet): then replace the `s.pos -> dest` segment by the waypoints.
- `control::MoveLines` records what was drawn; `input_test` step `move-line-and-ring` (26 checks) right-clicks the ground and expects a line and a ring.
- Move lines now only for ships (not squadrons / fighters / bombers). The destination ring is world sized (4 hull radii of the biggest ship sent there, 250..3500 units, estimates from video frames b0641..b0643: about 440 px radius at 1920 wide, glowing band of ~5 px) and disappears when the ships are inside it (a ship of a group move: once it is near its own formation slot, the end of its path; battle-sim RULES.md 9d).
- Hardpoint tooltip: the name was the internal object name; it now comes from the hardpoint's `Tooltip_Text` (`TEXT_WEAPON_TURBOLASER` ...). `input_test` step `hardpoint-tooltip` really hovers a ship, waits for its reticles, hovers one and checks the tip (and prints its text); 27 checks.
- Move lines follow `ShipSnap::path` (waypoints, sim space) via `ShipRender::path`; fighters / squadrons have none. Check render on the Bilbringi map: `SP_DEMO_MOVE=1 battle_demo --offscreen out.png --size 1920x1080 --map _SPACE_PLANET_VERGESSOASTEROIDS_02 --overlay --auto --seconds 6` (sends the capitals through the nearest rock; the lines bend at the waypoints).
- Group move render (battle-sim RULES.md 9d): `SP_DEMO_GROUP=6000 SP_DEMO_EXTRA=Star_Destroyer:19 STATION_ONLY=1 battle_demo --small --station Rebel_Star_Base_2 --offscreen out.png --size 1920x1080 --times 2,150` sends the player's capital ships 6000 towards the map centre at 2 s and frames them; they stand in 2 rows of 10.
- Hardpoint tooltip hit area: the closest reticle within max(projected 4 % of the hull radius, 24 px) of the cursor wins (was 16 px around the centre); works on any hovered ship, enemy or own. `input_test` hovers the edge of a reticle (20 px off its centre). The bone has no extent in the data, the 4 % is an estimate.
- Autocast: right click on `special_button_NN` toggles `Command::SetAutocast` for every ship of the focused slot that has the ability and `supports_autofire`; a gold ring child (`AutocastRing`) sits on the button while the first ship's `ShipSnap::autocast` is on. `input_test` step `autocast-toggle` is vacuous when the selected unit has no autocast ability (the Hoth scenario has none), so the toggle itself has no live test yet.
- Autocast live proof: smoke scenario `windowed-battle-input-autocast` (`--grant-unit X4 --at "The Yavin System"`: the X4 has `SENSOR_JAMMING` with `Supports_Autofire`). `input_test` clicks that ship (`select-autocast-unit`), right-clicks its ability button and sees `ShipSnap::autocast` flip (`autocast-toggle`); 29 checks PASS. Without such a unit both steps pass vacuously (their log line says so).
- The unit panel adds "Squadrons in reserve: N" for carriers / stations (`ShipSnap::reserve_squadrons`).
- `input_test` (30 checks): `autocast-toggle-and-ring` also counts the gold `AutocastRing` on the bar (1 after switching on); `carrier-panel-shows-reserve` hovers a ship with squadrons in reserve (spot away from its reticles, 140 frames dwell) and expects "in reserve" in the unit panel (reads "not judged" when a neighbouring fighter wins the pick). Not proven: that an autocast ability then fires by itself (sensor jamming needs enemy missiles; that is a battle-sim check).

## Tactical round 6 (2026-10-09): shield flash, hardpoint markers, reinforcement drag and drop
- **U17 shield flash:** `effects::impact_effects` only triggers `ShieldHit` while the ship's shield in the displayed snapshot is above 0, and `effects::hide_downed_shields` hides the
  shield meshes of a ship whose shield is down (a flash still fading when the shield broke). No shield, no bubble: only the hull impact particles remain.
- **U15 hardpoint markers:** `control::hardpoint_world` used the sim offset (`HpDef::offset`, bone position in battle space) without the model's `Scale_Factor` and, for hardpoints whose bone is
  missing in the model, a hash-spread guess. `update_ships` now stores `ShipRender::hp_pos`, the world position of every hardpoint's attachment bone from the loaded model
  (`GlobalTransform` of the bone entity: includes scale, bank and the arrival animation); the sim offset times `ShipVisual::scale` is only the fallback while the model loads.
  Check render: `SP_DEMO_PLAYER=1 SP_DEMO_FOCUS_TARGET=<dist> battle_demo --offscreen out.png --overlay ...` (player side 1 so the Empire ISD is the hovered enemy; `--station Rebel_Star_Base_2` with player 0 for a station).
- **U14 reinforcements (drag and drop):** the `b_reinforcement` button next to the minimap opens the EaW reserve window top left (`I_Main_Reinforce`): population `N/max` (`Snapshot::pop` =
  `Battle::pop_in_play`, the number the unit cap counts) and the reserve icons stacked `xN`. Pressing a slot picks the unit (`reinforce_drag`); holding the button and moving into the battle drags it:
  a ghost of the unit (its model, team colour green or red, plus a ring and the reason) follows the cursor; releasing it over the map on an accepted spot sends `Command::ReinforceAt`, on a refused
  spot the pick ends and the unit stays in the reserve; releasing over the interface cancels. A plain click on the slot still picks it for a later click on the map.
  While a unit is carried the no-spawn zones of enemy structures are drawn as red hatched circles (`draw_no_spawn_zone`).
- **Placement rule** (`battle_sim::placement_check`, shared by `Battle::reinforcement_zone_check` and the view's `tactical_ui::zone_check` / `place_check`): inside the battle area; outside the no-spawn
  zone of every enemy structure (radius = the structure's `Reinforcement_Prevention_Radius` where the data has one, else `BattleConfig::no_spawn_radius` = 2000, an **estimate**: none of the space structures
  in the data has the tag, only the ground planet shield (700)); inside an area the side sees (sensor disc `Space_FOW_Reveal_Range` of one of its ships or structures, or a revealed ping area; with fog of war
  off everywhere); population: pop in play + cost <= unit cap. Enemy mobile ships no longer block a spot (the old 2000 radius around every enemy ship and the "near own ships" rule are gone; the sensor disc
  replaces the latter). "Explored" = currently visible: the sim keeps no persistent explored map. The view does not know revealed ping areas, so the ghost can be red where the sim would accept.
- **Hyperspace slot:** `galaxy_sim::tactical::apply_hyperspace_slots` reads `Galaxy::hyperspace_unit(fleet)` when the request is built: with a slot ship in a fleet of the side only that unit (one of a stack) is deployed, every other unit goes to the reserve (carried fighters launch with the slot unit); star bases and structures stay. Without a slot the start rule is unchanged (everything up to the faction cap, the rest is the reserve). `battle_play` needs no switch.
- `input_test` (36 checks): `reinforcement-drag-Fog` (drag onto ground outside every sensor disc: red ghost "Area not explored", unit stays in the reserve), `reinforcement-drag-NoSpawn` (into an enemy
  structure's zone; reads "not judged" when the scenario has no enemy structure), `reinforcement-drag-Valid` (next to an own ship: green ghost, reserve count drops).

## Battle HUD round (2026-10-14)
- Opening camera frames the median spread of the player's fleet (`START_DISTANCE_MIN` 700 .. `BattleViewSettings::start_distance` 1300), not a fixed 3500.
- Offscreen `--battle-seconds T` is honoured, also together with `--battle-orders` (which used to capture ~1 s after the orders).
- Bar: with nothing of ours selected the unit cards show the whole fleet per type (no abilities, no focus); a click selects that type.
- Minimap: the camera footprint is a projected trapezoid (`eaw_ui::MinimapCameraQuad`), dots are green (own) / red (others).
- Hero portrait click selects and centres the camera (single click).
- B8 effects: capital/station deaths queue a bloom flash (`effects::BloomPulses`, camera-distance weighted, ~0.7 s, boosts `Bloom::intensity`); the hull of a destroyed capital stays 45 s as a burning wreck (`ships::Wreck`, the data's damage fires keep playing). Shield ripples, muzzle flashes, tracer bolts, death explosions and debris chunks already existed. Offscreen: `--battle-fightcam` puts the camera on the closest opposing big-ship pair before the `--battle-seconds` capture.
- U38: `Command::ReinforceAt::yaw_offset` (the ghost's wheel turn, clamped to ~15 degrees in the sim); the wheel no longer zooms while a ghost is carried; no-spawn zones are translucent red discs with a hatch texture and a soft rim (`tactical_ui::sync_zone_discs`). Offscreen check: `SP_PLACE_HOLD=1` with `--battle-reserve 3 --battle-click b_reinforcement,r_0000 --battle-seconds 14` keeps the scripted placement armed (ghost inside the first enemy zone).

## Backdrop planet discs (scene.rs)
- `w_planet_*_low_light.alo` has no sphere: `Planet` is a flat disc (about 72000 radius at scale 45, constant normal) and the map's
  rotation lays it nearly horizontal under (or, flipped, over) the battle plane. Left as placed it is a desert floor filling the
  start view (Tatooine, 31 of 93 maps failed the opening-frame sweep).
- `disc_slide` moves a disc that covers >97 % of the start view (no limb at all) sideways (camera right vector, on the side the map put it) until
  its limb shows (<= 85 %, planet-dominant like the real Bestine shot). Maps with terrain props (Skies) keep their disc. `SP_NO_DISC_SLIDE=1` turns it off.
- Sweep: `tools\space-map-sweep.ps1 -Exe <battle_demo.exe>` renders every planet space map and flags frames with under 3 % dark pixels (a planet with no limb or space in view).

## Battle FX pass (2026-10-15): shield bubble, debris, Death Star
- **Shield bubble:** a hit lights the model's own `MeshShield` mesh as a whole translucent bubble (0.3 of the hit glow, fading over `SHIELD_FLASH_SECS` = 0.6 s), brightest at the hit point (`alamo.wgsl`, `projectiles.rs::on_shield_hit`; glow radius 0.3 x model radius). All three numbers are look estimates; the Remake data has no shield particle beyond `Shield_Hit`. Seen in `--map _SPACE_PLANET_VERGESSOASTEROIDS_02` mid-battle captures (t 85..105 s).
- **Debris chunks (U68):** the orange glowing boxes are now dark squashed low-poly chunks with a faint cooling ember (`CHUNK_EMBER`, estimate). Burning wrecks (45 s) were already there; their damage fires and the data's `Death_Explosions` smoke plume match the CS9 look in captures.
- **Death Star super laser:** `battle_sim::Event::{SuperLaserCharging, SuperLaserFired}` drive `ships::ship_events` / `update_super_laser_fx`: the model's `p_planet_deathstar_laser_small` proxies (nine converging charge lasers) switch on at the order, the `p_planet_deathstar_laser_large` proxy (data particle `P_PLANET_DEATHSTAR_LASER_LARGE.ALO`: green `OuterGreen` + white `InnerWhite` glow) at the shot (`Fire_Frame` / 30), both off 1.5 s later (estimate). The bones are `vis 0` in the model (the ATTACK animation shows them), so the view also shows or hides the parent bone. A green additive beam line (0.8 s, width 1.2 % of the shooter radius, estimates: the data has no beam art) runs from the dish proxy to the target position, plus a bloom pulse. Check: `STATION_ONLY=1 SP_DEMO_NOFOG=1 SP_DEMO_SUPER=1 SP_DEMO_PITCH=30 SP_DEMO_DIST=14000 battle_demo --offscreen out.png --station Death_Star_II --map _SPACE_PLANET_VERGESSOASTEROIDS_02 --seconds 2.3 --speed 1`.
  Not verified: the glow core drawn about 7500 units below the beam origin in that capture (particle placement under the hidden bone), the look at normal battle zoom, a player-fired shot (only the AI shot was captured).
- `battle_demo` capture helpers: `SP_DEMO_PITCH`, `SP_DEMO_DIST` (override the camera), `SP_DEMO_DOGFIGHT` (closest opposing fighters), `SP_DEMO_SUPER` (Death Star dish), `SP_DEMO_WRECK` (first debris chunk).

## Battle FX pass 2 (2026-10-15): missile trails, fighter size, Death Star glow
- **Gray dashed lines** = the missile/torpedo smoke trails (`missiles.rs` gizmo polylines). They looked dashed only in 20x speed captures (one point per rendered frame); at speed 1 they were arena-long straight gray lines (5 s trail). Now 2 s, 2.5 px wide, white-blue, quadratic fade (estimates; Remake n93 f0023 shows short thin curved white trails).
- **Fighter size:** the data gives a TIE about 4 units (Scale_Factor 1) next to a 760 unit Star Destroyer, so at the default camera (distance 930, fov 60) a fighter was ~5 px with an invisible hull. The Remake frames n93 f0011/f0013 show fighters at ~15 px of 576 (about 1/7 of a capital ship), so the original draws small craft far above true scale; no data field found for it. `ships::fighter_boost` lifts fighters/bombers to at least 2.6 % of the screen height (`FIGHTER_MIN_SCREEN_FRACTION`, estimate; cap `FIGHTER_MAX_BOOST` 40). Sim radius, hardpoints and bolt origins are not scaled. Hulls are still dark on the dark map lighting; engine glow rectangles dominate (not changed).
- **Death Star glow offset:** not a proxy transform bug. Logged proxy world positions are right (large proxy at the shooter + 29900 units along the bow, y about 65), the beam origin and the capture focus coincide, and a top-down capture puts the glow on the beam origin; the green haze and white core hang ~6000 units below it in the vertical direction. The particle stream follows the bone's up axis, which in the rest pose is not the beam direction (the ATTACK animation presumably turns it). Tried rotating the proxy towards the target and gravity of world-space emitters into the emitter frame: no visible change, both reverted. Fixed 2026-10-17 (A2b): the cause was the particle axis mapping (particle Z was turned onto the bone's -Y axis); with the bone's own frame the glow streams from the dish along the beam and the charge lattice meets at the dish (alamo-render STATUS "Particle axes").
- `battle_demo`: `SP_DEMO_PROJ=missile|laser|rocket` (close view of the first projectile of the category), `SP_DEMO_SQUAD=1` with `SP_DEMO_DOGFIGHT` (first squadron).

## Battle FX pass 3 (2026-10-15): hull brightness, Death Star ATTACK animation
- **Hull brightness:** checked Bestine (sun 0.6/0.67/0.87, ambient 0.05), Bespin (red sun, red ambient), Kuat and the Vergesso asteroid map. Ships are bright and readable wherever the map sun is strong (Bestine capture `battlefx_bst.png`, Bespin `battlefx_bes1.png`); hulls only read dark on the asteroid map, whose TED sun scalar is low (colour after `light_gain` 0.44/0.48/0.58) and ambient is 0.05/0.055/0.11. That is the map data plus the earlier deliberate compromise in `scene.rs::light_gain` (sqrt of scalar/0.5, 0.3..1.4) and `ambient_floor`; the shader (`alamo.wgsl`, saturated ambient + sun*ndl + fills, `LIT_GAIN`) matches the documented engine formula, so no shader fault found and no blanket boost made. Not verifiable without the engine: whether the Remake lights asteroid maps brighter than the TED scalar says. Side finding: the Bestine planet is a flat saturated blue next to the Remake frame (Bqc5 f0033 shows a dark, photographic planet); planet look is not part of this pass.
- **Death Star ATTACK animation:** playing `AlamoAnimation::once("ATTACK")` on the ship at the charge moment moved the whole camera target (the dish bone) and the beam and glow vanished from the frame; not a cheap fix, reverted. The proxy-bone glow offset stays open.

## Eclipse super laser (2026-10-17)
- **Charge:** every super laser but the Death Star's plays the shooter's `ATTACK` clip (`Attack_Anim_Sub_Index`; the Eclipse's `ATTACK_00`, 300 frames at 30 fps) on the battle clock from the order (`SuperLaserCharge`, `update_super_laser_charge`), so its `Fire_Frame` 229 falls on the sim's shot (`fire_in` 7.63 s). The clip's visibility tracks show the `P_Eclipse_Charge_L` / `_S` charge from frame 51 (red sparks along the trenches, green focus points about 1 s before the shot), the `Ambient_02` / `_03` glow meshes from 226 / 230 and the `P_Eclipse_Star_Dreadnaught_Muzzle` flare from 230. The particle proxies on those bones are lit while the clip runs; at its end they go off, every bone the clip drove gets its visibility back (no glow stays lit) and the model returns to `IDLE`. A model still loading at the order starts the clip late, still on the shot.
- **Beam:** from the unit's `Super_Laser_Ability` (`SuperLaserVisual`): origin `Owner_Particle_Bone_Name` (`MuzzleA_00`), `Beam_Texture_Name` `tractor_beam01.tga` on two crossed quads `Line_Width` 250 wide, tinted `Line_Color`, for `Fire_Continuation` 1 s, stepping through `Num_Anim_Frames` 4 (estimates: frames side by side along the texture's width, 15 frames/s). The texture's alpha is made opaque (it holds an unrelated streak; the beam is additive). The battle warm-up decodes the art and builds the beam's pipeline (the charge does it for scenes without a warm-up). The Death Star keeps its proxy charge and green line (its data comments the beam art out).
- **Sound:** `SFXEvent_Special_Ability_Loop` (`Unit_Death_Star_2_Fire`, `Preset_INSANE_LOOP`: heard across the map) plays at the muzzle from the shot. Its sample `GUN_Superlaser_2.wav` is a whole 3.45 s blast (full for 1.5 s, then a 2 s tail), so the loop is held one pass (3.30 s, `ShotSounds`) or the beam's time if that is longer; the sample is read on the audio worker while the laser charges. Estimate: how long the original holds this loop is not known; held only while the beam fires (1 s, the Death Star II's 0.25 s) it would be cut at full volume, held for `Expiration_Seconds` (5 s / 25 s) it would start over.
- Check: `SP_DEMO_EXTRA=Eclipse_Star_Dreadnought:1 battle_demo --offscreen out.png --size 1920x1080 --times 0.3,2.5,5,7.5,8,8.2,8.8,9.5,10.8 --cam 0,55,7000,-8000,-7500` (one `--cam` per time; the demo AI orders the shot at 0.5 s, it fires at 8.13 s, the clip ends at 10.5 s).

## Battle FX pass 4 (2026-10-15): backdrop planet look
- Data check (`w_planet_ocean_low_light.ALO`): two meshes, `Planet` (Planet.fx, a flat disc; textures `W_planet_ocean` diffuse, `_B` normal with city alpha, `W_PLANETCLOUDS_THICK` cloud + `_BC` cloud normal; `Atmosphere` 0.62/0.77/1.0, `AtmospherePower` 4, `CloudScrollRate` 0.001) and `Atmosphere_Ring` (MeshAdditive, `W_Planet_Atmosphere_Blue.tga`), both already used by the renderer. The cause of the flat saturated blue was the shader's own boosts for land (saturation x1.35, gain x1.2, clouds mixed 90 % at full brightness), not missing textures.
- `alamo.wgsl`: new constants `PLANET_SATURATION` 1.15, `PLANET_GAIN` 0.9, `PLANET_CLOUD_GAIN` 0.7, `PLANET_CLOUD_MIX` 0.6 (estimates tuned on the Bestine frame Bqc5 f0033: dark navy ocean, sparse soft clouds). Captures: `battlefx_pl_*` (before) vs `battlefx_pl2_*` (after) for Bestine, Hoth, Tatooine, Kuat (Kuat not viewed after the change; Tatooine and Hoth still read as the original colours).
- Not matched: a visible terminator (the disc is flat with one normal, so its shading is uniform; the Remake frame is dark on one side), and the rim is wider/brighter on the left than the Remake's thin rim on the lit side only. Both would need a sphere-like pseudo normal across the disc (not done: unknown whether the original does that).

## Battle FX pass 5 (2026-10-15): sphere-lit backdrop discs
- The backdrop planets are flat discs (`w_planet_*_low_light.alo` Planet mesh: 156 vertices, every normal -Z, z extent 0.003 at radius 1600), so no terminator was possible. `model.rs::flat_disc_radius` detects such a mesh (one normal, planar) and stores its local radius in `AlamoParams::disc_radius`; `alamo.wgsl` (planet branch) then tilts the normal by the disc-local radius (`Ns = N * sqrt(1 - rho^2) + perp / R`, centre and scale from the mesh's world matrix) and lights, clouds, night side and atmosphere rim use it. Inferred, not from the original's Planet.fx. Look constants (estimates): `PLANET_DISC_LIGHT` 0.45, specular x0.25, clouds x0.5 on discs. Real sphere planets (non-flat meshes) are unchanged.
- Captures `battlefx_pl3..pl6_*`: Bestine shows a lit side, terminator and dark night side; Hoth is darker with the terrain still readable. Bestine's lit side is still paler than the Remake frame (Bqc5 f0033 is dark navy with a thin lit limb); the cause is probably the bluish map sun x cloud/atmosphere terms, not tuned further.
- Capital hulls on Bestine: `LIT_GAIN` 1.4 was set deliberately from video frames (hulls near white); the blue cast is the map's sun colour (0.6/0.67/0.87 after `light_gain`) times that gain. No second formula error found; the original's treatment of sun colour is unknown, so not changed.

## Audio at every zoom (2026-10-17)
- User finding: zoomed out a little, nothing was audible any more. Cause: U79 (2026-10-11) moved the sound listener from the focus point up to 45 % of
  the way to the camera eye (750 units off the plane at zoom 2000, 2730 at 6064, 7200 at 16000), against `Audio_Space_3D_Listener_Z_Pullback_Dist` 0
  of the Remake's `Audio.xml`. With saturation distances of 40-750 x 1.5 (engines, guns) and a `sat / d` falloff, a far zoom put every engine and gun
  below the start threshold. `audio::battle_listener` puts the ears back on the battle plane at the focus, lifted by the data's pullback, whatever the zoom;
  the distance from the centre of the screen still shapes the mix (`sat / d`), explosions (`Preset_EXPL`, saturation 500000) are heard everywhere.
- Light battle with `--battle-orders` (SP_AUDIO_LOG), median distance attenuation of the positioned sounds and engine loops started, before -> after:
  zoom 2000: 0.133 (-17.5 dB), 11 loops -> 0.261 (-11.7 dB), 11; 6064: 0.051 (-26 dB), 3 -> 0.265, 11; 16000: 0.020 (-34 dB), 0 -> 0.249, 11.
- Space maps have no ambience in the data (`SFXEventsAmbient.xml` is land weather only); units' own `SFXEvent_Ambient_Loop` already plays.
  Flybys: see below.

## Flybys: `SFXEvent_Ambient_Moving` (2026-10-17, A3)
- **Count** (`alamo-audio/examples/ambient_moving.rs`): 224 tags in the Remake's XML; 169 loaded objects name the event themselves and 746 have
  one after `Variant_Of_Existing_Type` (332 ground infantry, 199 space units, 99 ground vehicles, 54 unique units ...). 261 of them have a
  `Space_Model_Name`, all with samples: delays 3-5 s (188), 5-10 s (65), 1-1 s (5), 2-6 s and 1-2 s (1 each), none (the `Ravager`). The old
  note "5 objects" was wrong. The space events are whooshes of 1.5-6.9 s, most 3.3-6.9 s (Z-95 5.2 s, X-wing 5.3-6.8 s, TIE fighter
  6.5-6.9 s), all `Preset_EGB`: 3D, `Max_Instances` 2, saturation 60, volume 80-100.
- **Clock** (`audio::AmbientMoving`): a unit that moves (snapshot speed > 1 unit/s, estimate) and that the player sees waits one gap, then plays its
  event, then waits a new gap uniform in `SFXEvent_Ambient_Moving_Min/Max_Delay_Seconds` (default 3-5 s, estimate: the most common pair).
  splitmix64 seeded by the simulation id: every unit has its own fixed sequence. Stopping resets it. Looping events are skipped
  (`Unit_Shuttle_Moving_Engine_Loop` of the shuttles: repeating a loop would stack loops that never end).
- **Voices** (`audio::flybys_to_start`): every fighter asks for a flyby every few seconds and each event has 2 voices. With the plugin's rule (the
  oldest voice gives way) the first build cut every Z-95 flyby after a median 0.11 s of its 5.2 s (offscreen Yavin -> Hoth battle, 928 flybys
  in 60 s, 473 of them cut for a farther one; `fxaudio-run/chop.pl`). Now a due flyby waits for a free voice of its event, the loudest
  (nearest the listener) first, and takes a voice only when it is at least twice as loud as every one playing (6 dB, estimate); the plugin
  then cuts the oldest, which is quieter by that much. Requests count as playing for 0.5 s before their voice exists (a sample read on the worker).
  Same battle with the rule: 266 flybys start in 60 s instead of 1369 (`Unit_Headhunter_Fly_By` 64 instead of 930: Cutlass 49, Z-95 14,
  Hullcracker 1; ARC-170 63, T-wing 62, X4 46, LAAT 31) while up to 58 units wait; the started ones are near the listener (median distance
  704 against 1076 before; Z-95 loudness median 0.52); 8 took a voice, each at least 2.0 times as loud as every playing one. Offscreen
  runs have no audio device, so a voice ends after 2 s there: on a real device the 5 s whooshes free their voice later and fewer start.
- The sound sits on the unit (`PlaySfx::source`) and follows it; the distance to the battle listener sets the volume as for every 3D sound.
- Cost: 26 us per frame on average, 146 us at most (in-game timing in the battle above, release; 40 us before the event data was cached per unit).
- `SP_AUDIO_LOG`: one line per flyby (`audio ambient moving: ship <id> <type> plays <event> at t=..., loudness, voices taken, units waiting, next gap`).

## Fighter size and squadron spread (2026-10-17)
- **Size:** `ships::small_craft_boost` draws fighters, bombers and squadron members with their bounding sphere at 3.2 % of the screen height (`SMALL_CRAFT_SCREEN_FRACTION`) at every camera distance up to 7277 (the Remake's widest view at our fov, `SMALL_CRAFT_FREEZE_DISTANCE`); farther out the enlargement holds and they shrink with the world; never below true size. One factor per squadron, from its centre, so members match. Measured against the user's video: TIEs 23-29 x 23-27 px (wings included) at 1080p in b0703s, b0732s, b0850s and still 22-25 px in b0760s; ours 18-20 x 24-26 px at camera 2000 (bounding sphere 35 px). Replaces `fighter_boost` (2.6 %, cap 40) of the FX pass 2 above.
- **Spacing:** squadron members fly 100 units apart (battle-sim `SQUADRON_GAP`), about 47 px at camera 2000: the Remake shows 36-51 px in b0703s. Squadrons carry the icon box only while selected or hovered.

## Lively flight (2026-10-17)
- `flight.rs`, view only: battle-sim is unchanged. Bank and pitch advance once per simulation step from the snapshots (frame-rate
  independent), the weave reads the battle clock and the ship id; nothing is random. A paused battle is a frozen picture.
- **Data (Remake XML):** `Bank_Turn_Angle` (TIE fighter and X-wing 360, which reads as no limit; `Spacecraft_Base` 90; Star Destroyer and
  Venator 20), `Max_Rate_Of_Roll` (TIE 2.56, X-wing 2.5, Star Destroyer 0.0375, Venator 0.05 rad/s), `Max_Lift` (TIE 2.8 of `Max_Speed` 4).
  No space model of the Remake has a turn or bank clip (TURNL/TURNR clips belong to ground units; its TIE fighter model has no clips, its
  X-wing only deploy and undeploy), so the original banks in code.
- **Measured** (YouTube wkiF9UTI3xo at 10-15 frames/s): TIE fighters bank up to about 35 degrees and X-wings 45-60 in a dogfight (15.3 s);
  a Venator rolls 6-8 degrees in a turn at its full rate and is level within 1.5 degrees after it (513-527 s); squadrons on the move keep
  rigid rows (23 tracked TIEs move under 0.8 px RMS against their row neighbours at 20-27 px spacing, 532 s). User video b0703s, b0727s:
  idle squadrons are loose clusters, neighbours 36-51 px apart.
- **Bank:** into the turn, `max_bank` x turn-rate fraction x speed fraction, critically damped and at most `Max_Rate_Of_Roll` fast. Small
  craft (fighters, bombers, squadron members): `Bank_Turn_Angle` up to 60 degrees (estimate: the video's steepest). Bigger ships: 35 % of
  `Bank_Turn_Angle` (estimate from the Venator: 7 degrees for the Star Destroyer and the Venator). Structures do not bank. The old code
  banked small craft and corvettes out of the turn (the right wing went down in a left turn), frigates and capital ships not at all.
- **Pitch:** the flight-path angle of the sim's climb or dive (speed taken as at least half the top speed), small craft up to
  `atan(Max_Lift / Max_Speed)` (inferred: TIE 35 degrees) or 30 degrees without `Max_Lift` (estimate), bigger ships 4 degrees (estimate);
  the pitch changes at most at the turn rate (estimate).
- **Weave (small craft):** phase and period per craft from its id: drift 6 to the side, 5 up and down, 2 fore and aft (world units), periods
  4.2 / 3.4 / 5.6 s x 0.8-1.2, nose +-10, bank +-14, pitch +-7 degrees (all estimates). At top speed 20 % of the drift and 35 % of the attitude
  stay (rows stay rigid on the move); it grows in over 3 s after a craft appears (it leaves the hangar on its exact path); the drift fades
  from camera distance 1800 to 2750 to the squadron, where neighbours would come too close on screen. `ShipRender::pos` is the drawn position,
  so selection rings, bars, flags, picking and turret aim follow the drawn craft.
- **Proof** (`battle_demo`, Byss, camera 2000): the four resting squadrons nearest the camera (2000-2480 units), 17 frames each, rendered
  alone (`SP_DEMO_ONLY_SQUAD`) against a render with every squadron hidden (`SP_DEMO_HIDE_SMALL`): no two silhouettes touch; closest gaps
  12 / 4 / 7 / 4 px (before, the rigid grid: 16 / 7 / 10 / 7), median gap to the nearest neighbour 17 / 9 / 13 / 8 px (before 18 / 8 / 12 /
  8). Every turning ship of the battle at 49-54 s (11 frames; the framed TIE squadron turns): before 13 small craft banked into the turn and
  184 out of it (steepest 32 degrees), after 197 into and 36 out (turn reversals while the roll catches up, and the sway in gentle turns;
  steepest 57), bigger ships 0 / 0 before and 43 into, 0 out after (steepest 5.6). Bigger ships on one heading for 10 s (8-16 s): 0 of 72
  samples lean.
  The app's `--battle-orders` shot (camera 2000) shows the selection, the bars, flags, order lines and attack markers as before.
- **Cost:** 27 us per frame for 292 ships (93 ns each, release, every ship weaving); `battle_demo` 140 s at 1080p (292 ships), six
  alternating runs each: mean frame 7.71 ms before and 7.74 after (run to run 7.12-8.37), p99 14.9 both; `SP_FRAME_LOG=20` main-thread
  spikes inside the battle, runs 3-6: before 3 / 8 / 0 / 0, after 0 / 22 / 0 / 0 (run 4 was busy for both, at 110-137 s with the render
  thread at 17-27 ms in the same frames).
- `battle_demo` switches: `SP_DEMO_IDLE=<side>` (that side has no AI: its squadrons rest where they arrive), `SP_DEMO_SQUAD=centre` (with
  `SP_DEMO_DOGFIGHT`: frames the middle of the first squadron), `SP_DEMO_FLIGHT=1` (probe lines `spacing`, `members`, `lean` at every capture),
  `SP_DEMO_HIDE_SMALL=1` and `SP_DEMO_ONLY_SQUAD=<n>` (silhouette renders).
  Check: `SP_DEMO_FLIGHT=1 SP_DEMO_IDLE=0 SP_DEMO_DOGFIGHT=1 SP_DEMO_SQUAD=centre SP_DEMO_DIST=2000 battle_demo --offscreen out.png --map
  _SPACE_PLANET_BYSS_01 --size 1920x1080 --times 8,9,10,11,12` (a resting squadron); without `SP_DEMO_IDLE` and with `--times 49.5,50.5,51.5,52.5`
  a squadron turning.

## One bar and flag per craft (2026-10-17)
- **The Remake** (user's 1080p video battle1s b0703s / b0732s / b0850s / b0760s / b0699s, screenshot 07_tactical_battle): every fighter and
  bomber, own and enemy, has its own health bar under it and a flag at the right end of the bar, at every zoom (bars and flags keep their screen
  size like the fighters do, b0760s); a selected craft also gets a white box of its own (b0699s). Art from `MT_CommandBar`: bar =
  `I_BAR_CONTROL_WIDE` (black track, alpha 0.31) + `I_BAR_HEALTH_SMALL<tenths>` fill (flat colours, fill as wide as the health: b0866s shows the
  dark track right of an 80 % fill; level 10 over the track reproduces the screenshot's pixel (55,210,65) exactly), flag = `I_GARRISON_FLAG`
  (12 x 14, pole 0xf6 / 0xa3, pennant 0xd8: the screenshot's flag pixels).
- **Measured**, Remake 1080p video | Remake 2047 px screenshot | ours at 1080p: bar 29 px (27-31) x 2 | 55 x 3 (= 29 at 1080p) | 29-31 x 2 (the craft's
  drawn sphere x 0.84, so 22-41 at camera 930 where a craft's true size beats the 3.2 % boost); flag pole 14-16 px | 30 px (= 15.8) | 16; bar top
  under the craft's silhouette 1.1 px | 2 px (= 1.1) | 1 px (median of 21 TIE fighters on a lit Bestine render, one outlier 2); bar centred on the
  craft within 2 px | 2 px | -4 to +2.5 px.
- **Chevron:** not every TIE carries a flag. 3 of 12 per squadron (b0703s, b0732s, b0850s, b0699s, which also shows 3 chevron cards in the selection
  tray; 07 too) show a small green rank chevron above the right wing instead (`I_HIGHLIGHT_RANK_*`-like art). The data has no field for who gets
  it (`TIE_Fighter` and the garrison variant `TIE_Fighter_G` differ only in text and `Destruction_Survivors`), so it is not drawn: every craft gets the flag.
- **Ours:** `tactical_ui::sync_member_bars` -> `eaw_ui::MemberBars` (one `MemberBar` per visible fighter / bomber / squadron member, sorted by id: its
  own hull and, when its definition has shields, its shield; off-screen craft skipped) -> `eaw-ui/src/member_bars.rs` (see its STATUS).
  `ShipRender::drawn_radius` is the bounding sphere as drawn, small-craft enlargement included, 0 until the model is built and while the craft
  streams in from hyperspace. Dead craft leave `RenderedShips` and fogged ones are `!visible`: no bar. Squadrons have no overlay of their own while
  unpicked; a picked or hovered one keeps its icon box (`ShipOverlay::icon_only`, no bars) and its craft keep theirs. Capital ships, frigates and
  structures are unchanged. `remove_bar` clears the bars with the battle.
- **Cost** (`SP_FRAME_LOG=0`, frame intervals from the log's timestamps, offscreen 1080p, battle 8-28 s, the machine busy with other builds, base
  803eec2): big battle (24 X-wing + 12 Y-wing + 12 A-wing squadrons granted to the player, 533 ships, camera 6064): before mean 19.0 / 19.1 ms
  (p50 16.8 / 16.9), after 17.6 / 16.3 (p50 15.4 / 14.4); process CPU per frame 31.9-35.0 ms before, 31.4-32.2 after (an earlier, quieter set of
  runs: 15.3-16.4 before, 14.1-14.9 after; one further after run, 127 ms frame interval, was disturbed by another process and is left out). Ordinary battle (245 ships, camera 2000): 11.7 / 12.7 before, 11.1 / 12.8 after (noise). The old squadron
  overlays were about 19 UI nodes each; the bars cost `member_quads` 8.8 us per frame for 500 craft. The first version put every rectangle into the
  UI batcher as an item of its own: +3-4 ms per frame and +5 ms CPU for about 2000 rectangles, hence two runs of glyphs now.
  Big battle: `eaw-remake --offscreen x.png --campaign GC_Full_Web_Way_Rebel --select "The Yavin System" --grant-unit X-Wing_Squadron,...
  --at "The Yavin System" --weeks 8 --move Hoth --battle-choice fight --size 1920x1080` with `SP_BATTLE_CAM=6064 SP_FRAME_LOG=0`.
- Not done: the selection box per craft (b0699s), the chevron, the lost-health track on capital-ship bars (`eaw_ui::ship_overlays` draws the level
  colour over the full length), an automated test of the render-world extraction (the offscreen renders cover it).

## Mouse pointer from the data (2026-10-17, cursorcam C1)
- `cursor.rs` replaces the order cursor of a7e667e (a command bar button picture with its background drawn next to the pointer): the pointer itself is the window's cursor (`CursorIcon::Custom`, bevy feature `custom_cursor`, which also adds five unused packages to `Cargo.lock`), set only while something special is going on; otherwise the system arrow stays. Pointers come from `XML/MousePointers.xml` (mod over game): first texture, `Hot_X` / `Hot_Y`, `Anim_Frame_Delay`; the frames are the textures with the following numbers (`ATTACKPOINTER_00..07`, 32 x 32 loose `.dds` of the Remake).
- Which pointer when (the data's entry names, the engine's rule is inferred from them): own mobile units selected and `Hover` on an enemy ship, structure or hardpoint -> `POINTER_ATTACK` (red crosshair, hotspot 16,16); attack button armed -> `POINTER_ATTACK_ONLY_MODE_UNIT_TARGETED` over an enemy and `..._NO_UNIT_TARGETED` (the greyed crosshair) elsewhere; attack move -> `POINTER_ATTACK_MOVE` (red arrow); move -> `POINTER_MOVE_ONLY_MODE_PASSABLE_TARGETED` (green arrow); targeted ability -> `POINTER_TARGET_SPECIAL_ABILITY_TO_SPACE_POSITION` or `..._INVALID` (`AbilityAim`, written by `ability_at`); reinforcement placement keeps the arrow. Over the interface, outside the window and while a selection box is dragged (`Drag::box_dragging`): the arrow.
- The data has no extra out-of-range crosshair for plain attacks: `POINTER_ATTACK_OUT_OF_RANGE` is `ATTACKPOINTER_00` in the base game and in the Remake, so there is no range logic. Open ground keeps the normal arrow (the user's rule: moves are right clicks with the normal pointer); `DEFAULTPOINTER_00` (the data's white arrow) is not used.
- Animation: one `Anim_Frame_Delay` step is 1/60 s (`DELAY_STEP_SECONDS`, an estimate: the data counts engine frames), real time, restarting when a pointer comes up. The Remake raised the speed (delay 6, 8 pictures; the base game has 10).
- The window is written only when the pointer or its picture changes (never once per frame), and the pictures are loaded once and kept: winit caches one OS cursor per image.
- `SP_CURSOR_LOG=1` logs every change (`cursor: system arrow -> POINTER_ATTACK at Some(..)`), also offscreen (no window to set there; the choice is still made and kept in `ShownCursor`).
- Tests: `cargo test -p battle-view cursor` (table, frames, animation, the real `selection` hover over 120 frames across a hull and a wing turret with the crosshair changing exactly at the boundaries and the window written only then, right-click orders with and without the crosshair, armed orders, interface / box drag). A windowed check of the real OS cursor was not run (the user plays on the machine): see the report of C1 for the rows to run.
