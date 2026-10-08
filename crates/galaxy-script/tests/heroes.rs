//! The Remake's hero respawn timers run on STORY_TACTICAL_DESTROY: a hero killed in a battle comes back after 300 s.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptEvent, ScriptedConfig, ScriptedSim};
use galaxy_sim::{Event, SimConfig};

#[test]
fn a_dead_hero_respawns_through_the_story_timer() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        return;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).unwrap());
    let data = Arc::new(load_galaxy(&*fs).0);
    let c = data.campaign("GC_Full_Web_Way_Rebel").unwrap().clone();
    let mut sim = ScriptedSim::new(fs, data, &c, SimConfig { victory_enabled: false, ..Default::default() }, ScriptedConfig { human: Some("Rebel".into()), remake_ai: false, difficulty: None, ..Default::default() }).unwrap();
    for _ in 0..5 {
        sim.step(1.0);
    }
    let (w, rebel, empire) = sim.with_sim(|s| (s.world().clone(), s.world().faction_id("Rebel").unwrap(), s.world().faction_id("Empire").unwrap()));
    let hero = w.unit_id("Mon_Mothma").unwrap();
    let target = sim.with_sim(|s| (0..w.planets.len() as u32).find(|&p| w.real[p as usize] && s.galaxy().planets[p as usize].owner.is_none() && s.galaxy().fleets_at(p).next().is_none()).unwrap());
    let yavin = w.planet_id("Yavin_WW").unwrap();
    // Make sure she is not already at Yavin from the start script.
    let mothma_count = |sim: &ScriptedSim| sim.with_sim(|s| s.galaxy().fleets.values().filter(|f| f.owner == rebel).map(|f| f.units.get(&hero).copied().unwrap_or(0)).sum::<u32>());
    sim.with_sim_mut(|s| {
        s.spawn_units(rebel, target, hero, 1);
        s.spawn_units(empire, target, w.unit_id("Star_Destroyer").unwrap(), 4);
    });
    let before = mothma_count(&sim);
    let (mut killed, mut returned) = (false, false);
    for t in 0..420 {
        let r = sim.step(1.0);
        killed |= r.sim_events.iter().any(|e| matches!(e, Event::HeroDestroyed { unit, .. } if *unit == hero));
        returned |= r.script_events.iter().any(|e| matches!(e, ScriptEvent::UnitSpawned { unit, .. } if unit.eq_ignore_ascii_case("Mon_Mothma")));
        if !r.script_events.is_empty() || t % 60 == 0 { println!("t={t} fired={} script_events={:?}", sim.runtime().story_fired(), r.script_events); }
        if t == 10 {
            assert!(killed, "the battle kills her");
            assert_eq!(mothma_count(&sim), before - 1);
        }
    }
    assert!(returned, "the story timer respawns the hero");
    assert_eq!(mothma_count(&sim), before, "she is back");
    let at_yavin = sim.with_sim(|s| s.galaxy().fleets_at(yavin).any(|f| f.units.contains_key(&hero)));
    println!("respawned at Yavin: {at_yavin}");
}
