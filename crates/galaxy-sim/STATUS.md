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
- `TacticalSide::may_retreat` (faction holds more than one planet and is not Pirates) feeds `BattleConfig::retreat_allowed`, so the battle AI may retreat only where the Remake rules allow it; `TacticalSide::defender` feeds `BattleConfig::defenders` (`IsDefender` of the retreat rule: a defending fleet without a station may leave after 30 s).

## Reinforcement pool and production cancel (new)
- `TacticalSide::{reserve, unit_cap}`: units of the fleets beyond the faction's `Space_Tactical_Unit_Cap` (`FactionInfo::space_unit_cap`, 150 if the data has none; population per unit from the unit data, fleet order, a group is split where the cap falls)
  wait in `reserve` instead of `groups`. The battle config carries the cap (`BattleConfig::unit_cap`). The player calls them in with `battle_sim::Command::Reinforce { ship, count }` (arrive after `reinforce_delay`, only if the cap allows); `battle_setup` hands the reserve of every AI side to its battle AI (`BattleConfig::ai_reserve`), which calls it in itself;
  `battle_setup_with(.., deploy_reserve = true)` deploys them at once instead. `TacticalResult::{reserve_called, reserve_survived}` (`[side][reserve index]`) report what was called and what lived; uncalled units never leave their fleets,
  called ones count as losses unless they survived. battle-sim reports reinforcements as results groups with `REINFORCE_TAG`, matched to reserve groups by ship type.
- `Command::CancelProduction { faction, planet, lane, index }` -> `CommandOk::Refunded(credits)`: removes a queue entry (0 = the one being built). The refund is cost x unspent share of the build time x `SimConfig::cancel_refund` (1.0). **Estimate**: the data has no refund rule; costs are paid when queued, the real game charges while it builds.
  Others' planets are refused (`NotOwner`), bad index `Invalid`. `PlayAction::CancelProduction { planet, lane, index }` in galaxy-play.

- Retreat outcome (QA 2026-10-09 night P1-2, build before 0e9a86c where Esc->Retreat still sent Aborted): `Sim::retreat_tactical` applies an explicit retreat of a faction (opponent wins, survivors retreat along the escape route, planet defenses of a retreating defender are lost so no instant second battle). The app's Esc->Retreat takes the real retreat order and returns the normal result; F10 / End battle stay "abandon + auto-resolve from the current state". Tests in `tests/tactical.rs`: `attacker_retreating_from_a_defended_planet_captures_nothing_and_leaves`, `defender_retreating_leaves_the_planet_to_the_attacker_by_the_normal_rule`, `retreat_order_in_the_battle_ends_without_capture`.
- Auto-resolve of a fight that was left (2026-10-17, retreat question Auto-Resolve and F10): `Sim::abandon_tactical` applies the losses so far and writes the survivors' hull and shield into `galaxy.damage` / `base_health` (`write_health`; since a47c680 it had healed them through `apply_health`, so the auto-resolve fought with full health); the statistical resolver starts every unit with that health (`combat::resolve_damaged`) and heals what survives the battle afterwards (EaW). Test `tactical::auto_resolving_a_battle_that_was_left_starts_from_the_survivors_and_their_health`.

## Build menu split, prerequisites and slots (QA 2026-10-09 U1-U5)
- Root causes: (1) the HUD put every structure on the ground tab; (2) `Required_Special_Structures` was flattened to "all of"
  although the XML writes alternatives with `|` (`A | B | C`), so terrain variants and multi-faction ships were mis-evaluated; (3) `Required_Planets` was only
  checked for ships (never for structures: Starforge, refineries, outposts of every terrain showed everywhere) and a list with one unknown planet name made the
  whole item unbuildable; (4) items needing a station level the planet can never reach stayed listed as "requires star base level 3"; (5) the menu printed raw
  object names (`E_Uplink_Command`). Hero cost 1 credit is the data value (`Build_Cost_Credits` 1, the upgrade items cost 25000+), not a bug.
- Fixes: `catalog.rs` parses requirements into groups (`req_structures: Vec<Vec<StructureTypeId>>`), structures get `req_planets`; `production.rs` enforces groups,
  planets and `Max_Space_Base` (item hidden via `RequirementsNotMet("planet")`) for ships and structures; `World::planet_capacity` exposes slot counts for the UI
  (`PlanetCapacity`); `BuildMenu::starbase_max` + display names of requirements (`galaxy-play/build_menu.rs`); `space-peace/play.rs::on_space_tab` puts orbital
  structures (`StructureType::orbital`: `Base_Position` Orbital / `Build_Tab_Space_Station` / `Buildings/Space/` source) on the space tab, ground structures on the ground tab.
  The AI uses the same `check_build_*` calls, so it obeys all of it.
