//! The Remake's galactic plan scripts on real data; skipped when the game or the mod is not installed.

use std::collections::BTreeSet;
use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{DifficultyTier, ScriptedConfig, ScriptedSim};
use galaxy_sim::{Event, SimConfig};

const UNITE: &str = "GC_Conquer_To_Unite_Territory_Ground";

fn build(campaign: &str, seed: u64) -> Option<ScriptedSim> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipped: Remake mod not installed");
        return None;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
    let (data, _) = load_galaxy(&*fs);
    let data = Arc::new(data);
    let campaign = data.campaign(campaign)?.clone();
    ScriptedSim::new(
        fs,
        data,
        &campaign,
        SimConfig { victory_enabled: false, seed, ..SimConfig::default() },
        ScriptedConfig { remake_ai: true, human: Some("none".into()), game_scoring: false, difficulty: Some(DifficultyTier::Normal), ..ScriptedConfig::default() },
    )
    .ok()
}

/// `Conquer_To_Unite_Territory_Ground.lua` attacks through `SynchronizedAssemble` alone: it never calls `Move_To`, and tests
/// `Planet_Has_Friendly_Space_Force_In_Orbit` on the target right after. The Empire of `GC_Full_Web_Way_Imperial` has no other offensive
/// goal with a desire (it never banks the credits the conquer goals ask for), so its attacks are this plan's: the force must reach the
/// target while the goal runs, and open the battle there.
#[test]
fn the_unite_plan_brings_its_force_to_the_target() {
    let Some(mut sim) = build("GC_Full_Web_Way_Imperial", 1) else { return };
    let empire = sim.with_sim(|s| s.world().faction_id("Empire")).expect("Empire");
    let (mut weeks, mut attacks_on_unite_targets, mut unite_targets) = (0, 0u32, BTreeSet::new());
    while weeks < 20 {
        let targets: BTreeSet<u32> = sim.ai_active_goals(empire).into_iter().filter(|(g, _, _)| g.eq_ignore_ascii_case(UNITE)).filter_map(|(_, t, _)| t).collect();
        unite_targets.extend(targets.iter().copied());
        for e in sim.step(1.0).sim_events {
            match e {
                Event::WeekEnded { week } => weeks = week,
                Event::BattlePending(b) if targets.contains(&b.planet) && b.defender != Some(empire) => {
                    let attacking = sim.with_sim(|s| b.fleets.iter().filter_map(|f| s.galaxy().fleets.get(f)).any(|f| f.owner == empire));
                    attacks_on_unite_targets += u32::from(attacking);
                }
                _ => {}
            }
        }
    }
    let st = sim.ai_stats().expect("Remake AI is on");
    let started = st.plans_started.get(&(empire, UNITE.to_string())).copied().unwrap_or(0);
    println!("20 weeks: Empire started {started} unite plans on {} planets, opened {attacks_on_unite_targets} battles at their targets", unite_targets.len());
    assert!(started > 0, "the Empire should want to unite its territory");
    assert!(attacks_on_unite_targets > 0, "{started} unite plans, and no Empire fleet ever fought at one of their targets");
    assert!(sim.runtime().errors().is_empty());
}

