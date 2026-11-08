//! What a faction can see on the galaxy map (fog of war).
//!
//! The data has almost no numbers for this: `Minimum_Visible_Base_Level`, the spy abilities and `Galactic_Stealth_Ability` (all
//! `Evade_Detection_Chance` 0) say who can see what, but not how far a planet or fleet sees. The model below is therefore an
//! **estimate** (named in docs/RULES.md): presence in a system gives full detail, one lane around it gives a rough picture, the rest of
//! the galaxy shows only owners. It is a pure function of the galaxy state, so it is deterministic.
//!
//! Spies follow the data: every `System_Spy_Ability` (units in orbit, structures, field agent reports) reveals what its `See_*` flags
//! name ([`Galaxy::intel_map`]) at its planet and within its `Galactic_Influence_Range`, and fleets in hyperspace within its
//! `Hyperspace_Fleet_Reveal_Range`.

use crate::agents::{SpyAbility, SpyFlags};
use crate::state::{Fleet, FleetLocation, Galaxy};
use crate::world::{FactionId, FleetId, PlanetId, UnitTypeId, World};

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

    /// The spies of `viewer` and its allies: every unit with a lasting `System_Spy_Ability` in a fleet in orbit, every such structure on a
    /// planet they own (a field agent, which is used up, reports through [`Galaxy::spy_reports`] instead).
    fn spy_sources(&self, world: &World, viewer: FactionId) -> Vec<SpySource> {
        let mut out = Vec::new();
        for f in self.fleets.values().filter(|f| self.is_friend(viewer, f.owner)) {
            let FleetLocation::AtPlanet(p) = f.location else { continue };
            for &u in f.units.keys() {
                if let Some(s) = world.units[u as usize].spy.filter(|s| !s.despawn) {
                    out.push(SpySource { planet: p, spy: s });
                }
            }
        }
        for (p, ps) in self.planets.iter().enumerate() {
            if !ps.owner.is_some_and(|o| self.is_friend(viewer, o)) {
                continue;
            }
            for &s in &ps.structures {
                if let Some(spy) = world.structures.get(s as usize).and_then(|st| st.spy) {
                    out.push(SpySource { planet: p as PlanetId, spy });
                }
            }
        }
        out
    }

    /// Which details of every planet `viewer` knows (`See_*` flags): everything of its own and allied planets; elsewhere what its spies
    /// reveal at the planet they are at and at every planet within their `Galactic_Influence_Range` (galaxy map distance), plus running
    /// field agent reports.
    pub fn intel_map(&self, world: &World, viewer: FactionId) -> Vec<SpyFlags> {
        self.intel_with(world, viewer, &self.spy_sources(world, viewer))
    }

    fn intel_with(&self, world: &World, viewer: FactionId, sources: &[SpySource]) -> Vec<SpyFlags> {
        let mut out: Vec<SpyFlags> = self.planets.iter().map(|p| if p.owner.is_some_and(|o| self.is_friend(viewer, o)) { SpyFlags::ALL } else { SpyFlags::default() }).collect();
        for s in sources {
            let at = world.planets[s.planet as usize].position;
            for (q, o) in out.iter_mut().enumerate() {
                if q == s.planet as usize || (s.spy.range > 0.0 && world.real[q] && distance(at, world.planets[q].position) <= s.spy.range) {
                    *o = o.union(s.spy.see);
                }
            }
        }
        for r in self.spy_reports.iter().filter(|r| self.is_friend(viewer, r.faction)) {
            if let Some(o) = out.get_mut(r.planet as usize) {
                *o = o.union(r.see);
            }
        }
        out
    }

    /// The sight of `viewer` on every planet: `Detailed` where it is present, `Seen` on the lanes' neighbours of those (one jump),
    /// `Hidden` elsewhere; a spied planet is `Detailed` with `See_Fleet_Contents` and at least `Seen` with `See_Num_Fleets` (see
    /// [`Galaxy::intel_map`]).
    pub fn sight_map(&self, world: &World, viewer: FactionId) -> Vec<Sight> {
        let here = self.presence(viewer);
        let intel = self.intel_map(world, viewer);
        self.sight_with(world, &here, &intel)
    }

    fn sight_with(&self, world: &World, here: &[bool], intel: &[SpyFlags]) -> Vec<Sight> {
        let mut out: Vec<Sight> = here.iter().map(|&h| if h { Sight::Detailed } else { Sight::Hidden }).collect();
        for p in (0..here.len()).filter(|&p| here[p]) {
            for &(n, _) in &world.adjacency[p] {
                if out[n as usize] == Sight::Hidden {
                    out[n as usize] = Sight::Seen;
                }
            }
        }
        for (o, i) in out.iter_mut().zip(intel) {
            *o = (*o).max(fleet_detail(*i));
        }
        out
    }

    /// What `viewer` sees of one planet (see [`Galaxy::sight_map`] to ask for all of them).
    pub fn planet_sight(&self, world: &World, viewer: FactionId, planet: PlanetId) -> Sight {
        self.sight_map(world, viewer)[planet as usize]
    }

    /// What `viewer` sees of one fleet. Own and allied fleets are always `Detailed`. A fleet in orbit is seen like its planet; a fleet in
    /// hyperspace is `Seen` when either end of its lane is, or when it is within the `Hyperspace_Fleet_Reveal_Range` of a spy of the viewer
    /// (`Detailed` if that spy has `See_Fleet_Contents`). A fleet with a `Galactic_Stealth_Ability` unit is seen by those present in its
    /// system (`Evade_Detection_Chance` is 0 everywhere, so they are not missed there), elsewhere only where the viewer's spies see stealthy
    /// heroes of its kind (`See_Minor_Stealth_Heroes` for agents, `See_Major_Stealth_Heroes` for the rest), and never in hyperspace.
    pub fn fleet_sight(&self, world: &World, viewer: FactionId, fleet: FleetId) -> Sight {
        let view = View::new(self, world, viewer);
        self.fleets.get(&fleet).map_or(Sight::Hidden, |f| view.fleet(self, world, f))
    }

    /// All fleets `viewer` can see with how much it sees of them (no `Hidden` ones), by fleet id.
    pub fn visible_fleets(&self, world: &World, viewer: FactionId) -> Vec<(FleetId, Sight)> {
        let view = View::new(self, world, viewer);
        self.fleets.iter().map(|(&id, f)| (id, view.fleet(self, world, f))).filter(|(_, s)| *s != Sight::Hidden).collect()
    }

    /// Planets whose exact forces the AI of `faction` may use (docs/RULES.md 1.12 "AI knowledge"): all of them when its
    /// `Difficulty::ai_sight_hops` is `None`, otherwise those within that many lanes of a planet it owns or a fleet it has (moving fleets count
    /// at both ends of their lane) plus every planet it sees `Detailed` (presence, which includes the `System_Spy_Ability` units).
    pub fn ai_knowledge(&self, world: &World, faction: FactionId) -> Vec<bool> {
        let n = self.planets.len();
        let Some(hops) = self.factions.get(faction as usize).and_then(|f| f.difficulty.ai_sight_hops) else { return vec![true; n] };
        let mut known: Vec<bool> = self.sight_map(world, faction).iter().map(|&s| s == Sight::Detailed).collect();
        let mut frontier: Vec<usize> = (0..n).filter(|&p| self.planets[p].owner == Some(faction)).collect();
        for f in self.fleets.values().filter(|f| f.owner == faction) {
            match f.location {
                FleetLocation::AtPlanet(p) => frontier.push(p as usize),
                FleetLocation::Moving { from, to, .. } => frontier.extend([from as usize, to as usize]),
            }
        }
        let mut seen = vec![false; n];
        for &p in &frontier {
            seen[p] = true;
        }
        for _ in 0..=hops {
            let mut next = Vec::new();
            for &p in &frontier {
                known[p] = true;
                for &(q, _) in &world.adjacency[p] {
                    if !seen[q as usize] {
                        seen[q as usize] = true;
                        next.push(q as usize);
                    }
                }
            }
            frontier = next;
        }
        known
    }
}

