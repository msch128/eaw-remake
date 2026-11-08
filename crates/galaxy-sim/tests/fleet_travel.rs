//! Travel time over stopovers: a fleet on a multi-hop route takes the sum of its legs' travel times, no more and no less.

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn load() -> Option<GameData> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    Some(load_galaxy(&fs).0)
}

/// A Rebel MC-80 at the start of a route of at least `hops` lanes; every planet is Rebel so nothing on the way stops it.
fn setup(data: &GameData, hops: usize) -> (Sim, FactionId, FleetId, Vec<PlanetId>) {
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(data, campaign, SimConfig { auto_resolve_battles: false, victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let rebel = w.faction_id("Rebel").unwrap();
    sim.galaxy_mut().fleets.clear();
    sim.galaxy_mut().battles.clear();
    for p in sim.galaxy_mut().planets.iter_mut() {
        p.owner = Some(rebel);
    }
    let path = (0..w.planets.len() as u32)
        .filter(|&a| w.real[a as usize])
        .flat_map(|a| (0..w.planets.len() as u32).map(move |b| (a, b)))
        .filter_map(|(a, b)| w.find_path(a, b))
        .find(|p| p.len() > hops)
        .expect("a long route");
    let fleet = sim.spawn_units(rebel, path[0], w.unit_id("MC-80_Star_Cruiser").unwrap(), 1);
    (sim, rebel, fleet, path)
}

/// Map units per second of `fleet` on the lane between `a` and `b`.
fn lane_speed(w: &World, fleet: &Fleet, a: PlanetId, b: PlanetId) -> f32 {
    w.fleet_speed(fleet) * w.routes[w.route_between(a, b).unwrap() as usize].hs_factor * w.config.galactic_speed_scale
}

#[test]
fn a_fleet_over_stopovers_arrives_at_its_travel_time() {
    let Some(data) = load() else { return };
    let (mut sim, rebel, fleet, path) = setup(&data, 4);
    sim.apply(Command::MoveFleet { faction: rebel, fleet, to: *path.last().unwrap() }).expect("move");
    let w = sim.world().clone();
    let FleetLocation::Moving { from, to, rest, .. } = sim.galaxy().fleets[&fleet].location.clone() else { panic!("not moving") };
    let route: Vec<PlanetId> = [from, to].into_iter().chain(rest).collect();
    assert!(route.len() >= 5, "{route:?}");
    let f = sim.galaxy().fleets[&fleet].clone();
    let eta: f32 = route.windows(2).map(|p| w.routes[w.route_between(p[0], p[1]).unwrap() as usize].length / lane_speed(&w, &f, p[0], p[1])).sum();
    let mut arrived = None;
    for tick in 1..10_000u32 {
        if sim.step(1.0).iter().any(|e| matches!(e, Event::FleetArrived { fleet: id, .. } if *id == fleet)) {
            arrived = Some(tick);
            break;
        }
    }
    // Ticks are 1 s: the fleet arrives with the first tick at or after its travel time. Before, every one of the stopovers gave
    // it a whole tick of travel extra and it arrived that many seconds early.
    let arrived = arrived.expect("arrived") as f32;
    assert!(arrived >= eta - 1e-3 && arrived < eta + 1.0, "arrived after {arrived} s, travel time {eta} s over {} stopovers", route.len() - 2);
}

#[test]
fn a_leg_that_ends_on_a_tick_goes_on_without_an_extra_tick() {
    let Some(data) = load() else { return };
    let (mut sim, _, fleet, path) = setup(&data, 2);
    let w = sim.world().clone();
    let f = sim.galaxy().fleets[&fleet].clone();
    let (v1, v2) = (lane_speed(&w, &f, path[0], path[1]), lane_speed(&w, &f, path[1], path[2]));
    let route = w.route_between(path[0], path[1]).unwrap();
    let len = w.routes[route as usize].length;
    assert!(len > 2.0 * v1, "first leg longer than two ticks");
    // Two ticks before the stopover, bound for the planet after it.
    sim.galaxy_mut().fleets.get_mut(&fleet).unwrap().location =
        FleetLocation::Moving { from: path[0], to: path[1], route, progress: len - 2.0 * v1, rest: path[2..3].to_vec() };
    sim.step(2.0);
    // At the stopover: the end of the first leg or the start of the second.
    match sim.galaxy().fleets[&fleet].location.clone() {
        FleetLocation::Moving { to, progress, .. } if to == path[1] => assert!(len - progress < 1e-3 * v1, "{progress} of {len}"),
        FleetLocation::Moving { to, progress, .. } if to == path[2] => assert!(progress < 1e-3 * v2, "{progress} on the second leg"),
        other => panic!("{other:?}"),
    }
    sim.step(1.0);
    let FleetLocation::Moving { to, progress, .. } = sim.galaxy().fleets[&fleet].location.clone() else { panic!("arrived early") };
    assert_eq!(to, path[2]);
    assert!((progress - v2).abs() < 1e-3 * v2, "one tick into the second leg: {progress}, expected {v2}");
}
