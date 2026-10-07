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

## Tactical hand-over (new, `tactical.rs`)
- `Sim::tactical_setup(battle_id) -> Option<TacticalRequest>`: plain data (`Clone`, no `Arc`): planet id/name, `.ted` map, star base level,
  seed, time limit and `sides` (attackers first, defender with star base/structures last) of `TacticalGroup { kind: Unit|Starbase|Structure,
  type_id, name, count, fleet, health }`. The request is frozen in the sim until applied/cancelled. `health` is always 1.0 (no persistent damage).
- `Sim::apply_tactical_result(battle_id, &TacticalResult) -> Result<Vec<Event>, TacticalError>`: `TacticalResult { winner: Option<side index>,
  survived/retreated: [side][group], duration }`. Removes losses, a destroyed star base/structures, emits `UnitsDestroyed/BattleWon/BattleLost/BattleResolved`,
  makes retreated losers (and every attacker after a draw) fall back to `previous_planet` (a fleet built on the spot goes to a friendly neighbour).
  Capture is then the normal rule. `Sim::cancel_tactical(id)` drops the frozen request.
- `run_headless(&request, &catalog, arena)` plays a request without rendering; `resolve_battle_tactical` now uses the same request path.
  Helpers for front ends: `TacticalRequest::{battle_config, battle_setup, result_from}`.
- Tests: `tests/tactical.rs` (full round trip incl. capture, star base defence, retreat fallback, bad results rejected).

## Persistent damage (new)
- `Galaxy::damage: BTreeMap<ShipId, UnitHealth>` (hull/shield fractions, absent = undamaged) and `PlanetState::base_health` carry damage between battles. `TacticalGroup::unit_health`
  feeds it into the battle (`BattleConfig::start_health`), `TacticalResult::health` (survivors, healthiest first) writes it back (`apply_tactical_result` and the headless tactical resolver).
  Repair (`Sim::repair`, every tick): ships and star bases at a planet their faction owns regain `repair_base_per_week` (5%) + `repair_per_starbase_level_per_week` (10% per level)
  of their hull per week, shields 4x faster. These three numbers are **estimates**: the data only has tactical repair (`Repair_Amount_Per_Frame` of repair hardpoints), nothing galactic.
  Not persisted: hardpoint damage (repaired), squadron fighters, structures, statistical auto-resolve results (they only remove units).
- `Command`, `SimConfig`, `TacticalResult` are serde-serialisable (used by the galaxy-script save journal).
