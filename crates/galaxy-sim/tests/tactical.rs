//! Galaxy -> tactical -> galaxy round trip. Real-data tests skip without the game.

use std::sync::Arc;

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use battle_sim::{Arena, Catalog};
use galaxy_sim::*;

struct Ctx {
    data: GameData,
    fs: GameFs,
}

fn load() -> Option<Ctx> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    let data = load_galaxy(&fs).0;
    Some(Ctx { data, fs })
}

fn new_sim(ctx: &Ctx) -> Sim {
    let campaign = ctx.data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    Sim::new(&ctx.data, campaign, SimConfig { auto_resolve_battles: false, victory_enabled: false, ..SimConfig::default() }).expect("sim")
}

fn add_fleet(sim: &mut Sim, owner: FactionId, unit: UnitTypeId, n: u32, at: PlanetId, from: PlanetId) -> FleetId {
    let g = sim.galaxy_mut();
    let id = g.fleets.keys().max().copied().unwrap_or(0) + 1;
    g.fleets.insert(id, Fleet { id, owner, units: [(unit, n)].into(), location: FleetLocation::AtPlanet(at), previous_planet: from, slot: 0 });
    id
}

fn empty_planet(sim: &Sim) -> PlanetId {
    let (w, g) = (sim.world(), sim.galaxy());
    (0..w.planets.len() as u32)
        .find(|&p| w.real[p as usize] && w.planets[p as usize].space_map.is_some() && g.planets[p as usize].owner.is_none() && g.fleets_at(p).next().is_none())
        .expect("empty planet with a map")
}

fn neighbour(w: &World, p: PlanetId) -> PlanetId {
    w.routes.iter().find_map(|r| (r.a == p).then_some(r.b).or((r.b == p).then_some(r.a))).expect("a route")
}

fn pending(sim: &mut Sim) -> PendingBattle {
    sim.step(1.0).into_iter().find_map(|e| if let Event::BattlePending(b) = e { Some(b) } else { None }).expect("pending battle")
}

fn catalog_for(ctx: &Ctx, req: &TacticalRequest) -> Arc<Catalog> {
    let names: Vec<&str> = req.sides.iter().flat_map(|s| s.groups.iter().map(|g| g.name.as_str())).collect();
    Arc::new(Catalog::build(&ctx.data, Some(&ctx.fs), &names))
}

#[test]
fn full_round_trip_headless() {
    let Some(ctx) = load() else { return };
    let mut sim = new_sim(&ctx);
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let target = empty_planet(&sim);
    let (isd, xw) = (w.unit_id("Star_Destroyer").unwrap(), w.unit_id("X-Wing_Squadron").unwrap());
    let ef = add_fleet(&mut sim, empire, isd, 2, target, target);
    let rf = add_fleet(&mut sim, rebel, xw, 3, target, target);
    let battle = pending(&mut sim);

    // 1. Setup: sides, groups, map.
    let req = sim.tactical_setup(battle.id).expect("request");
    assert_eq!(req.planet, target);
    assert_eq!(req.map, w.planets[target as usize].space_map);
    assert_eq!(req.sides.len(), 2);
    let side = |f| req.sides.iter().position(|s| s.faction == f).unwrap();
    let (es, rs) = (side(empire), side(rebel));
    assert_eq!(req.sides[es].groups[0].fleet, Some(ef));
    assert_eq!(req.sides[rs].groups[0].fleet, Some(rf));
    assert_eq!((req.sides[es].groups[0].count, req.sides[rs].groups[0].count), (2, 3));
    assert_eq!(sim.tactical_setup(battle.id).unwrap(), req, "setup is stable");

    // 2. Play it headless on the planet's real map.
    let catalog = catalog_for(&ctx, &req);
    let arena = req.map.as_ref().and_then(|m| Arena::load(&ctx.data, &ctx.fs, m)).map(Arc::new);
    let result = run_headless(&req, &catalog, arena).expect("result");
    assert_eq!(result.winner, Some(es), "2 ISD beat 3 X-wing squadrons: {result:?}");
    assert_eq!(result.survived[rs], vec![0]);
    assert!(result.survived[es][0] >= 1);

    // 3. Apply it.
    let events = sim.apply_tactical_result(battle.id, &result).expect("applied");
    assert!(events.iter().any(|e| matches!(e, Event::BattleResolved { winner: Some(f), .. } if *f == empire)));
    assert!(events.iter().any(|e| matches!(e, Event::BattleLost { faction, .. } if *faction == rebel)));
    let g = sim.galaxy();
    assert!(g.battles.is_empty());
    assert!(!g.fleets.contains_key(&rf), "the beaten fleet is gone");
    assert_eq!(g.fleets[&ef].units[&isd], result.survived[es][0]);
    assert_eq!(sim.apply_tactical_result(battle.id, &result), Err(TacticalError::UnknownBattle));

    // 4. The winner alone in orbit captures the unowned planet through the normal rule.
    let captured = sim.step(10.0).into_iter().any(|e| matches!(e, Event::PlanetCaptured { planet, new_owner, .. } if planet == target && new_owner == empire));
    assert!(captured);
    assert_eq!(sim.galaxy().planets[target as usize].owner, Some(empire));
}

#[test]
fn defender_with_starbase_beats_a_small_raid() {
    let Some(ctx) = load() else { return };
    let mut sim = new_sim(&ctx);
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let home = w.planet_id("Coruscant_WW").unwrap();
    assert!(sim.galaxy().planets[home as usize].starbase.is_some());
    let xw = w.unit_id("X-Wing_Squadron").unwrap();
    let rf = add_fleet(&mut sim, rebel, xw, 1, home, neighbour(&w, home));
    let battle = pending(&mut sim);
    let req = sim.tactical_setup(battle.id).unwrap();
    let ds = req.sides.len() - 1;
    assert!(req.starbase_level >= 1);
    assert!(req.sides[ds].defender);
    assert!(req.sides[ds].groups.iter().any(|g| g.kind == GroupKind::Starbase));
    let result = run_headless(&req, &catalog_for(&ctx, &req), None).expect("result");
    assert_eq!(result.winner, Some(ds), "{result:?}");
    sim.apply_tactical_result(battle.id, &result).unwrap();
    assert!(!sim.galaxy().fleets.contains_key(&rf));
    assert_eq!(sim.galaxy().planets[home as usize].owner, Some(empire));
    assert!(sim.galaxy().planets[home as usize].starbase.is_some());
}

