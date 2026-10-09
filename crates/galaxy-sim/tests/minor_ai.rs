//! Minor factions must play: with every faction but the player on AI, each non-pirate faction spends credits and builds ships.

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

#[test]
fn every_non_pirate_faction_builds_in_twenty_weeks() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, auto_resolve_battles: true, ..SimConfig::default() }).expect("sim");
    let w = sim.world().clone();
    let human = w.victory.active_player;
    let ai: Vec<FactionId> = (0..w.factions.len() as FactionId).filter(|f| Some(*f) != human).collect();
    sim.set_ai_factions(ai.clone());
    let ships = |sim: &Sim, f: FactionId| -> u32 { sim.galaxy().fleets.values().filter(|x| x.owner == f).map(|x| x.unit_count()).sum() };
    let before: Vec<(u32, f64)> = (0..w.factions.len() as FactionId).map(|f| (ships(&sim, f), sim.galaxy().factions[f as usize].stats.credits_spent)).collect();
    let weeks = 20.0 * w.config.week_seconds;
    let mut t = 0.0;
    while t < weeks {
        sim.step(w.config.tick_seconds.max(1.0));
        t += w.config.tick_seconds.max(1.0);
        for b in sim.pending_battles().iter().map(|b| b.id).collect::<Vec<_>>() {
            let _ = sim.resolve_battle(b);
        }
    }
    let mut idle = Vec::new();
    for &f in &ai {
        let fs = &sim.galaxy().factions[f as usize];
        let name = &w.factions[f as usize].name;
        let owned = sim.galaxy().owned_planets(f).count();
        let spent = fs.stats.credits_spent - before[f as usize].1;
        let built = fs.stats.ships_built;
        println!("{name:14} planets {owned:3} spent {spent:9.0} ships built {built:3} ships {} -> {} credits {:.0} buildable units {}", before[f as usize].0, ships(&sim, f), fs.credits, w.buildable_units[f as usize].len());
        if name.to_ascii_lowercase().contains("pirate") || owned == 0 {
            continue;
        }
        if spent <= 0.0 || built == 0 {
            idle.push(name.clone());
        }
    }
    assert!(idle.is_empty(), "factions that built nothing in 20 weeks: {idle:?}");
}
