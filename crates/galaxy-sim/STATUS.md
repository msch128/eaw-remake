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
  makes retreated losers (and every attacker after a draw) fall back along `World::escape_path` (nearest friendly planet, blockades ignored; else the nearest neighbour system).
  Capture is then the normal rule. `Sim::cancel_tactical(id)` drops the frozen request.
- `run_headless(&request, &catalog, arena)` plays a request without rendering; `resolve_battle_tactical` now uses the same request path.
  Helpers for front ends: `TacticalRequest::{battle_config, battle_setup, result_from}`.
- Tests: `tests/tactical.rs` (full round trip incl. capture, star base defence, retreat fallback, escape routes, bad results rejected).

## Persistent damage (new)
- `Galaxy::damage: BTreeMap<ShipId, UnitHealth>` (hull/shield fractions, absent = undamaged) and `PlanetState::base_health` carry damage between battles. `TacticalGroup::unit_health`
  feeds it into the battle (`BattleConfig::start_health`), `TacticalResult::health` (survivors, healthiest first) writes it back (`apply_tactical_result` and the headless tactical resolver).
  Repair (`Sim::repair`, every tick): ships and star bases at a planet their faction owns regain `repair_base_per_week` (5%) + `repair_per_starbase_level_per_week` (10% per level)
  of their hull per week, shields 4x faster. These three numbers are **estimates**: the data only has tactical repair (`Repair_Amount_Per_Frame` of repair hardpoints), nothing galactic.
  Not persisted: hardpoint damage (repaired), squadron fighters, structures, statistical auto-resolve results (they only remove units).
- `Command`, `SimConfig`, `TacticalResult` are serde-serialisable (used by the galaxy-script save journal).
- `TacticalSide::may_retreat` (faction holds more than one planet and is not Pirates) feeds `BattleConfig::retreat_allowed`, so the battle AI may retreat only where the Remake rules allow it.

## Reinforcement pool and production cancel (new)
- `TacticalSide::{reserve, unit_cap}`: units of the fleets beyond the faction's `Space_Tactical_Unit_Cap` (`FactionInfo::space_unit_cap`, 150 if the data has none; population per unit from the unit data, fleet order, a group is split where the cap falls)
  wait in `reserve` instead of `groups`. The battle config carries the cap (`BattleConfig::unit_cap`). A front end calls them in with `battle_sim::Command::Reinforce { ship, count }` (arrive after `reinforce_delay`, only if the cap allows);
  `battle_setup_with(.., deploy_reserve = true)` deploys them at once (what `run_headless` and the tactical resolver do). `TacticalResult::{reserve_called, reserve_survived}` (`[side][reserve index]`) report what was called and what lived; uncalled units never leave their fleets,
  called ones count as losses unless they survived. battle-sim reports reinforcements as results groups with `REINFORCE_TAG`, matched to reserve groups by ship type.
- `Command::CancelProduction { faction, planet, lane, index }` -> `CommandOk::Refunded(credits)`: removes a queue entry (0 = the one being built). The refund is cost x unspent share of the build time x `SimConfig::cancel_refund` (1.0). **Estimate**: the data has no refund rule; costs are paid when queued, the real game charges while it builds.
  Others' planets are refused (`NotOwner`), bad index `Invalid`. `PlayAction::CancelProduction { planet, lane, index }` in galaxy-play.

- Retreat outcome (QA 2026-10-09 night P1-2, build before 0e9a86c where Esc->Retreat still sent Aborted): `Sim::retreat_tactical` applies an explicit retreat of a faction (opponent wins, survivors retreat along the escape route, planet defenses of a retreating defender are lost so no instant second battle). The app's Esc->Retreat takes the real retreat order and returns the normal result; F10 / End battle stay "abandon + auto-resolve from the current state". Tests in `tests/tactical.rs`: `attacker_retreating_from_a_defended_planet_captures_nothing_and_leaves`, `defender_retreating_leaves_the_planet_to_the_attacker_by_the_normal_rule`, `retreat_order_in_the_battle_ends_without_capture`.
