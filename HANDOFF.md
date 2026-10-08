# Handoff – state as of 2026-10-08 (night session)

Read `CLAUDE.md` (rules) first. Every crate has a `STATUS.md` with details, API and next steps.
First live test guide for the user: `PLAYTEST.md`.

## Overall
Roughly **58 / 100** of the goal "play Remake 4.0 Galactic Conquest with space battles, EaW look & feel, 60+ FPS in 4K".
The full loop runs (menu -> campaign -> galaxy with Remake scripts + AI -> battle pending -> Fight or Auto-resolve ->
result applied -> galaxy; save/load; audio wired), but real-window play shows the feel is not there yet. Honest split:

| Area | Score | Note |
|---|---|---|
| Simulation (galaxy, battle, AI, scripts) | ~85 | deterministic, tested against real data |
| Stability / memory | ~80 | image leak fixed; soak: galaxy 10 min 1.8 GB flat, battle 5 min 2.5 GB flat (was 12-14 GB) |
| Galaxy map look | ~60 | planets and discs good; routes, fleet/station/hero models, background, labels not EaW yet |
| Galaxy controls | ~65 | select/build/move work; polish missing |
| Battle look | ~50 | ships and effects fine; UI bars and feedback missing |
| Battle controls | ~35 | no action cam, reinforcement placement, ability buttons, shield + hull bars |
| Battle depth | ~45 | abilities and AI exist, balance unverified |
| Sound | ~30 | wired, never heard live |

Measured (release, 3840x2160): galaxy ~5 ms/frame, battle with ~290 ships ~7 ms mean / 10 ms p99 (offscreen). Sim worker
~150-210 ms per game week.

## Priorities
1. **Galaxy map EaW feel:** yellow trade routes, 3D fleet / station / hero models, background, labels.
2. **Battle controls:** action cam, reinforcement placement, ability buttons, shield + hull bars.
3. Then battle depth, sound, balance.

## Regression guards (rule: no exe goes out without smoke + soak green)
- `tools/smoke.ps1`: offscreen / hidden scenarios with a pass/fail table.
- `tools/soak.ps1`: hidden 5 min battle + 10 min galaxy with `SP_ASSET_LOG=1`; fails on memory > 8 GB, growth > 5 % / min,
  linearly rising asset counts, ERROR / panic / OOM / validation failures, WARN spam, p99 frame time >= 33 ms.
- Input tests: `SP_UI_TEST=1` (galaxy screens, `space-peace/src/input_test.rs`) and `SP_INPUT_TEST` (tactical screen,
  `battle-view/src/input_test.rs`) inject window events; real-mouse driver for manual QA: `tools/playtest/driver.ps1`.
- QA findings: `docs/QA-2026-10-08.md`.

## Crates
| Crate | What it is | State |
|---|---|---|
| `alamo-fs` | MEG archives, layered GameFs, install detection | done |
| `alamo-data` | XML, text, campaigns, game objects, difficulty table | done |
| `alamo-model` | ALO/ALA/particles/DDS | done |
| `alamo-render` | Bevy renderer, reconstructed EaW shaders, particles, LDR light saturation, Toksvig | good |
| `alamo-map` | .ted space maps | done |
| `alamo-audio` | SFX/speech/music, MusicPlanner/Director from MusicEvents.xml, advisor hooks, volume buses | done (not heard live) |
| `alamo-lua` | Lua 5.0-compatible runtime, story engine (incl. *_Auto triggers), bytecode cache | done |
| `galaxy-sim` | headless deterministic GC: economy, builds (+cancel), fleets, capture, tactical setup/result, damage + repair, reinforcement reserve | good |
| `battle-sim` | deterministic space battles; tactical AI (retreat, focus, abilities, bombing runs, intercept, formations) | good |
| `galaxy-script` | Remake GC scripts on galaxy-sim, ScriptedSim, save/load (input journal) | good |
| `galaxy-ai` | Remake galactic AI + modern strategy layer (influence maps, utility, flanks, pincers, fog) + difficulty tiers | good |
| `galaxy-play` | sim worker thread (ScriptedSim), fleets, orders, save/load, soak harness | good |
| `battle-view` | tactical battle view, TacticalUiPlugin, tactical minimap, audio hooks | good |
| `eaw-ui` | original EaW UI from game data | good |
| `space-peace` | the app: menus, new-game options, galaxy, battles, save slots, options, first-run problem screen, log file | good |

## Key decisions (don't silently revert)
- Planet capture (space-only Remake): hostile faction alone in orbit + no enemy starbase → owner changes (inferred).
- Shields: continuous regen + `Depleted_Shield_Disable_Time`. No attack-move hardpoint sniping by default.
- Battle speed scale 30. Dummy planets filtered (145 real planets in the Full GC).
- Default AI = Remake AI + strategy layer, difficulty Normal. Easy/Normal/Hard from `DIFFICULTYADJUSTMENTS.XML`;
  Brutal and Fair are our own tiers (estimates). Classic stub AI optional (it snowballs).
- Save = real state snapshot (user decision): sim state, story engine, Lua script globals, AI memory. Lua coroutines
  restart idle with restored globals; continuation is not bit-identical (galaxy-sim part is). Load ~0.6 s flat.
- AI-vs-AI battles use auto-resolve (tactical resolver ~0.7 s per battle is too slow); player battles may be tactical.
- All invented numbers are named estimates in the RULES.md / STATUS.md of their crate.

## Next steps (older list, see Priorities above)
1. **First live playtest with the user** (`PLAYTEST.md`), then fix what they find (input feel, camera, sound, balance).
2. In progress: GC feature gaps (`galaxy-sim/GC-GAPS.md`: heroes, structures/bonuses, station levels, Underworld mechanics
   if the Remake GC uses them); reinforcement window + queue cancel in the UI; particle fidelity (explosions too dim).
3. AI goals (user): smart rather than aggressive – measured expansion, flanking; more variety per match if it feels predictable.
4. Polish: fonts, galaxy label LOD, command bar opacity, shadows/quality options.

## Verification assets
Offscreen renders live outside the repo (scratch folders). Useful app flags: `--offscreen out.png`, `--campaign`, `--select`,
`--weeks`, `--build`, `--move`, `--battle-choice fight|auto`, `--battle-auto`, `--battle-seconds`, `--battle-orders`,
`--via-menu`, `--screen`, `--slots`, `--difficulty`, `--classic-ai`, `--save-to`/`--load-from`.
Soak: `cargo run --release -p galaxy-play --example soak`.
