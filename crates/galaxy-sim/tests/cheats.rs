//! The cheat commands of the F2 menu against the real data (skipped without the game): each is a journaled command that
//! marks the game as cheated.

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::cheats::Cheat;
use galaxy_sim::*;

fn load() -> Option<GameData> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    Some(load_galaxy(&GameFs::for_mod(&install, Some("1770851727")).ok()?).0)
}

fn new_sim(data: &GameData) -> Sim {
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    Sim::new(data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim")
}

#[test]
fn credits_cheat_adds_credits_and_marks_the_game() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data);
    let empire = sim.world().faction_id("Empire").unwrap();
    assert!(!sim.galaxy().cheated);
    let before = sim.galaxy().factions[empire as usize].credits;
    sim.apply(Command::Cheat { faction: empire, cheat: Cheat::Credits(5000) }).unwrap();
    assert_eq!(sim.galaxy().factions[empire as usize].credits, before + 5000.0);
    assert!(sim.galaxy().cheated);
}

#[test]
fn finish_production_completes_the_front_item_on_the_next_tick() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data);
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let p = w.planet_id("Coruscant_WW").unwrap();
    sim.set_planet_owner(p, None);
    sim.set_planet_owner(p, Some(empire));
    sim.add_credits(empire, 1.0e9);
    let sb = w.first_starbase[empire as usize];
    sim.set_starbase(p, sb);
    let structure = (0..w.structures.len() as u32).find(|&s| {
        sim.world().check_build_structure(sim.galaxy(), empire, p, s).is_ok()
    });
    let Some(structure) = structure else { return };
    sim.apply(Command::BuildStructure { faction: empire, planet: p, structure }).unwrap();
    assert_eq!(sim.galaxy().planets[p as usize].queues.iter().map(|q| q.len()).sum::<usize>(), 1);
    // Not owned: refused.
    let rebel = w.faction_id("Rebel").unwrap();
    assert_eq!(sim.apply(Command::Cheat { faction: rebel, cheat: Cheat::FinishProduction { planet: p } }), Err(CommandError::NotOwner));
    sim.apply(Command::Cheat { faction: empire, cheat: Cheat::FinishProduction { planet: p } }).unwrap();
    sim.step(1.0);
    assert_eq!(sim.galaxy().planets[p as usize].queues.iter().map(|q| q.len()).sum::<usize>(), 0, "the build finished");
    assert!(sim.galaxy().planets[p as usize].structures.len() >= 1);
}

#[test]
fn tech_up_raises_the_level_until_the_maximum() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data);
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let start = sim.galaxy().factions[empire as usize].tech;
    sim.apply(Command::Cheat { faction: empire, cheat: Cheat::TechUp }).unwrap();
    assert_eq!(sim.galaxy().factions[empire as usize].tech, start + 1);
    for _ in 0..20 {
        let _ = sim.apply(Command::Cheat { faction: empire, cheat: Cheat::TechUp });
    }
    assert_eq!(sim.galaxy().factions[empire as usize].tech, w.factions[empire as usize].max_tech);
    assert!(matches!(sim.apply(Command::Cheat { faction: empire, cheat: Cheat::TechUp }), Err(CommandError::RequirementsNotMet(_))));
}

#[test]
fn ai_paused_stops_the_galaxy_ai() {
    let Some(data) = load() else { return };
    let run = |paused: bool| {
        let mut sim = new_sim(&data);
        let w = sim.world().clone();
        let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
        sim.set_ai_factions([empire]);
        if paused {
            sim.apply(Command::Cheat { faction: rebel, cheat: Cheat::AiPaused(true) }).unwrap();
        }
        let before = sim.galaxy().factions[empire as usize].stats.credits_spent;
        for _ in 0..600 {
            sim.step(1.0);
        }
        (sim.galaxy().factions[empire as usize].stats.credits_spent - before, sim.galaxy().ai_paused)
    };
    let (spent_running, flag_running) = run(false);
    let (spent_paused, flag_paused) = run(true);
    assert!(!flag_running && flag_paused);
    assert!(spent_running > 0.0, "the AI spends credits when running");
    assert_eq!(spent_paused, 0.0, "a paused AI orders nothing");
}

#[test]
fn cheats_replay_deterministically() {
    let Some(data) = load() else { return };
    let play = || {
        let mut sim = new_sim(&data);
        let empire = sim.world().faction_id("Empire").unwrap();
        sim.apply(Command::Cheat { faction: empire, cheat: Cheat::Credits(5000) }).unwrap();
        sim.apply(Command::Cheat { faction: empire, cheat: Cheat::TechUp }).unwrap();
        for _ in 0..50 {
            sim.step(0.5);
        }
        sim.galaxy().state_hash()
    };
    assert_eq!(play(), play());
}

#[test]
fn mark_cheated_changes_nothing_else() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data);
    let empire = sim.world().faction_id("Empire").unwrap();
    let before = sim.galaxy().factions[empire as usize].credits;
    sim.apply(Command::Cheat { faction: empire, cheat: Cheat::MarkCheated }).unwrap();
    assert!(sim.galaxy().cheated);
    assert_eq!(sim.galaxy().factions[empire as usize].credits, before);
    assert!(!sim.galaxy().ai_paused);
}

#[test]
fn tech_levels_get_exponentially_dearer_for_every_faction() {
    assert_eq!([2, 3, 4, 5].map(galaxy_sim::tech_level_cost), [5000.0, 10000.0, 20000.0, 50000.0]);
}
