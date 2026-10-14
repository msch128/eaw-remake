//! Build rules against the real data: structure prerequisites, per-planet restrictions, the space/ground split and the ground-army filter.

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

fn new_sim(data: &GameData) -> Sim {
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    Sim::new(data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim")
}

/// Makes `planet` an empty planet of `faction` with a star base of `level`; the faction is rich and at max tech.
fn bare_planet(sim: &mut Sim, faction: FactionId, planet: PlanetId, level: i32) {
    let w = sim.world().clone();
    sim.set_planet_owner(planet, None);
    sim.set_planet_owner(planet, Some(faction));
    let mut sb = w.first_starbase[faction as usize];
    for _ in 1..level {
        sb = sb.and_then(|b| w.starbases[b as usize].next);
    }
    sim.set_starbase(planet, sb);
    sim.add_credits(faction, 1.0e9);
    sim.set_tech_level(faction, 9);
}

fn unmet(r: Result<(), CommandError>) -> String {
    match r {
        Err(CommandError::RequirementsNotMet(s)) => s,
        other => panic!("expected an unmet requirement, got {other:?}"),
    }
}

#[test]
fn shipyard_three_needs_two_needs_one() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data);
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let p = w.planet_id("Coruscant_WW").unwrap();
    assert_eq!(w.planets[p as usize].max_space_base, 3);
    bare_planet(&mut sim, empire, p, 3);
    let (y1, y2, y3) = (w.structure_id("E_Shipyard_1").unwrap(), w.structure_id("E_Shipyard_2").unwrap(), w.structure_id("E_Shipyard_3").unwrap());
    assert_eq!(w.structures[y3 as usize].req_structures, vec![vec![y2]]);
    assert_eq!(w.structures[y2 as usize].req_structures, vec![vec![y1]]);
    assert!(w.structures[y1 as usize].orbital && w.structures[y3 as usize].orbital);

    assert_eq!(unmet(w.check_build_structure(sim.galaxy(), empire, p, y3)), "E_Shipyard_2");
    assert_eq!(unmet(w.check_build_structure(sim.galaxy(), empire, p, y2)), "E_Shipyard_1");
    w.check_build_structure(sim.galaxy(), empire, p, y1).expect("yard 1 has no prerequisite");
    assert!(sim.add_structure(p, y1));
    assert_eq!(unmet(w.check_build_structure(sim.galaxy(), empire, p, y3)), "E_Shipyard_2", "yard 1 alone does not unlock yard 3");
    w.check_build_structure(sim.galaxy(), empire, p, y2).expect("yard 2 after yard 1");
    assert!(sim.add_structure(p, y2));
    w.check_build_structure(sim.galaxy(), empire, p, y3).expect("yard 3 after yard 2");
    // The command path refuses the same way.
    let mut fresh = new_sim(&data);
    bare_planet(&mut fresh, empire, p, 3);
    let r = fresh.apply(Command::BuildStructure { faction: empire, planet: p, structure: y3 });
    assert!(matches!(r, Err(CommandError::RequirementsNotMet(_))), "{r:?}");
}

#[test]
fn alternatives_and_groups_are_parsed_from_the_requirement_text() {
    let Some(data) = load() else { return };
    let sim = new_sim(&data);
    let w = sim.world();
    // `A | B | C` = any of them; `A B` = both.
    let core = w.structure_id("Empire_Base_Core_Urban").unwrap();
    let groups = &w.structures[core as usize].req_structures;
    assert_eq!(groups.len(), 1, "one group of alternatives");
    assert_eq!(groups[0].len(), 4);
    let piett = w.unit_id("Piett_Procursator").unwrap();
    let groups = &w.units[piett as usize].req_structures;
    assert_eq!(groups.len(), 2, "shipyard 3 and uplink command, both required");
    // A multi-faction ship needs the yard of its own faction only.
    let interdictor = w.unit_id("Sith_Interdictor").unwrap();
    assert_eq!(w.units[interdictor as usize].req_structures.len(), 1);
    assert_eq!(w.units[interdictor as usize].req_structures[0].len(), 3);
}

#[test]
fn hero_needs_the_uplink_and_its_cost_is_the_data_value() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data);
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let p = w.planet_id("Coruscant_WW").unwrap();
    bare_planet(&mut sim, empire, p, 3);
    for n in ["E_Shipyard_1", "E_Shipyard_2", "E_Shipyard_3"] {
        assert!(sim.add_structure(p, w.structure_id(n).unwrap()));
    }
    let piett = w.unit_id("Piett_Procursator").unwrap();
    // Build_Cost_Credits is 1 in Units_Hero_Empire_Piett.xml (the hero is a cheap commander; his upgrades cost 25000).
    assert_eq!(w.units[piett as usize].cost, 1.0);
    assert_eq!(unmet(w.check_build_unit(sim.galaxy(), empire, p, piett)), "E_Uplink_Command");
    assert!(sim.add_structure(p, w.structure_id("E_Uplink_Command").unwrap()));
    w.check_build_unit(sim.galaxy(), empire, p, piett).expect("hero after the uplink");
}

