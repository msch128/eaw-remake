# galaxy-sim status

Headless, deterministic, `Send` Galactic Conquest simulation. Rules: `RULES.md`. API additions for the script layer: `NEEDS-DONE.md`.

## What works
- `World` (static, `Arc`) + `Galaxy` (mutable) + `Sim`; seeded RNG, no global state; determinism tests.
- Income (planet value x (1+pct)+abs, upkeep via Planet_Income_Bonus_Ability), production queues, tech levels, week = 60 s.
- Fleets and movement over routes, AI stub, victory checks, events (incl. `TechAdvanced`, `BattleWon/Lost`, `UnitsDestroyed`).
- Battle resolution via `BattleResolver::{Statistical, Tactical}`; Tactical runs battle-sim (`tactical.rs`), arenas via `set_tactical_arenas`.
- Script-facing API in `api.rs` (spawn units, structures, owner/tech/alliance changes, ShipId queries, neutral forces, ...).
- Example `headless_match`.

## Key decisions
- Planet capture rule is inferred from the AI scripts (see RULES.md).
- Tactical battles use battle-sim defaults: continuous shield regen plus the 4 s `Depleted_Shield_Disable_Time` lockout, no attack-move sniping.
- Scope: GC only, no story campaigns, no ground combat.

## Gaps / next steps
- Statistical resolver is a simple strength comparison, not tuned against tactical results.
- AI is a stub (`galaxy-ai` crate is meant to replace it).
- Capture rule and Remake plot-script mechanics need verification against the real game.
