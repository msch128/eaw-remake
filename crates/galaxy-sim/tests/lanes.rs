//! Hyperspace lanes: `HS_Speed_Factor` makes lanes faster and routing minimises travel time, not hops.

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

/// Travel time of a path in `length / HS_Speed_Factor` units.
fn path_time(w: &World, path: &[PlanetId]) -> f32 {
    path.windows(2)
        .map(|p| {
            let r = w.routes.iter().find(|r| (r.a == p[0] && r.b == p[1]) || (r.a == p[1] && r.b == p[0])).expect("a lane");
            r.length / r.hs_factor
        })
        .sum()
}

/// Fewest-hops path by breadth first search.
fn fewest_hops(w: &World, from: PlanetId, to: PlanetId) -> Vec<PlanetId> {
    let mut prev = vec![None; w.planets.len()];
    let mut seen = vec![false; w.planets.len()];
    seen[from as usize] = true;
    let mut q = std::collections::VecDeque::from([from]);
    while let Some(u) = q.pop_front() {
        for &(v, _) in &w.adjacency[u as usize] {
            if !seen[v as usize] {
                seen[v as usize] = true;
                prev[v as usize] = Some(u);
                q.push_back(v);
            }
        }
    }
    let mut path = vec![to];
    while let Some(p) = prev[*path.last().unwrap() as usize] {
        path.push(p);
    }
    path.reverse();
    path
}

#[test]
fn lanes_have_the_data_speed_and_routing_picks_the_fastest_route() {
    let Some(data) = load() else { return };
    let c = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let sim = Sim::new(&data, c, SimConfig::default()).unwrap();
    let w = sim.world();
    let factors: std::collections::BTreeSet<i32> = w.routes.iter().map(|r| (r.hs_factor * 100.0).round() as i32).collect();
    assert!(factors.contains(&100) && factors.contains(&200) && factors.contains(&300), "lane speeds come from the data: {factors:?}");
    // The visible line type and the speed agree: main lanes 3x, inter lanes 2x, capillaries 1x.
    for r in data.campaign_trade_routes(c) {
        let f = (r.hs_speed_factor * 100.0).round() as i32;
        match r.visible_line_name.as_deref() {
            Some("Web_Way_Main") => assert_eq!(f, 300, "{}", r.name),
            Some("Web_Way_Inter") => assert_eq!(f, 200, "{}", r.name),
            _ => {}
        }
    }
    let n = w.planets.len() as u32;
    let (mut longer_but_faster, mut checked) = (0, 0);
    for a in (0..n).filter(|&p| w.real[p as usize]).step_by(3) {
        for b in (0..n).filter(|&p| p != a && w.real[p as usize]).step_by(5) {
            let (Some(fast), hops) = (w.find_path(a, b), fewest_hops(w, a, b)) else { continue };
            checked += 1;
            let (tf, th) = (path_time(w, &fast), path_time(w, &hops));
            assert!(tf <= th * 1.02 + 2.0, "{a}->{b}: the chosen route takes {tf}, the fewest-hops route {th}");
            if fast.len() > hops.len() && tf < th * 0.98 {
                longer_but_faster += 1;
            }
        }
    }
    assert!(checked > 500);
    assert!(longer_but_faster > 0, "somewhere a detour over faster lanes beats the direct route");
}

#[test]
fn the_player_takes_the_fastest_route_and_the_ai_goes_around_blockades() {
    let Some(data) = load() else { return };
    let c = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let sim = Sim::new(&data, c, SimConfig::default()).unwrap();
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let base = w.starbases.iter().position(|b| b.level >= 1).expect("a star base type") as u32;
    // A pair whose fastest route has a middle planet that can be bypassed.
    let n = w.planets.len() as u32;
    let mut found = None;
    'search: for a in (0..n).filter(|&p| w.real[p as usize]) {
        for b in (0..n).filter(|&p| p != a && w.real[p as usize]) {
            let Some(path) = w.find_path(a, b) else { continue };
            if path.len() < 3 {
                continue;
            }
            let mid = path[1];
            if w.find_path_avoiding(a, b, |p| p == mid).is_some_and(|alt| !alt.contains(&mid)) {
                found = Some((a, b, mid));
                break 'search;
            }
        }
    }
    let (a, b, mid) = found.expect("a pair with a detour");
    let route_of = |around: bool| {
        let mut sim = Sim::new(&data, c, SimConfig::default()).unwrap();
        let g = sim.galaxy_mut();
        // Nothing else blockades: no owners, no fleets.
        for p in g.planets.iter_mut() {
            p.owner = None;
            p.starbase = None;
            p.structures.clear();
        }
        g.fleets.clear();
        g.planets[mid as usize].owner = Some(rebel);
        g.planets[mid as usize].starbase = Some(base);
        let id = g.fleets.keys().max().copied().unwrap_or(0) + 1;
        let unit = *w.units.iter().position(|u| u.name == "Star_Destroyer").map(|i| i as u32).as_ref().unwrap();
        g.fleets.insert(id, Fleet { id, owner: empire, units: [(unit, 1)].into(), location: FleetLocation::AtPlanet(a), previous_planet: a, slot: 0 });
        let cmd = if around { Command::MoveFleetAround { faction: empire, fleet: id, to: b } } else { Command::MoveFleet { faction: empire, fleet: id, to: b } };
        sim.apply(cmd).unwrap();
        let FleetLocation::Moving { to, rest, .. } = sim.galaxy().fleets[&id].location.clone() else { panic!("moving") };
        let mut route = vec![to];
        route.extend(rest);
        route
    };
    assert!(route_of(false).contains(&mid), "the player's order takes the fastest route, blockade or not");
    assert!(!route_of(true).contains(&mid), "the AI order goes around the blockading star base");
}
