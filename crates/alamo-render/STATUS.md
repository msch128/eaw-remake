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
- Shaders are approximations (additive brightness, cloud shading; the shield bubble flash is ours, the visible shield-shader glow shells are the decoded `MeshShield` technique); no fog, reflection maps, stencil shadow volumes.
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
- `P_Planet_Explode_Galactic` showed solid orange squares in an older galaxy render (integrator `sw4.png`: every particle quad a flat square, i.e. the sprite texture sampled as white). Current builds render it
  textured (round shockwave, smoke clouds, discs; checked in `render_scene` and in `space-peace --offscreen ... --grant-unit Death_Star --at Arda --fire confirm`), and the textures load (logged `texture loaded = true`).
  Most likely cause: Bevy binds its white fallback image until the sprite texture / storage buffers are prepared on the GPU, so the first frames of a new batch were solid squares. New batches now stay hidden for
  `BATCH_WARMUP_FRAMES` (3) frames. I could not catch the very first frames offscreen (the burst is already advanced at the earliest frame I could capture), so the warm-up is a mitigation, not a verified fix.
- Asynchronous preloading (`preload.rs`): `ModelPreloader::request(fs, cache, paths)` reads, parses and texture-decodes models on up to 4 worker threads; `integrate_preloads` (PostUpdate, before `spawn_models`)
  moves results into `ModelCache`/`TextureCache` within a 3 ms/frame budget; `load_model` then skips read+parse and the texture decode. `is_idle()` tells when everything requested is cached. `load_model` logs
  a breakdown (read / parse / textures / rest) for models that take >= 20 ms. Release numbers for the 13 distinct models of the demo battle before: Imperial_SD 88 ms, Executor 208 ms (read 58, parse 80, textures 51,
  rest 19), Titan 66, Allegiance 56, MC80 45, ...; textures were ~half of it, read+parse ~35 %.
- Preload covers everything a battle loads later (2026-10-17, reinforcement stutter): a model job also reads and parses the particle systems its proxies name (absent files are recorded, so
  `attach_proxy_particles` never touches the disk) and decodes their sprites; `request_particles` keeps the parsed effect systems (not only their textures) and builds the bolt geometry of every projectile
  with its texture (`w_turbolaser.dds` took 12.5 ms on the first shot); `request_animations` parses `.ala` clips. Results go to `ParticleCache`, `BoltCache` and `AnimationCache`. A model reference that is a
  particle system (`pp_plasma_bomb`) is preloaded as one. `frame_trace` (`SP_FRAME_LOG`) notes every load that still happens on the main thread.
- Replaced the 3-frame batch warm-up by a real readiness rule. Bevy's `AsBindGroup` derive returns `RetryNextUpdate` while a `Handle<Image>` or `Handle<ShaderBuffer>` has no prepared GPU asset
  (bevy_render_macros `as_bind_group.rs`), so a particle material, and with it the draw, simply waits until the sprite texture and both storage buffers are on the GPU; there was never a white-fallback frame for a
  texture that is merely not uploaded yet. The white fallback only happens for a `None` handle (texture missing), so a batch whose texture failed to load is now never drawn (`Batch::texture_ok`; the loader already
  warns once). The squares in the old `sw4.png` predate the storage-buffer change; current builds render the planet explosion textured.

## Byss look pass (Oct 2026)
- Atmosphere haze was missing in battles because the opaque sky dome (radius 90000, centred on the camera) occluded the planet's `Atmosphere_Ring` mesh and the far part of the backdrop disc (backdrop planets are flat discs of radius ~72000 plus an additive ring
  that flares up at the rim). Sky/nebula models are now `AlamoModel::as_background()`: `FLAG_BACKGROUND` -> pipeline def `ALAMO_BACKGROUND` writes `frag_depth = 0` (reverse-Z far) and no depth write, so the sky can never occlude scene geometry.
  The ring gets `ATMOSPHERE_RING_GAIN` (2.2) in `model.rs`.