#[test]
fn retreating_loser_falls_back_and_bad_results_are_rejected() {
    let Some(ctx) = load() else { return };
    let mut sim = new_sim(&ctx);
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let target = empty_planet(&sim);
    let from = neighbour(&w, target);
    let (isd, xw) = (w.unit_id("Star_Destroyer").unwrap(), w.unit_id("X-Wing_Squadron").unwrap());
    let ef = add_fleet(&mut sim, empire, isd, 2, target, target);
    let rf = add_fleet(&mut sim, rebel, xw, 3, target, from);
    let battle = pending(&mut sim);
    let req = sim.tactical_setup(battle.id).unwrap();
    let rs = req.sides.iter().position(|s| s.faction == rebel).unwrap();
    let es = 1 - rs;

    let bad = TacticalResult { winner: None, survived: vec![vec![1]], retreated: vec![vec![0]], duration: 1.0, ..Default::default() };
    assert_eq!(sim.apply_tactical_result(battle.id, &bad), Err(TacticalError::ShapeMismatch));
    assert_eq!(sim.apply_tactical_result(9999, &bad), Err(TacticalError::UnknownBattle));
    assert_eq!(sim.galaxy().battles.len(), 1, "a rejected result leaves the battle pending");

    // The rebels retreat with 2 of 3 squadrons, the Empire loses nothing.
    let mut result = TacticalResult { winner: Some(es), survived: vec![vec![0]; 2], retreated: vec![vec![0]; 2], duration: 90.0, ..Default::default() };
    result.survived[es] = vec![2];
    result.survived[rs] = vec![2];
    result.retreated[rs] = vec![2];
    sim.apply_tactical_result(battle.id, &result).unwrap();
    let g = sim.galaxy();
    assert_eq!(g.fleets[&ef].units[&isd], 2);
    assert_eq!(g.fleets[&rf].units[&xw], 2);
    assert!(matches!(g.fleets[&rf].location, FleetLocation::Moving { to, .. } if to == from), "retreat goes back to the origin planet");
}

#[test]
fn surviving_ships_heal_to_full_health_after_battle() {
    let Some(ctx) = load() else { return };
    let mut sim = new_sim(&ctx);
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let target = empty_planet(&sim);
    let (isd, xw) = (w.unit_id("Star_Destroyer").unwrap(), w.unit_id("X-Wing_Squadron").unwrap());
    let _ = xw;
    let ef = add_fleet(&mut sim, empire, isd, 2, target, target);
    add_fleet(&mut sim, rebel, w.unit_id("MC-80_Star_Cruiser").unwrap(), 1, target, target);
    let battle = pending(&mut sim);
    let req = sim.tactical_setup(battle.id).unwrap();
    assert!(req.sides.iter().flat_map(|s| &s.groups).all(|g| g.unit_health.is_empty()), "fresh ships are undamaged");
    let catalog = catalog_for(&ctx, &req);
    let result = run_headless(&req, &catalog, None).expect("result");
    let es = req.sides.iter().position(|s| s.faction == empire).unwrap();
    let survivors = result.survived[es][0];
    assert!(survivors >= 1, "2 ISD should beat 1 MC-80: {result:?}");
    let hp = &result.health[es][0];
    assert_eq!(hp.len() as u32, survivors);
    assert!(hp.iter().any(|h| h.hull < 0.999 || h.shield < 0.999), "winners of a real fight take some damage during combat: {hp:?}");
    sim.apply_tactical_result(battle.id, &result).unwrap();
    // In Empire at War, surviving ships heal back to 100% hull and shields upon battle completion:
    assert!(sim.galaxy().damage.is_empty(), "surviving ships are fully healed in the galactic layer upon battle completion");
    assert!(sim.galaxy().planets[target as usize].base_health.is_none(), "starbase health is also fully intact");

    // A subsequent battle starts the surviving ships completely undamaged:
    let rf2 = add_fleet(&mut sim, rebel, w.unit_id("MC-80_Star_Cruiser").unwrap(), 1, target, target);
    let _ = rf2;
    let b2 = pending(&mut sim);
    let req2 = sim.tactical_setup(b2.id).unwrap();
    let g = req2.sides.iter().flat_map(|s| &s.groups).find(|g| g.fleet == Some(ef)).unwrap();
    assert!(g.unit_health.is_empty(), "the new battle request has no damage for surviving ships");
}

#[test]
fn units_beyond_the_population_cap_wait_in_a_reserve_that_can_be_called_in() {
    let Some(ctx) = load() else { return };
    let mut sim = new_sim(&ctx);
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let target = empty_planet(&sim);
    let isd = w.unit_id("Star_Destroyer").unwrap();
    let pop = w.units[isd as usize].population;
    let cap = w.factions[empire as usize].space_unit_cap;
    let n = (cap / pop.max(1)) as u32 + 8;
    let ef = add_fleet(&mut sim, empire, isd, n, target, target);
    add_fleet(&mut sim, rebel, w.unit_id("Nebulon_B_Frigate").unwrap(), 1, target, target);
    let battle = pending(&mut sim);
    let req = sim.tactical_setup(battle.id).unwrap();
    let es = req.sides.iter().position(|s| s.faction == empire).unwrap();
    let side = &req.sides[es];
    let deployed: u32 = side.groups.iter().map(|g| g.count).sum();
    let reserve: u32 = side.reserve.iter().map(|g| g.count).sum();
    println!("cap {cap}, pop {pop}: {deployed} deployed, {reserve} in reserve");
    assert_eq!(deployed + reserve, n);
    assert!(reserve >= 8 && deployed as i32 * pop <= cap);
    assert_eq!(side.unit_cap, cap);

    // Interactive: the front end calls the reserve in; a battle that ends with them alive reports them.
    let catalog = catalog_for(&ctx, &req);
    let setup = req.battle_setup(&catalog, req.battle_config(None), |_| true).unwrap();
    let mut b = battle_sim::Battle::new(setup);
    let ship = catalog.ship_id(&side.reserve[0].name).unwrap();
    // Over the cap the sim refuses; once the deployed fleet has shrunk there is room. Here the request simply is not allowed to exceed it.
    assert!(b.command(es as u8, battle_sim::Command::Reinforce { ship, count: 8 }).is_err(), "no room above the cap");
    drop(b);

    // Headless: the reserve is called in only as losses make room under the cap, and everything that survives goes back into the fleet.
    let result = run_headless(&req, &catalog, None).unwrap();
    let called = result.reserve_called[es].iter().sum::<u32>();
    let lost = deployed.saturating_sub(result.survived[es].iter().sum::<u32>());
    assert!(called <= lost + 1 && called <= reserve, "called {called} of {reserve} with {lost} lost");
    assert_eq!(result.winner, Some(es));
    sim.apply_tactical_result(battle.id, &result).unwrap();
    let left = sim.galaxy().fleets.get(&ef).map_or(0, |f| f.unit_count());
    assert!(left + 2 >= n, "the winner keeps (nearly) all ships: {left}/{n}");
}

