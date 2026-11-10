//! Forty weeks of the real campaign plots. The Devastator and 181st rewards wait on a construct
//! (`Black_Squadron_Upgrade`, `181ST_UPGRADE_LN_TO_IN`, `181ST_UPGRADE_IN_TO_D`) and must not fire
//! just because the plot loaded. Skips without the game and mod.

use std::collections::BTreeMap;
use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::{Event, SimConfig};

const GATED: &[&str] = &[
    "Devastator_Upgrade",
    "Spawn_Devastator",
    "181_Interceptor_Upgrade",
    "Spawn_181_Interceptor",
    "181_Defender_Upgrade",
    "Spawn_181_Defender",
];

struct Stack {
    fs: Arc<dyn FileSource>,
    data: Arc<alamo_data::GameData>,
}

fn stack(install: &GameInstall, id: &str) -> Option<Stack> {
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(install, Some(id)).ok()?);
    let data = Arc::new(load_galaxy(&*fs).0);
    Some(Stack { fs, data })
}

fn play(stack: &Stack, campaign_name: &str, weeks: u64) {
    let campaign = stack.data.campaign(campaign_name).unwrap_or_else(|| panic!("no {campaign_name}")).clone();
    let mut sim = ScriptedSim::new(
        stack.fs.clone(),
        stack.data.clone(),
        &campaign,
        SimConfig { victory_enabled: false, seed: 1, ..SimConfig::default() },
        ScriptedConfig { variety: 0.0, game_scoring: false, ..ScriptedConfig::default() },
    )
    .unwrap_or_else(|e| panic!("{campaign_name}: {e}"));
    assert!(sim.start_errors.is_empty(), "{campaign_name} start: {:?}", sim.start_errors);
    let mut week = 0u64;
    while week < weeks {
        for e in sim.step(1.0).sim_events {
            if let Event::WeekEnded { week: w } = e {
                week = w;
            }
        }
    }
    let errors: Vec<_> = sim.runtime().errors().iter().map(|e| format!("{}: {}", e.script, e.message)).collect();
    assert!(errors.is_empty(), "{campaign_name}: {errors:?}");
    let log = sim.host_stats().reward_log;
    let mut by_kind: BTreeMap<&str, u32> = BTreeMap::new();
    let mut distinct: BTreeMap<(bool, String, String, String, String), (u64, u64, u32)> = BTreeMap::new();
    let mut gated = Vec::new();
    for f in &log {
        *by_kind.entry(f.reward.kind.as_str()).or_default() += 1;
        let key = (
            f.applied,
            f.reward.kind.clone(),
            f.reward.params.join("|"),
            f.reward.event.clone(),
            f.reward.faction.clone().unwrap_or_default(),
        );
        let slot = distinct.entry(key).or_insert((f.week, f.week, 0));
        slot.0 = slot.0.min(f.week);
        slot.1 = slot.1.max(f.week);
        slot.2 += 1;
        if GATED.iter().any(|n| f.reward.event.eq_ignore_ascii_case(n)) {
            gated.push(format!("week {} {} {} event {}", f.week, f.reward.kind, f.reward.params.join("|"), f.reward.event));
        }
    }
    eprintln!("== {campaign_name} week {week}: {} fires", log.len());
    for (k, n) in &by_kind {
        eprintln!("  {k} {n}");
    }
    eprintln!("-- other than build-menu lock and unlock");
    for ((applied, kind, params, event, faction), (first, last, n)) in &distinct {
        if kind == "BUILDABLE_UNIT" || kind == "LOCK_UNIT" {
            continue;
        }
        let mark = if *applied { "ok" } else { "IGNORED" };
        eprintln!("  {n:>5} w{first:>2}-{last:<2} {mark:<7} {kind} [{params}] {event} ({faction})");
    }
    assert!(gated.is_empty(), "{campaign_name}: construct gates fired: {gated:?}");
    assert!(week >= weeks, "{campaign_name} stopped at week {week}");
}

#[test]
fn forty_weeks_keep_the_devastator_and_181st_gates_shut() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return;
    }
    let Some(base) = stack(&install, "1770851727") else { return };
    for name in ["GC_Full_Web_Way_Imperial", "GC_Full_Web_Way_Rebel", "GC_Full_Web_Way_Underworld"] {
        play(&base, name, 40);
    }
    if ["3229239424", "2794270450", "1770851727"].iter().all(|id| install.mod_data(id).is_dir())
        && let Some(fft) = stack(&install, "fft")
    {
        play(&fft, "GC_Full_Web_Way_Shatterpoint_Republic", 40);
    } else {
        eprintln!("skipping Clone Wars stack");
    }
}
