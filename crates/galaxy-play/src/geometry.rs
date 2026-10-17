//! Map geometry: planet layout, fleet placement along lanes, ETAs and route planning.
//!
//! Everything here is pure (no Bevy systems) so it can be tested without an app.

use std::collections::HashMap;

use bevy::prelude::{Resource, Vec2, Vec3};
use galaxy_sim::{Fleet, FleetLocation, Galaxy, PlanetId, UnitTypeId, World};

/// Converts EaW galactic coordinates (x/y plane, z up) to Bevy world space (y up). Identical to
/// the `galaxy_to_world` of the `space-peace` app so both draw the same map.
pub fn galaxy_to_world(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0], p[2], -p[1])
}

/// World-space position and size of every planet, indexed by [`PlanetId`].
///
/// The app should overwrite `radii` with the radii of its planet models (see
/// [`GalaxyLayout::set_radius_by_name`]); the defaults are an estimate from the planet density.
#[derive(Resource, Debug, Clone)]
pub struct GalaxyLayout {
    /// Bevy world position of each planet.
    pub positions: Vec<Vec3>,
    /// World radius of each planet's visual.
    pub radii: Vec<f32>,
    /// World radius of a normal planet; fleet icons are sized relative to it.
    pub unit: f32,
    /// Lower-cased planet object name to id.
    pub index: HashMap<String, PlanetId>,
    /// Whether the pointer can pick the planet (empty = all): only real planets. The campaigns also list faction icons (24 like
    /// `Icon_Republic_Rendili_WW`, 15 units above Rendili) and dummies as planets.
    pub pickable: Vec<bool>,
}

/// `radii` is the model's bounding radius, which includes the atmosphere glow: the visible globe measures about 0.54 of it in our
/// renders. Vanilla 05 puts the pad orbit at 2.2 and the pad radius at 0.6 visible globe radii, hence these two factors.
const GLOBE_PER_RADIUS: f32 = 0.54;
const ORBIT_PER_RADIUS: f32 = 2.2 * GLOBE_PER_RADIUS;
const PAD_PER_RADIUS: f32 = 0.6 * GLOBE_PER_RADIUS;

impl GalaxyLayout {
    /// Layout derived from the planet positions of `world`.
    pub fn from_world(world: &World) -> Self {
        let positions: Vec<Vec3> = world.planets.iter().map(|p| galaxy_to_world(p.position)).collect();
        let pickable: Vec<bool> =
            world.planets.iter().enumerate().map(|(i, p)| world.real.get(i).copied().unwrap_or(true) && !p.name.to_ascii_lowercase().contains("dummy")).collect();
        // Median nearest-neighbour distance of the visible planets, like the app's `MapScale`.
        let flat: Vec<Vec2> = (0..positions.len()).filter(|&i| pickable[i]).map(|i| Vec2::new(positions[i].x, positions[i].z)).collect();
        let mut nn: Vec<f32> = flat
            .iter()
            .enumerate()
            .filter_map(|(i, p)| flat.iter().enumerate().filter(|&(j, q)| j != i && p.distance(*q) > 0.01).map(|(_, q)| p.distance(*q)).min_by(f32::total_cmp))
            .collect();
        nn.sort_by(f32::total_cmp);
        let median = nn.get(nn.len() / 2).copied().unwrap_or(30.0);
        let unit = (0.15 * median).clamp(0.5, 200.0);
        Self { radii: vec![unit; positions.len()], positions, unit, index: world.planet_index.clone(), pickable }
    }

    /// Sets the radius of the planet with this object name (case-insensitive); no-op if unknown.
    pub fn set_radius_by_name(&mut self, name: &str, radius: f32) {
        if let Some(&id) = self.index.get(&name.trim().to_ascii_lowercase()) {
            self.radii[id as usize] = radius;
        }
    }

    /// Radius of the visible globe (`radii` without the atmosphere glow): the pointer is on the planet itself inside it.
    pub fn globe_radius(&self, planet: PlanetId) -> f32 {
        self.radii[planet as usize] * GLOBE_PER_RADIUS
    }

