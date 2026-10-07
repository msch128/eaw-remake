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
- `ScriptedSim::{save(path), load(path, fs, data), load_with(.., setup_hook), to_save, from_save, checkpoint}`: versioned JSON (`SAVE_VERSION` 1, serde). A save is the match
  setup (campaign, `SimConfig` with seed, `ScriptedConfig`) plus the **journal** of external inputs (steps run-length coded, commands, tactical setups/results,
  auto-resolves, AI factions) and a checkpoint (state hash, time, per-faction summary). Live Lua coroutines cannot be serialised, so loading replays the journal; galaxy-sim,
  scripts, AI engine (task forces, accounts, reservations), strategy memory and all RNGs are deterministic, which makes the continuation identical
  (`tests/save.rs`: save at week 8, +5 weeks on the original and on the loaded copy give the same state hash, AI statistics and script log). The replay is
  verified against the checkpoint (`SaveError::Diverged` for changed mod data or code, `DataMismatch`, `Version`, `UnknownCampaign`).
  Load time ~0.2 s per game week plus ~1 s setup (8 weeks: 1.9 s); the file is a few KB per hundred steps.
- **App needs:** route every input through the journaled methods (`apply`, `set_ai_factions`, `tactical_setup`, `apply_tactical_result`, `resolve_battle_auto`,
  `cancel_tactical`, `step`). `with_sim_mut` is for tests; after it `save` returns `UnjournaledChanges`. When battles are resolved tactically the catalog and arenas
  are runtime-only: restore them with `load_with(.., |s| s.with_sim_mut(|sim| sim.set_tactical_catalog(..)))`. A save list can read `SaveFile::checkpoint` (week, factions) without replaying.
- **Defaults:** `ScriptedConfig::default()` now means Remake AI + strategic layer + `DifficultyTier::Normal` (use `remake_ai: false, difficulty: None` for the old stub).
  Reason: over 40 weeks all-AI the stub snowballs (Rebel 38 planets, factions eliminated, 449 enemy-owner captures), the Remake AI stays sane (largest faction 24 planets,
  nobody eliminated, 18 enemy-owner captures).
- Added for galaxy-play: `ScriptedSim::from_sim` (wrap a `Send` `Sim` built elsewhere; `ScriptedSim` itself is not `Send`, build it on the thread that runs it), `from_save_progress` with `LoadProgress`
  (steps done/total, week), `SaveFile::read`/`total_steps`, journaled `resolve_battle` / `resolve_battle_tactical`, and `with_sim_runtime` for runtime-only setup that does not block saving.
