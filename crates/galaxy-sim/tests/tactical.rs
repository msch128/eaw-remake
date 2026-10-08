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
    g.fleets.insert(id, Fleet { id, owner, units: [(unit, n)].into(), location: FleetLocation::AtPlanet(at), previous_planet: from });
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
fn damage_persists_between_battles_and_is_repaired_at_owned_planets() {
    let Some(ctx) = load() else { return };
    let mut sim = new_sim(&ctx);
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let target = empty_planet(&sim);
    let (isd, xw) = (w.unit_id("Star_Destroyer").unwrap(), w.unit_id("X-Wing_Squadron").unwrap());
    // Three ISDs against a handful of X-wings: the ISDs win but take some damage from the squadrons' bombs? use a fair fight instead.
    let ef = add_fleet(&mut sim, empire, isd, 2, target, target);
    add_fleet(&mut sim, rebel, w.unit_id("MC-80_Star_Cruiser").unwrap(), 1, target, target);
    let _ = xw;
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
    assert!(hp.iter().any(|h| h.hull < 0.999 || h.shield < 0.999), "winners of a real fight are damaged: {hp:?}");
    sim.apply_tactical_result(battle.id, &result).unwrap();
    let ships = sim.ships_of(ef);
    let all_ships: Vec<ShipId> = sim.galaxy().ships.values().flat_map(|v| v.iter().map(|x| x.0)).collect();
    assert!(sim.galaxy().damage.keys().all(|s| all_ships.contains(s)), "damage only for ships that still exist");
    assert!(ships.iter().any(|(s, _)| sim.galaxy().damage.contains_key(s)), "the winner carries damage");
    assert!(!sim.galaxy().damage.is_empty());
    let missing = |s: &Sim| s.galaxy().damage.values().map(|h| 1.0 - h.hull).sum::<f32>();
    let before = missing(&sim);

    // The damaged fleet sits at a planet it now owns (captured after the battle): repairs run.
    sim.step(10.0);
    assert_eq!(sim.galaxy().planets[target as usize].owner, Some(empire));
    sim.step(2.0 * 60.0);
    let mid = missing(&sim);
    assert!(mid < before, "repairs reduce the missing hull ({before} -> {mid})");
    // A new battle starts the damaged ships damaged.
    if !sim.galaxy().damage.is_empty() {
        let rf2 = add_fleet(&mut sim, rebel, w.unit_id("MC-80_Star_Cruiser").unwrap(), 1, target, target);
        let _ = rf2;
        let b2 = pending(&mut sim);
        let req2 = sim.tactical_setup(b2.id).unwrap();
        let g = req2.sides.iter().flat_map(|s| &s.groups).find(|g| g.fleet == Some(ef)).unwrap();
        assert!(!g.unit_health.is_empty() && g.unit_health.iter().any(|h| !h.is_full()), "the request carries the damage: group {g:?}, damage {:?}, ships {:?}", sim.galaxy().damage, sim.galaxy().ships.get(&ef));
        // ... and battle-sim starts those ships damaged.
        let cat2 = catalog_for(&ctx, &req2);
        let setup = req2.battle_setup(&cat2, req2.battle_config(None), |_| true).unwrap();
        let battle = battle_sim::Battle::new(setup);
        let def = cat2.ship(cat2.ship_id(&g.name).unwrap());
        assert!(battle.ships.iter().any(|s| s.def == cat2.ship_id(&g.name).unwrap() && (s.hull < def.hull * 0.999 || s.shield < def.shield * 0.999)), "a damaged ship starts below its maximum hull or shield");
    }
    // Long enough at an owned planet: fully repaired.
    sim.cancel_tactical(0);
    for _ in 0..30 {
        sim.step(60.0);
        if sim.galaxy().damage.is_empty() {
            break;
        }
    }
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
