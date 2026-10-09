//! Formations and idle behaviour: idle capitals hold position and facing, squadrons keep their formation, hulls do not overlap.
//! The metric (`measure`) is the navcheck of this crate: idle path length per minute, squadron spread, closest hull approach.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use battle_sim::*;

fn real_data() -> Option<(alamo_data::GameData, GameFs)> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    let data = load_galaxy(&fs).0;
    Some((data, fs))
}

#[derive(Debug, Default)]
struct Metric {
    /// Distance flown by every idle capital in the measured time, per minute, worst ship.
    capital_path_per_min: f32,
    /// Largest heading change of a capital in degrees.
    capital_turn_deg: f32,
    /// Largest distance of a fighter from its formation slot (leader position plus offset), in units, over the run (after settling).
    squadron_spread: f32,
    /// Distance flown by the busiest fighter after settling (first 10 s), per minute.
    fighter_path_per_min: f32,
    /// Closest approach of two capitals as a share of the keep-out distance.
    hull_approach: f32,
}

fn measure(idle_secs: f32, with_move: bool) -> Option<Metric> {
    let (data, fs) = real_data()?;
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Star_Destroyer", "Victory_Destroyer", "TIE_Fighter_Squadron", "MC-80_Star_Cruiser"]));
    let mk = |name: &str, units: &[(&str, u32)]| SideSetup { name: name.into(), units: units.iter().map(|&(n, c)| UnitSetup { ship: cat.ship_id(n).unwrap(), count: c, tag: 1 }).collect(), structures: vec![], ai: false };
    let sides = vec![mk("Empire", &[("Star_Destroyer", 2), ("Victory_Destroyer", 2), ("TIE_Fighter_Squadron", 3)]), mk("Rebel", &[("MC-80_Star_Cruiser", 1)])];
    let config = BattleConfig { record_events: false, seed: 1, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config });
    // The enemy is far away: nothing to engage.
    for s in b.ships.iter_mut().filter(|s| s.side == 1) {
        s.pos = Vec3::new(s.pos.x + 60000.0, s.pos.y, s.pos.z);
    }
    let mine: Vec<u32> = b.ships.iter().filter(|s| s.side == 0).map(|s| s.id).collect();
    let capitals: Vec<u32> = mine.iter().copied().filter(|&i| b.ships[i as usize].squad == u32::MAX).collect();
    let mut m = Metric { hull_approach: f32::MAX, ..Metric::default() };
    for _ in 0..(5.0 * 30.0) as usize {
        b.step();
    }
    if with_move {
        for &i in &capitals {
            let p = b.ships[i as usize].pos + Vec3::new(3000.0, 0.0, 0.0);
            b.command(0, Command::Move { ships: vec![i], to: p }).unwrap();
        }
    }
    let start: Vec<(Vec3, f32)> = capitals.iter().map(|&i| (b.ships[i as usize].pos, b.ships[i as usize].yaw)).collect();
    let mut path = vec![0.0f32; capitals.len()];
    let fighters: Vec<u32> = mine.iter().copied().filter(|&i| b.ships[i as usize].squad != u32::MAX).collect();
    let mut fpath = vec![0.0f32; fighters.len()];
    let mut flast: Vec<Vec3> = fighters.iter().map(|&i| b.ships[i as usize].pos).collect();
    let mut last: Vec<Vec3> = start.iter().map(|s| s.0).collect();
    let steps = (idle_secs * 30.0) as usize;
    for t in 0..steps {
        b.step();
        for (k, &i) in capitals.iter().enumerate() {
            let p = b.ships[i as usize].pos;
            path[k] += p.dist(last[k]);
            last[k] = p;
            let dyaw = (b.ships[i as usize].yaw - start[k].1 + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            m.capital_turn_deg = m.capital_turn_deg.max(dyaw.abs().to_degrees());
        }
        for (a, &i) in capitals.iter().enumerate() {
            for &j in &capitals[a + 1..] {
                let (x, y) = (&b.ships[i as usize], &b.ships[j as usize]);
                let r = |s: &Ship| cat.ship(s.def).radius * 0.75;
                m.hull_approach = m.hull_approach.min(x.pos.dist(y.pos) / ((r(x) + r(y)) * 0.85));
            }
        }
        for (k, &i) in fighters.iter().enumerate() {
            let p = b.ships[i as usize].pos;
            if t > 10 * 30 {
                fpath[k] += p.dist(flast[k]);
            }
            flast[k] = p;
        }
        if t > 10 * 30 {
            for s in b.ships.iter().filter(|s| s.side == 0 && s.squad != u32::MAX && s.alive) {
                let leader = b.ships.iter().filter(|o| o.squad == s.squad && o.alive).map(|o| o.id).min().unwrap();
                if leader != s.id {
                    let l = &b.ships[leader as usize];
                    let slot = l.pos + s.formation.rotate_yaw(l.yaw);
                    m.squadron_spread = m.squadron_spread.max(s.pos.dist(slot));
                }
            }
        }
    }
    m.fighter_path_per_min = fpath.iter().copied().fold(0.0, f32::max) * 60.0 / (idle_secs - 10.0).max(1.0);
    m.capital_path_per_min = path.iter().copied().fold(0.0, f32::max) * 60.0 / idle_secs;
    Some(m)
}

