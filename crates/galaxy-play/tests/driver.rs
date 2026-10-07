//! Tests of the threaded driver against the real Remake data; skipped when the game is missing.

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use galaxy_play::{BuildMenu, BuildTarget, PlayCatalog, PlayConfig, ResolveMode, SimDriver, build_command};
use galaxy_sim::{Command, CommandError, Event, FactionId, Sim, SimConfig};

fn load() -> Option<GameData> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    Some(load_galaxy(&fs).0)
}

fn new_sim(data: &GameData, seed: u64) -> Sim {
    let cfg = PlayConfig { sim: SimConfig { seed, victory_enabled: false, ..SimConfig::default() }, ..PlayConfig::default() };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    Sim::new(data, campaign, cfg.sim_config()).expect("sim")
}

#[derive(Clone)]
enum Op {
    Cmd(Command),
    Advance(f32),
}

/// What the driver is specified to do, written directly against `Sim`.
fn reference(mut sim: Sim, human: Option<FactionId>, ops: &[Op]) -> (u64, Vec<Event>) {
    let ai: Vec<FactionId> = (0..sim.world().factions.len() as FactionId).filter(|&f| Some(f) != human).collect();
    sim.set_ai_factions(ai);
    let mut events = Vec::new();
    let held = |sim: &Sim| sim.pending_battles().iter().any(|b| human.is_some_and(|h| b.factions.contains(&h)));
    for op in ops {
        match op {
            Op::Cmd(c) => {
                let _ = sim.apply(c.clone());
            }
            Op::Advance(dt) => {
                if held(&sim) {
                    continue;
                }
                let mut remaining = *dt;
                while remaining > 0.0 {
                    let d = remaining.min(sim.world().config.tick_seconds);
                    remaining -= d;
                    events.extend(sim.step(d));
                    let ids: Vec<u32> =
                        sim.pending_battles().iter().filter(|b| !human.is_some_and(|h| b.factions.contains(&h))).map(|b| b.id).collect();
                    for id in ids {
                        events.extend(sim.resolve_battle(id).unwrap_or_default());
                    }
                    if held(&sim) {
                        break;
                    }
                }
            }
        }
    }
    (sim.galaxy().state_hash(), events)
}

fn script(sim: &Sim, human: FactionId) -> Vec<Op> {
    let w = sim.world();
    let g = sim.galaxy();
    let names = PlayCatalog::plain(w);
    let home = w.factions[human as usize].home.expect("home planet");
    let menu = BuildMenu::for_planet(w, g, &names, human, home).expect("menu");
    let mut ops = vec![Op::Advance(5.0)];
    for o in menu.options.iter().filter(|o| o.availability.is_available() && matches!(o.target, BuildTarget::Unit(_))).take(3) {
        ops.push(Op::Cmd(build_command(human, home, o.target)));
        ops.push(Op::Advance(0.25));
    }
    let fleet = g.fleets.values().find(|f| f.owner == human && f.planet().is_some()).expect("fleet");
    // A quiet destination, so the human is not held in a battle and time keeps flowing.
    let to = w.adjacency[fleet.planet().unwrap() as usize]
        .iter()
        .map(|&(n, _)| n)
        .find(|&n| {
            let ps = &g.planets[n as usize];
            ps.owner.is_none_or(|o| o == human) && ps.starbase.is_none() && g.fleets_at(n).all(|f| f.owner == human)
        })
        .expect("quiet neighbour");
    ops.push(Op::Cmd(Command::MoveFleet { faction: human, fleet: fleet.id, to }));
    // A refused command must not change anything either way.
    ops.push(Op::Cmd(Command::BuildUnit { faction: human, planet: 99_999, unit: 0 }));
    for _ in 0..40 {
        ops.push(Op::Advance(2.25));
    }
    ops.push(Op::Advance(60.0));
    ops
}