/// `Needs_Income_Gases` is `500 + ...` on every friendly planet ("we ALWAYS want gas refineries/depots"), but `Gas_Depot_Default` and
/// `Gas_Refinery_Default` name the gas giants in `Required_Planets`. A plan that could build nothing at its target is not started: it would
/// only use up the goal starts of the evaluation (three per evaluation here) with a script that fails after a second.
#[test]
fn structure_plans_that_can_build_nothing_are_not_started() {
    let Some(mut sim) = build("GC_Full_Web_Way_Imperial", 1) else { return };
    let empire = sim.with_sim(|s| s.world().faction_id("Empire")).expect("Empire");
    let mut weeks = 0;
    while weeks < 15 {
        for e in sim.step(1.0).sim_events {
            if let Event::WeekEnded { week } = e {
                weeks = week;
            }
        }
    }
    let st = sim.ai_stats().expect("Remake AI is on");
    let gas = (empire, "Build_Income_Gases".to_string());
    let (started, failed) = (st.plans_started.get(&gas).copied().unwrap_or(0), st.plans_failed.get(&gas).copied().unwrap_or(0));
    let infrastructure: u32 = st.plans_started.iter().filter(|((p, g), _)| *p == empire && g.starts_with("Build_")).map(|(_, n)| *n).sum();
    println!("15 weeks: Empire started {started} gas plans ({failed} failed) of {infrastructure} build plans, {} activations refused", st.activation_failures);
    // The first plans of a script show the engine what it builds (up to three start in one evaluation); from then on the check runs before the start.
    assert!(failed <= 3, "{failed} of {started} gas plans failed: the Empire holds no gas giant it could build on");
    assert!(st.activation_failures > 0);
    assert!(infrastructure > started, "the Empire still builds");
}

/// The front garrisons (an estimate, `TaskForces::update_garrisons`) may tie up at most `GARRISON_TOTAL_CAP` of a player's fleet strength; the
/// rest is the free store the plans' `Are_All_Units_On_Free_Store` and `Produce_Force` draw on.
#[test]
fn garrisons_leave_most_of_the_fleet_to_the_plans() {
    let Some(mut sim) = build("GC_Full_Web_Way_Imperial", 1) else { return };
    let mut weeks = 0;
    while weeks < 20 {
        for e in sim.step(1.0).sim_events {
            if let Event::WeekEnded { week } = e {
                weeks = week;
            }
        }
    }
    let share = sim.ai_stats().expect("Remake AI is on").taskforces.garrison_share_max;
    println!("20 weeks: garrisons held at most {:.0}% of a player's fleet strength", 100.0 * share);
    assert!(share > 0.0, "front planets keep a garrison");
    assert!(share <= galaxy_ai::taskforce::GARRISON_TOTAL_CAP + 1e-9, "garrisons held {:.0}% of a player's fleet strength", 100.0 * share);
}

/// A planet whose production queue is full is busy, not unable: the Remake AI is told so for a ship, an orbital structure and the star base
/// level of a full space queue. What the planet could not build with an empty queue either stays "unavailable", and the ground queue answers
/// for itself.
#[test]
fn a_full_queue_answers_busy_to_the_ai() {
    use galaxy_ai::sim::{AiSim, Refused};
    use galaxy_sim::{Command, Lane, Sim, UnitRole};
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipped: Remake mod not installed");
        return;
    }
    let data = load_galaxy(&GameFs::for_mod(&install, Some("1770851727")).unwrap()).0;
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let (empire, rebel, p) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap(), w.planet_id("Coruscant_WW").unwrap());
    sim.add_credits(empire, 1.0e9);
    sim.set_tech_level(empire, 4);
    sim.set_starbase(p, w.first_starbase[empire as usize].and_then(|b| w.starbases[b as usize].next));
    let g = sim.galaxy();
    let unit = |want: fn(&Result<(), galaxy_sim::CommandError>) -> bool| {
        let u = w.buildable_units[empire as usize]
            .iter()
            .copied()
            .find(|&u| w.units[u as usize].role == UnitRole::Ship && w.units[u as usize].limit_current.is_none() && want(&w.check_build_unit(g, empire, p, u)))
            .expect("a ship of that kind");
        w.units[u as usize].name.clone()
    };
    let structure = |orbital: bool| {
        let s = (0..w.structures.len() as u32).find(|&s| w.structures[s as usize].orbital == orbital && w.check_build_structure(g, empire, p, s).is_ok()).expect("a structure to order");
        w.structures[s as usize].name.clone()
    };
    let (ship, needs_more) = (unit(|r| r.is_ok()), unit(|r| matches!(r, Err(galaxy_sim::CommandError::RequirementsNotMet(_)))));
    let (orbital, ground) = (structure(true), structure(false));

    let (allies, mut pending) = (std::collections::HashSet::new(), Vec::new());
    let mut ai = galaxy_script::ai::SimAdapter { sim: &mut sim, world: w.clone(), human: rebel, allies: &allies, pending: &mut pending };
    assert_eq!(ai.can_build_unit(empire, p, &needs_more), Err(Refused::Unavailable));
    assert_eq!((ai.can_build_unit(empire, p, &ship), ai.can_build_structure(empire, p, &orbital), ai.can_upgrade_starbase(empire, p)), (Ok(()), Ok(()), Ok(())));
    for _ in 0..w.config.max_queue_len {
        ai.queue_unit(empire, p, &ship).expect("queued");
    }
    assert_eq!(ai.can_build_unit(empire, p, &ship), Err(Refused::Busy));
    assert_eq!(ai.queue_unit(empire, p, &ship), Err(Refused::Busy));
    assert_eq!(ai.can_build_structure(empire, p, &orbital), Err(Refused::Busy));
    assert_eq!(ai.queue_structure(empire, p, &orbital), Err(Refused::Busy));
    assert_eq!(ai.can_upgrade_starbase(empire, p), Err(Refused::Busy));
    assert_eq!(ai.can_build_unit(empire, p, &needs_more), Err(Refused::Unavailable), "a full queue does not make it buildable later");
    assert_eq!(ai.can_build_structure(empire, p, &ground), Ok(()), "the ground queue is empty");
    // A star base level that already waits is another answer: the planet is getting its level, there is nothing to wait for.
    ai.sim.apply(Command::CancelProduction { faction: empire, planet: p, lane: Lane::Space, index: 4 }).expect("cancel");
    ai.sim.apply(Command::CancelProduction { faction: empire, planet: p, lane: Lane::Space, index: 3 }).expect("cancel");
    ai.queue_starbase(empire, p).expect("star base level");
    assert_eq!(ai.can_upgrade_starbase(empire, p), Err(Refused::Unavailable));
}

