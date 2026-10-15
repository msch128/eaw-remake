//! Super and heavy capital ships against the real Remake 4.0 data: which planets may build them.
//! `Required_Planets` of the unit decides (Eclipse: Byss, Kuat, Fondor; Subjugator: four planets; Executor: any planet with a
//! level 3 shipyard). Every real planet gets a star base of level 3 and the faction's shipyards 1 to 3; the ship must pass the
//! planet rules on exactly the listed planets. Skips without the game and mod.

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

/// Planets (by name) on which `unit` of `faction` passes the planet rules; `None` as the expectation means every real planet.
fn hosts(sim: &mut Sim, faction: &str, shipyard_prefix: &str, unit: &str) -> Vec<String> {
    let w = sim.world().clone();
    let f = w.faction_id(faction).unwrap();
    let u = w.unit_id(unit).unwrap_or_else(|| panic!("no unit {unit}"));
    let yards: Vec<StructureTypeId> = (1..=3).map(|n| w.structure_id(&format!("{shipyard_prefix}_Shipyard_{n}")).unwrap()).collect();
    let mut out = Vec::new();
    for p in (0..w.planets.len() as u32).filter(|&p| w.real[p as usize] && w.max_starbase_level(p) >= 3) {
        sim.set_planet_owner(p, None);
        sim.set_planet_owner(p, Some(f));
        let mut sb = w.first_starbase[f as usize];
        for _ in 1..3 {
            sb = sb.and_then(|b| w.starbases[b as usize].next);
        }
        sim.set_starbase(p, sb);
        sim.add_credits(f, 1.0e9);
        sim.set_tech_level(f, 9);
        for &y in &yards {
            sim.add_structure(p, y);
        }
        // Limits, population and credits are not planet rules: only an unmet requirement counts as "this planet cannot build it".
        match w.check_build_unit(sim.galaxy(), f, p, u) {
            Err(CommandError::RequirementsNotMet(_) | CommandError::NotAvailable) => {}
            _ => out.push(w.planets[p as usize].name.clone()),
        }
    }
    out.sort();
    out
}

fn names(list: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = list.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

#[test]
fn super_ships_are_tied_to_the_planets_of_their_data() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let real = {
        let w = sim.world();
        (0..w.planets.len() as u32).filter(|&p| w.real[p as usize] && w.max_starbase_level(p) >= 3).count()
    };
    let restricted: [(&str, &str, &str, &[&str]); 8] = [
        ("Empire", "E", "Eclipse_Star_Dreadnought_P1", &["Byss_WW", "Kuat_WW", "Fondor_WW"]),
        ("Empire", "E", "Subjugator_E", &["Mustafar_WW", "Muunilinst_WW", "Cato_Neimoidia_WW", "MonCalamari_WW"]),
        ("Empire", "E", "Praetor_Battlecruiser_E", &["Ringo_Vinda_WW", "Rendili_WW", "Rothana_WW", "Allanteen_WW"]),
        ("Empire", "E", "Mandator_II_E", &["Ringo_Vinda_WW", "Rendili_WW", "Rothana_WW", "Allanteen_WW"]),
        ("Rebel", "R", "Subjugator_R", &["Mustafar_WW", "Muunilinst_WW", "Cato_Neimoidia_WW", "MonCalamari_WW"]),
        ("Rebel", "R", "Praetor_Battlecruiser_R", &["Ringo_Vinda_WW", "Rendili_WW", "Rothana_WW", "Allanteen_WW"]),
        ("Underworld", "U", "Subjugator_U", &["Mustafar_WW", "Muunilinst_WW", "Cato_Neimoidia_WW", "MonCalamari_WW"]),
        ("Empire", "E", "Arc_Hammer", &["Kuat_WW", "Fondor_WW"]),
    ];
    for (faction, prefix, unit, expected) in restricted {
        if unit == "Arc_Hammer" {
            continue; // needs the Uplink Command too; covered by the hero test in build_rules.rs
        }
        assert_eq!(hosts(&mut sim, faction, prefix, unit), names(expected), "{unit} builds on its Required_Planets only");
    }
    let everywhere: [(&str, &str, &str); 4] = [
        ("Empire", "E", "Executor_Star_Dreadnought_P1"),
        ("Empire", "E", "Annihilator_Super_Star_Destroyer"),
        ("Rebel", "R", "Viscount_Star_Defender_P1"),
        ("Underworld", "U", "Subjugator_Zann_P1"),
    ];
    for (faction, prefix, unit) in everywhere {
        let h = hosts(&mut sim, faction, prefix, unit);
        assert_eq!(h.len(), real, "{unit} has no Required_Planets: any planet with a level 3 shipyard builds it ({} of {real})", h.len());
    }
}
