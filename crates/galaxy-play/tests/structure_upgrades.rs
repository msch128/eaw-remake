//! The build menu hides a structure swap until the planet has the structure it replaces, and lists it under Structures.

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use galaxy_play::{Availability, BuildCategory, BuildMenu, BuildTarget, PlayCatalog};
use galaxy_sim::{Sim, SimConfig};

#[test]
fn a_missing_golan_hides_the_upgrade_and_a_spawner_shows_it() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return;
    }
    let data = load_galaxy(&GameFs::for_mod(&install, Some("1770851727")).unwrap()).0;
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let names = PlayCatalog::from_data(&w, &data);
    let empire = w.faction_id("Empire").unwrap();
    let p = w.planet_id("Coruscant_WW").unwrap();
    sim.set_planet_owner(p, None);
    sim.set_planet_owner(p, Some(empire));
    sim.add_credits(empire, 1.0e9);
    sim.set_tech_level(empire, 9);
    let item = w.unit_id("UPGRADE_E_GOLAN_2_DUMMY").expect("Golan I to II item");

    let hidden = BuildMenu::for_planet(&w, sim.galaxy(), &names, empire, p).unwrap();
    assert!(hidden.options.iter().all(|o| o.target != BuildTarget::Unit(item)), "no Golan I, so the upgrade is not on the menu");

    assert!(sim.add_structure(p, w.structure_id("E_GOLAN_1_SPAWNER").unwrap()));
    let low = BuildMenu::for_planet(&w, sim.galaxy(), &names, empire, p).unwrap();
    let entry = low.options.iter().find(|o| o.target == BuildTarget::Unit(item)).expect("the spawner makes the item visible");
    assert_eq!(entry.category, BuildCategory::Structures);
    assert_eq!(entry.cost, 48000.0);
    assert!(matches!(entry.availability, Availability::Requires(_)), "star base 0 is not level 2: {:?}", entry.availability);

    let mut sb = w.first_starbase[empire as usize];
    for _ in 1..3 {
        sb = sb.and_then(|b| w.starbases[b as usize].next);
    }
    sim.set_starbase(p, sb);
    let ready = BuildMenu::for_planet(&w, sim.galaxy(), &names, empire, p).unwrap();
    let entry = ready.options.iter().find(|o| o.target == BuildTarget::Unit(item)).unwrap();
    assert_eq!(entry.availability, Availability::Available);
    assert_eq!(entry.cost, 48000.0);
    assert_eq!(entry.build_seconds, 150.0);
}