/// From the Ground Up, where a great power starts with one or three planets and their queues fill up at once: a force whose ship, structure
/// or star base level finds every queue full waits for a slot. No plan fails because of a full queue, and none waits until its time is up.
#[test]
fn a_full_queue_makes_a_force_wait_and_does_not_fail_it() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipped: Remake mod not installed");
        return;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).unwrap());
    let data = Arc::new(load_galaxy(&*fs).0);
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap().clone();
    let mut sim = ScriptedSim::new(
        fs,
        data,
        &campaign,
        SimConfig { victory_enabled: false, ground_up: true, ..SimConfig::default() },
        ScriptedConfig { remake_ai: true, human: Some("none".into()), game_scoring: false, ..ScriptedConfig::default() },
    )
    .unwrap();
    let mut weeks = 0;
    while weeks < 30 {
        for e in sim.step(1.0).sim_events {
            if let Event::WeekEnded { week } = e {
                weeks = week;
            }
        }
    }
    let st = sim.ai_stats().expect("Remake AI is on");
    let tf = &st.taskforces;
    let full: Vec<_> = st.fast_failures.iter().filter(|((_, detail), _)| detail.contains("QueueFull")).collect();
    let timed_out: Vec<_> = tf.failure_reasons.iter().filter(|(why, _)| why.ends_with(": deadline") || why.ends_with(": timeout")).collect();
    let star_bases: Vec<_> = st.fast_failures.iter().filter(|((_, detail), _)| detail.contains("Star_Base")).map(|((_, detail), n)| (detail, n)).collect();
    println!("30 weeks from the ground up: {} ships and {} structures queued, {} production failures, {} waits for a full queue", tf.ships_queued, tf.structures_queued, tf.production_failures, tf.queue_waits);
    println!("star base plans that failed: {star_bases:?}");
    assert!(full.is_empty(), "plans failed over a full queue: {full:?}");
    assert!(timed_out.is_empty(), "produce jobs waited until their time was up: {timed_out:?}");
    assert!(tf.queue_waits > 0, "the queues of a one-planet start fill up, so forces must have waited");
}
