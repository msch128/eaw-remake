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
    assert_eq!(sim.apply(Command::BuildUnit { faction: rebel, planet: yavin, unit: ship }), Err(CommandError::NoSlot));
    assert_eq!(sim.apply(Command::BuildUnit { faction: rebel, planet: yavin, unit: smuggler }), Ok(CommandOk::Done));
    let ps = &sim.galaxy().planets[yavin as usize];
    assert_eq!((w.queued_on_side(ps, false), w.queued_on_side(ps, true)), (w.config.max_queue_len, 1));
}