#[test]
fn cancelling_production_refunds_the_unspent_cost() {
    let Some(ctx) = load() else { return };
    let mut sim = new_sim(&ctx);
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let home = w.planet_id("Coruscant_WW").unwrap();
    let unit = *w.buildable_units[empire as usize].iter().find(|&&u| w.check_build_unit(sim.galaxy(), empire, home, u).is_ok() && w.units[u as usize].build_time > 5.0).expect("something Coruscant can build");
    let credits = |s: &Sim| s.galaxy().factions[empire as usize].credits;
    let start = credits(&sim);
    sim.apply(Command::BuildUnit { faction: empire, planet: home, unit }).unwrap();
    sim.apply(Command::BuildUnit { faction: empire, planet: home, unit }).unwrap();
    let cost = f64::from(w.units[unit as usize].cost);
    assert!((start - credits(&sim) - 2.0 * cost).abs() < 1.0);
    // The second entry has not started: full refund.
    let r = sim.apply(Command::CancelProduction { faction: empire, planet: home, lane: Lane::Units, index: 1 });
    assert_eq!(r, Ok(CommandOk::Refunded(cost)));
    // Let the first one run for a while: partial refund of the rest.
    sim.step(w.units[unit as usize].build_time * w.config.build_time_scale * 0.5);
    let before = credits(&sim);
    let Ok(CommandOk::Refunded(back)) = sim.apply(Command::CancelProduction { faction: empire, planet: home, lane: Lane::Units, index: 0 }) else { panic!("refund") };
    assert!(back > 0.3 * cost && back < 0.7 * cost, "about half is back: {back} of {cost}");
    assert!((credits(&sim) - before - back).abs() < 1e-6);
    assert!(sim.galaxy().planets[home as usize].queues[Lane::Units as usize].is_empty());
    // Others' planets and bad indices are refused.
    assert_eq!(sim.apply(Command::CancelProduction { faction: empire, planet: home, lane: Lane::Units, index: 5 }), Err(CommandError::Invalid));
    let rebel = w.faction_id("Rebel").unwrap();
    assert_eq!(sim.apply(Command::CancelProduction { faction: rebel, planet: home, lane: Lane::Units, index: 0 }), Err(CommandError::NotOwner));
}

#[test]
fn abandoning_a_half_fought_battle_keeps_the_losses() {
    let Some(ctx) = load() else { return };
    let mut sim = new_sim(&ctx);
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let target = empty_planet(&sim);
    let (isd, xw) = (w.unit_id("Star_Destroyer").unwrap(), w.unit_id("X-Wing_Squadron").unwrap());
    add_fleet(&mut sim, empire, isd, 2, target, target);
    let rf = add_fleet(&mut sim, rebel, xw, 6, target, neighbour(&w, target));
    let battle = pending(&mut sim);
    let req = sim.tactical_setup(battle.id).unwrap();
    // The player lost four of six squadrons before leaving the fight.
    let mut partial = TacticalResult { survived: req.sides.iter().map(|s| s.groups.iter().map(|g| g.count).collect()).collect(), retreated: req.sides.iter().map(|s| vec![0; s.groups.len()]).collect(), ..TacticalResult::default() };
    let rs = req.sides.iter().position(|s| s.faction == rebel).unwrap();
    let gi = req.sides[rs].groups.iter().position(|g| g.type_id == xw).unwrap();
    partial.survived[rs][gi] = 2;
    let events = sim.abandon_tactical(battle.id, &partial).expect("abandon");
    assert!(events.iter().any(|e| matches!(e, Event::UnitsDestroyed { faction, count: 4, .. } if *faction == rebel)), "{events:?}");
    assert_eq!(sim.galaxy().fleets[&rf].units[&xw], 2, "the losses are in the galaxy");
    assert!(sim.galaxy().battles.iter().any(|b| b.id == battle.id), "the battle is still pending");
    // Auto-resolving now starts from the two squadrons that are left.
    let events = sim.resolve_battle_auto(battle.id).expect("resolved");
    let lost: u32 = events.iter().filter_map(|e| if let Event::UnitsDestroyed { faction, count, .. } = e { (*faction == rebel).then_some(*count) } else { None }).sum();
    assert!(lost <= 2, "only what was left can be lost, got {lost}");
}

#[test]
fn a_sides_tech_index_counts_from_zero_at_the_games_start_level() {
    let Some(ctx) = load() else { return };
    let side_techs = |tech_of_empire: Option<i32>| {
        let mut sim = new_sim(&ctx);
        let w = sim.world().clone();
        let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
        if let Some(t) = tech_of_empire {
            sim.galaxy_mut().factions[empire as usize].tech = t;
        }
        let target = empty_planet(&sim);
        let (isd, xw) = (w.unit_id("Star_Destroyer").unwrap(), w.unit_id("X-Wing_Squadron").unwrap());
        add_fleet(&mut sim, empire, isd, 1, target, target);
        add_fleet(&mut sim, rebel, xw, 1, target, neighbour(&w, target));
        let battle = pending(&mut sim);
        let req = sim.tactical_setup(battle.id).unwrap();
        let by_name = |f: FactionId| req.sides.iter().find(|s| s.faction == f).unwrap().tech;
        (by_name(empire), by_name(rebel), sim.galaxy().factions[empire as usize].tech)
    };
    // The galaxy shows "TECH: 1" at the start: that is index 0, the `Tech_0` entries.
    let (empire, rebel, counter) = side_techs(None);
    assert_eq!((counter, empire, rebel), (1, 0, 0));
    assert_eq!(side_techs(Some(3)).0, 2);
    assert_eq!(side_techs(Some(5)).0, 4);
}

