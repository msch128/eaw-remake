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
