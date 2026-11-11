//! The two production queues of a planet against the real data (skipped without the game): which queue a build item belongs to, and that
//! only the first item of each queue is built.

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn load() -> Option<GameData> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    Some(load_galaxy(&GameFs::for_mod(&install, Some("1770851727")).ok()?).0)
}

/// The side of the production bar the data puts an object on: `Some(true)` for the planet tab (`Build_Tab_Land_Units`,
/// `Build_Tab_Special_Structures`, `Build_Tab_Heroes`), `Some(false)` for the space tab (`Build_Tab_Space_Units`, `Build_Tab_Space_Station`),
/// `None` when the object has no tab or tabs of both sides.
fn tab_side(data: &GameData, name: &str) -> Option<bool> {
    let r = data.objects.resolve(name)?;
    let any = |tabs: &[&str]| tabs.iter().any(|t| r.flag(t).unwrap_or(false));
    let space = any(&["build_tab_space_units", "build_tab_space_station"]);
    let ground = any(&["build_tab_land_units", "build_tab_special_structures", "build_tab_heroes"]);
    (space != ground).then_some(ground)
}

#[test]
fn every_build_item_is_on_the_side_of_its_build_tab() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let sim = Sim::new(&data, campaign, SimConfig::default()).expect("sim");
    let w = sim.world();
    let (mut space, mut ground) = (0, 0);
    let mut check = |what: &str, name: &str, kind: ProductionKind| {
        let side = tab_side(&data, name).unwrap_or_else(|| panic!("{what} {name} has no build tab of exactly one side"));
        assert_eq!(w.on_ground_side(&kind), side, "{what} {name}: the data puts it on the {} tab", if side { "planet" } else { "space" });
        *if side { &mut ground } else { &mut space } += 1;
    };
    for fi in 0..w.factions.len() {
        for &u in &w.buildable_units[fi] {
            check("unit", &w.units[u as usize].name, ProductionKind::Unit(u));
        }
        for &s in &w.buildable_structures[fi] {
            check("structure", &w.structures[s as usize].name, ProductionKind::Structure(s));
        }
        // Tech levels the data has an object for (`E_Tech-2` ...); the generic levels of the minor factions have none.
        for (i, t) in w.factions[fi].tech_upgrades.iter().enumerate().filter(|(_, t)| data.objects.resolve(&t.name).is_some()) {
            check("tech level", &t.name, ProductionKind::Tech(i));
        }
    }
    // Star base levels: every one the data gives a tab is on the space station tab; the rest are skirmish and tutorial bases.
    for (b, sb) in w.starbases.iter().enumerate().filter(|(_, sb)| sb.cost > 0.0) {
        assert!(!w.on_ground_side(&ProductionKind::Starbase(b as u32)), "{}", sb.name);
        assert_ne!(tab_side(&data, &sb.name), Some(true), "{}", sb.name);
    }
    println!("{space} items on the space side, {ground} on the ground side");
    // Ground side: the ground structures, the structure upgrades bought as ground companies and the agents.
    assert!(space > 400 && ground > 120, "{space} space, {ground} ground");
    let on_ground = |name: &str| w.on_ground_side(&ProductionKind::Unit(w.unit_id(name).unwrap_or_else(|| panic!("{name}"))));
    assert!(on_ground("Rebel_Smuggler_Team"), "an agent is a `HeroCompany` on the heroes tab");
    assert!(on_ground("UPGRADE_MINING_COMPANY_R_BARREN"), "a ground structure upgrade is a `GroundCompany` on the land units tab");
    assert!(!on_ground("UPGRADE_R_GOLAN_2_DUMMY"), "an orbital structure upgrade is a `SpaceUnit` on the space units tab");
    assert!(!on_ground("Ackbar_Upgrade") && !on_ground("Rebel_Shield_Upgrade"));
}

/// Queues the first ship the faction can order at the planet until `n` are queued; returns the type.
fn queue_ships(sim: &mut Sim, faction: FactionId, planet: PlanetId, n: usize) -> UnitTypeId {
    let w = sim.world().clone();
    let ship = w.buildable_units[faction as usize]
        .iter()
        .copied()
        .find(|&u| w.units[u as usize].role == UnitRole::Ship && w.units[u as usize].limit_current.is_none() && w.check_build_unit(sim.galaxy(), faction, planet, u).is_ok())
        .expect("a ship the planet can build");
    for _ in 0..n {
        sim.apply(Command::BuildUnit { faction, planet, unit: ship }).expect("queued");
    }
    ship
}

