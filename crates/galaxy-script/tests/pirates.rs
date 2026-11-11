//! The Pirates grow with the human player (docs/RULES.md 1.9), on the real Remake data in "From the Ground Up" games with the story scripts running.
//! Skipped without the game and the mod.

use std::sync::Arc;

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::{FactionId, Sim, SimConfig};

struct Ctx {
    fs: Arc<dyn FileSource>,
    data: Arc<GameData>,
}

fn ctx() -> Option<Ctx> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
    let (data, _) = load_galaxy(&*fs);
    Some(Ctx { fs, data: Arc::new(data) })
}

fn scripted(c: &Ctx, campaign: &str, human: &str) -> ScriptedSim {
    let campaign = c.data.campaign(campaign).expect("campaign").clone();
    ScriptedSim::new(
        c.fs.clone(),
        c.data.clone(),
        &campaign,
        SimConfig { victory_enabled: false, ground_up: true, seed: 5, ..SimConfig::default() },
        ScriptedConfig { remake_ai: false, human: Some(human.into()), game_scoring: false, ..ScriptedConfig::default() },
    )
    .expect("sim")
}

/// Pirate planets by their star base level.
fn base_levels(s: &Sim) -> std::collections::BTreeMap<i32, usize> {
    let (w, g) = (s.world(), s.galaxy());
    let pirates = w.passive_faction().unwrap();
    let mut m = std::collections::BTreeMap::new();
    for p in (0..w.planets.len() as u32).filter(|&p| w.real[p as usize] && g.planets[p as usize].owner == Some(pirates)) {
        *m.entry(w.starbase_level(g, p)).or_default() += 1;
    }
    m
}

fn faction(s: &Sim, name: &str) -> FactionId {
    s.world().faction_id(name).unwrap()
}

#[test]
fn an_all_ai_game_has_no_human_and_the_pirates_follow_the_best_major_faction() {
    let Some(c) = ctx() else { return };
    let mut sim = scripted(&c, "GC_Full_Web_Way_Rebel", "none");
    assert_eq!(sim.with_sim(|s| s.world().victory.active_player), None, "human \"none\": nobody is the human player");
    // Ground Up gives every main faction three planets when nobody is human.
    sim.with_sim(|s| {
        for f in ["Rebel", "Empire", "Underworld"] {
            assert_eq!(s.galaxy().owned_planets(faction(s, f)).count(), 3, "{f} keeps three planets");
        }
    });
    // The Empire reaches tech 4 while the Rebels stay at 1: the Pirates follow the best major faction (their bases rise to level 2).
    sim.with_sim_mut(|s| {
        let e = faction(s, "Empire");
        s.galaxy_mut().factions[e as usize].tech = 4;
    });
    let week = sim.with_sim(|s| s.world().config.week_seconds);
    for _ in 0..(week as u32 + 1) {
        sim.step(1.0);
    }
    let levels = sim.with_sim(base_levels);
    println!("pirate base levels after a week with the Empire at tech 4: {levels:?}");
    let total: usize = levels.values().sum();
    let raised: usize = levels.iter().filter(|(l, _)| **l >= 2).map(|(_, n)| n).sum();
    assert!(raised * 10 >= total * 9, "bases follow the best major faction's tech: {levels:?}");
}

#[test]
fn a_game_with_a_human_keeps_that_human() {
    let Some(c) = ctx() else { return };
    for (campaign, human) in [("GC_Full_Web_Way_Rebel", "Rebel"), ("GC_Full_Web_Way_Imperial", "Empire"), ("GC_Full_Web_Way_Underworld", "Underworld")] {
        let sim = scripted(&c, campaign, human);
        sim.with_sim(|s| {
            assert_eq!(s.world().victory.active_player, Some(faction(s, human)), "{campaign}");
            assert_eq!(s.galaxy().owned_planets(faction(s, human)).count(), 1, "{campaign}: the human starts with one planet");
        });
    }
}
