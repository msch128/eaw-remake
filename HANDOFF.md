# Handoff – state as of 2026-10-07

Read `CLAUDE.md` (rules) first. Every crate has a `STATUS.md` with details, API and next steps.

## Overall
Roughly **35%** of the goal "play Remake 4.0 Galactic Conquest with space battles, EaW look & feel, 60+ FPS in 4K".
All building blocks exist and are individually tested; **there is no end-to-end playable match yet.**
The next milestone is integration (see below) – that is the critical path.

## Crates
| Crate | What it is | State |
|---|---|---|
| `alamo-fs` | MEG archives, layered GameFs (mod → FoC → EaW, megafiles.xml, patches, audio MEGs), install detection | done |
| `alamo-data` | lenient XML, English .dat text, campaigns/planets/routes/factions, 17k game objects, typed space defs, damage table, starting ownership | done |
| `alamo-model` | ALO models, ALA animations, particle systems, DDS/TGA/PNG decode | done (all Remake files parse) |
| `alamo-render` | Bevy plugin: models with reconstructed EaW shaders, particles, projectiles, impacts, shields, animations, turrets, hyperspace, death clones, LOD culling, optional shadows | good |
| `alamo-map` | .ted space maps (bounds, lights, markers, objects) | done |
| `alamo-audio` | SFX/speech/music events, WAV decode, Bevy audio plugin + MusicDirector | done (not live-tested) |
| `alamo-lua` | Lua 5.0-compatible runtime for Remake scripts, story/plot engine, `ScriptHost` trait | done |
| `galaxy-sim` | headless deterministic GC simulation (economy, builds, fleets, capture, battles, stub AI) | good |
| `battle-sim` | headless deterministic space battle sim (hardpoints, arcs, shields, fighters, abilities, real maps) | good, balance plausible |
| `galaxy-script` | runs Remake's GC plot scripts + GameScoring on galaxy-sim | good |
| `galaxy-ai` | real Remake galactic AI (goals/perceptions/Lua plans) | **in progress** – see its STATUS.md |
| `galaxy-play` | Bevy plugin: sim on worker thread, fleets on the map, build/move commands | **in progress** – see STATUS.md |
| `battle-view` | Bevy plugin: renders/drives a battle-sim battle EaW-style (camera, selection, orders) | **in progress** – see STATUS.md |
| `eaw-ui` | original EaW UI from game data (command bars, panels, minimaps, tactical UI, 3 faction skins) | good |
| `space-peace` | the app: loading screen, main menu, campaign select, galaxy map with real models, options, pause, SpaceBattle seam | good shell |

## Key decisions (don't silently revert)
- Planet capture (space-only Remake): a hostile faction alone in orbit + no enemy starbase → owner changes (inferred from Remake AI scripts; see `galaxy-sim/RULES.md`).
- Shields: continuous regen (`Shield_Refresh_Rate` per 2 s) + `Depleted_Shield_Disable_Time` (4 s offline at zero). No attack-move hardpoint sniping by default.
- Battle speed scale 30 (30 Hz logic tick). ISD vs MC80 1v1 ≈ 4.5 min, MC80 wins small fights, Empire wins large ones.
- Remake dummy planets (`*_Dummy_WW`, invisible) are filtered from the map (145 real planets in the Full GC).
- Reconstructed shaders (game ships only compiled `.fxo`); look is EaW-like, not pixel-exact.

## Next steps (in order)
1. **Integrate into `space-peace`:** `galaxy-play` (sim driver, fleets, build/move) + `eaw-ui` command bar on the galaxy map.
2. **Battle loop:** BattlePending → `battle-view` with a `BattleSetup` from the fleets and the planet's .ted map → result applied back into galaxy-sim (an API to apply a tactical result is still missing – see `space-peace/STATUS.md`) → return to galaxy. `eaw-ui` tactical bar in battle.
3. Hook `alamo-audio` into battles (weapon SFX, unit responses, battle music intensity).
4. Switch `galaxy-script` to the new galaxy-sim APIs (`galaxy-sim/NEEDS-DONE.md`), then finish `galaxy-ai` and replace the stub AI.
5. First live play test with the user (ask when it suits them – they game on this machine), then balance/feel polish.

## Verification assets
Offscreen renders live outside the repo (scratch folders). Re-create them with the crates' examples, e.g.
`space-peace --offscreen out.png`, `alamo-render` `render_scene`/`showcase`/`battle`, `eaw-ui` `ui_demo`, `battle-view` `battle_demo`.