- Lit hulls: `LIT_GAIN` (1.4, `alamo.wgsl`) overbrightens `albedo * light` (clamped to 1) because the sunlit Remake hulls are near white while the hull albedo is ~0.55 grey. 1.6 matched the video better on Byss but blew out Kuat.
- Montages: `target/vis/final_hull_haze.png`, `haze_cmp.png`, `gain_kuat_kash.png`. Combat effects (death explosion, bolts) were compared, not changed: fire and dark smoke match, the video has more white smoke and hotter cores.
- Combat pass (Oct 2026): additive particle sprites (bolts, fire, flashes, sparks) now use `ADDITIVE_GAMMA` 1.5 (`particle.wgsl`, estimate) instead of the full 2.2 curve: the original engine blends in gamma space, our framebuffer is linear, so additive effects were far too dim (bolts, hot cores). Default battle start distance is 3500 (`BattleViewSettings::start_distance`, estimate from the Byss video). Still different: our damage/death smoke (`smoke_red-black`, `bigsmoke`, alpha, data colours) forms a much larger, more opaque black cloud than the video's compact dark patch, and the video's big white plume is not reproduced. Montages: `target/vis/fx_bolts_cmp.png`, `fx_expl_cmp.png`.
- Ship death sequence (Oct 2026, data-driven): a capital's `Death_Clone` unit (`Death_Clone_Star_Destroyer` -> `Empire_Imperial_SD_DeathClone.ALO`) is played with its `*_DeathClone_DIE_00.ala` (ISD: 1500 frames at 30 fps, about 50 s). The clone carries ~55 `pd_*` explosion proxy bones (`pd_proton_rocket` from 0.7 s, `pd_magpulse_missile*`, `pd_assault_missile*`, `pd_medium_2/3_*`, finale `pd_medium_3_750` at 49.3 s) and `P_Ship_Fire*` flame bones; the clip's visibility track switches each on at its key frame and hides the hull at 49.7 s. `sequences.rs` now only enables the proxies' particles (they spawn disabled) and lets the animation gate them; the invented 0.2-0.9 s explosion cadence and finale are gone (the clone stays 30 s past the clip for the last cloud). The ISD/MC80 units have no `Death_Explosions`; `Death_Clone_Is_Obstacle` appears only on ground walkers (AT-AT), not on space units. The clone is a free render entity (no `ShipView`, not in the sim), so it is not selectable/targetable/counted and nothing waits for it. Ships without a clone (`update_dying`, `battle-view/src/ships.rs`) play their effect once per 1.5 s (estimate) instead of stacking a `Large_Explosion_Space` every 0.12-0.6 s. Result: the first 6 s are sparse bursts on an intact hull, clouds grow to about the hull size by 10 s (pieces drift out of a fixed camera, the showcase needs `--cam 0.5,0.3,14+` after 10 s); the 50 s climax explosion coincides with the hull vanishing. `showcase death|death_mc`; montages `target/vis/death_clone_cmp.png`, `death_climax_cmp.png`. Not rendered: clone-less frigate/corvette and fighter deaths (code paths only).