/// A lost battle at an empty planet: the rebels (3 squadrons) all retreat, the Empire wins. Returns the sim, the rebel fleet and the battle planet.
fn lost_battle(ctx: &Ctx, pick: impl FnOnce(&Sim) -> PlanetId, prepare: impl FnOnce(&mut Sim)) -> (Sim, FleetId, PlanetId, FactionId) {
    let mut sim = new_sim(ctx);
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let target = pick(&sim);
    prepare(&mut sim);
    let (isd, xw) = (w.unit_id("Star_Destroyer").unwrap(), w.unit_id("X-Wing_Squadron").unwrap());
    add_fleet(&mut sim, empire, isd, 2, target, target);
    let rf = add_fleet(&mut sim, rebel, xw, 3, target, neighbour(&w, target));
    let battle = pending(&mut sim);
    let req = sim.tactical_setup(battle.id).unwrap();
    let rs = req.sides.iter().position(|s| s.faction == rebel).unwrap();
    let mut result = TacticalResult { winner: Some(1 - rs), survived: vec![vec![0]; 2], retreated: vec![vec![0]; 2], duration: 90.0, ..Default::default() };
    result.survived[1 - rs] = vec![2];
    result.survived[rs] = vec![2];
    result.retreated[rs] = vec![2];
    sim.apply_tactical_result(battle.id, &result).unwrap();
    (sim, rf, target, rebel)
}

fn destination(sim: &Sim, fleet: FleetId) -> (PlanetId, Vec<PlanetId>) {
    match sim.galaxy().fleets[&fleet].location.clone() {
        FleetLocation::Moving { to, rest, .. } => {
            let last = rest.last().copied().unwrap_or(to);
            (last, std::iter::once(to).chain(rest).collect())
        }
        FleetLocation::AtPlanet(p) => (p, vec![p]),
    }
}

#[test]
fn escapees_fly_to_the_nearest_friendly_planet_and_blockades_do_not_stop_them() {
    let Some(ctx) = load() else { return };
    let (mut sim, rf, target, rebel) = lost_battle(&ctx, empty_planet, |_| {});
    let w = sim.world().clone();
    let (dest, path) = destination(&sim, rf);
    assert_eq!(sim.galaxy().planets[dest as usize].owner, Some(rebel), "the escape goes to a friendly planet");
    assert!(sim.galaxy().escaping.contains(&rf));
    // No other friendly planet is nearer (travel time of the fastest path).
    let time = |p: PlanetId| w.find_path(target, p).map(|path| path.windows(2).map(|e| { let r = &w.routes[w.route_between(e[0], e[1]).unwrap() as usize]; r.length / r.hs_factor }).sum::<f32>()).unwrap_or(f32::MAX);
    let best = sim.galaxy().owned_planets(rebel).filter(|&p| w.real[p as usize] && p != target).map(&time).fold(f32::MAX, f32::min);
    assert!((time(dest) - best).abs() < 1e-3, "nearest friendly planet: {} vs {best}", time(dest));
    // Put an Empire star base on a planet in the middle of the route (if it has one): the escapees still pass it, and nothing stops them on the way.
    let empire = w.faction_id("Empire").unwrap();
    let base = w.starbases.iter().position(|b| b.level >= 1).expect("a star base type") as u32;
    if let Some(&mid) = path.iter().take(path.len().saturating_sub(1)).skip(1).next() {
        let g = sim.galaxy_mut();
        g.planets[mid as usize].owner = Some(empire);
        g.planets[mid as usize].starbase = Some(base);
    }
    for _ in 0..4000 {
        if matches!(sim.galaxy().fleets.get(&rf).map(|f| &f.location), Some(FleetLocation::AtPlanet(_))) {
            break;
        }
        sim.step(5.0);
    }
    assert_eq!(sim.galaxy().fleets[&rf].location, FleetLocation::AtPlanet(dest), "no blockade stopped the escape move");
    assert!(!sim.galaxy().escaping.contains(&rf), "the escape ends on arrival");
}

#[test]
fn escapees_without_a_friendly_planet_take_the_nearest_neighbour_system() {
    let Some(ctx) = load() else { return };
    let (sim, rf, target, rebel) = lost_battle(&ctx, lane_not_first, |sim| {
        let g = sim.galaxy_mut();
        for p in g.planets.iter_mut() {
            p.owner = None;
            p.starbase = None;
            p.structures.clear();
        }
    });
    let w = sim.world().clone();
    assert_eq!(sim.galaxy().owned_planets(rebel).count(), 0, "the setup leaves no friendly planet");
    let (dest, path) = destination(&sim, rf);
    assert_eq!(path.len(), 1, "one lane to the neighbour: {path:?}");
    let cost = |r: &galaxy_sim::RouteInfo| r.length / r.hs_factor;
    let lanes: Vec<f32> = w.routes.iter().filter(|r| (r.a == target || r.b == target) && w.real[if r.a == target { r.b } else { r.a } as usize]).map(cost).collect();
    let used = w.routes[w.route_between(target, dest).unwrap() as usize].clone();
    assert!(lanes.iter().all(|&c| cost(&used) <= c + 1e-3), "the nearest neighbour system");
    assert!(sim.galaxy().escaping.contains(&rf));
}

/// An empty planet whose cheapest lane does not lead to the first lane's neighbour (so "the first route" and "the nearest neighbour" differ).
fn lane_not_first(sim: &Sim) -> PlanetId {
    let (w, g) = (sim.world(), sim.galaxy());
    let cost = |r: u32| w.routes[r as usize].length / w.routes[r as usize].hs_factor;
    (0..w.planets.len() as u32)
        .filter(|&p| w.real[p as usize] && g.planets[p as usize].owner.is_none() && g.fleets_at(p).next().is_none())
        .find(|&p| {
            let cheapest = w.adjacency[p as usize].iter().filter(|&&(to, _)| w.real[to as usize]).min_by(|a, b| cost(a.1).total_cmp(&cost(b.1)));
            cheapest.is_some_and(|&(to, r)| to != neighbour(w, p) && cost(r) + 1e-3 < cost(w.route_between(p, neighbour(w, p)).unwrap()))
        })
        .expect("a planet whose first lane is not its cheapest")
}

