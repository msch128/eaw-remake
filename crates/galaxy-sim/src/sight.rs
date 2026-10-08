//! What a faction can see on the galaxy map (fog of war).
//!
//! The data has almost no numbers for this: `Minimum_Visible_Base_Level`, the spy abilities and `Galactic_Stealth_Ability` (all
//! `Evade_Detection_Chance` 0) say who can see what, but not how far a planet or fleet sees. The model below is therefore an
//! **estimate** (named in RULES.md): presence in a system gives full detail, one lane around it gives a rough picture, the rest of
//! the galaxy shows only owners. It is a pure function of the galaxy state, so it is deterministic.

use crate::state::{FleetLocation, Galaxy};
use crate::world::{FactionId, FleetId, PlanetId, World};

/// How much of a planet or fleet a faction sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sight {
    /// Nothing: a fleet is not drawn, a planet shows only its owner.
    Hidden,
    /// A fleet is drawn with an unknown composition (show "?" in tooltips and orbit boxes); a planet shows its fleets as unknown groups.
    Seen,
    /// Everything: fleet contents, planet details.
    Detailed,
}

impl Galaxy {
    /// True if `other` is the viewer or an ally of it.
    pub fn is_friend(&self, viewer: FactionId, other: FactionId) -> bool {
        viewer == other || self.allied.contains(&(viewer.min(other), viewer.max(other)))
    }

    /// Planets where `viewer` is present: it or an ally owns the planet or has a fleet in its orbit.
    fn presence(&self, viewer: FactionId) -> Vec<bool> {
        let mut here: Vec<bool> = self.planets.iter().map(|p| p.owner.is_some_and(|o| self.is_friend(viewer, o))).collect();
        for f in self.fleets.values() {
            if let FleetLocation::AtPlanet(p) = f.location
                && self.is_friend(viewer, f.owner)
            {
                here[p as usize] = true;
            }
        }
        here
    }

    /// The sight of `viewer` on every planet: `Detailed` where it is present, `Seen` on the lanes' neighbours of those (one jump),
    /// `Hidden` elsewhere.
    pub fn sight_map(&self, world: &World, viewer: FactionId) -> Vec<Sight> {
        let here = self.presence(viewer);
        let mut out: Vec<Sight> = here.iter().map(|&h| if h { Sight::Detailed } else { Sight::Hidden }).collect();
        for p in (0..here.len()).filter(|&p| here[p]) {
            for &(n, _) in &world.adjacency[p] {
                if out[n as usize] == Sight::Hidden {
                    out[n as usize] = Sight::Seen;
                }
            }
        }
        out
    }

    /// What `viewer` sees of one planet (see [`Galaxy::sight_map`] to ask for all of them).
    pub fn planet_sight(&self, world: &World, viewer: FactionId, planet: PlanetId) -> Sight {
        self.sight_map(world, viewer)[planet as usize]
    }

    /// What `viewer` sees of one fleet. Own and allied fleets are always `Detailed`. A fleet in orbit is seen like its planet; a fleet in
    /// hyperspace is `Seen` when either end of its lane is. A fleet with a `Galactic_Stealth_Ability` unit is only seen by those present
    /// in its system (`Evade_Detection_Chance` is 0 everywhere, so they are not missed there) and never in hyperspace.
    pub fn fleet_sight(&self, world: &World, viewer: FactionId, fleet: FleetId) -> Sight {
        let Some(f) = self.fleets.get(&fleet) else { return Sight::Hidden };
        if self.is_friend(viewer, f.owner) {
            return Sight::Detailed;
        }
        let stealthy = f.units.keys().any(|&u| world.units[u as usize].galactic_stealth);
        let map = self.sight_map(world, viewer);
        match f.location {
            FleetLocation::AtPlanet(p) => {
                if stealthy {
                    // Only presence in the system (Detailed) finds a stealthy fleet.
                    if map[p as usize] == Sight::Detailed { Sight::Detailed } else { Sight::Hidden }
                } else {
                    map[p as usize]
                }
            }
            FleetLocation::Moving { from, to, .. } => {
                if stealthy || (map[from as usize] == Sight::Hidden && map[to as usize] == Sight::Hidden) {
                    Sight::Hidden
                } else {
                    Sight::Seen
                }
            }
        }
    }

    /// All fleets `viewer` can see with how much it sees of them (no `Hidden` ones), by fleet id.
    pub fn visible_fleets(&self, world: &World, viewer: FactionId) -> Vec<(FleetId, Sight)> {
        self.fleets.keys().map(|&id| (id, self.fleet_sight(world, viewer, id))).filter(|(_, s)| *s != Sight::Hidden).collect()
    }
}