## Particle group fix (Oct 2026)
- Cause of the "explosion clouds ~2.3x too large vs the video": the emitter property groups were mis-assigned (see `alamo-model/STATUS.md`, source https://modtools.petrolution.net/docs/ParticleFileFormat). The old "position" box (about +-181 for `pd_medium_3_*`) was the speed group; the real spawn volume is the third group (sphere radius 90 for smoke, 150 for fire); the "velocity" +Y of 13..19 u/s was the lifetime group. `spawn_particle` now samples position from `position_group`, initial velocity from `speed_group` (`sample_group`: exact/box/cube/sphere/cylinder), life as `lifetime*(1-randomizedLifetime*u)` and a per-particle size factor `1-randomizedScale*u` (the last formula is inferred, the doc does not give it). No global scale factor was introduced; size tracks are unchanged.
- Result (ISD death, `showcase death`, `target/vis/fx_group_cmp.png`: video, then before/after at 1 s, 3 s, 6 s): the cloud is about 0.7-0.8x as wide as before and the grey early puffs are smaller, but fire and smoke still span about 1.5x the hull length at 3 s. The "needed ~0.43" is therefore only partly explained; the rest is probably battle-view explosion cadence (a new `Large_Explosion_Space` every 0.12-0.6 s while dying, `update_dying`) and the unknown exact randomizedScale formula, not verified. No-regression (identical by eye): `fx_noreg_engine_battle.png` (ISD engines, bolts/shields battle), `fx_noreg_planet.png` (Coruscant + atmosphere particles; planet-explosion and atmosphere particles are barely visible in both). Kuat/Coruscant full battle maps could not be run: `battle-view` did not compile (another agent mid-edit in `audio.rs`).

## Map distance fog (Oct 2026)
- Brentaal (`_SPACE_PLANET_GMOUNTAIN`) check: the skydome (`W_Skydome_Ground_Sunset_Lower_Golden`) renders and shows above the horizon at tilts below ~30 deg; at the default 72 deg pitch the terrain prop
  (`W_Land_Terrain_GrassyMountainsHuge`, y -2500, peaks at -771) fills the whole screen, so no sky is visible there by geometry. The map's fog chunk (orange `0.47,0.2,0`, linear 0..40000) was dropped; the SpaceProps
  XML notes terrain props interact with fog. Now `SpaceScene::fog` -> camera `DistanceFog` (linear, colour `pow(c,2.2)`) and `alamo.wgsl::apply_map_fog` (lit/planet/alpha kinds only, `#ifdef DISTANCE_FOG`; sky/additive unfogged).
  Maps with fog off (Kuat, Byss) are unchanged. Renders: `target/vis/br_before_t*.png` vs `br_after_t*.png`.
- Ground-terrain maps: GMOUNTAIN (+_NIGHT_NEB), GVULCANO, POLUS_01, SKIES_ISLAND, SULLUST_01. SKIES_ISLAND is NOT broken: `W_Land_Terrain_GTAV` is a huge ocean disc with a small island at the map origin (`battle_demo --map _SPACE_PLANET_SKIES_ISLAND --cam 0,25,2500,0,0` shows it); the fleet spawns far from the island, so the default view shows only water. GVULCANO is dark on purpose (its sun points up, lava light from below). Renders: `target/vis/si_*.png`, `t_*.png`. Minimap on ground maps: the tint is now the fog/light colour (no backdrop planet blob there) and terrain props no longer draw a grey dot (`battle-view/src/lib.rs`, `is_ground_model`); not rendered (the bar needs the full app).

## Underlay glows (Oct 2026)
- `FLAG_UNDERLAY` (additive kind only): the glow draws in Bevy's alpha-mask phase instead of the transparent one, still adding (blend One/One, destination alpha kept) and without depth writes. That phase comes after the opaque one and before the transmissive one, so transmissive materials find the glow in the view transmission texture. The galaxy map sets it on the disc art's `core` and `region_lines` parts, so the lanes can add over them in display values (space-peace `LaneMaterial`, G1b). Drawn colour unchanged (galaxy core samples 97.1/62.2/60.8 before, 97.2/62.2/60.8 after). Only for glows that lie under everything else: an alpha-mask item behind an underlay glow and drawn after it would cover it (none on the galaxy map). Test: `material::tests::an_underlay_glow_draws_in_the_mask_phase_and_nothing_else_moves`.

## Particle axes (2026-10-17, A2b)
- Particle space is an Alamo frame (Z-up), not Y-up: on a model proxy the emitter uses the bone's own axes (`particle_frame(true)` is the
  identity), a standalone system (`SpawnParticleEffect`, `--fx`) gets the Z-up to Y-up turn, so its particle `+Z` is the effect's up / hit normal.
  The old `(x, y, z) -> (x, -z, y)` mapping came from the time the lifetime group (always along Y) was read as a velocity.
- Evidence (`alamo-model` examples `particle_axes`, `particle_dir_hist`, `particle_frame_change`): 1139 of 1157 engine proxies blow out of the
  stern with the bone frame (1 with the old one); damage fires stream away from the hull axis in 1645 of 2974 proxies only with the bone frame, 175 only with the old one;
  directed emitters point along Z (844 speeds and 817 offsets on +-Z) far more than along Y (74 / 21). The Death Star's nine charge emitters put their
  focus points (5900 units out) within 33-84 units of the dish proxy; the Eclipse's `P_Eclipse_Charge_L/_S` foci land on the centre-line trench
  (y -2999 / -550, hull height) instead of 1000-6100 units above or below the hull.