/// A defended, enemy-owned planet with a map and no star base: the Empire attacks with 2 ISD from a neighbour, the Rebels defend with a fleet.
/// Returns the sim, the pending battle id, both fleets and the planet.
fn defended_planet_battle(ctx: &Ctx) -> (Sim, u32, FleetId, FleetId, PlanetId) {
    let mut sim = new_sim(ctx);
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let target = (0..w.planets.len() as u32)
        .find(|&p| {
            let ps = &sim.galaxy().planets[p as usize];
            w.real[p as usize] && w.planets[p as usize].space_map.is_some() && ps.owner == Some(rebel)
        })
        .expect("a rebel planet with a map");
    let (isd, mc80) = (w.unit_id("Star_Destroyer").unwrap(), w.unit_id("MC-80_Star_Cruiser").unwrap());
    sim.galaxy_mut().planets[target as usize].starbase = None;
    let ef = add_fleet(&mut sim, empire, isd, 2, target, neighbour(&w, target));
    let df = add_fleet(&mut sim, rebel, mc80, 1, target, target);
    let battle = pending(&mut sim);
    (sim, battle.id, ef, df, target)
}

/// The fight as it stood when the side was given up: nobody lost anything.
fn untouched(req: &TacticalRequest) -> TacticalResult {
    TacticalResult {
        survived: req.sides.iter().map(|s| s.groups.iter().map(|g| g.count).collect()).collect(),
        retreated: req.sides.iter().map(|s| vec![0; s.groups.len()]).collect(),
        duration: 94.0,
        ..TacticalResult::default()
    }
}

#[test]
fn attacker_retreating_from_a_defended_planet_captures_nothing_and_leaves() {
    let Some(ctx) = load() else { return };
    let (mut sim, id, ef, df, target) = defended_planet_battle(&ctx);
    let (empire, rebel) = (sim.world().faction_id("Empire").unwrap(), sim.world().faction_id("Rebel").unwrap());
    let req = sim.tactical_setup(id).unwrap();
    let events = sim.retreat_tactical(id, empire, &untouched(&req)).expect("retreat");
    assert!(events.iter().any(|e| matches!(e, Event::BattleResolved { winner: Some(f), .. } if *f == rebel)), "the defender wins: {events:?}");
    assert!(sim.galaxy().battles.is_empty());
    assert_eq!(sim.galaxy().planets[target as usize].owner, Some(rebel));
    // The attacker is already on its way (not at the planet for a tick) to a friendly planet.
    let g = sim.galaxy();
    assert!(matches!(g.fleets[&ef].location, FleetLocation::Moving { .. }), "{:?}", g.fleets[&ef].location);
    assert!(g.escaping.contains(&ef));
    let (dest, _) = destination(&sim, ef);
    assert_eq!(g.planets[dest as usize].owner, Some(empire), "the escape goes to a friendly planet");
    assert_ne!(dest, target);
    assert_eq!(g.fleets[&df].location, FleetLocation::AtPlanet(target));
    // Time passes: no capture and no second battle at the planet; the fleet arrives at the friendly planet.
    for _ in 0..400 {
        let events = sim.step(5.0);
        assert!(!events.iter().any(|e| matches!(e, Event::PlanetCaptured { planet, .. } if *planet == target)), "{events:?}");
        assert!(!events.iter().any(|e| matches!(e, Event::BattlePending(b) if b.planet == target)), "{events:?}");
        assert_eq!(sim.galaxy().planets[target as usize].owner, Some(rebel));
        assert!(sim.galaxy().battles.iter().all(|b| b.planet != target));
    }
    assert_eq!(sim.galaxy().fleets[&ef].location, FleetLocation::AtPlanet(dest));
}

#[test]
fn defender_retreating_leaves_the_planet_to_the_attacker_by_the_normal_rule() {
    let Some(ctx) = load() else { return };
    let (mut sim, id, ef, df, target) = defended_planet_battle(&ctx);
    let (empire, rebel) = (sim.world().faction_id("Empire").unwrap(), sim.world().faction_id("Rebel").unwrap());
    let req = sim.tactical_setup(id).unwrap();
    let events = sim.retreat_tactical(id, rebel, &untouched(&req)).expect("retreat");
    assert!(events.iter().any(|e| matches!(e, Event::BattleResolved { winner: Some(f), .. } if *f == empire)), "the attacker wins: {events:?}");
    assert!(sim.galaxy().battles.is_empty());
    let g = sim.galaxy();
    assert!(matches!(g.fleets[&df].location, FleetLocation::Moving { .. }) && g.escaping.contains(&df), "the defender fleet escapes");
    assert_eq!(g.fleets[&ef].location, FleetLocation::AtPlanet(target));
    let (dest, _) = destination(&sim, df);
    assert_eq!(g.planets[dest as usize].owner, Some(rebel));
    for _ in 0..20 {
        let events = sim.step(5.0);
        assert!(!events.iter().any(|e| matches!(e, Event::BattlePending(b) if b.planet == target)), "{events:?}");
    }
    assert_eq!(sim.galaxy().planets[target as usize].owner, Some(empire), "the winner alone in orbit captures");
}

