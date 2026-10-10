//! "From the Ground Up": a setup transform over the loaded campaign (`SimConfig::ground_up`, applied by [`Sim::new`]).
//!
//! The player's faction starts with one planet (its home), every other playable main faction (Rebel, Empire, Underworld) with
//! three (home plus the two nearest it owns in the campaign). The planets they give up and every planet nobody owns belong to
//! the passive faction (the Remake's orange `Pirates`): it defends but never builds, moves or expands. Minor factions keep their
//! planets as in the campaign. Star bases of the start planets are reset to level 1; fleets stay only where a planet is kept.

use crate::state::Command;
use crate::sim::Sim;
use crate::world::{FactionId, PlanetId, StarbaseTypeId, UnitTypeId, World};

/// The playable main factions that get the reduced start.
const MAIN_FACTIONS: [&str; 3] = ["Rebel", "Empire", "Underworld"];
/// The passive owner of everything else.
const PASSIVE_FACTION: &str = "Pirates";

impl World {
    /// The passive faction (the Pirates) in every game: it defends and rearms (see [`Sim::rearm_passive`]) but never builds, moves, attacks or expands.
    pub fn passive_faction(&self) -> Option<FactionId> {
        self.faction_id(PASSIVE_FACTION)
    }

    /// The star base upgrade chain of `faction` from level 1 upwards.
    pub fn starbase_chain(&self, faction: FactionId) -> Vec<StarbaseTypeId> {
        let mut chain = Vec::new();
        let mut cur = self.first_starbase.get(faction as usize).copied().flatten();
        while let Some(id) = cur {
            if chain.contains(&id) {
                break;
            }
            chain.push(id);
            cur = self.starbases[id as usize].next;
        }
        chain
    }
}

impl Command {
    /// The faction that issues the command.
    pub fn faction(&self) -> Option<FactionId> {
        use Command::*;
        match self {
            BuildUnit { faction, .. }
            | UpgradeStarbase { faction, .. }
            | BuildStructure { faction, .. }
            | UpgradeTech { faction, .. }
            | DebugGrant { faction, .. }
            | DestroyPlanet { faction, .. }
            | MoveFleet { faction, .. }
            | MoveFleetAround { faction, .. }
            | MergeFleets { faction, .. }
            | SplitFleet { faction, .. }
            | TransferUnits { faction, .. }
            | SetHyperspace { faction, .. }
            | CancelProduction { faction, .. }
            | Retreat { faction, .. }
            | SellStructure { faction, .. }
            | Cheat { faction, .. } => Some(*faction),
        }
    }
}

