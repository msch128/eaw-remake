//! Remake tech levels come from the Research Institute: `E_Tech-2` ... `R_Tech-5` (`Build_Advances_Tech_Level`, variants of `Dummy_<F>`) need the
//! faction's `<F>_Research_Institute` on the planet, take 300 / 450 / 600 / 750 s, and the Clone Wars stack adds `CIS_Tech-*` / `Republic_Tech-*`.
//! Prices are the design rule (`tech_level_cost`), not the object's. Skipped when a mod folder is missing.

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn load(alias: &str, mods: &[&str]) -> Option<GameData> {
    let install = GameInstall::detect()?;
    for id in mods {
        if !install.mod_data(id).is_dir() {
            eprintln!("skipping: workshop mod {id} not found");
            return None;
        }
    }
    Some(load_galaxy(&GameFs::for_mod(&install, Some(alias)).ok()?).0)
}

/// The chain, the institute requirement and the purchase rule of every main faction of `campaign`.
fn check_chain(data: &GameData, campaign: &str, name: &str, prefix: &str) -> bool {
    let c = data.campaign(campaign).unwrap();
    let mut sim = Sim::new(data, c, SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let f = w.faction_id(name).unwrap();
    let info = &w.factions[f as usize];
    assert!(!info.tech_upgrades.is_empty(), "{name} has a tech chain");
    for (n, up) in info.tech_upgrades.iter().enumerate() {
        let level = info.start_tech + n as i32 + 1;
        assert_eq!(up.name, format!("{prefix}_Tech-{level}"), "{name}: level {level} is the mod's upgrade object");
        let seconds = match level { 1 => 50.0, 2 => 300.0, 3 => 450.0, 4 => 600.0, _ => 750.0 };
        assert_eq!(up.time, seconds, "{name} level {level} time");
        assert_eq!(up.req_structures.len(), 1, "{name} level {level} needs one structure group");
        let inst = &w.structures[up.req_structures[0][0] as usize].name;
        assert!(inst.to_ascii_lowercase().ends_with("research_institute"), "{name} level {level} needs {inst}");
    }
    // Bought only where the institute stands.
    let owned: Vec<PlanetId> = sim.galaxy().owned_planets(f).collect();
    let req = &info.tech_upgrades[(sim.galaxy().factions[f as usize].tech - info.start_tech).max(0) as usize].req_structures;
    let with: Vec<PlanetId> = owned.iter().copied().filter(|&p| w.missing_structures(&sim.galaxy().planets[p as usize], req).is_none()).collect();
    if with.is_empty() {
        return false;
    }
    let without = owned.iter().copied().find(|p| !with.contains(p));
    if let Some(p) = without {
        assert!(
            matches!(w.check_upgrade_tech(sim.galaxy(), f, p), Err(CommandError::RequirementsNotMet(_))),
            "{name}: no tech at a planet without the institute"
        );
    }
    sim.galaxy_mut().factions[f as usize].credits = 1.0e6;
    let p = with.iter().copied().find(|&p| w.starbase_level(sim.galaxy(), p) >= 1);
    if let Some(p) = p {
        assert!(w.check_upgrade_tech(sim.galaxy(), f, p).is_ok(), "{name}: tech can be bought at the institute planet");
    }
    true
}

#[test]
fn clone_wars_factions_research_too() {
    let Some(data) = load("fft", &["3229239424", "2794270450", "1770851727"]) else { return };
    for (faction, suffix) in [("CIS", "_cis"), ("Republic", "_republic")] {
        // Only some campaigns start with the institute (Shatterpoint); the chain itself is the same everywhere.
        let found = data.campaigns.iter().filter(|c| c.has_galaxy() && c.name.to_ascii_lowercase().ends_with(suffix)).any(|c| check_chain(&data, &c.name, faction, faction));
        assert!(found, "a {faction} campaign starts with the Research Institute");
    }
}

#[test]
fn remake_tech_levels_need_the_research_institute() {
    let Some(data) = load("1770851727", &["1770851727"]) else { return };
    // Each campaign gives the institute to its own player faction (Starting_Forces); the others reach tech through the story scripts.
    for (campaign, faction, prefix) in [("GC_Full_Web_Way_Rebel", "Rebel", "R"), ("GC_Full_Web_Way_Imperial", "Empire", "E"), ("GC_Full_Web_Way_Underworld", "Underworld", "U")] {
        assert!(check_chain(&data, campaign, faction, prefix), "{faction} starts with a Research Institute in {campaign}");
    }
}

#[test]
fn the_ai_buys_levels_without_the_institute() {
    let Some(data) = load("1770851727", &["1770851727"]) else { return };
    let mut sim = Sim::new(&data, data.campaign("GC_Full_Web_Way_Rebel").unwrap(), SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    assert_ne!(w.victory.active_player, Some(empire), "the Empire is the AI in the Rebel campaign");
    sim.galaxy_mut().factions[empire as usize].credits = 1.0e6;
    let planet = sim.galaxy().owned_planets(empire).find(|&p| w.starbase_level(sim.galaxy(), p) >= 1).expect("an Empire planet with a star base");
    assert!(w.check_upgrade_tech(sim.galaxy(), empire, planet).is_ok(), "the AI needs no Research Institute");
}
