//! The worker on the scripted backend (Remake scripts, Remake AI, save/load) against the real data; skipped without the game.

use std::sync::Arc;
use std::time::{Duration, Instant};

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_play::{PlayConfig, PlayInit, SimDriver};

struct Ctx {
    fs: Arc<dyn FileSource>,
    data: Arc<GameData>,
}

fn ctx() -> Option<Ctx> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
    let (data, _) = load_galaxy(&*fs);
    Some(Ctx { fs, data: Arc::new(data) })
}

fn config(seed: u64, classic: bool) -> PlayConfig {
    PlayConfig { sim: galaxy_sim::SimConfig { seed, victory_enabled: false, ..Default::default() }, classic_ai: classic, ..PlayConfig::default() }
}

fn start(c: &Ctx, cfg: PlayConfig) -> SimDriver {
    let campaign = c.data.campaign("GC_Full_Web_Way_Rebel").unwrap().clone();
    let init = PlayInit::scripted(c.data.clone(), c.fs.clone(), &campaign, cfg).unwrap();
    spawn(init)
}

fn spawn(init: PlayInit) -> SimDriver {
    let script = init.script.expect("scripted init");
    let human = match script.config.human.as_deref() {
        Some(n) if !n.eq_ignore_ascii_case("none") => init.sim.world().faction_index.get(&n.to_ascii_lowercase()).copied(),
        _ => None,
    };
    SimDriver::spawn_scripted(init.sim, script, human)
}

/// Advances whole game weeks in 1 s requests (a multiple of the 1 s tick, so batching cannot change the step sequence).
fn run_weeks(d: &mut SimDriver, weeks: u64) -> Arc<galaxy_play::Snapshot> {
    for _ in 0..weeks * 60 {
        d.advance(1.0);
    }
    d.wait_idle().pop().expect("a snapshot")
}