impl Sim {
    /// Applies the "From the Ground Up" start to a freshly built simulation; `human` is the player's faction.
    pub(crate) fn apply_ground_up(&mut self, human: Option<FactionId>) {
        let world = self.world.clone();
        let Some(passive) = world.passive_faction() else { return };
        let main: Vec<FactionId> = MAIN_FACTIONS.iter().filter_map(|n| world.faction_id(n)).collect();
        for &f in &main {
            let mut owned: Vec<PlanetId> = self.galaxy.owned_planets(f).collect();
            if owned.is_empty() {
                continue;
            }
            let wanted = if Some(f) == human { 1 } else { 3 };
            let home = world.factions[f as usize]
                .home
                .filter(|h| owned.contains(h))
                .or_else(|| owned.iter().copied().max_by(|&a, &b| world.planets[a as usize].credit_value.total_cmp(&world.planets[b as usize].credit_value)))
                .expect("owned is not empty");
            let at = world.planets[home as usize].position;
            let dist = |p: PlanetId| {
                let q = world.planets[p as usize].position;
                (0..3).map(|i| (q[i] - at[i]) * (q[i] - at[i])).sum::<f32>()
            };
            owned.retain(|&p| p != home);
            owned.sort_by(|&a, &b| dist(a).total_cmp(&dist(b)).then(a.cmp(&b)));
            let mut kept = vec![home];
            kept.extend(owned.iter().copied().take(wanted - 1));
            for &p in &owned[(wanted - 1).min(owned.len())..] {
                self.set_planet_owner(p, Some(passive));
            }
            // Fleets of the faction stay only at kept planets.
            let gone: Vec<_> = self
                .galaxy
                .fleets
                .iter()
                .filter(|(_, fl)| fl.owner == f && fl.planet().is_none_or(|p| !kept.contains(&p)))
                .map(|(&id, _)| id)
                .collect();
            for id in gone {
                self.galaxy.fleets.remove(&id);
            }
            // Star base level 1 on every start planet.
            if let Some(first) = world.first_starbase[f as usize] {
                for &p in &kept {
                    self.galaxy.planets[p as usize].starbase = Some(first);
                }
            }
        }
        // Everything nobody owns goes to the passive faction too.
        for p in 0..world.planets.len() as PlanetId {
            if self.galaxy.planets[p as usize].owner.is_none() && !self.galaxy.planets[p as usize].destroyed {
                self.set_planet_owner(p, Some(passive));
            }
        }
        // The passive faction defends: every planet it holds gets its level 1 star base (the owner change above cleared the
        // buildings), so arriving fleets meet a base and its garrison instead of taking the planet by presence.
        if let Some(first) = world.first_starbase[passive as usize] {
            for p in self.galaxy.planets.iter_mut().filter(|p| p.owner == Some(passive) && !p.destroyed && p.starbase.is_none()) {
                p.starbase = Some(first);
            }
        }
        self.galaxy.sync_ships();
        world.refresh_economy(&mut self.galaxy);
    }
}

/// Garrison units per level of the human player's tech that a passive planet holds before it stops growing (estimate: the user's rule, "2 x player tech").
pub const PASSIVE_GARRISON_PER_TECH: i32 = 2;

impl Sim {
    /// The tech level the passive faction rearms to: the human player's (`victory.active_player`), else the best of the playable main factions.
    fn rearm_tech(&self) -> i32 {
        let world = &self.world;
        let tech = |f: FactionId| self.galaxy.factions.get(f as usize).map_or(1, |s| s.tech);
        match world.victory.active_player.filter(|&p| Some(p) != world.passive_faction()) {
            Some(p) => tech(p),
            None => MAIN_FACTIONS.iter().filter_map(|n| world.faction_id(n)).map(tech).max().unwrap_or(1),
        }
        .max(1)
    }

