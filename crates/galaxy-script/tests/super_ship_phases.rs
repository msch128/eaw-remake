//! Super ship damage phases on the galaxy map, against the real Remake data: the levels the `Set_SSD_*_State` object scripts set in a
//! tactical battle reach the `GlobalValue` store, and `Story_SSD_Viscount_Damage.lua` brings the phase the battle ended with back to the map.
//! Skipped without the game and mod.
//!
//! Before: the battle reported no level and its `STORY_WIN_BATTLES` / `STORY_LOSE_BATTLES` never reached the story (only battles the
//! sim's own step settled did), so a Viscount that lost phase 1 and fought on as phase 3 was gone from the map.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::{SimConfig, TacticalResult};

fn scripted() -> Option<ScriptedSim> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        return None;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
    let data = Arc::new(load_galaxy(&*fs).0);
    let c = data.campaign("GC_Full_Web_Way_Rebel")?.clone();
    let config = SimConfig { victory_enabled: false, auto_resolve_battles: false, ..Default::default() };
    ScriptedSim::new(fs, data, &c, config, ScriptedConfig { human: Some("Rebel".into()), remake_ai: true, difficulty: None, ..Default::default() }).ok()
}

#[test]
fn a_viscount_that_ends_a_battle_in_phase_3_is_back_on_the_map_as_phase_3() {
    let Some(mut sim) = scripted() else { return };
    for _ in 0..5 {
        sim.step(1.0);
    }
    let (w, rebel, empire) = sim.with_sim(|s| (s.world().clone(), s.world().faction_id("Rebel").unwrap(), s.world().faction_id("Empire").unwrap()));
    let (p1, p3) = (w.unit_id("Viscount_Star_Defender_P1").unwrap(), w.unit_id("Viscount_Star_Defender_P3").unwrap());
    // An empty planet made Rebel: the Viscount defends it against two Star Destroyers.
    let planet = sim
        .with_sim(|s| (0..w.planets.len() as u32).find(|&p| w.real[p as usize] && s.galaxy().planets[p as usize].owner.is_none() && s.galaxy().fleets_at(p).next().is_none()))
        .expect("an empty planet");
    sim.with_sim_mut(|s| {
        s.set_planet_owner(planet, Some(rebel));
        s.spawn_units(rebel, planet, p1, 1);
        s.spawn_units(empire, planet, w.unit_id("Star_Destroyer").unwrap(), 2);
    });
    let count = |sim: &ScriptedSim, unit| sim.with_sim(|s| s.galaxy().fleets.values().filter(|f| f.owner == rebel).map(|f| f.units.get(&unit).copied().unwrap_or(0)).sum::<u32>());
    let mut battle = None;
    for _ in 0..30 {
        sim.step(1.0);
        battle = sim.with_sim(|s| s.pending_battles().iter().find(|b| b.planet == planet).map(|b| b.id));
        if battle.is_some() {
            break;
        }
    }
    let battle = battle.expect("a battle at the planet");
    let request = sim.tactical_setup(battle).expect("tactical request");
    // The Rebels win; phase 1 is destroyed, phase 3 fought on (`Set_SSD_Viscount_State`: 1 when P1 appears, 2 for P2, 3 for P3), every
    // Imperial ship is lost.
    let rebels = request.sides.iter().position(|s| s.faction == rebel).unwrap();
    let survived: Vec<Vec<u32>> = request
        .sides
        .iter()
        .enumerate()
        .map(|(si, s)| s.groups.iter().map(|g| if si == rebels && g.type_id != p1 { g.count } else { 0 }).collect())
        .collect();
    let retreated = survived.iter().map(|r| vec![0; r.len()]).collect();
    let globals = (1..=3).map(|l| ("Viscount_Level".to_string(), l)).collect();
    let result = TacticalResult { winner: Some(rebels), survived, retreated, duration: 300.0, globals, ..TacticalResult::default() };
    assert_eq!(count(&sim, p1), 1);
    let events = sim.apply_tactical_result(battle, &result).expect("applied");
    eprintln!("events {:?}", events.iter().take(6).collect::<Vec<_>>());
    // `Check_Viscount` runs on the battle's story event, inside `apply_tactical_result`.
    for _ in 0..5 {
        sim.step(1.0);
    }
    let errors: Vec<_> = sim.runtime().errors().iter().filter(|e| format!("{e:?}").contains("SSD")).collect();
    assert!(errors.is_empty(), "{errors:?}");
    eprintln!("P1 {} P3 {}, Viscount_Level {:?}", count(&sim, p1), count(&sim, p3), sim.runtime().global_number("Viscount_Level"));
    assert_eq!(count(&sim, p1), 0, "phase 1 was destroyed");
    assert_eq!(count(&sim, p3), 1, "Check_Viscount spawned phase 3");
    let at = sim.with_sim(|s| s.galaxy().fleets_at(planet).any(|f| f.units.contains_key(&p3)));
    assert!(at, "at the planet of the battle, which the Rebels still hold");
    assert_eq!(sim.runtime().global_number("Viscount_Level"), Some(0.0), "the plot resets the level");
}

#[test]
fn an_auto_resolved_battle_sets_no_level_and_a_lost_phase_stays_lost() {
    // The object scripts exit outside space battles (`Get_Game_Mode() ~= "Space"`): auto-resolve has no phase outcome in the Remake.
    let Some(mut sim) = scripted() else { return };
    for _ in 0..5 {
        sim.step(1.0);
    }
    let (w, rebel, empire) = sim.with_sim(|s| (s.world().clone(), s.world().faction_id("Rebel").unwrap(), s.world().faction_id("Empire").unwrap()));
    let p1 = w.unit_id("Viscount_Star_Defender_P1").unwrap();
    let planet = sim
        .with_sim(|s| (0..w.planets.len() as u32).find(|&p| w.real[p as usize] && s.galaxy().planets[p as usize].owner.is_none() && s.galaxy().fleets_at(p).next().is_none()))
        .expect("an empty planet");
    sim.with_sim_mut(|s| {
        s.set_planet_owner(planet, Some(rebel));
        s.spawn_units(rebel, planet, p1, 1);
        s.spawn_units(empire, planet, w.unit_id("Executor_Star_Dreadnought").unwrap(), 6);
    });
    let mut battle = None;
    for _ in 0..30 {
        sim.step(1.0);
        battle = sim.with_sim(|s| s.pending_battles().iter().find(|b| b.planet == planet).map(|b| b.id));
        if battle.is_some() {
            break;
        }
    }
    let events = sim.resolve_battle_auto(battle.expect("a battle")).expect("resolved");
    assert!(events.iter().any(|e| matches!(e, galaxy_sim::Event::BattleLost { faction, .. } if *faction == rebel)), "six Executors win");
    for _ in 0..5 {
        sim.step(1.0);
    }
    let phases: u32 = sim.with_sim(|s| {
        s.galaxy().fleets.values().filter(|f| f.owner == rebel).flat_map(|f| f.units.iter()).filter(|(u, _)| w.units[**u as usize].name.starts_with("Viscount_Star_Defender_P")).map(|(_, n)| *n).sum()
    });
    eprintln!("Viscount phases left {phases}, Viscount_Level {:?}", sim.runtime().global_number("Viscount_Level"));
    assert_eq!(phases, 0, "no phase comes back");
    assert_eq!(sim.runtime().global_number("Viscount_Level").unwrap_or(0.0), 0.0);
}
