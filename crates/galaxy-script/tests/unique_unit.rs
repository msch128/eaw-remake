//! `UNIQUE_UNIT` adds the named ship. A production upgrade that already swapped that hull
//! (phase repair in `Story_Sandbox_Empire_Dreadnoughts.xml`) must not leave two.
//! Skipped without the game and the Remake mod.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::{Command, Lane, ProductionItem, ProductionKind, SimConfig};

fn imperial() -> Option<ScriptedSim> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipped: Remake mod not installed");
        return None;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
    let data = Arc::new(load_galaxy(&*fs).0);
    let campaign = data.campaign("GC_Full_Web_Way_Imperial")?.clone();
    ScriptedSim::new(
        fs,
        data,
        &campaign,
        SimConfig { victory_enabled: false, ..SimConfig::default() },
        ScriptedConfig { remake_ai: false, human: Some("Empire".into()), ..ScriptedConfig::default() },
    )
    .ok()
}

fn count(sim: &ScriptedSim, owner: u32, unit: u32) -> u32 {
    sim.with_sim(|s| s.galaxy().fleets.values().filter(|f| f.owner == owner).map(|f| f.units.get(&unit).copied().unwrap_or(0)).sum())
}

fn at(sim: &ScriptedSim, owner: u32, planet: u32, unit: u32) -> u32 {
    sim.with_sim(|s| {
        s.galaxy()
            .fleets
            .values()
            .filter(|f| f.owner == owner && f.location == galaxy_sim::FleetLocation::AtPlanet(planet))
            .map(|f| f.units.get(&unit).copied().unwrap_or(0))
            .sum()
    })
}

#[test]
fn unique_unit_spawns_the_devastator_and_the_181st() {
    // Story_SandBox_Empire_Web_Way_Full.xml: constructing Black_Squadron_Upgrade removes Black_Squadron
    // and UNIQUE_UNIT Devastator_Star_Destroyer at Kuat. The 181st chain does the same at Coruscant.
    let Some(mut sim) = imperial() else { return };
    let (empire, kuat, coruscant, devastator, black) = sim.with_sim(|s| {
        let w = s.world();
        (
            w.faction_id("Empire").unwrap(),
            w.planet_id("Kuat_WW").unwrap(),
            w.planet_id("Coruscant_WW").unwrap(),
            w.unit_id("Devastator_Star_Destroyer").expect("Devastator_Star_Destroyer is a loaded unit"),
            w.unit_id("Black_Squadron").expect("Black_Squadron is a loaded unit"),
        )
    });
    sim.with_sim_mut(|s| {
        s.set_planet_owner(kuat, Some(empire));
        s.spawn_units(empire, kuat, black, 1);
    });
    let before = count(&sim, empire, devastator);
    sim.runtime_mut().story_notify("STORY_CONSTRUCT", "Black_Squadron_Upgrade");
    assert_eq!(count(&sim, empire, black), 0, "the plot removes Black_Squadron");
    assert_eq!(count(&sim, empire, devastator), before + 1, "UNIQUE_UNIT adds the Devastator");
    assert_eq!(at(&sim, empire, kuat, devastator), 1, "at Kuat, which the Empire holds");
    assert!(
        !sim.with_sim(|s| s.galaxy().factions[empire as usize].unlocked.contains("devastator_star_destroyer")),
        "UNIQUE_UNIT does not unlock the ship"
    );

    let (fighter, interceptor, defender) = sim.with_sim(|s| {
        let w = s.world();
        (w.unit_id("181st_Fighter_Squadron"), w.unit_id("181st_Interceptor_Squadron"), w.unit_id("181st_Defender_Squadron"))
    });
    let (Some(fighter), Some(interceptor)) = (fighter, interceptor) else {
        eprintln!("181st squadron objects are not in the unit catalog; the spawn check used Devastator_Star_Destroyer");
        return;
    };
    sim.with_sim_mut(|s| {
        s.set_planet_owner(coruscant, Some(empire));
        s.spawn_units(empire, coruscant, fighter, 1);
    });
    sim.runtime_mut().story_notify("STORY_CONSTRUCT", "181ST_UPGRADE_LN_TO_IN");
    assert_eq!(count(&sim, empire, fighter), 0, "the plot removes the 181st fighter");
    assert_eq!(at(&sim, empire, coruscant, interceptor), 1, "UNIQUE_UNIT adds the interceptor at Coruscant");
    let Some(defender) = defender else {
        eprintln!("181st_Defender_Squadron is not in the unit catalog");
        return;
    };
    sim.runtime_mut().story_notify("STORY_CONSTRUCT", "181ST_UPGRADE_IN_TO_D");
    assert_eq!(count(&sim, empire, interceptor), 0, "the plot removes the interceptor");
    assert_eq!(at(&sim, empire, coruscant, defender), 1, "UNIQUE_UNIT adds the defender at Coruscant");
}