    /// Radius of the orbit the three fleet pads sit on, in the planet plane. Measured from the vanilla close-up
    /// (`05_galaxy_closeup_fleet_heroes`, the Remake frames are too small to read): pad centres are about 2.2 planet radii out.
    pub fn orbit_radius(&self, planet: PlanetId) -> f32 {
        self.radii[planet as usize] * ORBIT_PER_RADIUS
    }

    /// Radius of one fleet pad (the dark green ellipse a fleet stands on): about 0.6 planet radii (same source as `orbit_radius`).
    pub fn pad_radius(&self, planet: PlanetId) -> f32 {
        self.radii[planet as usize] * PAD_PER_RADIUS
    }
}

/// Rank of a ship class for choosing the representative unit of a fleet.
fn category_rank(c: alamo_data::ShipCategory) -> u32 {
    use alamo_data::ShipCategory as C;
    match c {
        C::Super => 6,
        C::Capital => 5,
        C::Frigate => 4,
        C::Corvette => 3,
        C::Transport => 2,
        C::Bomber | C::Fighter => 1,
        _ => 0,
    }
}

/// The unit whose model represents the fleet on the map: hero units first, then largest ship class,
/// then strongest (`AI_Combat_Power`, else durability times damage), then the lowest type id.
pub fn flagship(world: &World, fleet: &Fleet) -> Option<UnitTypeId> {
    fleet
        .units
        .keys()
        .copied()
        .max_by(|&a, &b| {
            let key = |u: UnitTypeId| {
                let ut = &world.units[u as usize];
                let power = if ut.ai_power > 0.0 { ut.ai_power } else { ut.combat.durability() * ut.combat.raw_dps().sqrt() };
                (ut.hero, category_rank(ut.category), power)
            };
            let (ka, kb) = (key(a), key(b));
            ka.0.cmp(&kb.0).then(ka.1.cmp(&kb.1)).then(ka.2.total_cmp(&kb.2)).then(b.cmp(&a))
        })
}

/// Where a fleet is, in galactic coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FleetPlace {
    /// Parked in orbit.
    Orbit(PlanetId),
    /// Travelling along a lane.
    Lane {
        /// Planet the leg started at.
        from: PlanetId,
        /// Planet the leg ends at.
        to: PlanetId,
        /// 0..=1 along the leg.
        t: f32,
        /// Distance covered on the leg (map units).
        travelled: f32,
    },
}

/// Map units per second a fleet covers on route `route`.
pub fn lane_speed(world: &World, fleet: &Fleet, route: u32) -> f32 {
    world.fleet_speed(fleet) * world.routes[route as usize].hs_factor * world.config.galactic_speed_scale
}

/// Where `fleet` is, advanced by `extra_secs` of galactic time past the snapshot (clamped to the
/// end of the current leg, since the next leg is only known to the simulation).
pub fn fleet_place(world: &World, fleet: &Fleet, extra_secs: f32) -> FleetPlace {
    match &fleet.location {
        FleetLocation::AtPlanet(p) => FleetPlace::Orbit(*p),
        FleetLocation::Moving { from, to, route, progress, .. } => {
            let len = world.routes[*route as usize].length.max(0.001);
            let travelled = (progress + lane_speed(world, fleet, *route) * extra_secs.max(0.0)).min(len);
            FleetPlace::Lane { from: *from, to: *to, t: (travelled / len).clamp(0.0, 1.0), travelled }
        }
    }
}

/// Seconds until `fleet` reaches its final destination; `None` for parked fleets.
pub fn eta_seconds(world: &World, fleet: &Fleet) -> Option<f32> {
    let FleetLocation::Moving { to, route, progress, rest, .. } = &fleet.location else { return None };
    let speed = world.fleet_speed(fleet) * world.config.galactic_speed_scale;
    let r = &world.routes[*route as usize];
    let mut secs = (r.length - progress).max(0.0) / (speed * r.hs_factor).max(1e-3);
    let mut prev = *to;
    for &next in rest {
        let route = world.route_between(prev, next)?;
        let r = &world.routes[route as usize];
        secs += r.length / (speed * r.hs_factor).max(1e-3);
        prev = next;
    }
    Some(secs)
}

