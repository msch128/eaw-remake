//! [`AiSim`] on top of galaxy-sim, used by the Remake AI (`galaxy-ai`).

use std::sync::Arc;

use galaxy_ai::sim::{AiSim, FleetView, PlanetId, PlayerId, QueueView, Refused, StructureInfo, UnitInfo};
use galaxy_sim::{Command, CommandError, CommandOk, FleetLocation, Lane, ProductionKind, Sim, UnitRole, World};

/// Ownership encoding of planet script objects (see `host.rs`).
pub const PLANET_OBJECT_TAG: u64 = 1 << 56;

/// Borrowed view of the simulation for one AI call.
pub struct SimAdapter<'a> {
    /// The simulation.
    pub sim: &'a mut Sim,
    /// World data.
    pub world: Arc<World>,
    /// Human faction (not AI controlled).
    pub human: u32,
    /// Alliances made by scripts.
    pub allies: &'a std::collections::HashSet<(u32, u32)>,
    /// Sim events caused by AI actions, reported with the next step.
    pub pending: &'a mut Vec<galaxy_sim::Event>,
}

fn power_of(w: &World, u: u32) -> f64 {
    let t = &w.units[u as usize];
    if t.ai_power >= 1.0 { f64::from(t.ai_power) } else { f64::from(t.cost) * 0.5 }
}

fn refuse(e: CommandError) -> Refused {
    match e {
        CommandError::InsufficientCredits => Refused::NoCredits,
        CommandError::Invalid => Refused::Invalid,
        _ => Refused::Unavailable,
    }
}

impl SimAdapter<'_> {
    fn view(&self, f: &galaxy_sim::Fleet) -> FleetView {
        FleetView {
            id: f.id,
            owner: f.owner,
            at: f.planet(),
            heading_to: match &f.location {
                FleetLocation::Moving { to, .. } => Some(*to),
                FleetLocation::AtPlanet(_) => None,
            },
            units: f.units.iter().map(|(u, c)| (self.world.units[*u as usize].name.clone(), *c)).collect(),
        }
    }
}

