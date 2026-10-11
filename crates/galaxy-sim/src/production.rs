//! Build requirement checks, economy and population bookkeeping.

use crate::state::*;
use crate::world::*;

/// Used and total slots of one kind on a planet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotUse {
    /// Finished structures.
    pub built: i32,
    /// Structures in the production queue.
    pub queued: i32,
    /// Slots the planet has.
    pub total: i32,
}

impl SlotUse {
    /// Slots still free for new orders.
    pub fn free(&self) -> i32 {
        self.total - self.built - self.queued
    }
}

/// Building capacity of one planet, see [`World::planet_capacity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanetCapacity {
    /// Orbital building slots (the star base is separate).
    pub space: SlotUse,
    /// Ground building slots.
    pub ground: SlotUse,
    /// Current star base level (0 = none); a planet holds one star base.
    pub starbase_level: i32,
    /// Highest star base level this planet can get.
    pub starbase_max: i32,
}

impl World {
    /// True if the planet has a structure that is, or inherits from, structure type `req`.
    pub fn has_structure(&self, ps: &PlanetState, req: StructureTypeId) -> bool {
        ps.structures.iter().any(|&s| s == req || self.structures[s as usize].ancestors.contains(&req))
    }

    /// Names of the structure groups `ps` still lacks (object names, alternatives joined by ` | `, groups by `, `); `None` if all are met.
    pub fn missing_structures(&self, ps: &PlanetState, groups: &[Vec<StructureTypeId>]) -> Option<String> {
        let missing: Vec<String> = groups
            .iter()
            .filter(|g| !g.iter().any(|&r| self.has_structure(ps, r)))
            .map(|g| g.iter().map(|&r| self.structures[r as usize].name.as_str()).collect::<Vec<_>>().join(" | "))
            .collect();
        (!missing.is_empty()).then(|| missing.join(", "))
    }

    /// Like [`World::missing_structures`], but structures waiting in the planet's structure queue count as present: the queue is
    /// worked off in order, so a requirement queued before an item is built by the time the item starts.
    pub fn missing_structures_queued(&self, ps: &PlanetState, groups: &[Vec<StructureTypeId>]) -> Option<String> {
        let queued: Vec<StructureTypeId> = ps.queues[Lane::Structures as usize]
            .iter()
            .filter_map(|i| match i.kind {
                ProductionKind::Structure(s) => Some(s),
                _ => None,
            })
            .collect();
        let met = |r: StructureTypeId| self.has_structure(ps, r) || queued.iter().any(|&s| s == r || self.structures[s as usize].ancestors.contains(&r));
        let missing: Vec<String> = groups
            .iter()
            .filter(|g| !g.iter().any(|&r| met(r)))
            .map(|g| g.iter().map(|&r| self.structures[r as usize].name.as_str()).collect::<Vec<_>>().join(" | "))
            .collect();
        (!missing.is_empty()).then(|| missing.join(", "))
    }

    /// Highest star base level the planet can hold (`Max_Space_Base`; planets without the tag take every level in the catalog).
    pub fn max_starbase_level(&self, planet: PlanetId) -> i32 {
        match self.planets[planet as usize].max_space_base {
            m if m > 0 => m,
            _ => self.starbases.iter().map(|b| b.level).max().unwrap_or(0),
        }
    }

    /// True if a star base of `level` fits on the planet (items that need a higher level can never be built there).
    pub fn planet_takes_starbase(&self, planet: PlanetId, level: i32) -> bool {
        level <= self.max_starbase_level(planet)
    }

    /// What a planet can hold: one star base (levels 1..max), orbital buildings and ground buildings, each counted from the data
    /// (`Max_Space_Base`, `Special_Structures_Space`, `Special_Structures_Land`). Queued structures take a slot already.
    pub fn planet_capacity(&self, galaxy: &Galaxy, planet: PlanetId) -> PlanetCapacity {
        let ps = &galaxy.planets[planet as usize];
        let info = &self.planets[planet as usize];
        let count = |orbital: bool, total: i32| SlotUse {
            built: ps.structures.iter().filter(|&&s| self.structures[s as usize].orbital == orbital).count() as i32,
            queued: ps.queues[Lane::Structures as usize]
                .iter()
                .filter(|i| matches!(i.kind, ProductionKind::Structure(s) if self.structures[s as usize].orbital == orbital))
                .count() as i32,
            total,
        };
        PlanetCapacity {
            space: count(true, info.space_slots),
            ground: count(false, info.ground_slots),
            starbase_level: self.starbase_level(galaxy, planet),
            starbase_max: self.max_starbase_level(planet),
        }
    }