/// A planned trip from one planet to another.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutePlan {
    /// Planets from the start to the destination, both included.
    pub path: Vec<PlanetId>,
    /// Galactic seconds the trip takes at the fleet's speed.
    pub eta_seconds: f32,
    /// Intermediate planets where the fleet would be stopped by a blockade.
    pub blockades: Vec<PlanetId>,
}

/// Plans the trip of a fleet of `owner` with the given hyperspace speed (`World::fleet_speed`).
pub fn plan_route(world: &World, galaxy: &Galaxy, owner: u32, speed: f32, from: PlanetId, to: PlanetId) -> Option<RoutePlan> {
    let path = world.find_path(from, to)?;
    let speed = speed * world.config.galactic_speed_scale;
    let mut eta = 0.0;
    for w in path.windows(2) {
        let r = &world.routes[world.route_between(w[0], w[1])? as usize];
        eta += r.length / (speed * r.hs_factor).max(1e-3);
    }
    // Intermediate planets only; a one-planet path (hovering the fleet's own planet) has none.
    let inner = if path.len() > 2 { &path[1..path.len() - 1] } else { &[][..] };
    let blockades = inner.iter().copied().filter(|&p| world.blockades(galaxy, p, owner)).collect();
    Some(RoutePlan { path, eta_seconds: eta, blockades })
}

/// Splits a fleet roughly in half: returns the units to take out, or an empty list if the fleet
/// is a single ship and cannot be split.
pub fn split_half(fleet: &Fleet) -> Vec<(UnitTypeId, u32)> {
    let total = fleet.unit_count();
    if total < 2 {
        return Vec::new();
    }
    let mut take: Vec<(UnitTypeId, u32)> = fleet.units.iter().filter(|&(_, &c)| c >= 2).map(|(&u, &c)| (u, c / 2)).collect();
    if take.is_empty() {
        // Only singletons of different types: hand every second type over.
        take = fleet.units.keys().copied().step_by(2).map(|u| (u, 1)).collect();
    }
    let taken: u32 = take.iter().map(|&(_, c)| c).sum();
    if taken == 0 || taken >= total {
        return Vec::new();
    }
    take
}

/// Formats galactic seconds as weeks given the week length (`"2.3 weeks"`, `"45 s"`).
pub fn format_duration(secs: f32, week_seconds: f32) -> String {
    if secs >= week_seconds * 0.5 {
        format!("{:.1} wk", secs / week_seconds)
    } else {
        format!("{:.0} s", secs.max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn fleet(units: &[(UnitTypeId, u32)]) -> Fleet {
        Fleet { id: 1, owner: 0, units: units.iter().copied().collect::<BTreeMap<_, _>>(), location: FleetLocation::AtPlanet(0), previous_planet: 0, slot: 0 }
    }

    #[test]
    fn coordinates_match_the_app() {
        assert_eq!(galaxy_to_world([1.0, 2.0, 3.0]), Vec3::new(1.0, 3.0, -2.0));
    }

    #[test]
    fn split_half_rules() {
        assert!(split_half(&fleet(&[(3, 1)])).is_empty());
        assert_eq!(split_half(&fleet(&[(3, 4), (5, 1)])), vec![(3, 2)]);
        assert_eq!(split_half(&fleet(&[(3, 1), (5, 1)])), vec![(3, 1)]);
        let three = split_half(&fleet(&[(1, 1), (2, 1), (3, 1)]));
        assert_eq!(three, vec![(1, 1), (3, 1)]);
    }

    #[test]
    fn durations() {
        assert_eq!(format_duration(20.0, 60.0), "20 s");
        assert_eq!(format_duration(90.0, 60.0), "1.5 wk");
    }
}

