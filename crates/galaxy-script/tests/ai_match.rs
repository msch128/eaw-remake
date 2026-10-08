//! AI-vs-AI scripted match with the Remake galactic AI; skipped when the game or the mod is not installed.

use std::sync::Arc;
use std::time::Instant;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::{Event, SimConfig};

fn build(remake_ai: bool) -> Option<ScriptedSim> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipped: Remake mod not installed");
        return None;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
    let (data, _) = load_galaxy(&*fs);
    let data = Arc::new(data);
    let campaign = data.campaign("GC_Full_Web_Way_Rebel")?.clone();
    let sim = ScriptedSim::new(
        fs,
        data,
        &campaign,
        SimConfig { victory_enabled: false, ..SimConfig::default() },
        // Nobody is human: every faction (the campaign's own Rebel included) is played by the AI.
        ScriptedConfig { remake_ai, human: Some("none".into()), game_scoring: false, ..ScriptedConfig::default() },
    )
    .ok()?;
    if !remake_ai {
        sim.with_sim_mut(|s| s.set_ai_factions(0..s.world().factions.len() as u32));
    }
    Some(sim)
}

#[test]
fn remake_ai_builds_moves_and_attacks() {
    let Some(mut sim) = build(true) else { return };
    assert!(sim.start_errors.is_empty(), "{:?}", sim.start_errors);
    let names: Vec<String> = sim.with_sim(|s| s.world().factions.iter().map(|f| f.name.clone()).collect());
    let ships = |sim: &ScriptedSim| sim.with_sim(|s| s.galaxy().fleets.values().map(|f| f.unit_count()).sum::<u32>());
    let ships_at_start = ships(&sim);
    let (mut weeks, mut captures, mut enemy_captures, mut battles, mut week_ms) = (0, 0, 0, 0, Vec::new());
    let mut t = Instant::now();
    while weeks < 20 {
        for e in sim.step(1.0).sim_events {
            match e {
                Event::WeekEnded { week } => {
                    weeks = week;
                    week_ms.push(t.elapsed().as_secs_f64() * 1e3);
                    t = Instant::now();
                }
                Event::PlanetCaptured { old_owner, .. } => {
                    captures += 1;
                    enemy_captures += u32::from(old_owner.is_some());
                }
                Event::BattlePending(_) | Event::BattleResolved { .. } => battles += 1,
                _ => {}
            }
        }
    }
    let st = sim.ai_stats().expect("Remake AI is on");
    let tf = &st.taskforces;
    week_ms.sort_by(f64::total_cmp);
    println!(
        "20 weeks: ships {ships_at_start} -> {}, queued {} ships / {} structures, free-store claims {}, plans {}, captures {captures} ({enemy_captures} from enemy owners), battle events {battles}",
        ships(&sim),
        tf.ships_queued,
        tf.structures_queued,
        tf.free_store_claims,
        st.plans_started.values().sum::<u32>()
    );
    println!("wall time per week: median {:.0} ms, worst {:.0} ms (sim {:?}, scripts {:?}, ai {:?} in total)", week_ms[week_ms.len() / 2], week_ms[week_ms.len() - 1], sim.timing.sim, sim.timing.scripts, sim.timing.ai);

    assert!(tf.ships_queued > 20, "the AI should build ships");
    assert!(tf.structures_queued > 20, "the AI should build structures");
    assert!(captures > 0, "the AI should conquer planets");
    assert!(enemy_captures >= 1, "the AI should take planets from enemy owners");
    // Every faction, the campaign's own Rebel included, started plans.
    let rebel = names.iter().position(|n| n == "Rebel").unwrap() as u32;
    assert!(st.plans_started.keys().any(|(p, _)| *p == rebel), "Rebel AI never acted");
    assert!(sim.runtime().errors().is_empty());
}

#[test]
fn remake_ai_is_the_default_and_the_stub_stays_available() {
    let Some(sim) = build(false) else { return };
    assert!(sim.ai_stats().is_none(), "the stub is available with remake_ai: false");
    let d = ScriptedConfig::default();
    assert!(d.remake_ai && d.strategy && d.difficulty == Some(galaxy_script::DifficultyTier::Normal));
}

struct RunStats {
    enemy_captures: u32,
    max_gain_week10: i64,
    credits_week10: f64,
    week_ms: f64,
    strategy_ms_per_week: f64,
}

