//! Hero and ship upgrade items (`Vessel_Upgrade_Template`, `Scripts/GameObject/Upgrade_Units.lua`) of Remake 4.0: Piett's Procursator becomes the Accuser,
//! Ackbar's Wingless the Home One. The item needs the ship in orbit (`Required_Orbiting_Units`) and the planet, structures and credits of its data;
//! completing it swaps the ship. Skips without the game and mod.

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn load() -> Option<GameData> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    Some(load_galaxy(&fs).0)
}

fn prepare(sim: &mut Sim, faction: &str, prefix: &str, planet: &str) -> (FactionId, PlanetId) {
    let w = sim.world().clone();
    let (f, p) = (w.faction_id(faction).unwrap(), w.planet_id(planet).unwrap());
    sim.set_planet_owner(p, None);
    sim.set_planet_owner(p, Some(f));
    let mut sb = w.first_starbase[f as usize];
    for _ in 1..3 {
        sb = sb.and_then(|b| w.starbases[b as usize].next);
    }
    sim.set_starbase(p, sb);
    sim.add_credits(f, 1.0e9);
    sim.set_tech_level(f, 9);
    for n in ["Shipyard_1", "Shipyard_2", "Shipyard_3", "Uplink_Command"] {
        assert!(sim.add_structure(p, w.structure_id(&format!("{prefix}_{n}")).unwrap()), "{n}");
    }
    (f, p)
}

fn give(sim: &mut Sim, owner: FactionId, planet: PlanetId, unit: &str) {
    let w = sim.world().clone();
    let u = w.unit_id(unit).unwrap_or_else(|| panic!("no unit {unit}"));
    let g = sim.galaxy_mut();
    let id = g.fleets.keys().max().copied().unwrap_or(0) + 1;
    g.fleets.insert(id, Fleet { id, owner, units: [(u, 1)].into(), location: FleetLocation::AtPlanet(planet), previous_planet: planet, slot: 0 });
}

fn count(sim: &Sim, owner: FactionId, planet: PlanetId, unit: &str) -> u32 {
    let u = sim.world().unit_id(unit).unwrap();
    sim.galaxy().fleets.values().filter(|f| f.owner == owner && f.location == FleetLocation::AtPlanet(planet)).map(|f| f.units.get(&u).copied().unwrap_or(0)).sum()
}

#[test]
fn piett_upgrades_his_procursator_to_the_accuser_at_kuat() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let w = sim.world().clone();
    let (empire, kuat) = prepare(&mut sim, "Empire", "E", "Kuat_WW");
    let item = w.unit_id("Piett_Procursator_Upgrade").expect("the upgrade item is in the catalog");
    assert!(w.buildable_units[empire as usize].binary_search(&item).is_ok(), "it is in the Empire build bar");
    // Without the ship in orbit the item is refused; elsewhere too (Required_Planets Kuat).
    assert!(matches!(w.check_build_unit(sim.galaxy(), empire, kuat, item), Err(CommandError::RequirementsNotMet(m)) if m.contains("Piett_Procursator")));
    give(&mut sim, empire, kuat, "Piett_Procursator");
    w.check_build_unit(sim.galaxy(), empire, kuat, item).expect("with Piett in orbit at Kuat the upgrade can be queued");
    let (other, _) = (w.planet_id("Coruscant_WW").unwrap(), ());
    prepare(&mut sim, "Empire", "E", "Coruscant_WW");
    give(&mut sim, empire, other, "Piett_Procursator");
    assert!(w.check_build_unit(sim.galaxy(), empire, other, item).is_err(), "only Kuat builds it (Required_Planets)");
    sim.apply(Command::BuildUnit { faction: empire, planet: kuat, unit: item }).expect("queued");
    for _ in 0..2000 {
        sim.step(1.0);
        if count(&sim, empire, kuat, "Accuser_Star_Destroyer") > 0 {
            break;
        }
    }
    assert_eq!(count(&sim, empire, kuat, "Accuser_Star_Destroyer"), 1, "the Accuser replaced the Procursator");
    assert_eq!(count(&sim, empire, kuat, "Piett_Procursator"), 0);
}

#[test]
fn every_hero_upgrade_of_the_data_leads_to_a_ship_that_exists() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let sim = Sim::new(&data, campaign, SimConfig::default()).expect("sim");
    let w = sim.world();
    for (item, to) in [("Ackbar_Upgrade", "Home_One"), ("Black_Squadron_Upgrade", "Devastator_Star_Destroyer"), ("Strikefast_Upgrade", "Admonitor_Star_Destroyer"), ("Piett_Accuser_Upgrade", "Piett_Executor_P1")] {
        let Some(i) = w.unit_id(item) else {
            eprintln!("{item}: not in this catalog (its source ship is not a unit here)");
            continue;
        };
        assert_eq!(w.units[i as usize].role, UnitRole::Upgrade { from: match w.units[i as usize].role { UnitRole::Upgrade { from, .. } => from, _ => unreachable!() }, to: w.unit_id(to).unwrap() }, "{item} leads to {to}");
    }
}
