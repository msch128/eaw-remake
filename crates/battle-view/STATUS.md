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
- Alamo shader uses only the first directional light: map fill lights are stored, not rendered
  (needs alamo-render shader change).
- No Death_Clone animation; death particle files run ~30 s.
- CPU-bound: ~5 ms (Coruscant) to ~9 ms (Kuat, 41 heavy props) per frame at 4K release, measured while
  another game ran. Props are not merged or culled by distance.
- Demo `--auto` camera zooms to max distance in the new large arena; `PROPS`/`NOPROPS` env switches
  in the demo are debug leftovers.
- No move-destination lines, sounds or muzzle flashes. Pointer/box select untested with a real mouse.
- Impact particles are one entity each (alamo-render `AutoDespawn`, default 10 s); capped by
  `max_effects`.

## Next steps
1. Test real-mouse selection/orders in a window; tune pick radii and `ui_scale` for 4K.
2. Merge or LOD static props; check Bevy extraction cost (transform propagation, batching).
3. Death-clone models or shorter death effects; engine glow tied to speed.
4. Shield bubble visibility from sim state; hull-damage smoke below 50% health.
5. Integrate with the GC launch path (build `BattleSetup` from fleets, `Arena::load`).

## Open questions
- Should fill lights be supported in the Alamo shader?
- Is the huge Kuat prop clutter (Executor etc. under the plane) wanted in battles?
