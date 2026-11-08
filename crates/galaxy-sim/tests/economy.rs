//! Income accrues as the data says: over one week every main faction without a market gamble gains its weekly income in credits
//! (`examples/audit_economy.rs` checks the weekly income itself against the XML for every campaign start). Skips without the game and mod.

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

#[test]
fn a_week_pays_the_weekly_income() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return;
    }
    let data = load_galaxy(&GameFs::for_mod(&install, Some("1770851727")).unwrap()).0;
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    for name in ["Rebel", "Empire"] {
        let f = w.faction_id(name).unwrap() as usize;
        sim.step(1.0);
        let (income, before) = (sim.galaxy().factions[f].income_per_week, sim.galaxy().factions[f].credits);
        assert!(income > 0.0, "{name} earns something: {income}");
        for _ in 0..w.config.week_seconds as u32 {
            sim.step(1.0);
        }
        let gained = sim.galaxy().factions[f].credits - before;
        assert!((gained - income).abs() < income * 0.02, "{name}: a week paid {gained:.0}, the weekly income is {income:.0}");
    }
}

#[test]
fn unit_percentage_income_modifier_boosts_planet_income() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return;
    }
    let data = load_galaxy(&GameFs::for_mod(&install, Some("1770851727")).unwrap()).0;
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let rebel = w.faction_id("Rebel").unwrap();
    let planet = sim.galaxy().planets.iter().position(|p| p.owner == Some(rebel)).unwrap() as PlanetId;
    let base_income = w.planet_income(sim.galaxy(), planet);
    let base_val = w.planets[planet as usize].credit_value;
    assert!(base_val > 0.0);

    let errant = w.unit_id("Errant_Venture_Star_Destroyer_II").expect("Errant Venture in catalog");
    let freighters = [
        ("Errant_Venture_Star_Destroyer_II", 0.40),
        ("Freighter_YT1300", 0.40),
        ("Freighter_ActionIV", 0.10),
        ("Freighter_GR75", 0.15),
        ("Freighter_Quasar", 0.25),
        ("Freighter_Acclamator", 0.40),
        ("Freighter_Venator", 0.55),
        ("Freighter_Lucrehulk", 1.50),
    ];
    for (name, expected_pct) in freighters {
        let uid = w.unit_id(name).unwrap_or_else(|| panic!("missing {name} in catalog"));
        assert_eq!(w.units[uid as usize].income_pct, expected_pct, "{name} income_pct mismatch");
    }

    let fid = sim.galaxy().fleets.keys().max().copied().unwrap_or(0) + 1;
    sim.galaxy_mut().fleets.insert(fid, Fleet {
        id: fid,
        owner: rebel,
        units: [(errant, 1)].into(),
        location: FleetLocation::AtPlanet(planet),
        previous_planet: planet,
        slot: 0,
    });
    let boosted_income = w.planet_income(sim.galaxy(), planet);
    assert!(boosted_income > base_income, "Errant Venture (+40%) should boost planet income, got {boosted_income} == {base_income}");
    assert!((boosted_income - (base_income + base_val * 0.4)).abs() < 1e-3);
}
