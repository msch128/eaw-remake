//! The income figure is one fiscal cycle (`Fiscal_Cycle_Time_In_Secs` in `Gameconstants.xml`). A new match's week is that same cycle.
//! `examples/audit_economy.rs` checks the figure itself against the XML. Skips without the game and mod.

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

#[test]
fn a_fiscal_cycle_pays_the_income_figure() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return;
    }
    let data = load_galaxy(&GameFs::for_mod(&install, Some("1770851727")).unwrap()).0;
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    assert_eq!(w.fiscal_cycle_seconds, 45.0, "Gameconstants.xml Fiscal_Cycle_Time_In_Secs");
    assert_eq!(w.config.week_seconds, w.fiscal_cycle_seconds, "a new match's week is the fiscal cycle");
    let secs = w.fiscal_cycle_seconds.round() as u32;
    for name in ["Rebel", "Empire"] {
        let f = w.faction_id(name).unwrap() as usize;
        sim.step(1.0);
        let (income, before) = (sim.galaxy().factions[f].income_per_week, sim.galaxy().factions[f].credits);
        assert!(income > 0.0, "{name} earns something: {income}");
        for _ in 0..secs {
            sim.step(1.0);
        }
        let gained = sim.galaxy().factions[f].credits - before;
        assert!((gained - income).abs() < income * 0.02, "{name}: {secs}s paid {gained:.2}, the cycle figure is {income:.2}");
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

#[test]
fn hero_gambling_ability_evaluates_at_planet() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return;
    }
    let data = load_galaxy(&GameFs::for_mod(&install, Some("1770851727")).unwrap()).0;
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { seed: 42, victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let rebel = w.faction_id("Rebel").unwrap();
    let mothma = w.unit_id("Mon_Mothma").expect("Mon Mothma in catalog");
    let g = w.units[mothma as usize].gamble.expect("Mon Mothma has gambling ability");
    assert_eq!(g.interval, 15.0);
    assert_eq!(g.min, -0.25);
    assert_eq!(g.max, 0.25);
    assert_eq!(g.chance, 0.65);

    // Verify other hero gambling abilities from data:
    let underworld_heroes = [
        ("Star_Jewel", -0.50, 1.0, 0.50, 15.0),
        ("Jabba_Destroyer", -0.50, 1.0, 0.50, 15.0),
        ("Tyber_Zann_Interceptor-IV", -0.50, 1.0, 0.50, 15.0),
        ("Tyber_Zann_Javelin", -0.50, 1.0, 0.50, 15.0),
        ("Tyber_Zann_The_Merciless", -0.50, 1.0, 0.50, 15.0),
        ("Xizor_Virago", -0.50, 1.0, 0.50, 15.0),
    ];
    for (name, min, max, chance, interval) in underworld_heroes {
        let uid = w.unit_id(name).unwrap_or_else(|| panic!("missing {name}"));
        let ug = w.units[uid as usize].gamble.unwrap_or_else(|| panic!("{name} missing gamble"));
        assert_eq!(ug.min, min, "{name} min");
        assert_eq!(ug.max, max, "{name} max");
        assert_eq!(ug.chance, chance, "{name} chance");
        assert_eq!(ug.interval, interval, "{name} interval");
    }

    // Place Mon Mothma at an owned Rebel planet
    let planet = sim.galaxy().planets.iter().position(|p| p.owner == Some(rebel)).unwrap() as PlanetId;
    sim.galaxy_mut().planets[planet as usize].hero_gamble = 0.0;
    sim.galaxy_mut().planets[planet as usize].hero_gamble_at = 0.0;

    let fid = sim.galaxy().fleets.keys().max().copied().unwrap_or(0) + 1;
    sim.galaxy_mut().fleets.insert(fid, Fleet {
        id: fid,
        owner: rebel,
        units: [(mothma, 1)].into(),
        location: FleetLocation::AtPlanet(planet),
        previous_planet: planet,
        slot: 0,
    });

    // Step the sim to trigger evaluation
    sim.step(1.0);
    let gamble_mod = sim.galaxy().planets[planet as usize].hero_gamble;
    assert!(gamble_mod == 0.25 || gamble_mod == -0.25, "gamble should evaluate to min or max, got {gamble_mod}");
    assert!(sim.galaxy().planets[planet as usize].hero_gamble_at >= 15.0);

    // Verify determinism with a second sim using the same seed 42
    let mut sim2 = Sim::new(&data, campaign, SimConfig { seed: 42, victory_enabled: false, ..SimConfig::default() }).unwrap();
    sim2.galaxy_mut().planets[planet as usize].hero_gamble = 0.0;
    sim2.galaxy_mut().planets[planet as usize].hero_gamble_at = 0.0;
    sim2.galaxy_mut().fleets.insert(fid, Fleet {
        id: fid,
        owner: rebel,
        units: [(mothma, 1)].into(),
        location: FleetLocation::AtPlanet(planet),
        previous_planet: planet,
        slot: 0,
    });
    sim2.step(1.0);
    assert_eq!(sim2.galaxy().planets[planet as usize].hero_gamble, gamble_mod);
}

#[test]
fn hero_and_structure_gambling_stack_additively() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return;
    }
    let data = load_galaxy(&GameFs::for_mod(&install, Some("1770851727")).unwrap()).0;
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { seed: 42, victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let rebel = w.faction_id("Rebel").unwrap();
    let mothma = w.unit_id("Mon_Mothma").expect("Mon Mothma in catalog");

    let planet = sim.galaxy().planets.iter().position(|p| p.owner == Some(rebel)).unwrap() as PlanetId;
    let base_val = w.planets[planet as usize].credit_value;
    let base_income = w.planet_income(sim.galaxy(), planet);

    // Add a structure with gambling to the planet
    let struct_with_gamble = w.structures.iter().position(|s| s.gamble.is_some()).expect("structure with gamble");
    sim.galaxy_mut().planets[planet as usize].structures.push(struct_with_gamble as StructureTypeId);
    sim.galaxy_mut().planets[planet as usize].gamble = 0.20;
    sim.galaxy_mut().planets[planet as usize].gamble_at = 100.0;
    let income_with_struct = w.planet_income(sim.galaxy(), planet);
    assert!((income_with_struct - (base_income + base_val * 0.20)).abs() < 1e-3);

    // Place hero in orbit
    let fid = sim.galaxy().fleets.keys().max().copied().unwrap_or(0) + 1;
    sim.galaxy_mut().fleets.insert(fid, Fleet {
        id: fid,
        owner: rebel,
        units: [(mothma, 1)].into(),
        location: FleetLocation::AtPlanet(planet),
        previous_planet: planet,
        slot: 0,
    });
    sim.galaxy_mut().planets[planet as usize].hero_gamble = 0.0;
    sim.galaxy_mut().planets[planet as usize].hero_gamble_at = 0.0;

    // Step sim to evaluate hero gambling
    sim.step(1.0);
    let hero_mod = sim.galaxy().planets[planet as usize].hero_gamble;
    assert!(hero_mod == 0.25 || hero_mod == -0.25);
    // Structure gamble (0.20) and hero gamble (hero_mod) both apply
    assert_eq!(sim.galaxy().planets[planet as usize].gamble, 0.20);
    let combined_income = w.planet_income(sim.galaxy(), planet);
    assert!((combined_income - (base_income + base_val * (0.20 + hero_mod))).abs() < 1e-3);
}
