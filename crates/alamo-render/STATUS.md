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
- Lit and planet paths now saturate the summed light (ambient + sun + fills) per channel before the albedo, like EaW's LDR renderer. On the Kuat
  map the renders are visually unchanged: the hull is bright because of its albedo, not because of overexposed light. The hull "speckle" in battle
  renders is the hull texture's own grain (finer at 4K); no bug found (near plane, MSAA, bloom, spec, normal map, stacked ships all ruled out).
- `battle_demo` frame-time stats now skip the frames after a screenshot (the 4K readback stall was the old 187 ms max). 4K, ~290 ships: mean 7.2 ms,
  p99 9.5 ms, max 57 ms (one remaining hitch, not located: probably a first-use asset/effect load mid-battle).
- Toksvig normal-map filtering (`toksvig_variance`, bump flattened by the filtered normal length on planets) added next to the screen-space specular AA.
  On the Kuat low-orbit view it changes nothing visible: the terrain normal map is magnified there (filtered length ~1), so the bright specks are the
  texture's real detail, not aliasing. A distance-based bump fade was not added (no EaW reference for the scale).
- Acclamator engine glow is not missing: `pe_acclamatorengines`/`_sml` render as cyan particle puffs behind the stern (seen from behind with `render_scene EV_ACCLAMATOR.ALO --cam 20,-10,1.8`); the earlier views showed the bow side.
- Additive particles now multiply by the texture alpha (`particle.wgsl`, modes 1 and 2). `P_PARTICLE_LIGHT.dds` (10x10 atlas, used by the Acclamator/ISD
  engine emitters and explosion sparks) stores hard-edged solid discs in RGB and the soft falloff only in alpha; ignoring alpha drew hard discs.
  Assumes EaW additive = SRCALPHA/ONE (not confirmed from engine data). Effects get somewhat dimmer; before/after: `target/vis/pb_*.png` vs `pa_*.png`.
- Particle decoding (corpus surveys: `alamo-model` examples `particle_tracks`, `particle_shapes`, `particle_frames`, `particle_one`):
  float tracks (size, frame, spin) carry keys like the color tracks (`f32 value, f32 time`; 2,000+ emitters with size keys, e.g. glowcenter 2->500 reaching 500 at 20% of life);
  they were dropped before, so sizes interpolated start->end linearly. Property 0x09 `speed` is a radial speed away from the emitter origin
  (negative sucks inward: `inwardsucking` -350, explosion fire +135) and was ignored. Frame end values 0 and 2 are editor defaults (7,300 of 8,200); ascending
  ends > 2 (16, 49 = sheet size, `explosion_frames_medium1` grid 4 frames 0->16) are flipbooks over the life and now animate.
  Still unknown: props 0x0b (-20..-100; drag or gravity), 0x12/0x13 (fractions), 0x17, 0x3c/0x3e/0x3f/0x40/0x42, shape kinds/params (positions are boxes, radial
  spawn shapes ignored), the exact additive blend factor. The capital explosion still lacks the dense central fireball (dim glow emitters, sparks dominate).
- Web search for public format docs (Petroglyph/UaW particle editor readme, ModDB, swrebellion mirror of the UaW mod tools readme, Blender-ALAMO-Plugin
  and AlamoEngine-Tools repositories) found only a high-level description (emitters + per-property plugins); no property ids, shapes or blend factors. Nothing was copied.
- Added from corpus evidence: property 0x0b as gravity along the up axis (`Emitter::gravity`, mostly 0, otherwise -0.3..-145); position kinds 3 (ellipsoid volume) and
  4 (ellipsoid surface) with semi-axes in shape params 6..8 (`debre` 32.4 vs 32.67 box extent) instead of filling the bounding cube.
  Spawn model (`max` live particles refilled every `interval`) is unchanged; it matches the data well enough. Blend factors are still unknown.
- Capital explosion (`target/vis/ex7_t*.png`): now an expanding spray of sparks with a small orange cloud in the middle, still not the dense fireball. Turbolaser
  impact (`p_damage_turbolaser_miss_large_green`, `ps_turbolaser`): grey smoke puffs only; its flash is made of type-4 light emitters that are not drawn.
  ISD engines from the stern (`render_scene EMPIRE_IMPERIAL_STAR_DESTROYER.ALO --bow --cam 0,0,1.6 --throttle 1`): nozzle cores glow blue, the 47-unit halo particles are hardly visible.
- Type-4 emitters (1,162, `particle_lights` survey): 835 are `light_*` on P_PARTICLE_LIGHT, but 190 are `smoke`/`fire`/`explosion`/`muzzle`/`projectile` on explosion sprite
  sheets, so type 4 is a sprite blend variant, not only dynamic lights. They are now drawn as additive sprites scaled by their alpha (SRCALPHA/ONE, mode 1) so the dim
  `light_*` glows stay subtle; emitters larger than 3000 units (`MAX_CORONA_SIZE`, about 235 of them) stay undrawn (they washed the screen out). `showcase death` (ISD death,
  `target/vis/death_t01000.png`) shows dense fireballs and smoke with no wash; `P_Explosion_Capital` has no type-4 emitters and is simply not the effect ship deaths use.
- Blender-ALAMO-Plugin and AlamoEngine-Tools/PetroglyphTools are MIT licensed but contain no particle format code (checked on their GitHub front pages only).
- Particle batches no longer rebuild a `Mesh` asset every frame. Each (texture, blend) batch has a fixed draw mesh of `cap_idx` dummy vertices (rebuilt only when the capacity doubles) and two storage buffers
  (`ParticleMaterial::indices` with a live-count header, `::verts` with 3 `vec4` per vertex: world position, uv, color) that are rewritten in place (`ShaderBuffer` with unchanged size -> `write_buffer`, no mesh allocator work).
  `particle.wgsl` has a vertex stage that reads them (indices past the live count give degenerate triangles). Gotcha: a non-indexed draw's `vertex_index` includes the mesh's first vertex in the shared mesh slab,
  so the shader subtracts `mesh[instance_index].first_vertex_index`. Bolts keep their arbitrary indices. Measured with the Chrome trace: `allocate_and_free_meshes` 1123 us -> 4 us per frame.
  Visual check: death-explosion scene mean abs diff 0.2/255 (0.13% of sampled pixels > 24), bolts scene 0.007, capital explosion 0.2 (`target/vis/death_before_t01000.png` vs `death_after5_t01000.png`).
