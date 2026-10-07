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