    /// Star base level of a planet (0 = none).
    pub fn starbase_level(&self, galaxy: &Galaxy, planet: PlanetId) -> i32 {
        galaxy.planets[planet as usize].starbase.map_or(0, |s| self.starbases[s as usize].level)
    }

    fn queued<'a>(&self, galaxy: &'a Galaxy, faction: FactionId) -> impl Iterator<Item = &'a ProductionItem> {
        galaxy
            .planets
            .iter()
            .filter(move |p| p.owner == Some(faction))
            .flat_map(|p| p.queues.iter().flat_map(|q| q.iter()))
    }

    /// Population currently reserved by queued units.
    fn queued_population(&self, galaxy: &Galaxy, faction: FactionId) -> i32 {
        self.queued(galaxy, faction)
            .filter_map(|i| match i.kind {
                ProductionKind::Unit(u) => Some(self.units[u as usize].population),
                _ => None,
            })
            .sum()
    }

    /// Checks whether `faction` could queue `unit` on `planet` right now.
    /// Price and build time (unscaled seconds) of `unit` for `faction` at `planet` after the production discounts of
    /// its heroes and ships anywhere (Mothma, taskforce cruisers) and of the planet's structures (Starforge).
    /// Abilities in the same `Stacking_Category` do not add up, the strongest wins; categories multiply (estimate:
    /// the data names the categories but not how they combine). Heroes count wherever they are (estimate).
    pub fn unit_cost(&self, galaxy: &Galaxy, faction: FactionId, planet: PlanetId, unit: UnitTypeId) -> (f32, f32) {
        use std::collections::BTreeMap;
        let ut = &self.units[unit as usize];
        let mut price: BTreeMap<i32, f32> = BTreeMap::new();
        let mut time: BTreeMap<i32, f32> = BTreeMap::new();
        let mut add = |mods: &[ProductionMod]| {
            for m in mods.iter().filter(|m| m.applies(&ut.name, ut.category)) {
                if m.price != 0.0 {
                    let e = price.entry(m.stacking).or_insert(0.0);
                    *e = e.max(m.price);
                }
                if m.time != 0.0 {
                    let e = time.entry(m.stacking).or_insert(0.0);
                    *e = e.max(m.time);
                }
            }
        };
        for f in galaxy.fleets.values().filter(|f| f.owner == faction) {
            for (&u, &n) in &f.units {
                if n > 0 {
                    add(&self.units[u as usize].prod_mods);
                }
            }
        }
        let ps = &galaxy.planets[planet as usize];
        if ps.owner == Some(faction) {
            // Planet-specific shipyard abilities (Kuat, Rendili, Rothana, Sullust, Vergesso, etc.)
            add(&self.planets[planet as usize].prod_mods);
            // Orbiting star base modifiers
            if let Some(sb) = ps.starbase {
                add(&self.starbases[sb as usize].prod_mods);
            }
            // Planetary structures (docks, shipyards, academies)
            for &s in &ps.structures {
                add(&self.structures[s as usize].prod_mods);
            }
        }
        let apply = |base: f32, m: BTreeMap<i32, f32>, floor: f32| m.values().fold(base, |c, r| c * (1.0 - r.min(1.0))).max(base * floor);
        (apply(ut.cost, price, 0.0), apply(ut.build_time, time, 0.01))
    }

    pub fn check_build_unit(&self, galaxy: &Galaxy, faction: FactionId, planet: PlanetId, unit: UnitTypeId) -> Result<(), CommandError> {
        let fs = galaxy.factions.get(faction as usize).ok_or(CommandError::Invalid)?;
        let ps = galaxy.planets.get(planet as usize).ok_or(CommandError::Invalid)?;
        let ut = self.units.get(unit as usize).ok_or(CommandError::Invalid)?;
        if fs.eliminated {
            return Err(CommandError::Eliminated);
        }
        if ps.owner != Some(faction) {
            return Err(CommandError::NotOwner);
        }
        if self.buildable_units[faction as usize].binary_search(&unit).is_err() {
            return Err(CommandError::NotAvailable);
        }
        if ps.queues[Lane::Units as usize].len() >= self.config.max_queue_len {
            return Err(CommandError::NoSlot);
        }
        if ut.tech > fs.tech {
            return Err(CommandError::RequirementsNotMet(format!("tech level {}", ut.tech)));
        }
        if ut.locked && !fs.unlocked.contains(&ut.name.to_ascii_lowercase()) {
            return Err(CommandError::RequirementsNotMet("locked".into()));
        }
        if (!ut.req_planets.is_empty() && !ut.req_planets.contains(&planet)) || !self.planet_takes_starbase(planet, ut.req_starbase) {
            return Err(CommandError::RequirementsNotMet("planet".into()));
        }
        if self.starbase_level(galaxy, planet) < ut.req_starbase {
            return Err(CommandError::RequirementsNotMet(format!("star base level {}", ut.req_starbase)));
        }
        if let Some(missing) = self.missing_structures(ps, &ut.req_structures) {
            return Err(CommandError::RequirementsNotMet(missing));
        }
        if ut.limit_current.is_some() || ut.limit_lifetime.is_some() {
            let alive: u32 = galaxy.fleets.values().filter(|f| f.owner == faction).map(|f| f.units.get(&unit).copied().unwrap_or(0)).sum();
            let queued = self
                .queued(galaxy, faction)
                .filter(|i| i.kind == ProductionKind::Unit(unit))
                .count() as u32;
            // A negative limit (`-1`) means no limit.
            if let Some(l) = ut.limit_current.filter(|l| *l >= 0)
                && alive + queued >= l as u32
            {
                return Err(CommandError::BuildLimit);
            }
            if let Some(l) = ut.limit_lifetime.filter(|l| *l >= 0)
                && fs.built_lifetime.get(&unit).copied().unwrap_or(0) + queued >= l as u32
            {
                return Err(CommandError::BuildLimit);
            }
        }
        if let UnitRole::Research(target) = &ut.role
            && fs.unlocked.contains(&target.to_ascii_lowercase())
        {
            return Err(CommandError::BuildLimit);
        }
        if ut.population > 0 && fs.population_used + self.queued_population(galaxy, faction) + ut.population > fs.population_cap {
            return Err(CommandError::PopulationCap);
        }
        if fs.credits < f64::from(self.unit_cost(galaxy, faction, planet, unit).0) {
            return Err(CommandError::InsufficientCredits);
        }
        Ok(())
    }

    /// Checks whether `faction` could queue `structure` on `planet` right now.
    pub fn check_build_structure(
        &self,
        galaxy: &Galaxy,
        faction: FactionId,
        planet: PlanetId,
        structure: StructureTypeId,
    ) -> Result<(), CommandError> {
        let fs = galaxy.factions.get(faction as usize).ok_or(CommandError::Invalid)?;
        let ps = galaxy.planets.get(planet as usize).ok_or(CommandError::Invalid)?;
        let st = self.structures.get(structure as usize).ok_or(CommandError::Invalid)?;
        if fs.eliminated {
            return Err(CommandError::Eliminated);
        }
        if ps.owner != Some(faction) {
            return Err(CommandError::NotOwner);
        }
        if self.buildable_structures[faction as usize].binary_search(&structure).is_err() {
            return Err(CommandError::NotAvailable);
        }
        if ps.queues[Lane::Structures as usize].len() >= self.config.max_queue_len {
            return Err(CommandError::NoSlot);
        }
        if st.tech > fs.tech {
            return Err(CommandError::RequirementsNotMet(format!("tech level {}", st.tech)));
        }
        if st.locked && !fs.unlocked.contains(&st.name.to_ascii_lowercase()) {
            return Err(CommandError::RequirementsNotMet("locked".into()));
        }
        if (!st.req_planets.is_empty() && !st.req_planets.contains(&planet)) || !self.planet_takes_starbase(planet, st.req_starbase) {
            return Err(CommandError::RequirementsNotMet("planet".into()));
        }
        if self.starbase_level(galaxy, planet) < st.req_starbase {
            return Err(CommandError::RequirementsNotMet(format!("star base level {}", st.req_starbase)));
        }
        if let Some(missing) = self.missing_structures_queued(ps, &st.req_structures) {
            return Err(CommandError::RequirementsNotMet(missing));
        }
        let queued_same = ps.queues[Lane::Structures as usize].iter().filter(|i| i.kind == ProductionKind::Structure(structure)).count();
        let existing_same = ps.structures.iter().filter(|&&s| s == structure).count();
        if (queued_same + existing_same) as i32 >= st.max_per_planet {
            return Err(CommandError::NoSlot);
        }
        let capacity = self.planet_capacity(galaxy, planet);
        if (if st.orbital { capacity.space } else { capacity.ground }).free() <= 0 {
            return Err(CommandError::NoSlot);
        }
        if fs.credits < st.cost as f64 {
            return Err(CommandError::InsufficientCredits);
        }
        Ok(())
    }

    /// Star base type that the next star base order on this planet would build.
    pub fn next_starbase(&self, galaxy: &Galaxy, faction: FactionId, planet: PlanetId) -> Option<StarbaseTypeId> {
        match galaxy.planets[planet as usize].starbase {
            None => self.first_starbase[faction as usize],
            Some(cur) => self.starbases[cur as usize].next,
        }
    }

    /// Checks whether the star base can be built/upgraded.
    pub fn check_upgrade_starbase(&self, galaxy: &Galaxy, faction: FactionId, planet: PlanetId) -> Result<StarbaseTypeId, CommandError> {
        let fs = galaxy.factions.get(faction as usize).ok_or(CommandError::Invalid)?;
        let ps = galaxy.planets.get(planet as usize).ok_or(CommandError::Invalid)?;
        if fs.eliminated {
            return Err(CommandError::Eliminated);
        }
        if ps.owner != Some(faction) {
            return Err(CommandError::NotOwner);
        }
        if !ps.queues[Lane::Base as usize].is_empty() {
            return Err(CommandError::NoSlot);
        }
        let next = self.next_starbase(galaxy, faction, planet).ok_or(CommandError::NotAvailable)?;
        let sb = &self.starbases[next as usize];
        let limit = self.planets[planet as usize].max_space_base;
        if limit > 0 && sb.level > limit {
            return Err(CommandError::RequirementsNotMet(format!("this planet takes star bases up to level {limit}")));
        }
        if sb.tech > fs.tech {
            return Err(CommandError::RequirementsNotMet(format!("tech level {}", sb.tech)));
        }
        if fs.credits < sb.cost as f64 {
            return Err(CommandError::InsufficientCredits);
        }
        Ok(next)
    }

    /// Checks whether the next tech level can be bought at this planet.
    pub fn check_upgrade_tech(&self, galaxy: &Galaxy, faction: FactionId, planet: PlanetId) -> Result<usize, CommandError> {
        let fs = galaxy.factions.get(faction as usize).ok_or(CommandError::Invalid)?;
        let ps = galaxy.planets.get(planet as usize).ok_or(CommandError::Invalid)?;
        let info = &self.factions[faction as usize];
        if fs.eliminated {
            return Err(CommandError::Eliminated);
        }
        if ps.owner != Some(faction) {
            return Err(CommandError::NotOwner);
        }
        if !ps.queues[Lane::Base as usize].is_empty() {
            return Err(CommandError::NoSlot);
        }
        if galaxy.planets.iter().any(|p| p.owner == Some(faction) && p.queues[Lane::Base as usize].iter().any(|i| matches!(i.kind, ProductionKind::Tech(_)))) {
            return Err(CommandError::NoSlot);
        }
        let idx = (fs.tech - info.start_tech).max(0) as usize;
        if fs.tech >= info.max_tech || idx >= info.tech_upgrades.len() {
            return Err(CommandError::NotAvailable);
        }
        if self.starbase_level(galaxy, planet) < 1 {
            return Err(CommandError::RequirementsNotMet("star base".into()));
        }
        if fs.credits < info.tech_upgrades[idx].cost as f64 {
            return Err(CommandError::InsufficientCredits);
        }
        Ok(idx)
    }

    /// Weekly income of one planet for its owner (before the faction accumulation factor).
    pub fn planet_income(&self, galaxy: &Galaxy, planet: PlanetId) -> f32 {
        let ps = &galaxy.planets[planet as usize];
        let base = self.planets[planet as usize].credit_value;
        let (mut pct, mut abs) = (ps.gamble, 0.0f32);
        let mining = self.planets[planet as usize].mining_colony;
        for &s in &ps.structures {
            let st = &self.structures[s as usize];
            pct += st.income_pct;
            abs += st.income_abs;
            if mining {
                pct += st.income_pct_mining;
                abs += st.income_abs_mining;
            }
        }
        if let Some(sb) = ps.starbase {
            let sb = &self.starbases[sb as usize];
            pct += sb.income_pct;
            abs += sb.income_abs;
        }
        base * (1.0 + pct) + abs
    }

    /// Recomputes income, population use and capacity of every faction.
    pub(crate) fn refresh_economy(&self, galaxy: &mut Galaxy) {
        let n = self.factions.len();
        let mut income = vec![0.0f64; n];
        let mut cap = vec![self.config.base_population_cap; n];
        let mut owned = vec![0u32; n];
        for (i, ps) in galaxy.planets.iter().enumerate() {
            let Some(o) = ps.owner else { continue };
            let o = o as usize;
            owned[o] += 1;
            income[o] += self.planet_income(galaxy, i as PlanetId) as f64;
            cap[o] += self.planets[i].population_capacity;
            if let Some(sb) = ps.starbase {
                cap[o] += self.starbases[sb as usize].population_capacity;
            }
            cap[o] += ps.structures.iter().map(|&s| self.structures[s as usize].population_capacity).sum::<i32>();
        }
        let mut used = vec![0i32; n];
        for f in galaxy.fleets.values() {
            for (&u, &c) in &f.units {
                let ut = &self.units[u as usize];
                income[f.owner as usize] += (ut.upkeep * c as f32) as f64;
                used[f.owner as usize] += ut.population * c as i32;
            }
        }
        for (i, fs) in galaxy.factions.iter_mut().enumerate() {
            fs.income_per_week = income[i] * self.factions[i].credits_factor as f64;
            fs.population_cap = cap[i];
            fs.population_used = used[i];
            fs.planets_owned = owned[i];
        }
    }
}

