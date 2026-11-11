//! The income hover card against the simulation's own figure, on the real data.

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use galaxy_play::income::{faction_income_lines, planet_income_lines, whole_credits};
use galaxy_sim::{Sim, SimConfig};

/// Every faction's lines sum to its `income_per_week` (the header's month income) and every planet's to `planet_income`: at the start
/// of the full Imperial Web Way campaign, through 20 months, and with a market swing and an enemy siphon on the Empire's capital.
#[test]
fn the_income_lines_sum_to_the_figure_the_header_shows() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return;
    }
    let data = load_galaxy(&GameFs::for_mod(&install, Some("1770851727")).unwrap()).0;
    let campaign = data.campaign("GC_Full_Web_Way_Imperial").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let rebel = w.faction_id("Rebel").unwrap();
    let check = |sim: &Sim, when: &str| {
        let g = sim.galaxy();
        for f in 0..w.factions.len() {
            let figure = g.factions[f].income_per_week;
            let lines = faction_income_lines(&w, g, f as u32);
            let sum: f64 = lines.iter().map(|l| l.credits).sum();
            assert!((sum - figure).abs() <= 1e-6 * figure.abs().max(1.0) + 1e-3, "{when}: {} lines {sum} figure {figure} {lines:?}", w.factions[f].name);
            let shown = figure as i64;
            assert_eq!(whole_credits(&lines, shown).iter().sum::<i64>(), shown);
        }
        for p in 0..g.planets.len() {
            let figure = f64::from(w.planet_income(g, p as u32));
            let sum: f64 = planet_income_lines(&w, g, p as u32).iter().map(|l| l.credits).sum();
            assert!((sum - figure).abs() <= 1e-3, "{when}: planet {} lines {sum} figure {figure}", w.planets[p].name);
        }
    };
    check(&sim, "start");
    let empire_lines = faction_income_lines(&w, sim.galaxy(), empire);
    assert!(empire_lines.iter().any(|l| l.label.starts_with("Planet values (")), "{empire_lines:?}");
    assert!(empire_lines.iter().any(|l| l.label == "Ship upkeep" && l.credits < 0.0), "{empire_lines:?}");
    for month in 1..=20 {
        sim.step(w.fiscal_cycle_seconds);
        check(&sim, &format!("month {month}"));
    }
    // A trade hub's market swing and a Rebel agent siphoning a third of the capital's income.
    let capital = (0..sim.galaxy().planets.len()).filter(|&p| sim.galaxy().planets[p].owner == Some(empire)).max_by(|&a, &b| w.planet_income(sim.galaxy(), a as u32).total_cmp(&w.planet_income(sim.galaxy(), b as u32))).unwrap();
    {
        let g = sim.galaxy_mut();
        g.planets[capital].gamble = 0.7;
        g.planets[capital].siphon = Some(galaxy_sim::agents::ActiveSiphon { by: rebel, agent: 0, share: 1.0 / 3.0, cycles_left: 3, cycle_end: f64::MAX });
    }
    sim.step(0.0);
    check(&sim, "swing and siphon");
    let lines = faction_income_lines(&w, sim.galaxy(), empire);
    assert!(lines.iter().any(|l| l.label == "Market swings" && l.credits > 0.0), "{lines:?}");
    assert!(lines.iter().any(|l| l.label == "Siphoned off by enemy agents" && l.credits < 0.0), "{lines:?}");
    let rebel_lines = faction_income_lines(&w, sim.galaxy(), rebel);
    assert!(rebel_lines.iter().any(|l| l.label == "Siphoned from enemy planets" && l.credits > 0.0), "{rebel_lines:?}");
}