#[test]
fn only_listed_planets_build_a_restricted_ship() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data);
    let w = sim.world().clone();
    let empire = w.faction_id("Rebel").unwrap();
    let (kuat, cor) = (w.planet_id("Kuat_WW").unwrap(), w.planet_id("Coruscant_WW").unwrap());
    let ship = w.unit_id("Star_Destroyer_R").unwrap();
    assert!(w.units[ship as usize].buildable && w.units[ship as usize].req_planets.contains(&kuat), "Required_Planets keeps the planets that exist in this galaxy");
    for p in [kuat, cor] {
        bare_planet(&mut sim, empire, p, 3);
        for n in ["R_Shipyard_1", "R_Shipyard_2", "R_Shipyard_3"] {
            assert!(sim.add_structure(p, w.structure_id(n).unwrap()), "{n} on {}", w.planets[p as usize].name);
        }
    }
    w.check_build_unit(sim.galaxy(), empire, kuat, ship).expect("Kuat builds it");
    assert_eq!(unmet(w.check_build_unit(sim.galaxy(), empire, cor, ship)), "planet");
}

#[test]
fn small_planets_cannot_reach_high_levels() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data);
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    // Any real planet whose star base stops below level 3.
    let small = (0..w.planets.len() as PlanetId).find(|&p| w.real[p as usize] && (1..3).contains(&w.planets[p as usize].max_space_base)).expect("a small planet");
    let max = w.planets[small as usize].max_space_base;
    bare_planet(&mut sim, empire, small, 1);
    let yard3 = w.structure_id("E_Shipyard_3").unwrap();
    let cap = w.planet_capacity(sim.galaxy(), small);
    assert_eq!((cap.starbase_max, cap.starbase_level), (max, 1));
    let needs = w.structures[w.structure_id("E_Uplink_Command").unwrap() as usize].req_starbase;
    assert_eq!(needs, 3);
    let uplink = w.structure_id("E_Uplink_Command").unwrap();
    assert_eq!(unmet(w.check_build_structure(sim.galaxy(), empire, small, uplink)), "planet", "a level 3 station never exists on a planet that stops at {max}");
    let _ = yard3;
    // The station upgrade stops at the planet's maximum.
    bare_planet(&mut sim, empire, small, max);
    assert!(w.check_upgrade_starbase(sim.galaxy(), empire, small).is_err(), "no level above {max}");
}

#[test]
fn structures_obey_their_planet_lists() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data);
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let cor = w.planet_id("Coruscant_WW").unwrap();
    bare_planet(&mut sim, empire, cor, 3);
    // Starforge: Lehon only. Gas refinery: gas giants only.
    for n in ["E_Starforge", "E_Gas_Refinery"] {
        let s = w.structure_id(n).unwrap();
        assert_eq!(unmet(w.check_build_structure(sim.galaxy(), empire, cor, s)), "planet", "{n}");
    }
    // Terrain variants: Coruscant is urban, so a temperate outpost is not offered.
    let temperate = w.structure_id("Empire_Outpost_Temperate").unwrap();
    assert_eq!(unmet(w.check_build_structure(sim.galaxy(), empire, cor, temperate)), "planet");
}

#[test]
fn ground_menu_holds_no_army_buildings_and_space_menu_the_yards() {
    let Some(data) = load() else { return };
    let sim = new_sim(&data);
    let w = sim.world();
    let name = |s: &StructureTypeId| w.structures[*s as usize].name.clone();
    for f in ["Empire", "Rebel", "Underworld"] {
        let list = &w.buildable_structures[w.faction_id(f).unwrap() as usize];
        let (space, ground): (Vec<_>, Vec<_>) = list.iter().partition(|&&s| w.structures[s as usize].orbital);
        let ground: Vec<String> = ground.iter().map(|s| name(s)).collect();
        let space: Vec<String> = space.iter().map(|s| name(s)).collect();
        for bad in [
            "Barracks_Base_Urban",
            "Factory_Base_Urban",
            "Factory_Heavy_Base_Temperate",
            "Hangar_Base_Urban",
            "Training_Grounds_Base_Urban",
            "Workshop_Base_Barren",
            "Proving_Grounds_Base_Urban",
            "Support_Hangar_Base_Urban",
            "Starport_Urban",
            "Planet_Rocket_Tower_E",
            "Tower_Defense_Grid_E",
            "Flak_Tower_Defense_Grid_E",
        ] {
            assert!(!ground.iter().any(|g| g.eq_ignore_ascii_case(bad)), "{f}: {bad} is ground-army only");
        }
        assert!(list.iter().all(|&s| w.structures[s as usize].galactic), "{f}: only galactic structures are buildable");
        assert!(!ground.is_empty() && !space.is_empty(), "{f}: both menus have entries");
        assert!(space.iter().any(|n| n.to_ascii_lowercase().contains("shipyard")), "{f}: yards are in the space list: {space:?}");
        assert!(!ground.iter().any(|n| n.to_ascii_lowercase().contains("shipyard")), "{f}: no yard in the ground list");
    }
    let e = &w.buildable_structures[w.faction_id("Empire").unwrap() as usize];
    let has = |n: &str| e.contains(&w.structure_id(n).unwrap());
    // Kept: income, mines, outposts (prerequisite of the economy), ground-to-space weapons, space-side defences.
    for keep in [
        "Empire_Outpost_Urban",
        "MINING_COMPANY_E_BARREN_SPAWNER",
        "TAXATION_AGENCY_URBAN_SPAWNER",
        "Hypervelocity_Gun",
        "Empire_Fortress_Urban",
        "Empire_Watchtower",
        "Empire_Base_Core_Urban",
    ] {
        assert!(has(keep), "{keep} affects the galactic game");
    }
}