/// A spy of the viewer at a planet (see `Galaxy::spy_sources`).
struct SpySource {
    planet: PlanetId,
    spy: SpyAbility,
}

/// Galaxy map distance (x/y of `Galactic_Position`, like lane lengths).
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// How much of the fleets at a planet the flags show.
fn fleet_detail(see: SpyFlags) -> Sight {
    if see.fleet_contents {
        Sight::Detailed
    } else if see.num_fleets {
        Sight::Seen
    } else {
        Sight::Hidden
    }
}

/// Everything one viewer's fleet sight needs, computed once for all fleets.
struct View {
    viewer: FactionId,
    here: Vec<bool>,
    intel: Vec<SpyFlags>,
    map: Vec<Sight>,
    sources: Vec<SpySource>,
}

impl View {
    fn new(g: &Galaxy, world: &World, viewer: FactionId) -> View {
        let here = g.presence(viewer);
        let sources = g.spy_sources(world, viewer);
        let intel = g.intel_with(world, viewer, &sources);
        let map = g.sight_with(world, &here, &intel);
        View { viewer, here, intel, map, sources }
    }

    fn fleet(&self, g: &Galaxy, world: &World, f: &Fleet) -> Sight {
        if g.is_friend(self.viewer, f.owner) {
            return Sight::Detailed;
        }
        // Stealthy agents are minor heroes (`TEXT_ENCYCLOPEDIA_CLASS_HERO_MINOR`), every other stealthy unit counts as major.
        let stealth: Vec<UnitTypeId> = f.units.keys().copied().filter(|&u| world.units[u as usize].galactic_stealth).collect();
        let minor = stealth.iter().all(|&u| world.is_agent(u));
        match &f.location {
            FleetLocation::AtPlanet(p) => {
                let p = *p as usize;
                if stealth.is_empty() || self.here[p] {
                    return self.map[p];
                }
                let i = self.intel[p];
                if (minor && i.minor_stealth) || (!minor && i.major_stealth) { fleet_detail(i).max(Sight::Seen) } else { Sight::Hidden }
            }
            FleetLocation::Moving { from, to, route, progress, .. } => {
                if !stealth.is_empty() {
                    return Sight::Hidden;
                }
                let mut s = if self.map[*from as usize] == Sight::Hidden && self.map[*to as usize] == Sight::Hidden { Sight::Hidden } else { Sight::Seen };
                let (a, b) = (world.planets[*from as usize].position, world.planets[*to as usize].position);
                let t = (progress / world.routes[*route as usize].length.max(1.0)).clamp(0.0, 1.0);
                let at = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, 0.0];
                for src in self.sources.iter().filter(|s| s.spy.fleet_reveal_range > 0.0) {
                    if distance(at, world.planets[src.planet as usize].position) <= src.spy.fleet_reveal_range {
                        s = s.max(if src.spy.see.fleet_contents { Sight::Detailed } else { Sight::Seen });
                    }
                }
                s
            }
        }
    }
}
