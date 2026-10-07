//! Direct manipulation API for scripts and tools: spawning, buildings, ownership, economy, tech,
//! alliances, stable ship ids and perception queries. Everything here keeps the derived state
//! (income, population, ship ids) consistent, unlike writing to [`crate::Galaxy`] directly.

use crate::sim::{Sim, add_units};
use crate::state::*;
use crate::world::*;

impl Sim {
    // ---- spawning --------------------------------------------------------------------------

    /// Adds `count` ships of `unit` to the faction's fleet orbiting `planet` (creating one if there is
    /// none) and returns the fleet id. Population and upkeep are updated immediately; costs, requirements
    /// and population caps are *not* checked (this is a script/story spawn, not a purchase).
    pub fn spawn_units(&mut self, faction: FactionId, planet: PlanetId, unit: UnitTypeId, count: u32) -> FleetId {
        let id = add_units(&mut self.galaxy, faction, planet, unit, count.max(1));
        self.galaxy.sync_ships();
        self.world.clone().refresh_economy(&mut self.galaxy);
        id
    }

    /// The faction that owns neutral and pirate forces: `Neutral`, else `Hostile`, else `Pirates` if the
    /// campaign has it.
    pub fn neutral_faction(&self) -> Option<FactionId> {
        ["neutral", "hostile", "pirates"].iter().find_map(|n| self.world.faction_id(n))
    }

    /// Spawns defending ships for the neutral/pirate faction at `planet` (see [`Sim::neutral_faction`]).
    /// They fight any hostile fleet that arrives. Returns `None` if the campaign has no such faction.
    pub fn spawn_neutral_units(&mut self, planet: PlanetId, unit: UnitTypeId, count: u32) -> Option<FleetId> {
        let f = self.neutral_faction()?;
        Some(self.spawn_units(f, planet, unit, count))
    }

    // ---- buildings --------------------------------------------------------------------------

    /// Adds a structure to a planet if it has a free slot of the right kind (orbital or ground) and the
    /// structure's per-planet maximum is not reached. Returns whether it was added.
    pub fn add_structure(&mut self, planet: PlanetId, structure: StructureTypeId) -> bool {
        let w = self.world.clone();
        let (Some(info), Some(st)) = (w.planets.get(planet as usize), w.structures.get(structure as usize)) else { return false };
        let ps = &mut self.galaxy.planets[planet as usize];
        let same = ps.structures.iter().filter(|&&s| s == structure).count() as i32;
        let used = ps.structures.iter().filter(|&&s| w.structures[s as usize].orbital == st.orbital).count() as i32;
        let slots = if st.orbital { info.space_slots } else { info.ground_slots };
        if same >= st.max_per_planet || used >= slots {
            return false;
        }
        ps.structures.push(structure);
        w.refresh_economy(&mut self.galaxy);
        true
    }

    /// Removes one instance of a structure. Returns whether there was one.
    pub fn remove_structure(&mut self, planet: PlanetId, structure: StructureTypeId) -> bool {
        let Some(ps) = self.galaxy.planets.get_mut(planet as usize) else { return false };
        let Some(pos) = ps.structures.iter().position(|&s| s == structure) else { return false };
        ps.structures.remove(pos);
        self.world.clone().refresh_economy(&mut self.galaxy);
        true
    }

    /// Sets (or with `None` removes) the star base of a planet.
    pub fn set_starbase(&mut self, planet: PlanetId, level: Option<StarbaseTypeId>) {
        if let Some(ps) = self.galaxy.planets.get_mut(planet as usize) {
            ps.starbase = level;
            self.world.clone().refresh_economy(&mut self.galaxy);
        }
    }

    // ---- ownership ---------------------------------------------------------------------------

