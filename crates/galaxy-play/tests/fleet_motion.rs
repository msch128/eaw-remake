//! The map's fleet placement against the simulation on the real Remake data; skipped when the game is missing.

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use bevy::prelude::Vec3;
use galaxy_play::{FleetPlace, GalaxyLayout};
use galaxy_play::geometry::fleet_place;
use galaxy_sim::{Command, Galaxy, Sim, SimConfig};

fn load() -> Option<GameData> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    Some(load_galaxy(&fs).0)
}

/// Map position of a place; a fleet in orbit counts as standing on its planet (the end of the lane it came by).
fn position(p: FleetPlace, layout: &GalaxyLayout) -> Vec3 {
    match p {
        FleetPlace::Lane { from, to, t, .. } => layout.positions[from as usize].lerp(layout.positions[to as usize], t),
        FleetPlace::Orbit(at) => layout.positions[at as usize],
    }
}

/// A fleet on a multi-hop trip: extrapolated from an old snapshot, it is where the simulation has it seconds later, across the
/// planets on the way and up to the arrival. Before, the placement stopped at the end of the snapshot's leg until the next one.
#[test]
fn extrapolation_follows_the_route_across_legs() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { auto_resolve_battles: false, victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let rebel = w.faction_id("Rebel").unwrap();
    sim.galaxy_mut().fleets.clear();
    sim.galaxy_mut().battles.clear();
    // Every planet the fleet's own: no star base on the way stops it.
    for p in sim.galaxy_mut().planets.iter_mut() {
        p.owner = Some(rebel);
    }
    // A planet and a destination at least four hops away.
    let g = sim.galaxy().clone();
    let path = (0..w.planets.len() as u32)
        .filter(|&p| w.real[p as usize] && g.planets[p as usize].owner == Some(rebel))
        .flat_map(|a| (0..w.planets.len() as u32).filter(|&b| w.real[b as usize]).map(move |b| (a, b)))
        .filter_map(|(a, b)| w.find_path(a, b))
        .find(|p| p.len() >= 5 && p[1..p.len() - 1].iter().all(|&q| !w.blockades(&g, q, rebel)))
        .expect("a long free route");
    let fleet = sim.spawn_units(rebel, path[0], w.unit_id("MC-80_Star_Cruiser").unwrap(), 1);
    sim.apply(Command::MoveFleet { faction: rebel, fleet, to: *path.last().unwrap() }).expect("move");
    const LAG: usize = 3;
    let mut history: std::collections::VecDeque<Galaxy> = Default::default();
    let layout = GalaxyLayout::from_world(&w);
    let mut legs = std::collections::BTreeSet::new();
    for tick in 0..20_000 {
        history.push_back(sim.galaxy().clone());
        if history.len() > LAG + 1 {
            history.pop_front();
        }
        if tick >= LAG {
            let old = &history[0];
            let now = sim.galaxy();
            let want = fleet_place(&w, now, &now.fleets[&fleet], 0.0);
            let got = fleet_place(&w, old, &old.fleets[&fleet], LAG as f32);
            let miss = position(got, &layout).distance(position(want, &layout));
            // Exact up to rounding; the old clamp at the end of a leg was up to 3 s of travel off.
            assert!(miss < 0.01, "tick {tick}: extrapolated {got:?} is {miss} away from the simulation's {want:?}");
            if let FleetPlace::Lane { from, to, .. } = want {
                legs.insert((from, to));
            }
            if now.fleets[&fleet].planet().is_some() {
                break;
            }
        }
        sim.step(1.0);
    }
    assert_eq!(sim.galaxy().fleets[&fleet].planet(), path.last().copied(), "the fleet arrived");
    assert!(legs.len() >= 4, "crossed {} legs", legs.len());
}

/// The simulation stops a fleet at a planet on the way that blockades it: so does the extrapolation, instead of running on into
/// the next leg and jumping back.
#[test]
fn extrapolation_stops_at_a_blockade() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { auto_resolve_battles: false, victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let (rebel, empire) = (w.faction_id("Rebel").unwrap(), w.faction_id("Empire").unwrap());
    sim.galaxy_mut().fleets.clear();
    sim.galaxy_mut().battles.clear();
    // Every planet the fleet's own: no star base on the way stops it.
    for p in sim.galaxy_mut().planets.iter_mut() {
        p.owner = Some(rebel);
    }
    let g = sim.galaxy().clone();
    let path = (0..w.planets.len() as u32)
        .filter(|&p| w.real[p as usize] && g.planets[p as usize].owner == Some(rebel))
        .flat_map(|a| (0..w.planets.len() as u32).filter(|&b| w.real[b as usize]).map(move |b| (a, b)))
        .filter_map(|(a, b)| w.find_path(a, b))
        .find(|p| p.len() >= 4 && p[1..p.len() - 1].iter().all(|&q| !w.blockades(&g, q, rebel)))
        .expect("a free route");
    let fleet = sim.spawn_units(rebel, path[0], w.unit_id("MC-80_Star_Cruiser").unwrap(), 1);
    sim.apply(Command::MoveFleet { faction: rebel, fleet, to: *path.last().unwrap() }).expect("move");
    // An Empire warship turns up at the first planet on the way after the fleet set out.
    sim.spawn_units(empire, path[1], w.unit_id("MC-80_Star_Cruiser").unwrap(), 1);
    let g = sim.galaxy();
    assert!(w.blockades(g, path[1], rebel), "the Empire ship blockades");
    let place = fleet_place(&w, g, &g.fleets[&fleet], 1.0e5);
    assert!(matches!(place, FleetPlace::Lane { from, to, t, .. } if from == path[0] && to == path[1] && t == 1.0), "{place:?}");
}