#[test]
fn driver_matches_direct_sim() {
    let Some(data) = load() else { return };
    let human = new_sim(&data, 3).world().victory.active_player;
    let human_id = human.expect("active player");
    let ops = script(&new_sim(&data, 3), human_id);

    let (want_hash, want_events) = reference(new_sim(&data, 3), human, &ops);

    let mut driver = SimDriver::spawn(new_sim(&data, 3), human);
    let first = driver.wait_initial();
    assert_eq!(first.seq, 0);
    for op in &ops {
        match op {
            Op::Cmd(c) => {
                driver.send_command(c.clone());
            }
            Op::Advance(dt) => driver.advance(*dt),
        }
    }
    let snaps = driver.wait_idle();
    let last = snaps.last().expect("snapshots");
    assert_eq!(last.galaxy.state_hash(), want_hash, "driver state differs from the direct simulation");
    let got_events: Vec<Event> = snaps.iter().flat_map(|s| s.events.clone()).collect();
    assert_eq!(got_events, want_events);
    // Results: the three builds and the move succeeded, the bogus build was refused.
    let results: Vec<_> = snaps.iter().flat_map(|s| s.results.clone()).collect();
    assert_eq!(results.iter().filter(|r| r.result.is_ok()).count(), 4);
    assert_eq!(results.iter().filter(|r| r.result == Err(CommandError::Invalid)).count(), 1);
    assert!(results.windows(2).all(|w| w[0].ticket < w[1].ticket));
    assert!(last.time >= 155.0, "time stopped at {}", last.time);
    println!("final time {} week {} held {} batch ms {:?}", last.time, last.week, last.human_battles.len(), snaps.iter().map(|s| s.work_micros / 1000).collect::<Vec<_>>());
}

#[test]
fn driver_is_repeatable_across_batching() {
    let Some(data) = load() else { return };
    let human = new_sim(&data, 5).world().victory.active_player;
    // Same advances, delivered in one big request versus many small ones.
    let run = |chunks: usize| {
        let mut d = SimDriver::spawn(new_sim(&data, 5), human);
        d.wait_initial();
        for _ in 0..chunks {
            d.advance(600.0 / chunks as f32);
        }
        d.wait_idle().last().expect("snapshot").galaxy.state_hash()
    };
    assert_eq!(run(1), run(8));
    assert_eq!(run(8), run(24));
}

#[test]
fn human_battle_blocks_time_until_resolved() {
    let Some(data) = load() else { return };
    let sim = new_sim(&data, 11);
    let w = sim.world().clone();
    let human = w.victory.active_player.expect("active player");
    // Send the biggest human fleet at the strongest enemy base.
    let fleet = sim.galaxy().fleets.values().filter(|f| f.owner == human && f.planet().is_some()).max_by_key(|f| f.unit_count()).expect("fleet").id;
    let target = (0..w.planets.len() as u32)
        .find(|&p| {
            let ps = &sim.galaxy().planets[p as usize];
            ps.starbase.is_some() && ps.owner.is_some_and(|o| o != human) && w.real[p as usize]
        })
        .expect("enemy base");
    let mut driver = SimDriver::spawn(sim, Some(human));
    driver.wait_initial();
    driver.send_command(Command::MoveFleet { faction: human, fleet, to: target });
    let mut battle = None;
    for _ in 0..60 {
        driver.advance(60.0);
        let snaps = driver.wait_idle();
        let last = snaps.last().unwrap();
        if let Some(b) = last.human_battles.first() {
            battle = Some((b.clone(), last.time));
            break;
        }
    }
    let (battle, time) = battle.expect("a battle involving the human should occur");
    // Time is frozen while the decision is open.
    driver.advance(120.0);
    let frozen = driver.wait_idle();
    assert_eq!(frozen.last().unwrap().time, time);
    assert_eq!(frozen.last().unwrap().human_battles.len(), 1);
    // Resolving frees the game again.
    driver.resolve_battle(battle.id, ResolveMode::Auto);
    let resolved = driver.wait_idle();
    let last = resolved.last().unwrap();
    assert!(last.events.iter().any(|e| matches!(e, Event::BattleResolved { id, .. } if *id == battle.id)));
    assert!(last.human_battles.iter().all(|b| b.id != battle.id));
    driver.advance(10.0);
    let after = driver.wait_idle();
    assert!(after.last().unwrap().time > time);
}

#[test]
fn all_ai_galaxy_runs_and_reports_weeks() {
    let Some(data) = load() else { return };
    let mut driver = SimDriver::spawn(new_sim(&data, 1), None);
    driver.wait_initial();
    driver.advance(60.0 * 6.0);
    let snaps = driver.wait_idle();
    let last = snaps.last().unwrap();
    assert_eq!(last.week, 6);
    assert!(last.human_battles.is_empty());
    let weeks: Vec<u64> = snaps.iter().flat_map(|s| s.events.iter()).filter_map(|e| if let Event::WeekEnded { week } = e { Some(*week) } else { None }).collect();
    assert_eq!(weeks, (1..=6).collect::<Vec<_>>());
}



