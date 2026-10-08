# galaxy-script status

Runs the Remake's Lua plot scripts and `GameScoring` on top of `galaxy-sim` (`ScriptedSim`).

## What works
- `ScriptedSim` steps the sim and the script scheduler together; scripts are serviced at their `ServiceRate` in game time.
- Sim events become story notifications (planet captured, construction complete, tech level).
- `SimHost` implements `alamo_lua::ScriptHost`: spawn units, credits, unlock objects, dialogs/messages, reported as `ScriptEvent`s.
- Optional `remake_ai`: `ai.rs` implements `galaxy_ai::AiSim` on galaxy-sim so the Remake AI drives the AI factions.
- Examples `scripted_match`, `ai_match`; `tests/scripted.rs`.

## Key decisions
- Scripts change the sim only through `SimHost`; the sim itself stays script-agnostic and deterministic.
- `GalacticFreeStore` is experimental (`ScriptedConfig::freestore`).

## Gaps / next steps
- (done) the NEEDS.md workarounds were replaced by the galaxy-sim API; `ShipId`s are not yet used as script object ids (objects are still `(fleet, unit type)`).
- Ground units, heroes and markers are not modelled.
- **Migrated to the galaxy-sim API** (`NEEDS-DONE.md`): spawns use `Sim::spawn_units`, structures `add_structure/remove_structure`, owners
  `set_planet_owner[_keeping_buildings]` (script-driven transfers keep buildings), credits/tech/unlock/alliances/despawn/change-owner use the
  new calls. Events caused by script or AI calls (`PlanetCaptured`, `TechAdvanced`, ...) are queued in `Inner::pending_events` and are
  reported (and fed to the story) with the next `ScriptedSim::step`. No `galaxy_mut()` writes remain except none in host/ai.
- **AI switch:** `ScriptedConfig::remake_ai` (default `false` = galaxy-sim's stub AI via `Sim::set_ai_factions`; `true` = Remake AI, which clears the
  stub). `human: Some("none")` makes every faction AI controlled. The campaign's own active player has the `SandboxHuman` AI type; it is mapped to
  `GC_<Faction>Ai` when that player is not human (before, Rebel never acted in AI-vs-AI runs).
- Test `tests/ai_match.rs`: 20 weeks AI vs AI (ships 182 -> ~750, ~120 ships and ~140 structures queued by plans, 8 captures, ~1150 plans, all factions
  active). Per game week (60 s): median ~310 ms wall, worst ~410 ms; of that sim ~3 ms, story scripts ~33 ms, AI ~240 ms (goal evaluation ~60 ms, task
  force jobs ~5 ms, the rest is starting plan scripts: a fresh Lua VM with API and prelude per plan, ~3 ms each, ~57 plans per week).
- alamo-lua now caches compiled chunks (bytecode per script path and the prelude) across plan VMs: AI cost per game week fell from ~240 ms to ~85 ms
  (20 weeks all-AI: median 121 ms wall per week including scripts, worst 155 ms).

## Save/load, defaults (save.rs)
- **Snapshot saves (format version 2, `save.rs`)**: `ScriptedSim::{save(path), load(path, fs, data), load_with(.., setup_hook), to_save, from_save, state, checkpoint}`. A save is the
  actual state, not a replay, so loading takes the same time for week 5 and week 200 (week-200 AI game: 251 KB, save 7 ms, load 0.57 s, almost all of it building the match:
  world, story scripts, AI players). JSON: header (campaign, `SimConfig`, `ScriptedConfig`, data fingerprint), checkpoint (state hash, week, faction summary), and the state:
  * galaxy-sim `SimSnapshot` (planets, queues, fleets, damage, statistics, cooldowns, RNG; exact: `galaxy-sim/tests/gaps.rs` restores a sim and both continue identically);
  * `alamo_lua::RuntimeSnapshot`: story plots (event states, counters, timers incl. hero respawns), `GlobalValue` store, story flags, interned types, and for each long-lived script
    the global variables that changed since set-up (numbers, strings, bools, tables of those, player/object/type handles by id);
  * `galaxy_ai::AiMemory`: goal timers, learned failures, budgets/accounts, sizing, reserved fleets, strategic-layer memory (fog, phases).
  **Not saved** (Lua coroutines cannot be serialised): where scripts sleep and their locals, AI plans in flight and the task forces they own, events produced but not yet reported by
  `step`, frozen tactical set-ups. Long-lived scripts (Remake: starting forces, neutral reinforcements, unit filters, ship phase stories, GameScoring) are started afresh and get their globals
  back; they are event driven (`Story_Event_Trigger`, state machines) and the story engine knows which events fired, so nothing is repeated. A loaded game's ships and queues stay, AI goals start new
  plans. So the continuation is **not bit-identical** to the original game (the tests print whether it is; it is not). Loading checks format, version, campaign, data fingerprint and that the
  restored state hashes like the saved one (`SaveError::{Version, DataMismatch, Corrupt, Setup}`); `ScriptedSim::load_warnings` lists parts of the save that did not fit.
- The journal is gone (no `with_sim_mut` taint, `Diagnostics::journal_*` are 0, `SaveFile::total_steps()` is 1, `LoadProgress` has one step). `PlayAction::Save/Load` are unchanged.
  Runtime-only things (tactical catalog and arenas) are still restored with `load_with(.., |s| s.with_sim_runtime(..))`.
- **Defaults:** `ScriptedConfig::default()` now means Remake AI + strategic layer + `DifficultyTier::Normal` (use `remake_ai: false, difficulty: None` for the old stub).
  Reason: over 40 weeks all-AI the stub snowballs (Rebel 38 planets, factions eliminated, 449 enemy-owner captures), the Remake AI stays sane (largest faction 24 planets,
  nobody eliminated, 18 enemy-owner captures).
- Added for galaxy-play: `ScriptedSim::from_sim` (wrap a `Send` `Sim` built elsewhere; `ScriptedSim` itself is not `Send`, build it on the thread that runs it), `from_save_progress` with `LoadProgress`
  (steps done/total, week), `SaveFile::read`/`total_steps`, journaled `resolve_battle` / `resolve_battle_tactical`, and `with_sim_runtime` for runtime-only setup that does not block saving.
