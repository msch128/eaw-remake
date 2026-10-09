//! "From the Ground Up": a setup transform over the loaded campaign (`SimConfig::ground_up`, applied by [`Sim::new`]).
//!
//! The player's faction starts with one planet (its home), every other playable main faction (Rebel, Empire, Underworld) with
//! three (home plus the two nearest it owns in the campaign). The planets they give up and every planet nobody owns belong to
//! the passive faction (the Remake's orange `Pirates`): it defends but never builds, moves or expands. Minor factions keep their
//! planets as in the campaign. Star bases of the start planets are reset to level 1; fleets stay only where a planet is kept.

use crate::state::Command;
use crate::sim::Sim;
use crate::world::{FactionId, PlanetId, World};

/// The playable main factions that get the reduced start.
const MAIN_FACTIONS: [&str; 3] = ["Rebel", "Empire", "Underworld"];
/// The passive owner of everything else.
const PASSIVE_FACTION: &str = "Pirates";

impl World {
    /// The passive faction of a "From the Ground Up" game (`None` in every other game).
    pub fn passive_faction(&self) -> Option<FactionId> {
        if self.config.ground_up { self.faction_id(PASSIVE_FACTION) } else { None }
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
        self.galaxy.sync_ships();
        world.refresh_economy(&mut self.galaxy);
    }
}
