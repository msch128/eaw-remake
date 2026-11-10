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
