//! Everything that keeps one build item off a planet, as a list: the build menu shows every missing requirement at once (the game's
//! "Required : ..." lines), while [`World::check_build_unit`] and [`World::check_build_structure`] stop at the first.

use crate::state::*;
use crate::world::*;

/// One unmet build requirement of a unit or structure at a planet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unmet {
    /// The faction's tech level is below `Tech_Level`.
    TechLevel(i32),
    /// The planet's star base is below `Required_Star_Base_Level`.
    StarBaseLevel(i32),
    /// None of these structures (alternatives, `A | B`) stands on the planet.
    Structures(Vec<StructureTypeId>),
    /// The item can only be built on these planets (`Required_Planets`) and this is not one of them.
    Planets(Vec<PlanetId>),
    /// Story-locked or not yet unlocked by research.
    Locked,
    /// An upgrade needs this ship in orbit (`Required_Orbiting_Units`).
    OrbitingUnit(UnitTypeId),
}

impl World {
    /// Requirements of unit `unit` that `faction` does not meet at `planet` (empty = none missing; credits, population and limits are not requirements).
    pub fn unmet_for_unit(&self, galaxy: &Galaxy, faction: FactionId, planet: PlanetId, unit: UnitTypeId) -> Vec<Unmet> {
        let (Some(fs), Some(ps), Some(ut)) = (galaxy.factions.get(faction as usize), galaxy.planets.get(planet as usize), self.units.get(unit as usize)) else { return Vec::new() };
        let name = ut.name.to_ascii_lowercase();
        let mut out = Vec::new();
        if ut.tech > fs.tech {
            out.push(Unmet::TechLevel(ut.tech));
        }
        if fs.story_locked.contains(&name) || (ut.locked && !fs.unlocked.contains(&name)) {
            out.push(Unmet::Locked);
        }
        if !ut.req_planets.is_empty() && !ut.req_planets.contains(&planet) {
            out.push(Unmet::Planets(ut.req_planets.clone()));
        }
        if self.starbase_level(galaxy, planet) < ut.req_starbase {
            out.push(Unmet::StarBaseLevel(ut.req_starbase));
        }
        out.extend(self.unmet_structures(ps, &ut.req_structures, false));
        if let UnitRole::Upgrade { from, .. } = ut.role
            && !galaxy.fleets.values().any(|f| f.owner == faction && f.location == FleetLocation::AtPlanet(planet) && f.units.get(&from).is_some_and(|&n| n > 0))
        {
            out.push(Unmet::OrbitingUnit(from));
        }
        out
    }

    /// Requirements of structure `structure` that `faction` does not meet at `planet`.
    pub fn unmet_for_structure(&self, galaxy: &Galaxy, faction: FactionId, planet: PlanetId, structure: StructureTypeId) -> Vec<Unmet> {
        let (Some(fs), Some(ps), Some(st)) = (galaxy.factions.get(faction as usize), galaxy.planets.get(planet as usize), self.structures.get(structure as usize)) else { return Vec::new() };
        let name = st.name.to_ascii_lowercase();
        let mut out = Vec::new();
        if st.tech > fs.tech {
            out.push(Unmet::TechLevel(st.tech));
        }
        if fs.story_locked.contains(&name) || (st.locked && !fs.unlocked.contains(&name)) {
            out.push(Unmet::Locked);
        }
        if !st.req_planets.is_empty() && !st.req_planets.contains(&planet) {
            out.push(Unmet::Planets(st.req_planets.clone()));
        }
        if self.starbase_level(galaxy, planet) < st.req_starbase {
            out.push(Unmet::StarBaseLevel(st.req_starbase));
        }
        out.extend(self.unmet_structures(ps, &st.req_structures, true));
        out
    }

    /// The requirement groups no structure of `ps` meets (structures: also none queued before the item, as in `check_build_structure`).
    fn unmet_structures(&self, ps: &PlanetState, groups: &[Vec<StructureTypeId>], count_queued: bool) -> Vec<Unmet> {
        let queued: Vec<StructureTypeId> = ps.queues[Lane::Structures as usize]
            .iter()
            .filter(|_| count_queued)
            .filter_map(|i| match i.kind {
                ProductionKind::Structure(s) => Some(s),
                _ => None,
            })
            .collect();
        let met = |r: StructureTypeId| self.has_structure(ps, r) || queued.iter().any(|&s| s == r || self.structures[s as usize].ancestors.contains(&r));
        groups.iter().filter(|g| !g.iter().any(|&r| met(r))).map(|g| Unmet::Structures(g.clone())).collect()
    }
}
