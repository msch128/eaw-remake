//! Match variety: personality jitter and openings come from the match seed.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::{Event, SimConfig};

struct Run {
    hash: u64,
    queued: u32,
    characters: Vec<String>,
}

fn run(seed: u64, variety: f64, weeks: u64) -> Option<Run> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        return None;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
    let data = Arc::new(load_galaxy(&*fs).0);
    let campaign = data.campaign("GC_Full_Web_Way_Rebel")?.clone();
    let mut sim = ScriptedSim::new(
        fs,
        data,
        &campaign,
        SimConfig { victory_enabled: false, seed, ..SimConfig::default() },
        ScriptedConfig { remake_ai: true, human: Some("none".into()), game_scoring: false, variety, ..ScriptedConfig::default() },
    )
    .ok()?;
    let characters = ["Empire", "Rebel", "Underworld"].iter().map(|f| format!("{:?}", sim.ai_character(f))).collect();
    let mut done = 0;
    while done < weeks {
        for e in sim.step(1.0).sim_events {
            if let Event::WeekEnded { week } = e {
                done = week;
            }
        }
    }
    Some(Run { hash: sim.with_sim(|s| s.galaxy().state_hash()), queued: sim.ai_stats()?.taskforces.ships_queued, characters })
}

#[test]
fn seeds_change_the_opening_and_the_same_seed_repeats() {
    let Some(a) = run(1, 0.5, 15) else { return };
    let a2 = run(1, 0.5, 15).unwrap();
    assert_eq!((a.hash, a.queued), (a2.hash, a2.queued), "same seed, same match");
    assert_eq!(a.characters, a2.characters);
    let b = run(2, 0.5, 15).unwrap();
    assert_ne!(a.characters, b.characters, "another seed draws other characters: {:?} vs {:?}", a.characters, b.characters);
    assert_ne!((a.hash, a.queued), (b.hash, b.queued), "and plays another opening");
    // The variation itself (not just the sim's own seed) changes the play.
    let flat = run(1, 0.0, 15).unwrap();
    assert!(flat.characters.iter().all(|c| c.contains("Balanced")), "{:?}", flat.characters);
    assert_ne!((a.hash, a.queued), (flat.hash, flat.queued), "variety 0 plays the old way");
    println!("{:?}\n{:?}", a.characters, b.characters);
}