#[test]
fn scripted_worker_runs_the_ai_without_stalling_the_main_thread() {
    let Some(c) = ctx() else { return };
    let mut d = start(&c, config(1, false));
    let initial = d.wait_initial();
    assert!(!initial.loading);
    let start_ships: u32 = initial.galaxy.fleets.values().filter(|f| Some(f.owner) != d.human()).map(|f| f.unit_count()).sum();

    // A render loop at 60 fps and 16x game speed (the fastest level): 0.27 game seconds per frame, never blocking on the worker.
    let frame = 16.0 / 60.0;
    let t0 = Instant::now();
    let (mut last, mut worst_batch, mut worst_poll, mut work_total) = (initial.clone(), 0u64, Duration::ZERO, 0u64);
    while last.week < 20 {
        if d.advances_in_flight(&last) < 3 {
            d.advance(frame);
        }
        let p = Instant::now();
        let snaps = d.poll();
        worst_poll = worst_poll.max(p.elapsed());
        for s in snaps {
            worst_batch = worst_batch.max(s.work_micros);
            work_total += s.work_micros;
            last = s;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let wall = t0.elapsed();
    println!(
        "20 weeks at 16x: worker {:.0} ms per game week on average, worst batch {:.0} ms, worst poll on the main thread {:?}, {:.1} s wall",
        work_total as f64 / 1000.0 / 20.0,
        worst_batch as f64 / 1000.0,
        worst_poll,
        wall.as_secs_f64()
    );
    let ai_ships: u32 = last.galaxy.fleets.values().filter(|f| Some(f.owner) != d.human()).map(|f| f.unit_count()).sum();
    assert!(ai_ships > start_ships, "the AI should build ships ({start_ships} -> {ai_ships})");
    assert!(worst_poll < Duration::from_millis(20), "polling must never stall the render thread: {worst_poll:?}");
    assert!(worst_batch < 600_000, "worst batch {worst_batch} us");
}

#[test]
fn save_and_load_through_the_driver_continue_identically() {
    let Some(c) = ctx() else { return };
    let path = std::env::temp_dir().join("galaxy_play_test_save.json");
    let mut d = start(&c, config(2, false));
    d.wait_initial();
    run_weeks(&mut d, 5);
    d.save(&path);
    let snaps = d.wait_idle();
    let saved = snaps.iter().flat_map(|s| &s.saves).next().expect("save result");
    assert_eq!(saved.result, Ok(()));
    let h5 = snaps.last().unwrap().galaxy.state_hash();
    let h8 = run_weeks(&mut d, 3).galaxy.state_hash();
    assert_ne!(h5, h8);

    // Load into the running game: back at week 5, same continuation.
    d.load(&path);
    let snaps = d.wait_idle();
    let load = snaps.iter().flat_map(|s| &s.loads).next().expect("load result");
    assert_eq!(load.result, Ok(5));
    assert_eq!(snaps.last().unwrap().galaxy.state_hash(), h5);
    assert_eq!(run_weeks(&mut d, 3).galaxy.state_hash(), h8, "a loaded game continues exactly like the original");

    // A new process starting from the save (PlayInit::from_save) sees progress and ends at the same state.
    let init = PlayInit::from_save(&path, c.data.clone(), c.fs.clone(), PlayConfig::default()).unwrap();
    let mut d2 = spawn(init);
    let mut seen = Vec::new();
    let t = Instant::now();
    while let Some(p) = d2.load_progress() {
        seen.push(p.fraction());
        assert!(t.elapsed() < Duration::from_secs(120));
        std::thread::sleep(Duration::from_millis(5));
    }
    let first = d2.wait_initial();
    assert_eq!(first.loads.first().map(|l| l.result.clone()), Some(Ok(5)));
    assert_eq!(first.galaxy.state_hash(), h5);
    assert!(seen.windows(2).all(|w| w[1] >= w[0]) && !seen.is_empty(), "progress is monotone: {seen:?}");
    println!("load progress samples: {} (last {:.2})", seen.len(), seen.last().copied().unwrap_or(1.0));
    assert_eq!(run_weeks(&mut d2, 3).galaxy.state_hash(), h8);

    // A save of the same campaign and player loads even into a game with other AI settings (the save's settings win).
    let mut classic = start(&c, config(2, true));
    classic.wait_initial();
    classic.load(&path);
    let snaps = classic.wait_idle();
    assert_eq!(snaps.iter().flat_map(|s| &s.loads).next().map(|l| l.result.clone()), Some(Ok(5)));
    assert_eq!(snaps.last().unwrap().galaxy.state_hash(), h5);
}

#[test]
fn classic_ai_games_can_be_saved_too() {
    let Some(c) = ctx() else { return };
    let path = std::env::temp_dir().join("galaxy_play_test_classic.json");
    let mut d = start(&c, config(3, true));
    d.wait_initial();
    run_weeks(&mut d, 4);
    d.save(&path);
    let h4 = d.wait_idle().last().unwrap().galaxy.state_hash();
    let h6 = run_weeks(&mut d, 2).galaxy.state_hash();
    d.load(&path);
    let snaps = d.wait_idle();
    assert_eq!(snaps.last().unwrap().galaxy.state_hash(), h4);
    assert_eq!(run_weeks(&mut d, 2).galaxy.state_hash(), h6);
}

#[test]
fn plain_backend_reports_that_it_cannot_save() {
    let Some(c) = ctx() else { return };
    let campaign = c.data.campaign("GC_Full_Web_Way_Rebel").unwrap().clone();
    let init = PlayInit::from_data(&c.data, &campaign, config(1, false)).unwrap();
    let mut d = SimDriver::spawn(init.sim, None);
    d.wait_initial();
    d.save(std::env::temp_dir().join("never.json"));
    let snaps = d.wait_idle();
    assert!(snaps.iter().flat_map(|s| &s.saves).any(|s| s.result.is_err()));
}