/// The app path of Esc -> Retreat: `Command::Retreat` once the countdown allows it, the jump timer runs, the battle ends and its normal result
/// (winner = defender, `retreated` set) is applied. No capture, the attacker flees, no second battle.
#[test]
fn retreat_order_in_the_battle_ends_without_capture() {
    let Some(ctx) = load() else { return };
    let (mut sim, id, ef, _df, target) = defended_planet_battle(&ctx);
    let (empire, rebel) = (sim.world().faction_id("Empire").unwrap(), sim.world().faction_id("Rebel").unwrap());
    let req = sim.tactical_setup(id).unwrap();
    let es = req.sides.iter().position(|s| s.faction == empire).unwrap();
    let catalog = catalog_for(&ctx, &req);
    let config = battle_sim::BattleConfig { record_events: false, retreat_countdown: 5.0, ..req.battle_config(None) };
    let dt = config.dt;
    let mut battle = battle_sim::Battle::new(req.battle_setup(&catalog, config, |si| si != es).expect("setup"));
    let mut steps = 0u64;
    let mut ordered = false;
    while !battle.is_over() && steps < (900.0 / dt) as u64 {
        battle.step();
        steps += 1;
        if !ordered && steps as f32 * dt > 6.0 {
            battle.command(es as u8, battle_sim::Command::Retreat).expect("retreat allowed after the countdown");
            ordered = true;
        }
    }
    assert!(battle.is_over(), "the retreat ends the battle");
    let result = req.result_from(&catalog, &battle.result()).expect("result");
    assert_eq!(result.winner, Some(1 - es), "the defender wins a battle the attacker left: {result:?}");
    assert!(result.retreated[es].iter().sum::<u32>() > 0, "{result:?}");
    sim.apply_tactical_result(id, &result).unwrap();
    assert!(matches!(sim.galaxy().fleets[&ef].location, FleetLocation::Moving { .. }), "the attacker is on its way out");
    for _ in 0..200 {
        let events = sim.step(5.0);
        assert!(!events.iter().any(|e| matches!(e, Event::PlanetCaptured { planet, .. } if *planet == target)), "{events:?}");
        assert!(!events.iter().any(|e| matches!(e, Event::BattlePending(b) if b.planet == target)), "{events:?}");
    }
    assert_eq!(sim.galaxy().planets[target as usize].owner, Some(rebel));
}

#[test]
fn an_occupied_hyperspace_slot_deploys_only_that_unit_and_an_empty_one_changes_nothing() {
    let Some(ctx) = load() else { return };
    let w = new_sim(&ctx).world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let (isd, vsd) = (w.unit_id("Star_Destroyer").unwrap(), w.unit_id("Victory_Destroyer").unwrap());
    // The request is built once per pending battle, so every slot setting gets its own sim. Returns (deployed ISD, VSD, reserve ISD, VSD).
    let run = |slot: Option<UnitTypeId>| -> (u32, u32, u32, u32) {
        let mut sim = new_sim(&ctx);
        let target = empty_planet(&sim);
        let ef = add_fleet(&mut sim, empire, isd, 2, target, target);
        sim.galaxy_mut().fleets.get_mut(&ef).unwrap().units.insert(vsd, 1);
        add_fleet(&mut sim, rebel, w.unit_id("Nebulon_B_Frigate").unwrap(), 1, target, target);
        if slot.is_some() {
            sim.apply(Command::SetHyperspace { faction: empire, fleet: ef, unit: slot }).unwrap();
        }
        let battle = pending(&mut sim);
        let req = sim.tactical_setup(battle.id).unwrap();
        let es = req.sides.iter().position(|s| s.faction == empire).unwrap();
        let count = |reserve: bool, unit: &str| -> u32 {
            let list = if reserve { &req.sides[es].reserve } else { &req.sides[es].groups };
            list.iter().filter(|g| g.name.eq_ignore_ascii_case(unit)).map(|g| g.count).sum()
        };
        let out = (count(false, "Star_Destroyer"), count(false, "Victory_Destroyer"), count(true, "Star_Destroyer"), count(true, "Victory_Destroyer"));
        println!("slot {slot:?}: deployed ISD {} VSD {}, reserve ISD {} VSD {}", out.0, out.1, out.2, out.3);
        if slot == Some(vsd) {
            // The battle really starts with the slot unit only (plus what it carries): no Star Destroyer is in it.
            let catalog = catalog_for(&ctx, &req);
            let b = battle_sim::Battle::new(req.battle_setup(&catalog, req.battle_config(None), |_| true).unwrap());
            let names: Vec<String> = b.ships.iter().filter(|s| s.side == es as u8).map(|s| catalog.ship(s.def).name.to_ascii_lowercase()).collect();
            println!("battle starts with {names:?}");
            assert!(names.iter().any(|n| n.contains("victory")) && !names.iter().any(|n| n.contains("star_destroyer")), "{names:?}");
        }
        out
    };
    assert_eq!(run(None), (2, 1, 0, 0), "empty slot: everything under the cap is deployed");
    assert_eq!(run(Some(vsd)), (0, 1, 2, 0), "slot holds the Victory: only it jumps in");
    assert_eq!(run(Some(isd)), (1, 0, 1, 1), "slot holds one Star Destroyer of the stack: one deployed, the rest wait");
}

/// A pirate planet without a fleet (it has a star base and its garrison) attacked by `n` units of `unit`: (auto-resolve preview of the attacker: start, alive;
/// tactical headless result: start, survived).
fn pirate_planet_attack(ctx: &Ctx, unit: &str, n: u32) -> ((u32, u32), (u32, u32), bool) {
    let mut sim = new_sim(ctx);
    let w = sim.world().clone();
    let (pirates, rebel) = (w.faction_id("Pirates").unwrap(), w.faction_id("Rebel").unwrap());
    let target = {
        let g = sim.galaxy();
        (0..w.planets.len() as u32)
            .find(|&p| g.planets[p as usize].owner == Some(pirates) && g.fleets_at(p).next().is_none() && w.planets[p as usize].space_map.is_some())
            .expect("pirate planet without a fleet")
    };
    add_fleet(&mut sim, rebel, w.unit_id(unit).unwrap(), n, target, target);
    let battle = pending(&mut sim);
    let preview = sim.preview_auto_resolve(battle.id, 1).expect("preview");
    let me = preview.sides.iter().find(|s| s.0 == rebel).unwrap();
    let req = sim.tactical_setup(battle.id).unwrap();
    let catalog = catalog_for(ctx, &req);
    let result = run_headless(&req, &catalog, None).unwrap();
    let rs = req.sides.iter().position(|s| s.faction == rebel).unwrap();
    let survived: u32 = result.survived[rs].iter().sum();
    ((me.1, me.2), (n, survived), preview.winner == Some(rebel))
}