#[test]
fn idle_fleets_hold_position_and_squadrons_keep_their_slots() {
    let Some(m) = measure(60.0, false) else { return };
    println!("idle fleet: {m:?}");
    assert!(m.capital_path_per_min < 1.0, "idle capitals hold position: {m:?}");
    assert!(m.capital_turn_deg < 1.0, "idle capitals hold their facing: {m:?}");
    assert!(m.squadron_spread < 100.0, "idle squadrons stay in formation: {m:?}");
    assert!(m.fighter_path_per_min < 1000.0, "idle fighters hover instead of circling: {m:?}");
    assert!(m.hull_approach >= 0.9, "no hull overlaps: {m:?}");
}

#[test]
fn a_moving_fleet_keeps_its_squadrons_together_and_its_hulls_apart() {
    let Some(m) = measure(30.0, true) else { return };
    println!("moving fleet: {m:?}");
    assert!(m.squadron_spread < 250.0, "squadrons stay together while the carriers move: {m:?}");
    assert!(m.hull_approach >= 0.9, "no hull overlaps: {m:?}");
}

#[test]
fn a_squadron_ordered_to_move_flies_there_in_formation() {
    let Some((data, fs)) = real_data() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["X-Wing_Squadron", "TIE_Fighter_Squadron"]));
    let mk = |name: &str, n: &str| SideSetup { name: name.into(), units: vec![UnitSetup { ship: cat.ship_id(n).unwrap(), count: 2, tag: 1 }], structures: vec![], ai: false };
    let config = BattleConfig { record_events: false, seed: 1, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![mk("Rebel", "X-Wing_Squadron"), mk("Empire", "TIE_Fighter_Squadron")], config });
    for s in b.ships.iter_mut().filter(|s| s.side == 1) {
        s.pos = Vec3::new(s.pos.x + 60000.0, s.pos.y, s.pos.z);
    }
    let mine: Vec<u32> = b.ships.iter().filter(|s| s.side == 0).map(|s| s.id).collect();
    let to = b.ships[mine[0] as usize].pos + Vec3::new(0.0, 1500.0, 0.0);
    b.command(0, Command::Move { ships: mine.clone(), to }).unwrap();
    let mut worst = 0.0f32;
    for t in 0..30 * 40 {
        b.step();
        if t > 5 * 30 {
            for s in b.ships.iter().filter(|s| s.side == 0 && s.alive) {
                let leader = b.ships.iter().filter(|o| o.squad == s.squad && o.alive).map(|o| o.id).min().unwrap();
                if leader != s.id {
                    let l = &b.ships[leader as usize];
                    worst = worst.max(s.pos.dist(l.pos + s.formation.rotate_yaw(l.yaw)));
                }
            }
        }
    }
    println!("ordered squadrons: worst distance from the slot {worst:.0}");
    assert!(worst < 150.0, "members keep their slots on the way: {worst}");
    let leader_of = |sq: u32| b.ships.iter().filter(|o| o.squad == sq).map(|o| o.id).min().unwrap();
    let far = mine.iter().map(|&i| leader_of(b.ships[i as usize].squad)).map(|l| b.ships[l as usize].pos.dist(to)).fold(0.0f32, f32::max);
    println!("farthest leader from the goal {far:.0}");
    assert!(far < 150.0, "and the squadrons arrived: {far}");
}
