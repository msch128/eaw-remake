# alamo-render status

Bevy 0.19.1 plugin that spawns original EaW models with an EaW-style look. Depends on `alamo-fs`, `alamo-model`.

## What works
- `AlamoRenderPlugin::new(Arc<GameFs>)`; `AlamoModel { path, team_color, lod, bow_forward, unit, attached }`
  spawns bones, rigid + skinned meshes (bind pose + animation), proxies. Shared mesh/material/texture caches
  (500 TIEs = 18 draw calls with 6 teams). DDS stays compressed (BC1/2/3); TGA/PNG decoded + mips.
- One custom material (`alamo.wgsl`, reconstructed, no HLSL source exists): bump/gloss/colorize, planet
  (clouds, atmosphere, city lights), additive, alpha, shield (local hit flashes), nebula, emissive*base.
  Gamma-space shading; use `Tonemapping::None`. Sun = first `DirectionalLight` (`spawn_sun`, `spawn_sun_shadowed`).
- Particles: CPU sim, one dynamic mesh per texture/blend (`AlamoParticles`, auto-attached to proxies;
  `pe_`/`pte_` start on, others wait for `enabled`). Projectile bolts batched (`ProjectileVisual`), impact
  events (`ProjectileImpact`, `SpawnParticleEffect`, `ShieldHit`), XML defs in `XmlDefs`.
- Animations (`AlamoAnimation`, `<MODEL>_<KIND>_<NN>.ALA` lookup), `TurretAim` (limits/speed from hardpoint XML),
  hardpoints (`with_unit`, `AimTurrets`, `HardpointDestroyed`), `EngineThrottle`, `HyperspaceArrival`,
  `PlayDeath` (death clone + chained explosions), `LodSettings` (distance culling).

## Run (offscreen only, never a window)
`$env:CARGO_TARGET_DIR="...\target\render"; cargo run -p alamo-render --example <name> -- ...`
`render_scene <model|-> <out.png> [--team --sun --cam yaw,pitch,dist --fx --times --many N --teams N --shadows --nolod --throttle t --hdr]`,
`battle <out.png> [--units --times ...]` (ISD vs MC80 fight), `showcase <idle|xwing|turret|hyper|death> <out.png>`,
`xml_grep <text>`, `tex_dump`, `tangent_check`, `fire_scale`, `bone_scales`. Needs a GPU; sim time is held ~3 s for shader warm-up.

## Gaps / next steps
- Shaders are approximations (shield, additive brightness, cloud shading); no fog, reflection maps, stencil shadow volumes.
- Particle scale/semantics unverified vs original; missile smoke does not trail; custom-render projectiles undrawn.
- No animation blending; light emitters (type 4) not used as lights; hardpoints without models are bare bones.
- Asset loading is synchronous (one frame stall on big models); skinned meshes not batched across instances.
- Run `cargo clippy -p alamo-render --all-targets`; tests: `cargo test -p alamo-render`.

## Fill lights
`alamo.wgsl` adds the diffuse term of every directional light after the first (up to 2 fills, no shadows/specular) via `fill_light(n)`; the sun stays
the first directional light. `battle-view` spawns the map's fill lights from the `.ted` lights 2 and 3. Not yet checked against the real game look.

## Visual fidelity pass (Oct 2026)
- Added specular antialiasing (`filtered_shininess` in `alamo.wgsl`, screen-space normal variance) to the lit and planet paths; renders of ISD, MC80,
  Acclamator, X-wing and TIE are otherwise unchanged (compared before/after offscreen, no regression, no visible change on ships).
- `render_scene --offset x,y,z --look x,y,z` places the model far from the origin: an ISD at (9000, 0, -9000) renders clean, so f32 precision of the
  world transform is not the cause of the hull speckle seen in `battle-view` battle renders (black/white dots on the ISD hull at distance).
  The same ISD is clean in `render_scene`/`battle` (also with `--shadows`), so the speckle comes from the battle-view setup (camera, scale, lights),
  not from the Alamo shader. Not yet identified.
- Not done: 4K frame-time comparison, sun glare/lens flare, engine-glow tuning (the X-wing glow is present, the Acclamator shows none without bloom).