/// The bar has five slots per side. Five ships fill the space side; an agent is bought on the planet side and is still accepted.
#[test]
fn a_full_space_side_refuses_a_ship_and_takes_a_ground_item() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let w = sim.world().clone();
    let rebel = w.faction_id("Rebel").unwrap();
    let yavin = w.planet_id("Yavin_WW").unwrap();
    let smuggler = w.unit_id("Rebel_Smuggler_Team").unwrap();
    sim.add_credits(rebel, 1.0e6);
    let ship = queue_ships(&mut sim, rebel, yavin, w.config.max_queue_len);
    assert_eq!(sim.apply(Command::BuildUnit { faction: rebel, planet: yavin, unit: ship }), Err(CommandError::QueueFull));
    assert_eq!(sim.apply(Command::BuildUnit { faction: rebel, planet: yavin, unit: smuggler }), Ok(CommandOk::Done));
    let ps = &sim.galaxy().planets[yavin as usize];
    assert_eq!((w.queued_on_side(ps, false), w.queued_on_side(ps, true)), (w.config.max_queue_len, 1));
}

/// The campaign start with the Empire rich, at tech level 4 and Coruscant at star base level 2, so that every kind of order is open there.
fn coruscant(data: &GameData) -> (Sim, FactionId, PlanetId) {
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let mut sim = Sim::new(data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let w = sim.world().clone();
    let (empire, p) = (w.faction_id("Empire").unwrap(), w.planet_id("Coruscant_WW").unwrap());
    sim.add_credits(empire, 1.0e9);
    sim.set_tech_level(empire, 4);
    let level2 = w.first_starbase[empire as usize].and_then(|b| w.starbases[b as usize].next);
    sim.set_starbase(p, level2);
    (sim, empire, p)
}

/// Queues the first structure of the kind the planet accepts.
fn queue_structure(sim: &mut Sim, faction: FactionId, planet: PlanetId, orbital: bool) -> StructureTypeId {
    let w = sim.world().clone();
    let s = (0..w.structures.len() as u32)
        .find(|&s| w.structures[s as usize].orbital == orbital && w.check_build_structure(sim.galaxy(), faction, planet, s).is_ok())
        .unwrap_or_else(|| panic!("no {} structure to order", if orbital { "orbital" } else { "ground" }));
    sim.apply(Command::BuildStructure { faction, planet, structure: s }).expect("queued");
    s
}

fn queue(sim: &Sim, planet: PlanetId, lane: Lane) -> Vec<(ProductionKind, f32, f32)> {
    sim.galaxy().planets[planet as usize].queues[lane as usize].iter().map(|i| (i.kind.clone(), i.remaining, i.total)).collect()
}

fn ships_of(sim: &Sim, faction: FactionId, unit: UnitTypeId) -> u32 {
    sim.galaxy().fleets.values().filter(|f| f.owner == faction).map(|f| f.units.get(&unit).copied().unwrap_or(0)).sum()
}

/// A ship, an orbital structure, a star base level and a second ship are four items of the one space queue; two ground structures are the
/// ground queue. Of each queue only the first item is built, the others wait untouched in the order they were bought.
#[test]
fn only_the_first_item_of_each_queue_is_built() {
    let Some(data) = load() else { return };
    let (mut sim, empire, p) = coruscant(&data);
    let ship = queue_ships(&mut sim, empire, p, 1);
    let orbital = queue_structure(&mut sim, empire, p, true);
    sim.apply(Command::UpgradeStarbase { faction: empire, planet: p }).expect("star base");
    sim.apply(Command::BuildUnit { faction: empire, planet: p, unit: ship }).expect("second ship");
    let ground = [queue_structure(&mut sim, empire, p, false), queue_structure(&mut sim, empire, p, false)];

    let space = queue(&sim, p, Lane::Space);
    let kinds: Vec<&ProductionKind> = space.iter().map(|i| &i.0).collect();
    assert!(
        matches!(kinds[..], [ProductionKind::Unit(a), ProductionKind::Structure(o), ProductionKind::Starbase(_), ProductionKind::Unit(b)] if *a == ship && *b == ship && *o == orbital),
        "{kinds:?}"
    );
    assert_eq!(queue(&sim, p, Lane::Ground).iter().map(|i| i.0.clone()).collect::<Vec<_>>(), ground.map(ProductionKind::Structure));
    let first_total = space[0].2;
    assert!(first_total > 3.0, "the first ship takes longer than the steps of this test ({first_total} s)");

    sim.step(2.0);
    for lane in [Lane::Space, Lane::Ground] {
        let q = queue(&sim, p, lane);
        assert!((q[0].1 - (q[0].2 - 2.0)).abs() < 0.01, "{lane:?}: the first item was built for 2 s: {:?}", q[0]);
        for waiting in &q[1..] {
            assert_eq!(waiting.1, waiting.2, "{lane:?}: {:?} waits and must not progress", waiting.0);
        }
    }

    // When the first ship is done the orbital structure takes its place and starts from zero; the star base still waits.
    let ships_before = ships_of(&sim, empire, ship);
    sim.step(first_total - 2.0);
    let space = queue(&sim, p, Lane::Space);
    assert_eq!(space.len(), 3, "the ship left the queue: {space:?}");
    assert_eq!(ships_of(&sim, empire, ship), ships_before + 1);
    assert_eq!((space[0].0.clone(), space[0].1), (ProductionKind::Structure(orbital), space[0].2), "the structure starts when the ship is done");
    sim.step(1.0);
    let space = queue(&sim, p, Lane::Space);
    assert!((space[0].1 - (space[0].2 - 1.0)).abs() < 0.01 && space[1].1 == space[1].2 && space[2].1 == space[2].2, "{space:?}");
}

/// A star base level and a tech level are two more items of the space queue: both are accepted at one planet and wait behind each other.
/// A second star base level of the planet and a second tech level of the faction are still refused while one is queued.
#[test]
fn a_star_base_level_and_a_tech_level_share_the_space_queue() {
    let Some(data) = load() else { return };
    let (mut sim, empire, p) = coruscant(&data);
    sim.set_tech_level(empire, 2);
    assert!(!sim.world().tech_needs_institute(empire), "the Empire is not the player of this campaign");
    sim.apply(Command::UpgradeStarbase { faction: empire, planet: p }).expect("star base");
    sim.apply(Command::UpgradeTech { faction: empire, planet: p }).expect("a tech level behind the star base level");
    let kinds: Vec<ProductionKind> = queue(&sim, p, Lane::Space).into_iter().map(|i| i.0).collect();
    assert!(matches!(kinds[..], [ProductionKind::Starbase(_), ProductionKind::Tech(_)]), "{kinds:?}");
    assert_eq!(sim.apply(Command::UpgradeStarbase { faction: empire, planet: p }), Err(CommandError::NoSlot));
    assert_eq!(sim.apply(Command::UpgradeTech { faction: empire, planet: p }), Err(CommandError::NoSlot));
    let other = sim.galaxy().owned_planets(empire).find(|&o| o != p && sim.galaxy().planets[o as usize].starbase.is_some()).expect("another Empire planet with a star base");
    assert_eq!(sim.apply(Command::UpgradeTech { faction: empire, planet: other }), Err(CommandError::NoSlot), "one tech level at a time per faction");
    // The tech level waits while the star base level is built.
    sim.step(5.0);
    let q = queue(&sim, p, Lane::Space);
    assert!(q[0].1 < q[0].2 && q[1].1 == q[1].2, "{q:?}");
}

/// A waiting item has not been built at all: cancelling it gives the whole price back and the items behind it move up.
#[test]
fn a_waiting_item_is_cancelled_by_its_place_in_its_queue() {
    let Some(data) = load() else { return };
    let (mut sim, empire, p) = coruscant(&data);
    let ship = queue_ships(&mut sim, empire, p, 1);
    let orbital = queue_structure(&mut sim, empire, p, true);
    sim.apply(Command::UpgradeStarbase { faction: empire, planet: p }).expect("star base");
    let ground = queue_structure(&mut sim, empire, p, false);
    sim.step(3.0);
    let credits = sim.galaxy().factions[empire as usize].credits;
    let price = f64::from(sim.world().structures[orbital as usize].cost);
    assert_eq!(sim.apply(Command::CancelProduction { faction: empire, planet: p, lane: Lane::Space, index: 1 }), Ok(CommandOk::Refunded(price)));
    assert_eq!(sim.galaxy().factions[empire as usize].credits, credits + price);
    let kinds: Vec<ProductionKind> = queue(&sim, p, Lane::Space).into_iter().map(|i| i.0).collect();
    assert!(matches!(kinds[..], [ProductionKind::Unit(u), ProductionKind::Starbase(_)] if u == ship), "{kinds:?}");
    assert_eq!(queue(&sim, p, Lane::Ground).len(), 1, "the ground queue is another queue");
    // The item being built gives back the share that is not built yet.
    let (_, remaining, total) = queue(&sim, p, Lane::Ground)[0].clone();
    let cost = f64::from(sim.world().structures[ground as usize].cost);
    let Ok(CommandOk::Refunded(got)) = sim.apply(Command::CancelProduction { faction: empire, planet: p, lane: Lane::Ground, index: 0 }) else { panic!("refund") };
    assert!((got - cost * f64::from(remaining / total)).abs() < 0.01 && got < cost, "{got} of {cost}");
    assert_eq!(sim.apply(Command::CancelProduction { faction: empire, planet: p, lane: Lane::Ground, index: 0 }), Err(CommandError::Invalid));
}

#[test]
fn a_planet_that_changes_hands_loses_both_queues() {
    let Some(data) = load() else { return };
    let (mut sim, empire, p) = coruscant(&data);
    queue_ships(&mut sim, empire, p, 2);
    queue_structure(&mut sim, empire, p, false);
    let rebel = sim.world().faction_id("Rebel").unwrap();
    sim.set_planet_owner(p, Some(rebel));
    assert!(sim.galaxy().planets[p as usize].queues.iter().all(|q| q.is_empty()));
}

/// The state a build before the two queues saved: three lanes per planet (ships and other units; structures, where the first orbital and the
/// first ground one were built at the same time; star base and tech levels), each with an item in progress. Loading sorts them into the two
/// queues. What was being built comes first, in the order of the lanes: the ship, the orbital structure and the star base level in the space
/// queue, the ground structure in the ground queue. What was waiting follows in the order of the lanes. Every item keeps what was built of it,
/// and from then on only the first of each queue is built.
#[test]
fn the_three_lanes_of_an_earlier_save_are_sorted_into_the_two_queues() {
    let Some(data) = load() else { return };
    let (mut sim, empire, p) = coruscant(&data);
    let w = sim.world().clone();
    let item = |kind: ProductionKind, remaining: f32, total: f32| ProductionItem { kind, remaining, total, cost: 1000.0 };
    let (ship, upgrade) = (w.unit_id("Victory_Destroyer").unwrap(), w.unit_id("UPGRADE_MINING_COMPANY_E_BARREN").unwrap());
    assert!(w.on_ground_side(&ProductionKind::Unit(upgrade)));
    let (ground_a, ground_b) = (w.structure_id("Empire_Watchtower").unwrap(), w.structure_id("TAXATION_AGENCY_URBAN_SPAWNER").unwrap());
    let yard = w.structure_id("E_Shipyard_3").unwrap();
    let base = w.first_starbase[empire as usize].unwrap();
    let mut snap = sim.snapshot();
    let ps = &mut snap.galaxy.planets[p as usize];
    assert!(ps.queues.iter().all(|q| q.is_empty()));
    ps.legacy_lanes = Some([
        [item(ProductionKind::Unit(ship), 10.0, 34.0), item(ProductionKind::Unit(upgrade), 20.0, 20.0), item(ProductionKind::Unit(ship), 34.0, 34.0)].into(),
        [item(ProductionKind::Structure(ground_a), 5.0, 15.0), item(ProductionKind::Structure(yard), 240.0, 250.0), item(ProductionKind::Structure(ground_b), 60.0, 60.0)].into(),
        [item(ProductionKind::Starbase(base), 80.0, 90.0), item(ProductionKind::Tech(1), 300.0, 300.0)].into(),
    ]);
    sim.restore(snap).expect("restore");
    assert_eq!(sim.galaxy().planets[p as usize].legacy_lanes, None);
    assert_eq!(
        queue(&sim, p, Lane::Space),
        vec![
            (ProductionKind::Unit(ship), 10.0, 34.0),
            (ProductionKind::Structure(yard), 240.0, 250.0),
            (ProductionKind::Starbase(base), 80.0, 90.0),
            (ProductionKind::Unit(ship), 34.0, 34.0),
            (ProductionKind::Tech(1), 300.0, 300.0),
        ]
    );
    assert_eq!(
        queue(&sim, p, Lane::Ground),
        vec![(ProductionKind::Structure(ground_a), 5.0, 15.0), (ProductionKind::Unit(upgrade), 20.0, 20.0), (ProductionKind::Structure(ground_b), 60.0, 60.0)]
    );
    sim.step(4.0);
    let (space, ground) = (queue(&sim, p, Lane::Space), queue(&sim, p, Lane::Ground));
    assert_eq!((space[0].1, ground[0].1), (6.0, 1.0), "the first of each queue is built");
    assert_eq!((space[1].1, space[2].1, space[3].1, ground[1].1), (240.0, 80.0, 34.0, 20.0), "what was built of the others is kept and they wait");
}

/// The bar has five slots per queue (`queue00..04`, `queue05..09`), whatever waits in them: a star base level and a tech level are refused
/// by a full space queue like a ship is, and accepted again when a slot is free.
#[test]
fn a_full_space_queue_refuses_a_star_base_level_and_a_tech_level() {
    let Some(data) = load() else { return };
    let (mut sim, empire, p) = coruscant(&data);
    sim.set_tech_level(empire, 2);
    let w = sim.world().clone();
    assert!(w.check_upgrade_starbase(sim.galaxy(), empire, p).is_ok() && w.check_upgrade_tech(sim.galaxy(), empire, p).is_ok());
    queue_ships(&mut sim, empire, p, w.config.max_queue_len);
    assert_eq!(sim.apply(Command::UpgradeStarbase { faction: empire, planet: p }), Err(CommandError::QueueFull));
    assert_eq!(sim.apply(Command::UpgradeTech { faction: empire, planet: p }), Err(CommandError::QueueFull));
    assert_eq!(sim.galaxy().planets[p as usize].queues[Lane::Space as usize].len(), w.config.max_queue_len);
    // One slot free: the star base level takes it, and the tech level is refused again.
    sim.apply(Command::CancelProduction { faction: empire, planet: p, lane: Lane::Space, index: 4 }).expect("cancel");
    sim.apply(Command::UpgradeStarbase { faction: empire, planet: p }).expect("a free slot takes the star base level");
    assert_eq!(sim.apply(Command::UpgradeTech { faction: empire, planet: p }), Err(CommandError::QueueFull));
}

/// `QueueFull` says that the order would be taken once a slot is free. An item the planet refuses for another reason (a missing shipyard, a
/// star base level, no structure slot) is refused for that reason, whether the queue is full or not; the other queue is not touched.
#[test]
fn a_full_queue_does_not_hide_why_an_item_is_refused() {
    let Some(data) = load() else { return };
    let (mut sim, empire, p) = coruscant(&data);
    let w = sim.world().clone();
    let answers = |sim: &Sim| -> Vec<(ProductionKind, Result<(), CommandError>)> {
        let units = w.buildable_units[empire as usize].iter().map(|&u| (ProductionKind::Unit(u), w.check_build_unit(sim.galaxy(), empire, p, u)));
        let structures = w.buildable_structures[empire as usize].iter().map(|&s| (ProductionKind::Structure(s), w.check_build_structure(sim.galaxy(), empire, p, s)));
        units.chain(structures).collect()
    };
    let before = answers(&sim);
    let ship = queue_ships(&mut sim, empire, p, w.config.max_queue_len);
    let (mut now_full, mut still_refused, mut ground) = (0, 0, 0);
    for ((kind, was), (_, is)) in before.iter().zip(answers(&sim)) {
        let name = match kind {
            ProductionKind::Unit(u) => &w.units[*u as usize].name,
            ProductionKind::Structure(s) => &w.structures[*s as usize].name,
            _ => unreachable!(),
        };
        // The queued ships count against the limits of their own type; every other answer must only change from accepted to a full queue.
        if *kind == ProductionKind::Unit(ship) {
            continue;
        }
        if w.lane_of(kind) == Lane::Ground {
            assert_eq!(&is, was, "{name}: the ground queue is not the full one");
            ground += 1;
        } else if was.is_ok() {
            assert_eq!(is, Err(CommandError::QueueFull), "{name}");
            now_full += 1;
        } else {
            assert_eq!(&is, was, "{name}: refused for its own reason");
            still_refused += usize::from(matches!(was, Err(CommandError::RequirementsNotMet(r)) if r != "locked" && r != "planet" && !r.starts_with("tech level")));
        }
    }
    println!("{now_full} space items wait for a slot, {still_refused} name a missing requirement, {ground} ground items unchanged");
    assert!(now_full > 10 && still_refused > 10 && ground > 10, "{now_full} / {still_refused} / {ground}");
}
