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

#[test]
fn flagship_prefers_hero_ship() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        return;
    }
    let data = load_galaxy(&GameFs::for_mod(&install, Some("1770851727")).unwrap()).0;
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world();

    let piett = w.unit_id("Piett_Procursator").expect("Piett hero unit");
    let isd = w.unit_id("Star_Destroyer").expect("Star Destroyer");

    let mut units = std::collections::BTreeMap::new();
    units.insert(isd, 2);
    units.insert(piett, 1);
    let fleet = galaxy_sim::Fleet {
        id: 1,
        owner: w.faction_id("Empire").unwrap(),
        units,
        location: galaxy_sim::FleetLocation::AtPlanet(0),
        previous_planet: 0,
        slot: 0,
    };

    assert_eq!(galaxy_play::flagship(w, &fleet), Some(piett), "Hero ship Piett must be chosen as flagship over ISD");
}

#[test]
fn build_bar_follows_the_games_order_not_the_price() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
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
    let menu = BuildMenu::for_planet(&w, sim.galaxy(), &names, empire, p).expect("menu");
    let structures: Vec<usize> = menu.options.iter().filter_map(|o| if let BuildTarget::Structure(s) = o.target { Some(s as usize) } else { None }).collect();
    assert!(structures.len() > 3, "some structures are offered");
    assert!(structures.windows(2).all(|x| (w.structures[x[0]].gui_row, x[0]) <= (w.structures[x[1]].gui_row, x[1])), "structures follow GUI_Row, then the load order of the data");
    let costs: Vec<f32> = structures.iter().map(|&s| w.structures[s].cost).collect();
    assert!(costs.windows(2).any(|c| c[0] > c[1]), "the order is not the price order: {costs:?}");
}

#[test]
fn a_greyed_ship_names_every_missing_requirement_in_the_games_words() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
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
    let menu = BuildMenu::for_planet(&w, sim.galaxy(), &names, empire, p).expect("menu");
    let isd = w.unit_id("Star_Destroyer").unwrap();
    let entry = menu.options.iter().find(|o| o.target == BuildTarget::Unit(isd)).expect("the Star Destroyer is listed (greyed)");
    assert!(!entry.availability.is_available());
    let level = format!("{} 2", names.rules.starbase);
    let yard = format!("{} {}", names.rules.structure, names.structure(w.structure_id("E_Shipyard_3").unwrap()));
    assert!(entry.reasons.contains(&level), "star base level named in the game's words: {:?} (wanted {level:?})", entry.reasons);
    assert!(entry.reasons.contains(&yard), "shipyard named: {:?} (wanted {yard:?})", entry.reasons);
    assert!(names.rules.starbase.starts_with("Required"), "text comes from TEXT_REQUIRED_STAR_BASE_LEVEL: {}", names.rules.starbase);
    // Without credits the funds line is added.
    let poor = {
        sim.add_credits(empire, -1.0e9);
        sim.add_credits(empire, -sim.galaxy().factions[empire as usize].credits);
        BuildMenu::for_planet(&w, sim.galaxy(), &names, empire, p).expect("menu")
    };
    let cheap = poor.options.iter().find(|o| o.availability == galaxy_play::Availability::NeedCredits);
    if let Some(c) = cheap {
        assert!(c.reasons.contains(&names.rules.funds), "{:?}", c.reasons);
    }
}

#[test]
fn the_tech_upgrade_waits_for_the_research_institute() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        return;
    }
    let data = load_galaxy(&GameFs::for_mod(&install, Some("1770851727")).unwrap()).0;
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let names = PlayCatalog::from_data(&w, &data);
    // The human player (Rebel) holds a second planet without institute.
    let empire = w.faction_id("Rebel").unwrap();
    let p = w.planet_id("Coruscant_WW").unwrap();
    sim.set_planet_owner(p, None);
    sim.set_planet_owner(p, Some(empire));
    sim.set_starbase(p, w.first_starbase[empire as usize]);
    sim.add_credits(empire, 1.0e9);
    let institute = w.structure_id("R_Research_Institute").unwrap();
    let tech = |sim: &Sim| {
        let menu = BuildMenu::for_planet(&w, sim.galaxy(), &names, empire, p).expect("menu");
        menu.options.into_iter().find(|o| matches!(o.target, BuildTarget::Tech(_))).expect("tech entry")
    };
    let before = tech(&sim);
    assert!(matches!(before.availability, Availability::Requires(_)), "{:?}", before.availability);
    assert!(before.reasons.iter().any(|r| r.contains(names.structure(institute))), "reasons {:?}", before.reasons);

    assert!(sim.add_structure(p, institute));
    let after = tech(&sim);
    assert_eq!(after.availability, Availability::Available, "reasons {:?}", after.reasons);
    assert!(after.reasons.is_empty());
}