- Gravity (property 0x0b) and the cylinder height now run along `Z`, so their world direction is unchanged. Invariant by construction: spheres,
  cubes, zero offsets; all 77 emitters of `pd_medium_3_300/750/1250`, the fighter and corvette death effects and `P_Planet_Explode_Galactic`.
  What visibly moves: engine exhausts (now behind the stern; they streamed up or down by the bone's Y axis), damage fires, shield-hit sparks
  (along the hit normal), the Eclipse charge and the Death Star lattice and dish glow (which streams along the beam instead of hanging below the dish).
- Proof: before/after sheets `a2b-eclipse-charge.png`, `a2b-deathstar.png`, `a2b-regression.png` (explosion, hyperspace arrival, ISD death clone,
  hardpoint fires, engines; outside the repo). Explosion `pd_medium_3_750` centroid moves 24-40 px between before/after with the same seed, a new
  random seed alone moves it 100-190 px. Tests: `particles::tests` (offsets along the bone's Z axis, exhaust out of the stern, standalone up axis,
  gravity and cylinder unchanged, and on the Remake data: Death Star lattice at the dish, Eclipse foci on the hull).
- `render_scene --show a,b` shows bones the model hides at rest (as an animation's visibility track does), e.g. the Death Star's lattice.
- Props 0x07/0x08 are `inBursts` / `linkToSystem` in the format doc: `0x07` now also drives the engine spawn model and `0x2e` (`isNotBillboarded`) the flat sprites, see "Engine glow".

## Engine glow (2026-10-17, polish2 E)
User report on the Eclipse: the engines pulsed, the colour was off and a white cone showed around the engines (screenshot: three nozzles at the stern edge over a bright
planet, each with a flat teal half disc cut off by the hull, a red flare and a white outline). Five causes, all in our reading of the data:
- **Engines were one particle per life.** Property `0x07` of an emitter is `inBursts` (format doc); the 1,071 emitters that loop without bursts, the whole Eclipse
  engine among them, carry the editor default `numParticlesPerBurst` 1 next to a rate of 1 to 20 per second. `step_emitter` read that as a burst of one particle
  per life, so every emitter faded in and out alone: the Eclipse's cores (life 1 s, triangular fade) pulsed between nothing and 0.5 every second, the ISD's
  (life 5 s) every five, and the 212 with `numBursts` 1 went out for good after one life. As authored 6 to 20 particles overlap and the triangular fades add up to a
  constant. Engine systems (`pe_`, `pte_`: `is_engine_system`, `ParticleInstance::engine`) now run `inBursts`-less emitters continuously at their rate (no `max_particles`
  cap, 512 live as a safety net). Every other system keeps the old reading (their look was tuned against the video; 1,116 emitters of other systems would change,
  see below).
- **A flat sprite was drawn as a billboard.** Property `0x2e` is `isNotBillboarded`; the Eclipse's `engine_pulseG` (a solid white disc, cell 3 of `P_PARTICLE_LIGHT`, tinted
  teal 0.2 / 0.5 / 0.45) is flat. As a camera-facing disc it was the teal half disc cut by the nozzle housing. Engine emitters with `Emitter::billboard` false now are quads in the
  emitter's XY plane (normal along the exhaust, inferred: a shockwave lies on the ground plane), seen from the stern only as a round disc and nearly invisible from the side;
  seen from behind the winding is mirrored so the batch's back-face culling keeps them (bolts share the batch and keep theirs).
- **The glow cones were treated as shield bubbles.** `AlamoPart::shield` was every `MeshShield` mesh, but 374 of the 646 shield-shader sub-meshes of the Remake are visible at rest (208 `Engine_Glow*`
  cones in 144 models, sails, waterfalls, hangar fields; example `shield_meshes`). `on_shield_hit` switched them to the hit flash and `update_shields` hid them afterwards: after the first hit of a
  ship its engine cones vanished, and each hit flashed them. Only meshes the model hides at rest are bubbles now (`is_shield_bubble`).
- **`MeshShield` ignored `Color`, the vertex colour and the textures.** The Eclipse's `Engine_Glow_Red` is `Engine_Glow_Fancy_White.tga` times `Color` red, drawn white with a rim term (the white cone).
  The shipped effect decodes (`alamo-model` examples `fx_states`, `fx_disasm`): technique `t1` is ONE/ONE, two-sided, no depth write, vertex `tint = clamp(object-space normal z + EdgeBrightness, 0, 1) *
  vertex colour * Color`, UV sets `uv * (BaseUVScale, WaveUVScale, DistortUVScale)` scrolled in v by the three scroll rates, pixel `base(uv0 + 0.25 * distortion(uv2).rg) * wave(uv1) * tint`.
  `alamo.wgsl::shield_glow` is that, with the wave and distortion textures in the planet's cloud slots (`describe_material`). The tint is `Color` for the visible meshes only; the hidden bubbles keep their neutral hit flash
  (a coloured flash would change every battle's look, not asked for).
- Sampler address modes of the glow textures were not decoded (repeat assumed; `MeshShield` `t1` has `op103/105/107/109` = 2 on sampler 0, which may be mirror): it only changes the very tip of a cone.
- `particle.wgsl` still multiplies additive sprites by the texture alpha (the P_PARTICLE_LIGHT note above); `PrimAdditive` itself is `tex * vertex colour` ONE/ONE with neither alpha used. The effect of
  dropping it was not tested (hard-edged RGB discs like cell 3 would show), so it stays.
- Proof (Eclipse side view, `render_scene --bow --cam 90,5,0.25 --look 0,-250,3500`, 4 s at 8 frames per second, mean of a 110 x 100 window around one nozzle, frames from 1 s on): red 16.0 to 20.8 (x1.31, period exactly 1 s) before,
  32.8 to 33.1 after (x1.01); green 9.3 to 13.7 before, 2.6 to 2.7 after (the teal half discs are gone). From directly behind (`--cam 0,0,0.12 --look 0,513,3940`, from 1.5 s): red x1.46 before, x1.05 after; green still runs
  89 to 135 once a second: that is `engine_pulseG` itself (colour halves over its 1 s life, rate 1 per second), the one pulse in the data, visible only from behind. `render_scene --hits 1.6,1.9`: before, the cones are
  gone after two hits; after, unchanged. Test: `particles::tests::an_engine_that_is_not_in_bursts_glows_steadily_instead_of_pulsing`, `the_eclipse_engine_glows_steadily_but_for_its_one_pulsing_disc` (data),
  `a_sprite_that_is_not_billboarded_lies_in_the_engine_bones_xy_plane`, `model::tests::only_the_shield_shader_meshes_the_model_hides_are_bubbles`, `the_eclipses_red_engine_cone_is_a_visible_shield_mesh_next_to_a_hidden_bubble`.
- Cost: the default `battle_demo` battle (292 ships) keeps 5,858 particles alive instead of 605 at 10 s (an engine proxy keeps 19 on average, 132 at most); `ParticleStats::cpu_ms` 0.69 to 2.14 ms, frame mean 12.98 to 14.35 ms at 1600 x 900.
- `render_scene` got `--hide a,b` (hide bones, to tell a mesh from a particle system), `--noauto` (no proxy particles), `--hits t,..` (scripted shield hits at the camera target) and prints the model's centre and radius.
  `battle_demo`: `SP_DEMO_STERN=<ship name>` follows that ship's engines from behind (`--cam yaw-off-astern,pitch,distance`), with the camera bounds lifted (the Eclipse's stern lies outside the map box).
- Open: the same two readings (`inBursts`, `isNotBillboarded`) are wrong for the other looping emitters too (missile smoke is already special-cased as a stream; beam fronts, hardpoint glows, shockwave rings are not);
  they were left alone because their look was checked against the video. `EngineThrottle` is not driven by any game code (all engines run at full intensity).
