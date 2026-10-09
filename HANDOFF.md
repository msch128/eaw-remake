# Handoff – state as of 2026-10-09 (morning after the first nightly run)

Read `CLAUDE.md` (rules) first. Every crate has a `STATUS.md` with details, API and next steps.
Live test guide: `PLAYTEST.md`. Work list: `docs/ROADMAP-2026-10-09.md`. Unattended runbook: `docs/NIGHTLY.md`.
QA reports: `docs/QA-2026-10-08.md`, `docs/QA-2026-10-09-night.md`.

## Overall
Roughly **70 / 100** of the goal "play Remake 4.0 Galactic Conquest with space battles, EaW look & feel, 60+ FPS in 4K".
Release gate green at 6866e81 (33/33 smoke, soak flat: battle 2.85 GB, galaxy 1.96 GB). User exe: `target\release`.

| Area | Score | Note |
|---|---|---|
| Simulation (galaxy, battle, AI, scripts) | ~88 | retreat rules + escape route, formations, capital movement, AI blockade/ready fixes, measured expansion brake |
| Stability / memory | ~87 | gate green; 4K large battle ~6 ms mean / <10 ms p99, RAM ~1.3 GB, VRAM ~1.1 GB |
| Galaxy map look | ~78 | compared with the user's refs and the Remake video; fog of war, heroes, planet view, labels/pips |
| Galaxy controls | ~78 | filters, class filter, fleet picking, planet view build row, objectives, save/load |
| Battle look | ~70 | camera like the video, lighting fixes, particle format fix, data-driven ship deaths, map fog |
| Battle controls | ~72 | Esc menu, retreat UI, move lines, abilities + autocast, HUD vs video; real-mouse coverage incomplete |
| Battle depth | ~72 | all space abilities, hangars by tech, fog, reinforcement placement, auto-resolve result dialog |
| Sound | ~55 | music flow, positional SFX, unit responses, advisor lines — verified by logs only, never heard live |

Balance rule (user): every area toward ~80 instead of a few at 100. Sound is the outlier and needs a live listening pass.

## Priorities
1. **Sound:** a live listening pass with the user (levels, response spam, music transitions), then fixes.
2. **Real-mouse QA of the battle:** abilities, reinforcements, retreat, winning a battle — not covered yet. The playtest
   driver must find the game window by title (winit's 0x0 helper window is reported as MainWindowHandle); needs the
   user's OK because the foreground guard is involved.
3. Open details from the QA reports and the video comparison (banner frame, ability icons per section, Coruscant fill
   light colour, explosion cloud size at the late stage of a ship death).

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
