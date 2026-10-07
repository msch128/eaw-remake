# Missing galaxy-sim API (requested by galaxy-script)

`galaxy-script` currently reaches into `Sim::galaxy_mut()` for everything below. Each item is something it
works around today; a proper API would remove the workaround.

## 1. Spawning ships without touching private counters
```rust
impl Sim {
    /// Adds `count` ships of `unit` to the faction's fleet orbiting `planet` (creating a fleet if there is none)
    /// and returns the fleet id. Must update population / economy like a finished build would.
    pub fn spawn_units(&mut self, faction: FactionId, planet: PlanetId, unit: UnitTypeId, count: u32) -> FleetId;
}
```
Workaround: fleets are inserted straight into `Galaxy::fleets` with ids from `0x4000_0000` upward because
`Galaxy::next_fleet_id` is `pub(crate)`; `refresh_economy` is not called, so population and upkeep lag
until the next tick.

## 2. Adding / removing structures and star bases
```rust
impl Sim {
    pub fn add_structure(&mut self, planet: PlanetId, structure: StructureTypeId) -> bool; // respects slots
    pub fn remove_structure(&mut self, planet: PlanetId, structure: StructureTypeId) -> bool;
    pub fn set_starbase(&mut self, planet: PlanetId, level: Option<StarbaseTypeId>);
}
```
Workaround: direct `PlanetState::structures` pushes (no slot check, no economy refresh).

## 3. Ownership change with all side effects
```rust
impl Sim {
    /// Changes the planet owner like a capture does (capture timer, structures kept/lost, events).
    pub fn set_planet_owner(&mut self, planet: PlanetId, owner: Option<FactionId>) -> Vec<Event>;
}
```
Workaround: `PlanetState::owner = ...` (no `PlanetCaptured` event, so the story's `STORY_CONQUER` is not
raised for script-caused changes).

## 4. Credits and tech through the API
```rust
impl Sim {
    pub fn add_credits(&mut self, faction: FactionId, amount: f64);          // respects Credit_Cap
    pub fn set_tech_level(&mut self, faction: FactionId, level: i32) -> Vec<Event>; // unlocks, events
    pub fn set_unlocked(&mut self, faction: FactionId, object: &str, unlocked: bool);
}
```
Workaround: direct field writes. `set_tech_level` should also emit what `UpgradeTech` completion emits, so
`STORY_TECH_LEVEL` plots fire.

## 5. Alliances at runtime
`World::factions[..].enemies` is immutable behind `Arc<World>`. Scripts call `Make_Ally`; galaxy-script keeps
its own ally set, which galaxy-sim's combat/blockade logic does not see.
```rust
impl Sim { pub fn set_alliance(&mut self, a: FactionId, b: FactionId, allied: bool); }
```

## 6. Neutral / pirate forces
The story spawn scripts populate every planet for its owner, including *Pirates* and the neutral player
(`Spawn_Random_Units_Space(pirate_space, ...)`, ground units, civil buildings). galaxy-sim has no unowned
or pirate armies, so ~97 % of the 15 k spawn requests of the Rebel start script (neutral/pirate planets) are
dropped. Needed: planets with an owner that is not a playable faction (`Option<FactionId>` owner already
exists) holding defence fleets that take part in battles, e.g.
```rust
impl Sim { pub fn spawn_neutral_units(&mut self, planet: PlanetId, unit: UnitTypeId, count: u32); }
```

## 7. Object identities
Scripts address individual units and structures (`Despawn`, `Change_Owner`, `Get_Hull`, `Prevent_AI_Usage`).
galaxy-sim only has fleets with unit *counts*. galaxy-script fabricates "ship group" ids from
`(fleet, unit type)`, which break when fleets merge/split. Requested: stable `UnitId` per ship, or at least
an event/stream for fleet merge/split so ids can be remapped:
```rust
pub struct ShipId(u64);
impl Fleet { pub ships: Vec<(ShipId, UnitTypeId)> }
```

## 8. Ground units, heroes and markers
`SPAWN_HERO`, `UNIQUE_UNIT` and the spawn tables name ground units, hero teams and buildings that have no
catalog entry (`Pirate_Government`, `Emperor_Palpatine_Team`). They are counted in
`HostStats::spawn_ignored` but not represented. Needs either catalog entries (with `UnitRole::Ground`) or an
explicit "not simulated" marker so scripts can still own them.

## 9. Perception queries
`EvaluatePerception("Is_Planet_Wealthy" | "Is_Planet_Very_Wealthy" | "Is_Planet_Surface_Accessible", player,
planet)` is called ~1.3 k times in 20 weeks (start scripts). Wanted queries on `Sim`:
```rust
pub fn planet_credit_value(&self, planet: PlanetId) -> f32;       // already in PlanetInfo
pub fn planet_has_ground_access(&self, planet: PlanetId) -> bool; // surface accessible
```
Thresholds for "wealthy" live in `Data/Xml/Perceptions` / Evaluators scripts and are not parsed by anything yet.

## 10. Events for scripts
`Event::PlanetCaptured` etc. exist. Missing for story plots: `StoryHook::TechLevel{faction, level}`,
`Event::TechAdvanced`, per-faction `BattleWon/BattleLost` (for `STORY_WIN_BATTLES` / `STORY_LOSE_BATTLES`),
`Event::UnitDestroyed{faction, unit}` (for `STORY_TACTICAL_DESTROY`). Currently only `PlanetCaptured`,
`ConstructionComplete` are forwarded.

## 11. Galactic AI scripts
The scripted AI path (`Scripts/AI/*.lua` plans, goals, task forces, `FreeStore.*`, `FindTarget`,
`Evaluators`) needs an AI framework (task force objects with `Produce_Force`, `Move_To`, plan results,
budgets, perceptions). That is a separate crate (alamo-ai) rather than galaxy-sim API; galaxy-sim's own AI
keeps playing meanwhile. `ScriptedConfig::freestore` starts `GalacticFreeStore` to measure what is missing:
it dies on the first access to the engine-provided `FreeStore` object.
