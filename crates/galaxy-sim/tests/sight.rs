//! Fog of war on the galaxy map: presence, one lane around it, stealth and allies.

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

fn fleet(sim: &mut Sim, owner: FactionId, unit: UnitTypeId, location: FleetLocation) -> FleetId {
    let g = sim.galaxy_mut();
    let id = g.fleets.keys().max().copied().unwrap_or(0) + 1;
    g.fleets.insert(id, Fleet { id, owner, units: [(unit, 1)].into(), location, previous_planet: 0 });
    id
}

#[test]
fn the_galaxy_map_is_foggy_outside_presence_and_one_lane_around_it() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig::default()).unwrap();
    let w = sim.world().clone();
    let (rebel, empire) = (w.faction_id("Rebel").unwrap(), w.faction_id("Empire").unwrap());
    sim.galaxy_mut().fleets.clear();
    let isd = w.unit_id("Star_Destroyer").unwrap();

    // The Rebel viewer: its own planets are detailed, their neighbours seen, planets two jumps away from all of them hidden.
    let own: Vec<PlanetId> = sim.galaxy().owned_planets(rebel).collect();
    let map = sim.galaxy().sight_map(&w, rebel);
    assert!(own.iter().all(|&p| map[p as usize] == Sight::Detailed));
    let neighbour = (0..w.planets.len() as u32).find(|&p| map[p as usize] == Sight::Seen).expect("a neighbour");
    let far = (0..w.planets.len() as u32).find(|&p| map[p as usize] == Sight::Hidden && w.real[p as usize]).expect("a hidden planet");
    assert_eq!(sim.galaxy().planet_sight(&w, rebel, far), Sight::Hidden);
    assert_eq!(map.iter().filter(|&&s| s == Sight::Detailed).count(), own.len(), "no fleets, so presence is just the owned planets");

    // Enemy fleets: unseen far away, seen (composition unknown) next door, in full on the viewer's own planet.
    let at_far = fleet(&mut sim, empire, isd, FleetLocation::AtPlanet(far));
    let at_near = fleet(&mut sim, empire, isd, FleetLocation::AtPlanet(neighbour));
    let at_home = fleet(&mut sim, empire, isd, FleetLocation::AtPlanet(own[0]));
    let g = sim.galaxy();
    assert_eq!(g.fleet_sight(&w, rebel, at_far), Sight::Hidden);
    assert_eq!(g.fleet_sight(&w, rebel, at_near), Sight::Seen);
    assert_eq!(g.fleet_sight(&w, rebel, at_home), Sight::Detailed);
    let visible: Vec<FleetId> = g.visible_fleets(&w, rebel).into_iter().map(|x| x.0).collect();
    assert_eq!(visible, vec![at_near, at_home], "hidden fleets are left out");
    // Everyone sees their own fleets, and the fog is per viewer.
    assert_eq!(g.fleet_sight(&w, empire, at_far), Sight::Detailed);
    assert_eq!(g.fleet_sight(&w, rebel, 9999), Sight::Hidden, "unknown fleet");
    assert_eq!(g.sight_map(&w, rebel), g.sight_map(&w, rebel), "deterministic");

    // A fleet that has not yet left the fog but is on a lane next to a seen planet is seen; one in the dark is not.
    let route = w.routes.iter().find(|r| (r.a == neighbour) != (r.b == neighbour)).expect("a lane at the neighbour");
    let other = if route.a == neighbour { route.b } else { route.a };
    let ridx = w.routes.iter().position(|r| std::ptr::eq(r, route)).unwrap() as u32;
    let moving = fleet(&mut sim, empire, isd, FleetLocation::Moving { from: neighbour, to: other, route: ridx, progress: 1.0, rest: vec![] });
    assert_eq!(sim.galaxy().fleet_sight(&w, rebel, moving), Sight::Seen);
    sim.galaxy_mut().fleets.get_mut(&moving).unwrap().location = FleetLocation::Moving { from: far, to: far, route: ridx, progress: 1.0, rest: vec![] };
    assert_eq!(sim.galaxy().fleet_sight(&w, rebel, moving), Sight::Hidden);

    // Allies share their sight.
    sim.galaxy_mut().allied.insert((rebel.min(empire), rebel.max(empire)));
    assert_eq!(sim.galaxy().fleet_sight(&w, rebel, at_far), Sight::Detailed);
}

#[test]
fn stealth_fleets_are_only_found_by_those_in_their_system() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig::default()).unwrap();
    let w = sim.world().clone();
    let (rebel, empire) = (w.faction_id("Rebel").unwrap(), w.faction_id("Empire").unwrap());
    sim.galaxy_mut().fleets.clear();
    let stealth_unit = (0..w.units.len() as u32).find(|&u| w.units[u as usize].galactic_stealth && matches!(w.units[u as usize].role, UnitRole::Ship)).expect("a stealth unit in the data");
    let map = sim.galaxy().sight_map(&w, rebel);
    let neighbour = (0..w.planets.len() as u32).find(|&p| map[p as usize] == Sight::Seen).unwrap();
    let hidden_friend = fleet(&mut sim, empire, stealth_unit, FleetLocation::AtPlanet(neighbour));
    // Next door a stealth fleet is not seen, although an ordinary fleet there is.
    assert_eq!(sim.galaxy().fleet_sight(&w, rebel, hidden_friend), Sight::Hidden);
    // A Rebel fleet in the same system finds it.
    let isd = w.unit_id("Star_Destroyer").unwrap();
    fleet(&mut sim, rebel, isd, FleetLocation::AtPlanet(neighbour));
    assert_eq!(sim.galaxy().fleet_sight(&w, rebel, hidden_friend), Sight::Detailed);
}

#[test]
fn corvette_provides_detailed_reconnaissance_sight_to_neighbor_systems() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig::default()).unwrap();
    let w = sim.world().clone();
    let (rebel, empire) = (w.faction_id("Rebel").unwrap(), w.faction_id("Empire").unwrap());
    sim.galaxy_mut().fleets.clear();
    let own: Vec<PlanetId> = sim.galaxy().owned_planets(rebel).collect();
    let map_before = sim.galaxy().sight_map(&w, rebel);
    let neighbour = (0..w.planets.len() as u32).find(|&p| map_before[p as usize] == Sight::Seen).expect("a neighbour");

    // An enemy fleet at the neighbour is only Seen without reconnaissance
    let isd = w.unit_id("Star_Destroyer").unwrap();
    let enemy_fleet = fleet(&mut sim, empire, isd, FleetLocation::AtPlanet(neighbour));
    assert_eq!(sim.galaxy().fleet_sight(&w, rebel, enemy_fleet), Sight::Seen);

    // Park a Corvette at a neighboring own planet
    let home = w.adjacency[neighbour as usize].iter().find(|&&(p, _)| own.contains(&p)).map(|&(p, _)| p).expect("home neighbor");
    let corvette = (0..w.units.len() as u32).find(|&u| w.units[u as usize].category == alamo_data::ShipCategory::Corvette).expect("corvette unit");
    fleet(&mut sim, rebel, corvette, FleetLocation::AtPlanet(home));

    // Now sight on the neighbour system and enemy fleet is Detailed
    let map_after = sim.galaxy().sight_map(&w, rebel);
    assert_eq!(map_after[neighbour as usize], Sight::Detailed);
    assert_eq!(sim.galaxy().fleet_sight(&w, rebel, enemy_fleet), Sight::Detailed);
}