impl AiSim for SimAdapter<'_> {
    fn time(&self) -> f64 {
        self.sim.galaxy().time
    }
    fn player_count(&self) -> u32 {
        self.world.factions.len() as u32
    }
    fn faction_name(&self, p: PlayerId) -> String {
        self.world.factions[p as usize].name.clone()
    }
    fn is_human(&self, p: PlayerId) -> bool {
        p == self.human
    }
    fn is_eliminated(&self, p: PlayerId) -> bool {
        self.sim.galaxy().factions[p as usize].eliminated
    }
    fn is_enemy(&self, a: PlayerId, b: PlayerId) -> bool {
        a != b && !self.allies.contains(&(a.min(b), a.max(b))) && self.world.factions[a as usize].enemies.contains(&b)
    }
    fn week_seconds(&self) -> f64 {
        f64::from(self.world.config.week_seconds)
    }
    fn credits(&self, p: PlayerId) -> f64 {
        self.sim.galaxy().factions[p as usize].credits
    }
    fn income_per_week(&self, p: PlayerId) -> f64 {
        self.sim.galaxy().factions[p as usize].income_per_week
    }
    fn tech_level(&self, p: PlayerId) -> i32 {
        self.sim.galaxy().factions[p as usize].tech
    }
    fn max_tech_level(&self, p: PlayerId) -> i32 {
        self.world.factions[p as usize].max_tech
    }
    fn planet_count(&self) -> u32 {
        self.world.planets.len() as u32
    }
    fn is_real(&self, pl: PlanetId) -> bool {
        self.world.real[pl as usize]
    }
    fn planet_name(&self, pl: PlanetId) -> String {
        self.world.planets[pl as usize].name.clone()
    }
    fn planet_position(&self, pl: PlanetId) -> [f32; 3] {
        self.world.planets[pl as usize].position
    }
    fn planet_owner(&self, pl: PlanetId) -> Option<PlayerId> {
        self.sim.galaxy().planets[pl as usize].owner
    }
    fn neighbors(&self, pl: PlanetId) -> Vec<PlanetId> {
        self.world.adjacency[pl as usize].iter().map(|(n, _)| *n).collect()
    }
    fn planet_income(&self, pl: PlanetId) -> f64 {
        f64::from(self.world.planets[pl as usize].credit_value)
    }
    fn structure_slots(&self, pl: PlanetId) -> (i32, i32) {
        let p = &self.world.planets[pl as usize];
        (p.space_slots, p.ground_slots)
    }
    fn structures(&self, pl: PlanetId) -> Vec<String> {
        self.sim.galaxy().planets[pl as usize].structures.iter().map(|s| self.world.structures[*s as usize].name.clone()).collect()
    }
    fn starbase_level(&self, pl: PlanetId) -> (i32, i32) {
        let level = self.sim.galaxy().planets[pl as usize].starbase.map_or(0, |sb| self.world.starbases[sb as usize].level);
        let max = self.world.starbases.iter().map(|s| s.level).max().unwrap_or(0);
        (level, max)
    }
    fn build_queue(&self, pl: PlanetId) -> Vec<QueueView> {
        let ps = &self.sim.galaxy().planets[pl as usize];
        let mut out = Vec::new();
        for lane in [Lane::Units, Lane::Structures, Lane::Base] {
            for it in &ps.queues[lane as usize] {
                let name = match &it.kind {
                    ProductionKind::Unit(u) => self.world.units[*u as usize].name.clone(),
                    ProductionKind::Structure(s) => self.world.structures[*s as usize].name.clone(),
                    ProductionKind::Starbase(s) => self.world.starbases[*s as usize].name.clone(),
                    ProductionKind::Tech(n) => format!("tech_{n}"),
                };
                out.push(QueueView { name, remaining: f64::from(it.remaining) });
            }
        }
        out
    }
    fn fleets_at(&self, pl: PlanetId) -> Vec<FleetView> {
        self.sim.galaxy().fleets_at(pl).map(|f| self.view(f)).collect()
    }
    fn fleet(&self, id: u32) -> Option<FleetView> {
        // A fleet that was merged into another (the three-slot rule) is seen as the fleet it became, with that fleet's id.
        let g = self.sim.galaxy();
        g.successor(id).and_then(|s| g.fleets.get(&s)).map(|f| self.view(f))
    }
    fn fleets_of(&self, p: PlayerId) -> Vec<FleetView> {
        self.sim.galaxy().fleets.values().filter(|f| f.owner == p).map(|f| self.view(f)).collect()
    }
    fn unit_info(&self, name: &str) -> Option<UnitInfo> {
        let &u = self.world.unit_index.get(&name.to_ascii_lowercase())?;
        let t = &self.world.units[u as usize];
        Some(UnitInfo {
            name: t.name.clone(),
            category: format!("{:?}", t.category).to_ascii_uppercase(),
            power: power_of(&self.world, u),
            cost: f64::from(t.cost),
            is_hero: t.limit_current == Some(1) || t.limit_lifetime == Some(1),
        })
    }
    fn buildable_units(&self, p: PlayerId) -> Vec<String> {
        self.world.buildable_units[p as usize]
            .iter()
            .filter(|&&u| matches!(self.world.units[u as usize].role, UnitRole::Ship))
            .map(|&u| self.world.units[u as usize].name.clone())
            .collect()
    }
    fn is_starbase_type(&self, name: &str) -> bool {
        self.world.starbase_index.contains_key(&name.to_ascii_lowercase())
    }
    fn structure_info(&self, name: &str) -> Option<StructureInfo> {
        let &s = self.world.structure_index.get(&name.to_ascii_lowercase())?;
        let t = &self.world.structures[s as usize];
        Some(StructureInfo { name: t.name.clone(), cost: f64::from(t.cost), orbital: t.orbital })
    }
    fn buildable_structures(&self, p: PlayerId) -> Vec<String> {
        self.world.buildable_structures[p as usize].iter().map(|&s| self.world.structures[s as usize].name.clone()).collect()
    }
    fn can_build_unit(&self, p: PlayerId, pl: PlanetId, unit: &str) -> Result<(), Refused> {
        let &u = self.world.unit_index.get(&unit.to_ascii_lowercase()).ok_or(Refused::Invalid)?;
        self.world.check_build_unit(self.sim.galaxy(), p, pl, u).map_err(refuse)
    }
    fn population_room(&self, p: PlayerId) -> f64 {
        f64::from(self.world.population_room(self.sim.galaxy(), p))
    }
    fn unit_population(&self, unit: &str) -> f64 {
        self.world.unit_index.get(&unit.to_ascii_lowercase()).map_or(0.0, |&u| f64::from(self.world.units[u as usize].population))
    }
    fn explain_build_unit(&self, p: PlayerId, pl: PlanetId, unit: &str) -> String {
        let Some(&u) = self.world.unit_index.get(&unit.to_ascii_lowercase()) else { return "unknown unit".into() };
        match self.world.check_build_unit(self.sim.galaxy(), p, pl, u) {
            Ok(()) => "ok".into(),
            Err(CommandError::RequirementsNotMet(what)) => format!("needs {what}"),
            Err(e) => format!("{e:?}"),
        }
    }
    fn can_build_structure(&self, p: PlayerId, pl: PlanetId, structure: &str) -> Result<(), Refused> {
        let &s = self.world.structure_index.get(&structure.to_ascii_lowercase()).ok_or(Refused::Invalid)?;
        self.world.check_build_structure(self.sim.galaxy(), p, pl, s).map_err(refuse)
    }
    fn can_upgrade_starbase(&self, p: PlayerId, pl: PlanetId) -> Result<(), Refused> {
        self.world.check_upgrade_starbase(self.sim.galaxy(), p, pl).map(|_| ()).map_err(refuse)
    }
    fn starbase_upgrade_cost(&self, p: PlayerId, pl: PlanetId) -> Option<f64> {
        let next = self.world.check_upgrade_starbase(self.sim.galaxy(), p, pl).ok()?;
        Some(f64::from(self.world.starbases[next as usize].cost))
    }
    fn queue_unit(&mut self, p: PlayerId, pl: PlanetId, unit: &str) -> Result<f64, Refused> {
        let &u = self.world.unit_index.get(&unit.to_ascii_lowercase()).ok_or(Refused::Invalid)?;
        self.sim.apply(Command::BuildUnit { faction: p, planet: pl, unit: u }).map_err(refuse)?;
        Ok(f64::from(self.world.units[u as usize].cost))
    }
    fn queue_structure(&mut self, p: PlayerId, pl: PlanetId, structure: &str) -> Result<f64, Refused> {
        let &s = self.world.structure_index.get(&structure.to_ascii_lowercase()).ok_or(Refused::Invalid)?;
        self.sim.apply(Command::BuildStructure { faction: p, planet: pl, structure: s }).map_err(refuse)?;
        Ok(f64::from(self.world.structures[s as usize].cost))
    }
    fn queue_starbase(&mut self, p: PlayerId, pl: PlanetId) -> Result<f64, Refused> {
        let cost = self.starbase_upgrade_cost(p, pl).unwrap_or(0.0);
        self.sim.apply(Command::UpgradeStarbase { faction: p, planet: pl }).map_err(refuse)?;
        Ok(cost)
    }
    fn queue_tech(&mut self, p: PlayerId, pl: PlanetId) -> Result<f64, Refused> {
        self.sim.apply(Command::UpgradeTech { faction: p, planet: pl }).map_err(refuse)?;
        Ok(0.0)
    }
    fn move_fleet(&mut self, p: PlayerId, fleet: u32, to: PlanetId) -> Result<(), Refused> {
        let fleet = self.sim.galaxy().successor(fleet).unwrap_or(fleet);
        self.sim.apply(Command::MoveFleetAround { faction: p, fleet, to }).map(|_| ()).map_err(refuse)
    }
    fn move_fleet_into(&mut self, p: PlayerId, fleet: u32, to: PlanetId, into: u32) -> Result<(), Refused> {
        let g = self.sim.galaxy();
        let (fleet, into) = (g.successor(fleet).unwrap_or(fleet), g.successor(into).unwrap_or(into));
        let slot = g.fleets.get(&into).filter(|f| f.planet() == Some(to)).map(|f| f.slot).filter(|s| (1..=galaxy_sim::FLEET_SLOTS).contains(s));
        match slot {
            Some(slot) => self.sim.apply(Command::MoveFleetAroundToSlot { faction: p, fleet, to, slot }).map(|_| ()).map_err(refuse),
            None => self.sim.apply(Command::MoveFleetAround { faction: p, fleet, to }).map(|_| ()).map_err(refuse),
        }
    }
    fn transfer_unit(&mut self, p: PlayerId, from: u32, to: u32, unit: &str) -> Result<(), Refused> {
        let &u = self.world.unit_index.get(&unit.to_ascii_lowercase()).ok_or(Refused::Invalid)?;
        let g = self.sim.galaxy();
        let (from, to) = (g.successor(from).unwrap_or(from), g.successor(to).unwrap_or(to));
        self.sim.apply(Command::TransferUnits { faction: p, from, to: Some(to), units: vec![(u, 1)], hyperspace: false }).map(|_| ()).map_err(refuse)
    }
    fn split_fleet(&mut self, p: PlayerId, fleet: u32, units: &[(String, u32)]) -> Result<u32, Refused> {
        let fleet = self.sim.galaxy().successor(fleet).unwrap_or(fleet);
        let mut list = Vec::new();
        for (n, c) in units {
            let &u = self.world.unit_index.get(&n.to_ascii_lowercase()).ok_or(Refused::Invalid)?;
            list.push((u, *c));
        }
        match self.sim.apply(Command::SplitFleet { faction: p, fleet, units: list }).map_err(refuse)? {
            CommandOk::NewFleet(f) => Ok(f),
            _ => Err(Refused::Invalid),
        }
    }
    fn merge_fleets(&mut self, p: PlayerId, into: u32, from: u32) -> Result<(), Refused> {
        let g = self.sim.galaxy();
        let (into, from) = (g.successor(into).unwrap_or(into), g.successor(from).unwrap_or(from));
        self.sim.apply(Command::MergeFleets { faction: p, into, from }).map(|_| ()).map_err(refuse)
    }
    fn capture_planet(&mut self, p: PlayerId, pl: PlanetId) -> Result<(), Refused> {
        if pl as usize >= self.world.planets.len() {
            return Err(Refused::Invalid);
        }
        // The events (PlanetCaptured ...) are picked up by ScriptedSim through the sim's next step report.
        let events = self.sim.set_planet_owner(pl, Some(p));
        self.pending.extend(events);
        Ok(())
    }
    fn path(&self, from: PlanetId, to: PlanetId) -> Option<Vec<PlanetId>> {
        self.world.find_path(from, to)
    }
    fn force_strength(&self, units: &[(String, u32)]) -> f64 {
        let members = units.iter().filter_map(|(n, c)| Some((galaxy_sim::combat::Source::Unit(*self.world.unit_index.get(&n.to_ascii_lowercase())?), *c)));
        f64::from(self.world.tuned_strength(members))
    }
    fn defense_strength(&self, pl: PlanetId, attacker: PlayerId) -> f64 {
        use galaxy_sim::combat::Source;
        let g = self.sim.galaxy();
        let mut members: Vec<(Source, u32)> = Vec::new();
        // Fog (user rule 2026-10-15): fleets in orbit and armed structures count only where the attacker's AI knows the planet; the star base is on the map.
        let known = g.ai_knowledge(&self.world, attacker)[pl as usize];
        for f in g.fleets_at(pl).filter(|_| known) {
            if g.hostile(&self.world, attacker, f.owner) {
                members.extend(f.units.iter().map(|(u, c)| (Source::Unit(*u), *c)));
            }
        }
        let ps = &g.planets[pl as usize];
        if ps.owner.is_some_and(|o| g.hostile(&self.world, attacker, o)) {
            if let Some(sb) = ps.starbase {
                members.push((Source::Starbase(sb), 1));
                members.extend(self.world.starbases[sb as usize].garrison.iter().map(|&(u, c)| (Source::Garrison(u), c)));
            }
            for &s in &ps.structures {
                if known && self.world.structures[s as usize].combat.is_some() {
                    members.push((Source::Structure(s), 1));
                }
            }
        }
        f64::from(self.world.tuned_strength(members))
    }
    fn planet_defense(&self, pl: PlanetId) -> f64 {
        use galaxy_sim::combat::Source;
        let ps = &self.sim.galaxy().planets[pl as usize];
        let mut members: Vec<(Source, u32)> = Vec::new();
        if let Some(sb) = ps.starbase {
            members.push((Source::Starbase(sb), 1));
            members.extend(self.world.starbases[sb as usize].garrison.iter().map(|&(u, c)| (Source::Garrison(u), c)));
        }
        for &s in &ps.structures {
            if self.world.structures[s as usize].combat.is_some() {
                members.push((Source::Structure(s), 1));
            }
        }
        if members.is_empty() { 0.0 } else { f64::from(self.world.tuned_strength(members)) }
    }
    fn eta(&self, fleet: u32, to: PlanetId) -> Option<f64> {
        let f = self.sim.galaxy().fleets.get(&fleet)?;
        let from = f.planet()?;
        if from == to {
            return Some(0.0);
        }
        let path = self.world.find_path(from, to)?;
        let speed = f64::from(self.world.fleet_speed(f)) * f64::from(self.world.config.galactic_speed_scale);
        let mut t = 0.0;
        for w in path.windows(2) {
            let r = &self.world.routes[self.world.route_between(w[0], w[1])? as usize];
            t += f64::from(r.length) / (speed * f64::from(r.hs_factor)).max(1e-6);
        }
        Some(t)
    }
    fn planet_object(&self, pl: PlanetId) -> u64 {
        PLANET_OBJECT_TAG | u64::from(pl)
    }
    fn object_planet(&self, object: u64) -> Option<PlanetId> {
        (object >> 56 == 1).then_some((object & 0xFFFF_FFFF) as u32)
    }
}