fn run_tier(tier: galaxy_script::DifficultyTier, strategy: bool, weeks_total: u64, seed: u64) -> Option<RunStats> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        return None;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
    let (data, _) = load_galaxy(&*fs);
    let data = Arc::new(data);
    let campaign = data.campaign("GC_Full_Web_Way_Rebel")?.clone();
    let mut sim = ScriptedSim::new(
        fs,
        data,
        &campaign,
        SimConfig { victory_enabled: false, seed, ..SimConfig::default() },
        ScriptedConfig { remake_ai: true, human: Some("none".into()), game_scoring: false, strategy, difficulty: Some(tier), ..ScriptedConfig::default() },
    )
    .ok()?;
    let n = sim.with_sim(|s| s.world().factions.len());
    let pirates = sim.with_sim(|s| s.world().faction_id("Pirates"));
    let owned = |sim: &ScriptedSim| -> Vec<i64> { sim.with_sim(|s| (0..n as u32).map(|f| s.galaxy().factions[f as usize].planets_owned as i64).collect()) };
    let start = owned(&sim);
    let (mut weeks, mut enemy_captures, mut max_gain, mut credits10) = (0, 0u32, 0i64, 0.0);
    let t0 = Instant::now();
    while weeks < weeks_total {
        for e in sim.step(1.0).sim_events {
            match e {
                Event::WeekEnded { week } => {
                    weeks = week;
                    if week == 10 {
                        let now = owned(&sim);
                        max_gain = (0..n).filter(|&f| Some(f as u32) != pirates).map(|f| now[f] - start[f]).max().unwrap_or(0);
                        credits10 = sim.with_sim(|s| s.galaxy().factions.iter().map(|f| f.credits).sum());
                    }
                }
                Event::PlanetCaptured { old_owner: Some(o), .. } if Some(o) != pirates => enemy_captures += 1,
                _ => {}
            }
        }
    }
    let strategy_ms = sim.ai_stats().map_or(0.0, |s| s.strategy_nanos as f64 / 1e6) / weeks as f64;
    Some(RunStats { enemy_captures, max_gain_week10: max_gain, credits_week10: credits10, week_ms: t0.elapsed().as_secs_f64() * 1e3 / weeks as f64, strategy_ms_per_week: strategy_ms })
}

#[test]
fn difficulty_tiers_scale_without_snowballing() {
    use galaxy_script::DifficultyTier::*;
    let Some(baseline) = run_tier(Normal, false, 30, 1) else { return };
    let mut credits = Vec::new();
    println!("normal without strategy: {} enemy captures, {:.0} ms/week", baseline.enemy_captures, baseline.week_ms);
    for (name, tier) in [("easy", Easy), ("normal", Normal), ("hard", Hard), ("brutal", Brutal), ("fair", Fair)] {
        let r = run_tier(tier, true, 30, 1).unwrap();
        println!(
            "{name:<7} {:>2} enemy captures, best faction +{} planets by week 10, treasury sum {:.0}k at week 10, {:.0} ms/week wall, strategy layer {:.1} ms/week",
            r.enemy_captures,
            r.max_gain_week10,
            r.credits_week10 / 1e3,
            r.week_ms,
            r.strategy_ms_per_week
        );
        assert!(r.max_gain_week10 <= 12, "{name}: a faction snowballed (+{} planets in 10 weeks)", r.max_gain_week10);
        assert!(r.strategy_ms_per_week < 30.0);
        if matches!(name, "easy" | "normal" | "hard" | "brutal") {
            credits.push(r.credits_week10);
        }
    }
    assert!(credits.windows(2).all(|w| w[1] > w[0]), "treasuries should grow with the tier: {credits:?}");
}

#[test]
fn strategic_layer_captures_more_enemy_planets_over_several_seeds() {
    use galaxy_script::DifficultyTier::Normal;
    let (mut with, mut without) = (0, 0);
    for seed in 1..=4 {
        let Some(a) = run_tier(Normal, true, 30, seed) else { return };
        let b = run_tier(Normal, false, 30, seed).unwrap();
        println!("seed {seed}: strategy {} vs plain {} enemy captures (best faction +{} / +{} planets by week 10)", a.enemy_captures, b.enemy_captures, a.max_gain_week10, b.max_gain_week10);
        with += a.enemy_captures;
        without += b.enemy_captures;
    }
    assert!(with > without, "{with} vs {without}");
}

#[test]
fn remake_ai_with_strategy_is_deterministic() {
    use galaxy_script::DifficultyTier::Normal;
    let hash = || -> Option<(u64, u32)> {
        let install = GameInstall::detect()?;
        if !install.mod_data("1770851727").is_dir() {
            return None;
        }
        let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
        let (data, _) = load_galaxy(&*fs);
        let data = Arc::new(data);
        let campaign = data.campaign("GC_Full_Web_Way_Rebel")?.clone();
        let mut sim = ScriptedSim::new(fs, data, &campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }, ScriptedConfig { remake_ai: true, human: Some("none".into()), game_scoring: false, difficulty: Some(Normal), ..ScriptedConfig::default() }).ok()?;
        let mut weeks = 0;
        while weeks < 12 {
            for e in sim.step(1.0).sim_events {
                if let Event::WeekEnded { week } = e {
                    weeks = week;
                }
            }
        }
        Some((sim.with_sim(|s| s.galaxy().state_hash()), sim.ai_stats()?.taskforces.ships_queued))
    };
    let Some(a) = hash() else { return };
    assert_eq!(a, hash().unwrap(), "same seed must give the same match");
}

#[test]
fn the_ai_spends_its_money_and_builds_star_bases() {
    let Some(mut sim) = build(true) else { return };
    let mut weeks = 0;
    while weeks < 15 {
        for e in sim.step(1.0).sim_events {
            if let Event::WeekEnded { week } = e {
                weeks = week;
            }
        }
    }
    sim.with_sim(|s| {
        let (w, g) = (s.world(), s.galaxy());
        for name in ["Rebel", "Empire", "Underworld"] {
            let f = w.faction_id(name).unwrap() as usize;
            let st = &g.factions[f].stats;
            assert!(st.credits_spent > 0.6 * st.credits_earned, "{name} sits on its money: earned {:.0}, spent {:.0}", st.credits_earned, st.credits_spent);
            let bases = g.planets.iter().filter(|p| p.owner == Some(f as u32) && p.starbase.is_some()).count();
            assert!(bases >= 2, "{name} built {bases} star bases");
        }
    });
}