    /// The passive faction (Pirates) never expands but rearms with the player's tech (user rule, estimates in `RULES.md`): every planet it holds gets the
    /// star base of its own chain at level `clamp(player tech, 1, top level)` (raised, never lowered; applied at the start and every week), and
    /// every week a planet with fewer than `2 x tech` passive units gets one more, picked deterministically from the ship types the passive faction fields
    /// in the campaign (no heroes, no planet destroyers, cycling by planet and week). It costs nothing and the units never move because the faction
    /// never gets orders.
    pub(crate) fn rearm_passive(&mut self, with_garrison: bool) {
        let world = self.world.clone();
        let Some(passive) = world.passive_faction() else { return };
        let tech = self.rearm_tech();
        // The passive star base chain by level.
        let chain = world.starbase_chain(passive);
        // The ship types the passive faction fields in the campaign (the data tags them with other factions' affiliations); without any, the neutral
        // buildable ships.
        let mut pool: Vec<UnitTypeId> = self.galaxy.fleets.values().filter(|f| f.owner == passive).flat_map(|f| f.units.keys().copied()).collect();
        pool.sort_unstable();
        pool.dedup();
        pool.retain(|&u| {
            let t = &world.units[u as usize];
            t.role == crate::world::UnitRole::Ship && !t.hero && !t.planet_destroyer && t.population > 0
        });
        if pool.is_empty() {
            pool = (0..world.units.len() as UnitTypeId)
                .filter(|&u| {
                    let t = &world.units[u as usize];
                    t.role == crate::world::UnitRole::Ship && t.buildable && !t.hero && !t.planet_destroyer && t.population > 0 && t.affiliation.iter().any(|a| a == "neutral")
                })
                .collect();
        }
        let week = self.galaxy.week;
        for p in 0..world.planets.len() as PlanetId {
            let ps = &self.galaxy.planets[p as usize];
            if ps.owner != Some(passive) || ps.destroyed || !world.real[p as usize] {
                continue;
            }
            if !chain.is_empty() {
                let is_lehon = world.planets[p as usize].name.to_ascii_lowercase().contains("lehon");
                let max_sb = if is_lehon {
                    world.planets[p as usize].max_space_base as usize
                } else {
                    // Non-Lehon pirate planets never get a Starforge (Pirate_Starbase_Lv3 is the Starforge model)
                    (world.planets[p as usize].max_space_base as usize).min(2)
                };
                let effective_chain_len = chain.len().min(max_sb);
                if effective_chain_len > 0 {
                    let mut want = (tech.clamp(1, effective_chain_len as i32) - 1) as usize;
                    let sb_name = &world.starbases[chain[want] as usize].name;
                    if !is_lehon && (sb_name.eq_ignore_ascii_case("Pirate_Starbase_Lv3") || sb_name.to_ascii_lowercase().contains("starforge")) {
                        want = want.min(1);
                    }
                    let have = ps.starbase.and_then(|s| chain.iter().position(|&c| c == s));
                    if have.is_none_or(|h| h < want) {
                        self.galaxy.planets[p as usize].starbase = Some(chain[want]);
                    } else if !is_lehon && have.is_some_and(|h| h > 1) {
                        // Downgrade any accidentally placed Starforge on non-Lehon planets
                        self.galaxy.planets[p as usize].starbase = Some(chain[1.min(chain.len() - 1)]);
                    }
                }
            }
            if with_garrison && !pool.is_empty() {
                let here: u32 = self.galaxy.fleets.values().filter(|f| f.owner == passive && f.location == crate::state::FleetLocation::AtPlanet(p)).map(|f| f.unit_count()).sum();
                if (here as i32) < PASSIVE_GARRISON_PER_TECH * tech {
                    let pick = pool[(p as usize + week as usize) % pool.len()];
                    crate::sim::add_units(&mut self.galaxy, passive, p, pick, 1);
                }
            }
        }
        self.galaxy.sync_ships();
    }

    /// For a battle where the passive faction is defending, scales its star base and garrison forces
    /// to the attacker's tech level (so an AI at tech 1 is not blocked by a level 4 base scaled to the human player).
    pub(crate) fn passive_defense_scaling(&self, battle: &crate::state::PendingBattle, passive: FactionId, current_sb: Option<StarbaseTypeId>) -> (Option<StarbaseTypeId>, u32) {
        let attacker_tech = battle
            .factions
            .iter()
            .filter(|&&f| f != passive)
            .map(|&f| self.galaxy.factions.get(f as usize).map_or(1, |s| s.tech))
            .max()
            .unwrap_or(1)
            .max(1);
        let max_units = (PASSIVE_GARRISON_PER_TECH * attacker_tech) as u32;
        let chain = self.world.starbase_chain(passive);
        let p = battle.planet;
        let is_lehon = self.world.planets.get(p as usize).is_some_and(|pl| pl.name.eq_ignore_ascii_case("Lehon"));
        let eff_sb = if !chain.is_empty() {
            let max_sb = if is_lehon { chain.len() } else { (self.world.planets[p as usize].max_space_base as usize).min(2) };
            let effective_chain_len = chain.len().min(max_sb);
            if effective_chain_len > 0 {
                let want = (attacker_tech.clamp(1, effective_chain_len as i32) - 1) as usize;
                Some(chain[want])
            } else {
                current_sb
            }
        } else {
            current_sb
        };
        (eff_sb, max_units)
    }
}