#[test]
fn a_pirate_star_base_with_its_garrison_counts_in_both_resolvers() {
    let Some(ctx) = load() else { return };
    // The star base, its spawned garrison squadrons (Hullcracker, Z-95) and structures are part of the defender in the request and in the statistical resolver:
    // a lone corvette dies in both, while a real fleet beats the level 1 base without losses in both (the data's pirate bases are that weak).
    let (auto, tactical, auto_win) = pirate_planet_attack(&ctx, "CR-90_Corvette", 1);
    println!("1 x CR-90: auto alive {}/{} win {auto_win}, tactical survived {}/{}", auto.1, auto.0, tactical.1, tactical.0);
    assert!(!auto_win && auto.1 == 0 && tactical.1 == 0, "a corvette alone cannot take a pirate base");
    let (auto, tactical, auto_win) = pirate_planet_attack(&ctx, "MC-80_Star_Cruiser", 4);
    println!("4 x MC-80: auto alive {}/{} win {auto_win}, tactical survived {}/{}", auto.1, auto.0, tactical.1, tactical.0);
    assert!(auto_win && auto.1 == 4 && tactical.1 == 4);
}

#[test]
fn the_battle_ends_with_the_last_brought_unit_whatever_was_spawned_meanwhile() {
    let Some(ctx) = load() else { return };
    let mut sim = new_sim(&ctx);
    let w = sim.world().clone();
    let (pirates, rebel) = (w.faction_id("Pirates").unwrap(), w.faction_id("Rebel").unwrap());
    let target = {
        let g = sim.galaxy();
        (0..w.planets.len() as u32)
            .find(|&p| g.planets[p as usize].owner == Some(pirates) && g.fleets_at(p).next().is_none() && w.planets[p as usize].space_map.is_some())
            .expect("pirate planet without a fleet")
    };
    add_fleet(&mut sim, rebel, w.unit_id("MC-80_Star_Cruiser").unwrap(), 4, target, target);
    let battle = pending(&mut sim);
    let req = sim.tactical_setup(battle.id).unwrap();
    let catalog = catalog_for(&ctx, &req);
    let mut b = battle_sim::Battle::new(req.battle_setup(&catalog, req.battle_config(None), |_| true).unwrap());
    let ps = req.sides.iter().position(|s| s.faction == pirates).unwrap() as u8;
    let mut base_died = None;
    let mut spawned_alive_then = 0;
    while !b.is_over() && b.time() < 600.0 {
        b.step();
        let base_alive = b.ships.iter().any(|s| s.side == ps && s.alive && catalog.ship(s.def).is_static);
        if base_died.is_none() && !base_alive {
            base_died = Some(b.time());
            spawned_alive_then = b.ships.iter().filter(|s| s.side == ps && s.alive).count();
        }
    }
    let died = base_died.expect("the star base died");
    println!("base died at {died:.1} s with {spawned_alive_then} spawned units alive, battle over at {:.1} s; side ships {:?}", b.time(), b.ships.iter().filter(|s| s.side == ps).map(|s| (catalog.ship(s.def).name.clone(), s.alive, s.group)).take(6).collect::<Vec<_>>());
    // Whatever of the garrison is still alive when the last brought unit (the star base) dies is ignored: the battle ends with that step.
    assert!(b.is_over() && b.time() - died < 2.0, "the battle ends with the last unit that was brought, not with the spawned ones");
    assert_eq!(b.result().winner, Some(req.sides.iter().position(|s| s.faction == rebel).unwrap() as u8));
}

/// Auto-resolve preview of `n` units of `unit` (Rebel) attacking `defender` at a planet it holds with its starbase / structures / fleets: `(attacker start, alive, defender start, alive, attacker wins)`.
fn auto_vs_planet(ctx: &Ctx, unit: &str, n: u32, planet_pick: impl Fn(&Sim, FactionId) -> Option<PlanetId>, defender: &str, extra_def: Option<(&str, u32)>) -> Option<(u32, u32, u32, u32, bool)> {
    let mut sim = new_sim(ctx);
    let w = sim.world().clone();
    let (rebel, def) = (w.faction_id("Rebel").unwrap(), w.faction_id(defender).unwrap());
    let target = planet_pick(&sim, def)?;
    if let Some((u, c)) = extra_def {
        add_fleet(&mut sim, def, w.unit_id(u).unwrap(), c, target, target);
    }
    add_fleet(&mut sim, rebel, w.unit_id(unit).unwrap(), n, target, target);
    let battle = pending(&mut sim);
    let p = sim.preview_auto_resolve(battle.id, 1)?;
    let a = p.sides.iter().find(|s| s.0 == rebel)?;
    let d = p.sides.iter().find(|s| s.0 == def)?;
    Some((a.1, a.2, d.1, d.2, p.winner == Some(rebel)))
}

/// Two fleets meet at an empty planet; auto-resolve preview over several seeds: `(attacker wins, draws, mean attacker alive, mean defender alive)` out of `seeds` runs.
fn auto_even(ctx: &Ctx, a: (&str, &str, u32), b: (&str, &str, u32), seeds: u64) -> (u32, u32, f32, f32) {
    let mut sim = new_sim(ctx);
    let w = sim.world().clone();
    let (fa, fb) = (w.faction_id(a.0).unwrap(), w.faction_id(b.0).unwrap());
    let target = empty_planet(&sim);
    add_fleet(&mut sim, fb, w.unit_id(b.1).unwrap(), b.2, target, target);
    add_fleet(&mut sim, fa, w.unit_id(a.1).unwrap(), a.2, target, target);
    let battle = pending(&mut sim);
    let (mut wins, mut draws, mut aa, mut ba) = (0, 0, 0.0, 0.0);
    for seed in 1..=seeds {
        let p = sim.preview_auto_resolve(battle.id, seed).unwrap();
        wins += u32::from(p.winner == Some(fa));
        draws += u32::from(p.winner.is_none());
        aa += p.sides.iter().find(|s| s.0 == fa).unwrap().2 as f32;
        ba += p.sides.iter().find(|s| s.0 == fb).unwrap().2 as f32;
    }
    (wins, draws, aa / seeds as f32, ba / seeds as f32)
}