    /// Changes the owner like a capture does: build queues, the star base and all structures are lost,
    /// `PlanetCaptured` is emitted for a new owner, and elimination / victory are re-evaluated.
    /// `None` makes the planet neutral (no event).
    pub fn set_planet_owner(&mut self, planet: PlanetId, owner: Option<FactionId>) -> Vec<Event> {
        let mut events = Vec::new();
        if (planet as usize) < self.galaxy.planets.len() {
            self.apply_owner_change(planet, owner, false, &mut events);
        }
        events
    }

    /// Like [`Sim::set_planet_owner`] but keeps star base and structures (they change hands with the planet).
    pub fn set_planet_owner_keeping_buildings(&mut self, planet: PlanetId, owner: Option<FactionId>) -> Vec<Event> {
        let mut events = Vec::new();
        if (planet as usize) < self.galaxy.planets.len() {
            self.apply_owner_change(planet, owner, true, &mut events);
        }
        events
    }

    // ---- economy and tech ----------------------------------------------------------------------

    /// Adds (or with a negative amount removes) credits, respecting the per-planet credit cap and never
    /// going below zero.
    pub fn add_credits(&mut self, faction: FactionId, amount: f64) {
        let cap_per_planet = self.world.config.credit_cap_per_planet;
        if let Some(fs) = self.galaxy.factions.get_mut(faction as usize) {
            fs.credits += amount;
            let cap = cap_per_planet * fs.planets_owned as f64;
            if amount > 0.0 && fs.planets_owned > 0 && fs.credits > cap {
                fs.credits = cap.max(fs.credits - amount);
            }
            fs.credits = fs.credits.max(0.0);
        }
    }

    /// Sets the tech level (clamped to the faction's maximum). Raising it emits, per level, what an
    /// upgrade completion emits (`ConstructionComplete` with `ProductionKind::Tech` and `TechAdvanced`),
    /// so tech-level story plots fire. Lowering it is silent.
    pub fn set_tech_level(&mut self, faction: FactionId, level: i32) -> Vec<Event> {
        let mut events = Vec::new();
        let Some(info) = self.world.factions.get(faction as usize) else { return events };
        let level = level.min(info.max_tech);
        let start = info.start_tech;
        let home = self.galaxy.owned_planets(faction).next().unwrap_or(0);
        let fs = &mut self.galaxy.factions[faction as usize];
        while fs.tech < level {
            let idx = (fs.tech - start).max(0) as usize;
            fs.tech += 1;
            events.push(Event::ConstructionComplete { faction, planet: home, kind: ProductionKind::Tech(idx) });
            events.push(Event::TechAdvanced { faction, level: fs.tech });
        }
        fs.tech = level;
        events
    }

    /// Unlocks or locks an object (unit, structure) for a faction, as a research item would.
    pub fn set_unlocked(&mut self, faction: FactionId, object: &str, unlocked: bool) {
        if let Some(fs) = self.galaxy.factions.get_mut(faction as usize) {
            let key = object.trim().to_ascii_lowercase();
            if unlocked {
                fs.unlocked.insert(key);
            } else {
                fs.unlocked.remove(&key);
            }
        }
    }

    // ---- diplomacy --------------------------------------------------------------------------

    /// Makes two factions allies (no battles, no blockades, no capture between them) or enemies again
    /// (restoring the data's relation).
    pub fn set_alliance(&mut self, a: FactionId, b: FactionId, allied: bool) {
        if a == b {
            return;
        }
        let pair = (a.min(b), a.max(b));
        if allied {
            self.galaxy.allied.insert(pair);
        } else {
            self.galaxy.allied.remove(&pair);
        }
    }

    /// True if the two factions are currently allied.
    pub fn are_allied(&self, a: FactionId, b: FactionId) -> bool {
        a == b || self.galaxy.allied.contains(&(a.min(b), a.max(b)))
    }

    // ---- ship identities -----------------------------------------------------------------------

    /// The stable ids and types of the ships of a fleet.
    pub fn ships_of(&mut self, fleet: FleetId) -> Vec<(ShipId, UnitTypeId)> {
        self.galaxy.sync_ships();
        self.galaxy.ships.get(&fleet).cloned().unwrap_or_default()
    }