- Ground structure rule (`classify_ground_structures`): a ground structure stays buildable (`galactic`) if it has positive income, a mining-colony income bonus,
  gambling income, population capacity, space-side weapons (hardpoints or `Is_Special_Weapon_In_Space`), a production discount that applies to ships
  (`Reduce_Production_*` with `All`, a ship category or a ship type), or unlocks a space unit; plus every prerequisite of a kept structure (outposts, base cores).
  Everything else supports ground units only and is not offered. Resulting removals (Empire/Rebel/Underworld): `Barracks_Base_*`, `Factory_Base_*`,
  `Factory_Heavy_Base_*`, `Factory_Advanced_Base_*`, `Hangar_Base_*`, `Support_Hangar_Base_*`, `Training_Grounds_Base_*`, `Proving_Grounds_Base_*`, `Workshop_Base_*`,
  `Starport_*`, `Echo_Base_Hoth`, `Planet_Rocket_Tower_*`, `Tower_Defense_Grid_*`, `Flak_Tower_Defense_Grid_*` (upkeep only, no galactic effect in the data).
  Kept: outposts, palaces, `*_Base_Core_*`, taxation agencies, agricultural communities, mining companies/facilities, cantinas, scrapyards, warehouses, casino, arena,
  senate, fortresses and watchtowers (space hardpoints), `Hypervelocity_Gun`, `Nuke_Launcher`, `Seismic_Cannon` (ground-to-space), and all orbital structures.
- Gaps: objects with `Tech_Level` 99 (`Planet_Shield_E`, `Communications_Array_E`, `Imperial_Command_Center` ...) are still never offered (story-unlocked, see RULES.md);
  `Required_Orbiting_Units` and `Required_Ground_Base_Level` are not enforced; whether the Remake treats a missing planet name in `Required_Planets` as "ignore" is assumed (any one listed planet suffices).
- Tests: `tests/build_rules.rs` (8 tests on the real data: yard 3 needs 2 needs 1, alternatives/groups parsing, hero needs the uplink + cost 1, Star_Destroyer_R only at Kuat/Fondor,
  small planet limits, planet lists of structures, no army buildings/yards in the right menus, slot API) and `galaxy-play/tests/build_menu_rules.rs` (display name of the unmet requirement).

## From the Ground Up (2026-10-09, U21)
`SimConfig::ground_up` (journaled in saves through the header's sim config) makes `Sim::new` apply `ground_up.rs`: the player's faction keeps its home planet, the other main factions (`World::main_factions`, derived from the campaign's `AI_Player_Control`: Rebel, Empire, Underworld, plus Republic / CIS in the Clone Wars stack) keep home + the two nearest planets they own; released planets and all unowned ones go to the passive faction `Pirates` (the orange independent faction, which defends but never builds / moves / expands: `World::passive_faction`, its commands are refused and `run_ai` skips it). Minor factions keep their planets. Start planets get star base level 1; fleets of the main factions stay only at kept planets. Menu: "From the Ground Up" row below the Full campaign (`campaign_select.rs`); CLI `--ground-up`. Tests: `tests/ground_up.rs` (base and `fft` stack). Remake research, user adaptations and known limits (Underworld one-planet start has negative income): `RULES.md` "From the Ground Up".

## Fleet slots (QA 2026-10-09 U18/U14/U31)
- `Fleet::slot` (`1..=FLEET_SLOTS`, serde default 0 = unassigned) is the stable slot of a fleet at the planet it orbits: the planet view's three boxes and the three circles in the galaxy view.
  `Galaxy::normalize_slots(&World)` (every tick after movement and before any fleet command) assigns slots and merges surplus fleets into slot 1; `Galaxy::settle_fleet` places an arriving fleet.
- Rules (bind every faction; `SimConfig::fleet_slot_cap_ai = false` lifts them for the AI, the human always keeps them, see `World::slot_limited`):
  arrival merges into the named slot (`Command::MoveFleetToSlot`, kept in `Galaxy::dest_slots`), else the lowest free slot, else slot 1; `SplitFleet`/`TransferUnits{to: None}` take the lowest free slot and are refused with `NoSlot` when all three are held;
  `TransferUnitsToSlot` drops units on a slot (merge into its fleet or a new fleet in exactly that slot); merging keeps the target's pathfinder, else takes the incoming one. `DebugGrantToSlot` is the debug/offscreen variant.
  The galaxy-play client-side `PendingFleetMerges` is gone; `PlayAction::MoveAndMerge` and the new `PlayAction::MoveToSlot` send `MoveFleetToSlot`.
- Battle start: `battle_sides`/reserve follow the slot order 1, 2, 3; the pathfinder rule is per fleet (a fleet with a pathfinder deploys only that unit first, a fleet without deploys fully up to the cap).
- Tests: `tests/fleet_slots.rs`.
- AI: `Galaxy::merged_into`/`Galaxy::successor` let the AI follow merged fleet ids (the `galaxy-script` adapter resolves ids). The task forces (`galaxy-ai` `taskforce.rs`) move pieces into the main fleet's slot (`AiSim::move_fleet_into`), move ships between fleets instead of splitting when the main fleet is at the planet (`AiSim::transfer_unit`), and merge two unused fleets to free a slot before a split.
