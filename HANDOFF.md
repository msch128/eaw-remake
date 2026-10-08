# Handoff – state as of 2026-10-08 (night session)

Read `CLAUDE.md` (rules) first. Every crate has a `STATUS.md` with details, API and next steps.
First live test guide for the user: `PLAYTEST.md`.

## Overall
Roughly **70%** of the goal "play Remake 4.0 Galactic Conquest with space battles, EaW look & feel, 60+ FPS in 4K".
The full loop works in offscreen/headless tests: menu → campaign (difficulty) → galaxy (Remake scripts + AI) → battle
pending → Fight (tactical battle on the planet's map) or Auto-resolve → result applied → galaxy; save/load; audio wired.
**Not yet live-tested with a real window, mouse and sound** – that is the next milestone (user's first playtest).

Measured (release, offscreen 3840x2160): galaxy late game ~4.8 ms/frame, battle with ~290 ships ~7.2 ms mean / 9.5 ms p99.
Sim worker ~150–210 ms per game week (6% of a core at 16x), memory flat ~400 MB over 100+ weeks (soak test).
Regression check: `tools/smoke.ps1` (18 offscreen scenarios, all passing as of 2026-10-08 morning).
Since the first handoff: end-of-game + battle result screens, superweapon, reinforcement window, queue cancel, hero
respawn, production discounts, AI match variety, auto-resolve calibrated against tactical (85–93% winner agreement),
unit-cap waves, title scene, loading art, shadows/quality presets, particle fidelity + persistent particle buffers,
async model preloading for battles.

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
- Save = deterministic input journal replayed on load (Lua coroutines can't be serialised). Load ~0.2 s per game week.
- AI-vs-AI battles use auto-resolve (tactical resolver ~0.7 s per battle is too slow); player battles may be tactical.
- All invented numbers are named estimates in the RULES.md / STATUS.md of their crate.

## Next steps
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
