//! AI tech levels in "From the Ground Up" games on the real Remake data (docs/RULES.md 1.12). Data: `Research/Story/Tech_Levels_<F>_WW.xml`
//! (the human's plot of `GC_Full_Web_Way_*`) gives the other two major factions every tech level the human player reaches
//! (`STORY_TECH_LEVEL` -> `SET_TECH_LEVEL`), and `Story_SandBox_*_Ai_Locks.xml` locks the research institute for the AI.
//! Skipped without the game and the mod.

use std::sync::Arc;

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{DifficultyTier, ScriptedConfig, ScriptedSim};
use galaxy_sim::{Command, Event, FactionId, SimConfig};

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

/// The Ground Up campaign of each playable major faction and the faction's name.
const CASES: [(&str, &str); 3] = [("GC_Full_Web_Way_Rebel", "Rebel"), ("GC_Full_Web_Way_Imperial", "Empire"), ("GC_Full_Web_Way_Underworld", "Underworld")];

fn game(c: &Ctx, campaign: &str, human: &str, seed: u64) -> ScriptedSim {
    let campaign = c.data.campaign(campaign).expect("campaign").clone();
    ScriptedSim::new(
        c.fs.clone(),
        c.data.clone(),
        &campaign,
        SimConfig { victory_enabled: false, ground_up: true, seed, ..SimConfig::default() },
        ScriptedConfig { remake_ai: true, human: Some(human.into()), game_scoring: false, strategy: true, difficulty: Some(DifficultyTier::Normal), ..ScriptedConfig::default() },
    )
    .expect("sim")
}

/// Tech level of every major faction, by name.
fn majors_tech(sim: &ScriptedSim) -> Vec<(String, i32)> {
    sim.with_sim(|s| s.world().main_factions.iter().map(|&f| (s.world().factions[f as usize].name.clone(), s.galaxy().factions[f as usize].tech)).collect())
}

/// The human buys tech level 2 the way the player does (its research institute at home, then the upgrade order): the other two major
/// factions are at level 2 in the same step the human's upgrade completes.
#[test]
fn the_ai_majors_reach_each_level_the_human_reaches_in_the_same_step() {
    let Some(c) = ctx() else { return };
    for (campaign, human) in CASES {
        let mut sim = game(&c, campaign, human, 1);
        let h: FactionId = sim.with_sim(|s| s.world().faction_index[&human.to_ascii_lowercase()]);
        assert!(majors_tech(&sim).iter().all(|(_, t)| *t == 1), "{campaign}: every major starts at tech 1: {:?}", majors_tech(&sim));
        sim.with_sim_mut(|s| {
            let home = s.galaxy().owned_planets(h).next().expect("home");
            let institute = s.world().structure_index[&format!("{}_research_institute", &human[..1]).to_ascii_lowercase()];
            s.galaxy_mut().planets[home as usize].structures.push(institute);
            s.add_credits(h, 100_000.0);
            s.apply(Command::UpgradeTech { faction: h, planet: home }).expect("the human orders tech level 2");
        });
        let mut followers_at_human_level = None;
        for _ in 0..(45 * 20) {
            let events = sim.step(1.0).sim_events;
            if events.iter().any(|e| matches!(e, Event::TechAdvanced { faction, level: 2 } if *faction == h)) {
                followers_at_human_level = Some(majors_tech(&sim));
                break;
            }
        }
        let tech = followers_at_human_level.unwrap_or_else(|| panic!("{campaign}: the human's tech level 2 completes within 20 weeks"));
        println!("{campaign}: when the human {human} reaches tech 2: {tech:?}");
        assert!(tech.iter().all(|(_, t)| *t == 2), "{campaign}: every major is at tech 2 in that step: {tech:?}");
    }
}
