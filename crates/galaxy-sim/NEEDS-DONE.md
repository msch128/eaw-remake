# galaxy-script NEEDS.md -> galaxy-sim API

All additions are backwards compatible except that `Event` gained variants (add a `_ =>` arm to exhaustive
matches). Tests: `crates/galaxy-sim/tests/api.rs`. Implementation: `src/api.rs`, `src/state.rs`, `src/sim.rs`.

| Need | New API |
|---|---|
| 1 spawn without private counters | `Sim::spawn_units(faction, planet, unit, count) -> FleetId` (joins/creates the idle fleet, refreshes population and upkeep at once; no cost / requirement / cap checks) |
| 2 structures and star bases | `Sim::add_structure(planet, structure) -> bool` (orbital/ground slot and per-planet maximum checked), `remove_structure(planet, structure) -> bool`, `set_starbase(planet, Option<StarbaseTypeId>)`; all refresh the economy |
| 3 ownership change | `Sim::set_planet_owner(planet, Option<FactionId>) -> Vec<Event>`: same side effects as a capture (queues, star base and structures lost, `PlanetCaptured`, elimination and victory checks). `set_planet_owner_keeping_buildings` keeps the buildings. |
| 4 credits and tech | `Sim::add_credits(faction, amount)` (credit cap, never below 0), `set_tech_level(faction, level) -> Vec<Event>` (per level `ConstructionComplete{kind: Tech}` + `TechAdvanced`; clamped to max tech), `set_unlocked(faction, object, bool)` |
| 5 alliances | `Sim::set_alliance(a, b, bool)`, `are_allied(a, b)`, `Galaxy::hostile(&World, a, b)` (battles, blockades, capture, auto-resolve and AI all use it) |
| 6 neutral / pirate forces | `Sim::neutral_faction()` (`Neutral`, else `Hostile`, else `Pirates`), `spawn_neutral_units(planet, unit, count) -> Option<FleetId>`. Their fleets defend unowned planets, take part in battles and block capture. `None` if the campaign has none of those factions. |
| 7 object identities | `ShipId(u64)`, stable and never reused. `Galaxy::ships: BTreeMap<FleetId, Vec<(ShipId, UnitTypeId)>>` (call `Galaxy::sync_ships()` after editing fleets directly), `Sim::ships_of(fleet)`, `locate_ship(ship)`, `despawn_ship(ship)`, `change_ship_owner(ship, faction)`. Ids move with merge/split; battle losses report the lost ids. `Fleet` itself is unchanged. |
| 8 ground units / heroes | `ObjectKind {SpaceUnit, Starbase, Structure, Ground, Hero, Other}` and `Sim::object_kind(name)`; ground units and heroes are named in `World::unsimulated` so scripts can own them without catalog entries. No `UnitRole::Ground` (it would break exhaustive matches). |
| 9 perception | `Sim::planet_credit_value(planet)`, `planet_has_ground_access(planet)` (`PlanetInfo::surface_accessible`). The "wealthy" thresholds are still not parsed. |
| 10 events | `Event::TechAdvanced{faction, level}`, `BattleWon{faction, planet}`, `BattleLost{faction, planet}`, `UnitsDestroyed{faction, planet, unit, count, ships}`; `TechAdvanced` is also emitted when an upgrade completes. |
| 11 galactic AI scripts | out of scope (separate crate). |
