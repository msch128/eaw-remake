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
    assert!(sim.galaxy().damage.keys().all(|s| ships.iter().any(|(x, _)| x == s)));
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
        assert!(!g.unit_health.is_empty() && g.health < 1.0, "the request carries the damage");
        // ... and battle-sim starts those ships damaged.
        let cat2 = catalog_for(&ctx, &req2);
        let setup = req2.battle_setup(&cat2, req2.battle_config(None), |_| true).unwrap();
        let battle = battle_sim::Battle::new(setup);
        let max_hull = cat2.ship(cat2.ship_id(&g.name).unwrap()).hull;
        assert!(battle.ships.iter().any(|s| s.def == cat2.ship_id(&g.name).unwrap() && s.hull < max_hull * 0.999), "a damaged ship starts below its maximum hull");
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