/// Why a superweapon cannot fire (see [`World::can_destroy_planet`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestroyRefusal {
    /// Unknown faction, fleet or planet, or an eliminated faction.
    Invalid,
    /// The fleet holds no unit with a `Planet_Destruction_Ability`.
    NoSuperweapon,
    /// The fleet is not orbiting the planet (or is not the faction's).
    NotInOrbit,
    /// The faction's superweapons are still recharging.
    Recharging,
    /// `Planet_Is_Destroyable_By_TSW` is No, or the location is not a real planet.
    Protected,
    /// Already an asteroid field.
    AlreadyDestroyed,
    /// The planet belongs to the firer or an ally.
    Friendly,
}

impl From<DestroyRefusal> for CommandError {
    fn from(r: DestroyRefusal) -> Self {
        match r {
            DestroyRefusal::Invalid => CommandError::Invalid,
            DestroyRefusal::NoSuperweapon | DestroyRefusal::NotInOrbit => CommandError::NotAvailable,
            _ => CommandError::WrongState,
        }
    }
}

impl World {
    /// Whether `fleet` of `faction` may fire its superweapon at `planet` right now. The UI uses this to enable the button; `Command::DestroyPlanet` checks the same.
    pub fn can_destroy_planet(&self, galaxy: &Galaxy, faction: FactionId, fleet: FleetId, planet: PlanetId) -> Result<(), DestroyRefusal> {
        let fs = galaxy.factions.get(faction as usize).filter(|f| !f.eliminated).ok_or(DestroyRefusal::Invalid)?;
        let info = self.planets.get(planet as usize).ok_or(DestroyRefusal::Invalid)?;
        let f = galaxy.fleets.get(&fleet).filter(|f| f.owner == faction).ok_or(DestroyRefusal::NotInOrbit)?;
        if f.location != FleetLocation::AtPlanet(planet) {
            return Err(DestroyRefusal::NotInOrbit);
        }
        if !f.units.iter().any(|(&u, &n)| n > 0 && self.units[u as usize].planet_destroyer) {
            return Err(DestroyRefusal::NoSuperweapon);
        }
        let ps = &galaxy.planets[planet as usize];
        if ps.destroyed {
            return Err(DestroyRefusal::AlreadyDestroyed);
        }
        if !self.real[planet as usize] || !info.destroyable {
            return Err(DestroyRefusal::Protected);
        }
        if ps.owner.is_some_and(|o| o == faction || !galaxy.hostile(self, faction, o)) {
            return Err(DestroyRefusal::Friendly);
        }
        if galaxy.time < fs.superweapon_ready_at {
            return Err(DestroyRefusal::Recharging);
        }
        Ok(())
    }
}