    /// The fleet a ship currently belongs to, and its type.
    pub fn locate_ship(&mut self, ship: ShipId) -> Option<(FleetId, UnitTypeId)> {
        self.galaxy.sync_ships();
        self.galaxy.ships.iter().find_map(|(f, list)| list.iter().find(|(s, _)| *s == ship).map(|&(_, u)| (*f, u)))
    }

    /// Removes a ship from the game. Returns whether it existed.
    pub fn despawn_ship(&mut self, ship: ShipId) -> bool {
        let Some((fleet, unit)) = self.locate_ship(ship) else { return false };
        if let Some(list) = self.galaxy.ships.get_mut(&fleet) {
            list.retain(|(s, _)| *s != ship);
        }
        if let Some(f) = self.galaxy.fleets.get_mut(&fleet) {
            if let Some(n) = f.units.get_mut(&unit) {
                *n -= 1;
                if *n == 0 {
                    f.units.remove(&unit);
                }
            }
            if f.units.is_empty() {
                self.galaxy.fleets.remove(&fleet);
            }
        }
        self.galaxy.sync_ships();
        self.world.clone().refresh_economy(&mut self.galaxy);
        true
    }

    /// Hands a ship over to another faction (`Change_Owner`): it joins that faction's idle fleet at the
    /// same planet. Only ships orbiting a planet can change owner. The ship keeps its id.
    pub fn change_ship_owner(&mut self, ship: ShipId, owner: FactionId) -> bool {
        let Some((fleet, unit)) = self.locate_ship(ship) else { return false };
        let Some(planet) = self.galaxy.fleets.get(&fleet).and_then(|f| f.planet()) else { return false };
        if self.galaxy.fleets[&fleet].owner == owner {
            return true;
        }
        // Detach from the old fleet.
        if let Some(list) = self.galaxy.ships.get_mut(&fleet) {
            list.retain(|(s, _)| *s != ship);
        }
        let f = self.galaxy.fleets.get_mut(&fleet).expect("located");
        let n = f.units.get_mut(&unit).expect("has the unit");
        *n -= 1;
        if *n == 0 {
            f.units.remove(&unit);
        }
        if f.units.is_empty() {
            self.galaxy.fleets.remove(&fleet);
        }
        // Attach to the new owner's fleet and give the existing id to the new unit.
        let target = add_units(&mut self.galaxy, owner, planet, unit, 1);
        self.galaxy.sync_ships();
        if let Some(list) = self.galaxy.ships.get_mut(&target)
            && let Some(last) = list.iter_mut().rev().find(|(_, t)| *t == unit)
        {
            last.0 = ship;
        }
        self.world.clone().refresh_economy(&mut self.galaxy);
        true
    }

    // ---- perception ------------------------------------------------------------------------------

    /// `Planet_Credit_Value` of a planet.
    pub fn planet_credit_value(&self, planet: PlanetId) -> f32 {
        self.world.planets.get(planet as usize).map_or(0.0, |p| p.credit_value)
    }

    /// `Planet_Surface_Accessible`: ground forces could land.
    pub fn planet_has_ground_access(&self, planet: PlanetId) -> bool {
        self.world.planets.get(planet as usize).is_some_and(|p| p.surface_accessible)
    }

    /// What `name` refers to: a catalog ship, structure or star base, or a ground unit / hero that the
    /// data defines but the simulation does not run.
    pub fn object_kind(&self, name: &str) -> ObjectKind {
        let key = name.trim().to_ascii_lowercase();
        if self.world.unit_index.contains_key(&key) {
            ObjectKind::SpaceUnit
        } else if self.world.starbase_index.contains_key(&key) {
            ObjectKind::Starbase
        } else if self.world.structure_index.contains_key(&key) {
            ObjectKind::Structure
        } else {
            self.world.unsimulated.get(&key).copied().unwrap_or(ObjectKind::Other)
        }
    }
}