#[test]
fn capacity_api_reports_slots_per_planet() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data);
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let cor = w.planet_id("Coruscant_WW").unwrap();
    bare_planet(&mut sim, empire, cor, 2);
    let cap = w.planet_capacity(sim.galaxy(), cor);
    assert_eq!((cap.ground.total, cap.space.total), (6, 7), "Special_Structures_Land / Special_Structures_Space of Coruscant_WW");
    assert_eq!((cap.space.built, cap.ground.built, cap.starbase_level, cap.starbase_max), (0, 0, 2, 3));
    let y1 = w.structure_id("E_Shipyard_1").unwrap();
    sim.apply(Command::BuildStructure { faction: empire, planet: cor, structure: y1 }).expect("queued");
    let cap = w.planet_capacity(sim.galaxy(), cor);
    assert_eq!((cap.space.queued, cap.space.free(), cap.ground.free()), (1, 6, 6));
}

#[test]
fn a_lifetime_limit_of_minus_one_means_unlimited() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data);
    let w = sim.world().clone();
    let rep = w.faction_id("Republic").unwrap();
    let p = w.planet_id("Corellia_WW").unwrap();
    // `Build_Limit_Current_Per_Player` 10, `Build_Limit_Lifetime_Per_Player` -1 (no lifetime limit) in the Consular's data.
    let u = w.unit_id("Consular_Rep").unwrap();
    assert_eq!((w.units[u as usize].limit_current, w.units[u as usize].limit_lifetime), (Some(10), Some(-1)));
    sim.add_credits(rep, 1.0e9);
    let r = w.check_build_unit(sim.galaxy(), rep, p, u);
    assert!(!matches!(r, Err(CommandError::BuildLimit)), "nothing alive, no lifetime limit: {r:?}");
}

#[test]
fn shipyard_planet_applies_production_modifiers() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data);
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let kuat = w.planet_id("Kuat_WW").unwrap();
    let cor = w.planet_id("Coruscant_WW").unwrap();
    bare_planet(&mut sim, empire, kuat, 3);
    bare_planet(&mut sim, empire, cor, 3);

    // Kuat has innate shipyard production modifiers (25% time reduction on ships)
    assert!(!w.planets[kuat as usize].prod_mods.is_empty(), "Kuat has innate production mods in Remake XML");

    if let Some(isd) = w.unit_id("Generic_Star_Destroyer") {
        let (_cor_cost, cor_time) = w.unit_cost(sim.galaxy(), empire, cor, isd);
        let (_kuat_cost, kuat_time) = w.unit_cost(sim.galaxy(), empire, kuat, isd);
        assert!(kuat_time < cor_time, "Kuat shipyard reduces build time: kuat={kuat_time} < cor={cor_time}");
    }
}

/// The star base cap of every planet is the `Max_Space_Base` of the planet XML, and the build checks follow it.
#[test]
fn planet_star_base_caps_come_from_the_planet_data() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data);
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let mut checked = 0;
    for (i, p) in w.planets.iter().enumerate() {
        let xml = data.planet(&p.name).expect("planet in the data").max_space_base;
        assert_eq!(p.max_space_base, xml, "{} takes the Max_Space_Base of its XML", p.name);
        checked += (w.real[i] && xml == 3) as u32;
    }
    assert!(checked > 50, "the Remake's real planets say 3: {checked}");

    // A structure that needs a star base level above a planet's cap cannot be built there; below it the check does not say "planet".
    let y3 = w.structure_id("E_Shipyard_3").unwrap();
    let need = w.structures[y3 as usize].req_starbase;
    let real: Vec<PlanetId> = (0..w.planets.len() as PlanetId).filter(|&p| w.real[p as usize]).collect();
    let capped = real.iter().copied().find(|&p| w.planets[p as usize].max_space_base < need);
    if let Some(p) = capped {
        bare_planet(&mut sim, empire, p, 1);
        assert_eq!(unmet(w.check_build_structure(sim.galaxy(), empire, p, y3)), "planet");
    }
    let open = real.iter().copied().find(|&p| w.planets[p as usize].max_space_base >= need && w.structures[y3 as usize].req_planets.is_empty()).expect("a planet that can take it");
    bare_planet(&mut sim, empire, open, 1);
    assert_ne!(w.check_build_structure(sim.galaxy(), empire, open, y3), Err(CommandError::RequirementsNotMet("planet".into())));
}