#[test]
fn auto_resolve_weighs_the_forces_of_both_sides() {
    let Some(ctx) = load() else { return };
    // Weak attacker against a defended planet (star base level 2, 12 structures, a Star Destroyer, hundreds of spawned fighters): it loses in both resolvers.
    let weak = auto_vs_planet(&ctx, "CR-90_Corvette", 2, best_defended_planet, "Empire", None).unwrap();
    println!("2 corvettes vs the best defended Empire planet: attacker {}/{} alive, defender {}/{} alive, attacker wins {}", weak.1, weak.0, weak.3, weak.2, weak.4);
    assert!(!weak.4 && weak.1 == 0 && weak.3 == weak.2, "{weak:?}");
    // Even forces: both sides lose ships, and over several seeds each side wins now and then.
    let (wins, draws, a_alive, d_alive) = auto_even(&ctx, ("Rebel", "Star_Destroyer", 3), ("Empire", "Star_Destroyer", 3), 20);
    println!("3 ISD vs 3 ISD over 20 seeds: attacker wins {wins}, draws {draws}, mean alive {a_alive:.2} / {d_alive:.2}");
    assert!((4..=16).contains(&wins) && a_alive < 3.0 && d_alive < 3.0, "an even fight must be a toss-up with losses on both sides");
    // Overwhelming force wins; the clearly outgunned side is wiped out.
    let (wins, _, a_alive, d_alive) = auto_even(&ctx, ("Rebel", "Star_Destroyer", 6), ("Empire", "Star_Destroyer", 1), 20);
    assert!(wins == 20 && a_alive >= 5.0 && d_alive == 0.0, "6 v 1: {wins} wins, alive {a_alive} / {d_alive}");
}

fn best_defended_planet(sim: &Sim, f: FactionId) -> Option<PlanetId> {
    let (w, g) = (sim.world(), sim.galaxy());
    (0..w.planets.len() as u32)
        .filter(|&p| g.planets[p as usize].owner == Some(f) && g.planets[p as usize].starbase.is_some() && w.planets[p as usize].space_map.is_some())
        .max_by_key(|&p| (g.planets[p as usize].starbase.map_or(0, |s| w.starbases[s as usize].level), g.planets[p as usize].structures.len()))
}

#[test]
fn a_fleet_arriving_at_a_pirate_planet_raises_a_battle_for_every_main_faction() {
    let Some(ctx) = load() else { return };
    for (fac, unit) in [("Rebel", "MC-80_Star_Cruiser"), ("Empire", "Star_Destroyer"), ("Underworld", "Pirate_Frigate_Placeholder")] {
        let campaigns = ["GC_Full_Web_Way_Rebel", "GC_Full_Web_Way_Empire", "GC_Full_Web_Way_Underworld"];
        for c in campaigns {
            let Some(campaign) = ctx.data.campaign(c) else { continue };
            let mut sim = Sim::new(&ctx.data, campaign, SimConfig { auto_resolve_battles: false, victory_enabled: false, ..SimConfig::default() }).unwrap();
            let w = sim.world().clone();
            let (Some(f), Some(pirates)) = (w.faction_id(fac), w.faction_id("Pirates")) else { continue };
            let Some(u) = w.unit_id(unit).or_else(|| w.units.iter().position(|u| u.role == UnitRole::Ship && u.category == w.units[w.unit_id("Star_Destroyer").unwrap() as usize].category).map(|i| i as u32)) else { continue };
            let g = sim.galaxy().clone();
            let pair = w.routes.iter().find_map(|r| {
                let own = |p: u32| g.planets[p as usize].owner == Some(f);
                let pir = |p: u32| g.planets[p as usize].owner == Some(pirates) && w.real[p as usize];
                if own(r.a) && pir(r.b) { Some((r.a, r.b)) } else if own(r.b) && pir(r.a) { Some((r.b, r.a)) } else { None }
            });
            let Some((from, to)) = pair else { println!("{c} / {fac}: no pirate planet next to an own one"); continue };
            let fid = add_fleet(&mut sim, f, u, 2, from, from);
            sim.apply(Command::MoveFleet { faction: f, fleet: fid, to }).unwrap();
            let mut pending_at_arrival = false;
            for _ in 0..4000 {
                let ev = sim.step(1.0);
                pending_at_arrival |= ev.iter().any(|e| matches!(e, Event::BattlePending(b) if b.planet == to));
                if pending_at_arrival || sim.galaxy().planets[to as usize].owner != Some(pirates) {
                    break;
                }
            }
            println!("{c} / {fac} -> {}: battle pending {pending_at_arrival}, owner now {:?}", w.planets[to as usize].name, sim.galaxy().planets[to as usize].owner.map(|o| w.factions[o as usize].name.clone()));
            assert!(pending_at_arrival && sim.galaxy().planets[to as usize].owner == Some(pirates), "{c}/{fac}: the planet must not flip without a battle");
        }
    }
}

#[test]
fn auto_resolve_agrees_with_the_tactical_battle_on_pirate_bases() {
    let Some(ctx) = load() else { return };
    // The tactical battle (3 seeds in the sweep that set `combat_base_factor`): a pirate star base with its garrison beats 1, 2, 3 and 5 Nebulons and 3 corvettes,
    // and loses to an MC-80. The statistical resolver must say the same (it used to rate stations at 15% and let a lone Nebulon win without loss).
    let pirate_planet = |sim: &Sim, f: FactionId| {
        let (w, g) = (sim.world(), sim.galaxy());
        (0..w.planets.len() as u32).find(|&p| g.planets[p as usize].owner == Some(f) && g.fleets_at(p).next().is_none() && w.planets[p as usize].space_map.is_some())
    };
    for (unit, n, attacker_wins) in [("Nebulon_B_Frigate", 1u32, false), ("Nebulon_B_Frigate", 2, false), ("Nebulon_B_Frigate", 3, false), ("CR-90_Corvette", 3, false), ("MC-80_Star_Cruiser", 1, true), ("MC-80_Star_Cruiser", 2, true)] {
        let r = auto_vs_planet(&ctx, unit, n, pirate_planet, "Pirates", None).unwrap();
        println!("{n} x {unit} vs a pirate base (auto-resolve): attacker {}/{} alive, wins {}", r.1, r.0, r.4);
        assert_eq!(r.4, attacker_wins, "{n} x {unit}: {r:?}");
    }
}
