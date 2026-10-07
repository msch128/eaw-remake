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
- Several operations reach into `Sim::galaxy_mut()` as workarounds; the API wishlist is in `NEEDS.md` (spawning, structures,
  ownership change, alliances, pirate forces, object identity, events). Switch once the `galaxy-sim/NEEDS-DONE.md` items land.
- Ground units, heroes and markers are not modelled.
