//! Long-running soak tests (`cargo test --release -p galaxy-play --test soak -- --ignored --nocapture`). The full matrix (campaigns, seeds,
//! tiers, tactical battles, memory and journal growth) is the `soak` example; these are the checks that must always hold.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_play::{PlayConfig, PlayInit, ResolveMode, SimDriver};

fn play(campaign: &str, seed: u64, weeks: u64, classic: bool) -> Option<(u64, Vec<String>)> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        return None;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
    let (data, _) = load_galaxy(&*fs);
    let data = Arc::new(data);
    let c = data.campaign(campaign)?.clone();
    let cfg = PlayConfig { sim: galaxy_sim::SimConfig { seed, victory_enabled: false, ..Default::default() }, classic_ai: classic, ..PlayConfig::default() };
    let init = PlayInit::scripted(data, fs, &c, cfg).ok()?;
    let human = init.sim.world().victory.active_player;
    let mut d = SimDriver::spawn_scripted(init.sim, init.script?, human);
    let mut last = d.wait_initial();
    let mut problems = Vec::new();
    while last.week < weeks {
        for _ in 0..60 {
            d.advance(1.0);
        }
        loop {
            let Some(s) = d.wait_idle().pop() else {
                problems.push("worker died".to_string());
                return Some((0, problems));
            };
            last = s;
            if last.human_battles.is_empty() {
                break;
            }
            for b in last.human_battles.clone() {
                d.resolve_battle(b.id, ResolveMode::Auto);
            }
        }
        for (i, f) in last.galaxy.factions.iter().enumerate() {
            if !f.credits.is_finite() || f.credits < -1.0 || f.credits > 5.0e7 {
                problems.push(format!("week {}: faction {i} credits {}", last.week, f.credits));
            }
        }
        if let Some(dg) = &last.diagnostics
            && dg.script_errors > 0
        {
            problems.push(format!("week {}: script errors {:?}", last.week, dg.last_script_error));
        }
    }
    Some((last.galaxy.state_hash(), problems))
}

#[test]
#[ignore = "long: about a minute"]
fn hundred_weeks_are_clean_and_deterministic() {
    let Some((h1, p1)) = play("GC_Full_Web_Way_Rebel", 1, 100, false) else { return };
    assert!(p1.is_empty(), "{p1:?}");
    let (h2, p2) = play("GC_Full_Web_Way_Rebel", 1, 100, false).unwrap();
    assert!(p2.is_empty(), "{p2:?}");
    assert_eq!(h1, h2, "the same game must end in the same state");
}

#[test]
#[ignore = "long"]
fn classic_ai_and_a_small_campaign_are_clean() {
    let Some((_, p)) = play("GC_Mini_Web_Way_Imperial", 2, 100, true) else { return };
    assert!(p.is_empty(), "{p:?}");
}
