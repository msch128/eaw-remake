# battle-view status

Bevy plugin that renders and drives a `battle-sim` space battle with the EaW look and controls.

## What works
- Scene from a space `.ted` (`SpaceScene`): skydome, backdrop planet, props, sun + ambient, bounds.
- Sim on a worker thread (`sim.rs`), one snapshot per step; render interpolates (smooth at any FPS).
  `blocking: true` makes offscreen runs reproducible.
- Ships as `AlamoModel` (team colour, bank into turns), hardpoint death (hide meshes, damage fires,
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
  retreat countdown/ready, population) and turns clicks into commands (`b_retreat` -> `Command::Retreat`, ability button ->
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