#[test]
fn a_phase_repair_does_not_spawn_a_second_hull() {
    // Upgrade_Units.lua swaps Piett_Executor_P2 for P1. Story_Sandbox_Empire_Dreadnoughts.xml then
    // REMOVE_UNIT of P2 and UNIQUE_UNIT of P1. No Web Way campaign lists that plot, so the test loads
    // Story_Plots_Empire.xml, which does.
    let Some(mut sim) = imperial() else { return };
    let (empire, kuat, item, p1, p2) = sim.with_sim(|s| {
        let w = s.world();
        (
            w.faction_id("Empire").unwrap(),
            w.planet_id("Kuat_WW").unwrap(),
            w.unit_id("Repair_Phase_2_Piett").expect("Repair_Phase_2_Piett"),
            w.unit_id("Piett_Executor_P1").expect("Piett_Executor_P1"),
            w.unit_id("Piett_Executor_P2").expect("Piett_Executor_P2"),
        )
    });
    sim.with_sim_mut(|s| {
        let w = s.world().clone();
        s.set_planet_owner(kuat, None);
        s.set_planet_owner(kuat, Some(empire));
        let mut sb = w.first_starbase[empire as usize];
        for _ in 1..3 {
            sb = sb.and_then(|b| w.starbases[b as usize].next);
        }
        s.set_starbase(kuat, sb);
        s.add_credits(empire, 1.0e9);
        s.set_tech_level(empire, 9);
        for n in ["Shipyard_1", "Shipyard_2", "Shipyard_3", "Uplink_Command"] {
            assert!(s.add_structure(kuat, w.structure_id(&format!("E_{n}")).unwrap()), "{n}");
        }
        s.spawn_units(empire, kuat, p2, 1);
    });
    sim.apply(Command::BuildUnit { faction: empire, planet: kuat, unit: item }).expect("repair queued");
    sim.with_sim_mut(|s| {
        s.galaxy_mut().planets[kuat as usize].queues[Lane::Units as usize].back_mut().unwrap().remaining = 0.05;
    });
    let started = sim.runtime_mut().load_story_plots("Story_Plots_Empire.xml", Some("Empire")).expect("plots");
    assert!(started >= 3, "vanilla empire plots include the dreadnought file, started {started}");
    sim.step(1.0);
    assert_eq!(count(&sim, empire, p2), 0, "phase 2 was swapped away");
    assert_eq!(at(&sim, empire, kuat, p1), 1, "one phase 1 hull, not a second from UNIQUE_UNIT");

    sim.with_sim_mut(|s| {
        let fleets: Vec<_> = s.galaxy().fleets.keys().copied().collect();
        for fleet in fleets {
            for (ship, u) in s.ships_of(fleet) {
                if u == p1 {
                    s.despawn_ship(ship);
                }
            }
        }
        s.galaxy_mut().planets[kuat as usize].queues[Lane::Units as usize].push_back(ProductionItem {
            kind: ProductionKind::Unit(item),
            remaining: 0.05,
            total: 0.05,
            cost: 0.0,
        });
    });
    sim.step(1.0);
    assert_eq!(at(&sim, empire, kuat, p1), 1, "with no hull to swap, UNIQUE_UNIT still adds the ship");
}
