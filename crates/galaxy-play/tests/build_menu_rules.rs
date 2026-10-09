//! Build menu texts against the real data: a missing structure is named by its display name, not by the object name.

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use galaxy_play::{Availability, BuildMenu, BuildTarget, PlayCatalog};
use galaxy_sim::{Sim, SimConfig};

#[test]
fn missing_requirements_show_display_names() {
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
    let mut sb = w.first_starbase[empire as usize];
    for _ in 1..3 {
        sb = sb.and_then(|b| w.starbases[b as usize].next);
    }
    sim.set_starbase(p, sb);
    sim.add_credits(empire, 1.0e9);
    sim.set_tech_level(empire, 9);
    for n in ["E_Shipyard_1", "E_Shipyard_2", "E_Shipyard_3"] {
        assert!(sim.add_structure(p, w.structure_id(n).unwrap()));
    }
    let piett = w.unit_id("Piett_Procursator").unwrap();
    let menu = BuildMenu::for_planet(&w, sim.galaxy(), &names, empire, p).expect("menu");
    let entry = menu.options.iter().find(|o| o.target == BuildTarget::Unit(piett)).expect("hero entry");
    assert_eq!(entry.cost, 1.0, "Build_Cost_Credits of the hero is 1 in the data");
    let Availability::Requires(text) = &entry.availability else { panic!("{:?}", entry.availability) };
    assert!(!text.contains("E_Uplink_Command") && !text.contains('_'), "raw object name in {text:?}");
    assert_eq!(text, names.structure(w.structure_id("E_Uplink_Command").unwrap()));
    println!("hero requirement text: {text:?}");
    assert_eq!(menu.starbase_max, 3);
}
