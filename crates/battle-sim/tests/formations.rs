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

// ---------------------------------------------------------------------------------------------------------------------
// Combat movement: ships with a target in weapon range hold position and turn their weapons to bear (no chasing, no orbiting).

#[derive(Debug, Default, Clone, Copy)]
#[allow(dead_code)] // all fields are printed (Debug) for the record
struct CombatMove {
    /// Samples (ship, step) of mobile non-fighter ships with a live target.
    samples: u32,
    /// Share of those samples with the target inside the longest weapon range.
    in_range_share: f32,
    /// Share of the in-range samples in which the ship moves faster than 10% of its top speed.
    moving_share: f32,
    /// Mean speed in range as a share of top speed.
    mean_speed_share: f32,
    /// Mean absolute yaw rate in range, degrees per second.
    mean_turn_deg_s: f32,
    /// Share of in-range samples with a yaw rate above 2 degrees per second.
    turning_share: f32,
    /// Samples with the target inside the standoff distance (preferred range): share moving faster than 10% of top speed, mean speed share.
    inside_moving_share: f32,
    inside_speed_share: f32,
    inside_turn_deg_s: f32,
    /// Settled = target inside the standoff for 10 s in a row (the approach and its braking are over): share of samples, moving share (> 10% of top speed), mean speed share, mean yaw rate.
    settled_share: f32,
    settled_moving_share: f32,
    settled_speed_share: f32,
    settled_turn_deg_s: f32,
}

fn combat_move(units: &[(&str, u32)], enemy: &[(&str, u32)], secs: f32, seed: u64) -> Option<CombatMove> {
    let (data, fs) = real_data()?;
    let names: Vec<&str> = units.iter().chain(enemy.iter()).map(|u| u.0).collect();
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &names));
    let mk = |name: &str, u: &[(&str, u32)]| SideSetup { name: name.into(), units: u.iter().map(|&(n, c)| UnitSetup { ship: cat.ship_id(n).unwrap(), count: c, tag: 1 }).collect(), structures: vec![], ai: true };
    let config = BattleConfig { record_events: false, seed, retreat_allowed: vec![false, false], ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![mk("A", units), mk("B", enemy)], config });
    let (mut n, mut in_range, mut moving, mut speed, mut turn, mut turning) = (0u32, 0u32, 0u32, 0.0f64, 0.0f64, 0u32);
    let (mut inside, mut inside_moving, mut inside_speed, mut inside_turn) = (0u32, 0u32, 0.0f64, 0.0f64);
    let (mut settled, mut settled_moving, mut settled_speed, mut settled_turn) = (0u32, 0u32, 0.0f64, 0.0f64);
    let mut since: Vec<u32> = Vec::new();
    let mut yaw: Vec<f32> = b.ships.iter().map(|s| s.yaw).collect();
    for _ in 0..(secs * 30.0) as usize {
        b.step();
        for s in b.ships.iter().filter(|s| s.alive && !s.retreated) {
            let def = cat.ship(s.def);
            let dyaw = (s.yaw - yaw.get(s.id as usize).copied().unwrap_or(s.yaw) + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            if s.squad != u32::MAX || def.is_static || def.max_speed <= 0.0 {
                continue;
            }
            // The target the ship positions itself against: the best non-fighter candidate.
            let Some(tid) = s.candidates.iter().copied().find(|&c| !matches!(cat.ship(b.ships[c as usize].def).class, TargetClass::Fighter | TargetClass::Bomber)) else { continue };
            if !b.ships[tid as usize].alive {
                continue;
            }
            if matches!(def.class, TargetClass::Fighter | TargetClass::Bomber) {
                continue;
            }
            n += 1;
            let t = &b.ships[tid as usize];
            if since.len() <= s.id as usize {
                since.resize(b.ships.len(), 0);
            }
            since[s.id as usize] = if s.pos.dist(t.pos) - cat.ship(t.def).radius <= def.preferred_range { since[s.id as usize] + 1 } else { 0 };
            if since[s.id as usize] > 300 {
                settled += 1;
                settled_moving += u32::from(s.speed > 0.1 * def.max_speed);
                settled_speed += f64::from(s.speed / def.max_speed);
                settled_turn += f64::from(dyaw.abs().to_degrees() * 30.0);
            }
            if s.pos.dist(t.pos) - cat.ship(t.def).radius > def.max_weapon_range {
                continue;
            }
            in_range += 1;
            let rate0 = dyaw.abs().to_degrees() * 30.0;
            if s.pos.dist(t.pos) - cat.ship(t.def).radius <= def.preferred_range {
                inside += 1;
                inside_moving += u32::from(s.speed > 0.1 * def.max_speed);
                inside_speed += f64::from(s.speed / def.max_speed);
                inside_turn += f64::from(rate0);
            }
            speed += f64::from(s.speed / def.max_speed);
            moving += u32::from(s.speed > 0.1 * def.max_speed);
            let rate = dyaw.abs().to_degrees() * 30.0;
            turn += f64::from(rate);
            turning += u32::from(rate > 2.0);
        }
        yaw = b.ships.iter().map(|s| s.yaw).collect();
        if b.result().winner.is_some() {
            break;
        }
    }
    let r = f64::from(in_range.max(1));
    Some(CombatMove {
        samples: n,
        in_range_share: in_range as f32 / n.max(1) as f32,
        moving_share: (f64::from(moving) / r) as f32,
        mean_speed_share: (speed / r) as f32,
        mean_turn_deg_s: (turn / r) as f32,
        turning_share: (f64::from(turning) / r) as f32,
        inside_moving_share: (f64::from(inside_moving) / f64::from(inside.max(1))) as f32,
        inside_speed_share: (inside_speed / f64::from(inside.max(1))) as f32,
        settled_share: settled as f32 / n.max(1) as f32,
        settled_moving_share: (f64::from(settled_moving) / f64::from(settled.max(1))) as f32,
        settled_speed_share: (settled_speed / f64::from(settled.max(1))) as f32,
        settled_turn_deg_s: (settled_turn / f64::from(settled.max(1))) as f32,
        inside_turn_deg_s: (inside_turn / f64::from(inside.max(1))) as f32,
    })
}

#[test]
fn ships_with_a_target_in_range_hold_position() {
    type Fleet<'a> = &'a [(&'a str, u32)];
    let matchups: [(&str, Fleet, Fleet); 3] = [
        ("ISD vs MC-80", &[("Star_Destroyer", 3)], &[("MC-80_Star_Cruiser", 3)]),
        ("Victory vs Nebulon-B", &[("Victory_Destroyer", 3)], &[("Nebulon_B_Frigate", 3)]),
        ("Corvettes", &[("CR-90_Corvette", 4)], &[("Tartan_Patrol_Cruiser", 4)]),
    ];
    let mut failed = Vec::new();
    for (name, a, b) in matchups {
        let mut ok = true;
        for seed in 1..=2 {
            let Some(m) = combat_move(a, b, 240.0, seed) else { return };
            println!("{name} seed {seed}: {m:?}");
            ok &= m.settled_moving_share < 0.05 && m.settled_turn_deg_s < 4.0;
        }
        if !ok {
            failed.push(name);
        }
    }
    assert!(failed.is_empty(), "ships with a target inside their standoff keep station instead of chasing or orbiting: {failed:?}");
}

#[test]
fn reinforced_squadron_has_offsets_and_flies_in_formation() {
    let Some((data, fs)) = real_data() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["X-Wing_Squadron", "CR-90_Corvette"]));
    let mk = |name: &str| SideSetup { name: name.into(), units: vec![UnitSetup { ship: cat.ship_id("CR-90_Corvette").unwrap(), count: 1, tag: 1 }], structures: vec![], ai: false };
    let config = BattleConfig { record_events: false, seed: 1, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![mk("Rebel"), mk("Empire")], config });
    let x_def = cat.ship_id("X-Wing_Squadron").unwrap();
    b.command(0, Command::Reinforce { ship: x_def, count: 1 }).unwrap();
    for _ in 0..400 {
        b.step();
    }
    let fighters: Vec<u32> = b.ships.iter().filter(|s| s.side == 0 && s.squad != u32::MAX).map(|s| s.id).collect();
    assert_eq!(fighters.len(), 6, "6 X-wings in reinforced squadron");
    for &m in &fighters[1..] {
        let s = &b.ships[m as usize];
        assert!(s.formation.len() > 5.0, "slot follower has non-zero offset: {:?}", s.formation);
    }
}

#[test]
fn capital_ship_approaches_and_aligns_broadside_weapons() {
    let Some((data, fs)) = real_data() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Star_Destroyer", "MC-80_Star_Cruiser"]));
    let mk = |name: &str, ship: &str| SideSetup {
        name: name.into(),
        units: vec![UnitSetup { ship: cat.ship_id(ship).unwrap(), count: 1, tag: 1 }],
        structures: vec![],
        ai: false,
    };
    let config = BattleConfig { record_events: false, seed: 1, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![mk("Empire", "Star_Destroyer"), mk("Rebel", "MC-80_Star_Cruiser")], config });
    let isd = b.ships.iter().find(|s| s.side == 0).map(|s| s.id).unwrap();
    let mc80 = b.ships.iter().find(|s| s.side == 1).map(|s| s.id).unwrap();
    b.ships[isd as usize].pos = b.ships[mc80 as usize].pos - Vec3::new(4500.0, 0.0, 0.0);
    b.command(0, Command::Attack { ships: vec![isd], target: mc80 }).unwrap();
    // Step forward until ISD reaches standoff and engages
    for _ in 0..(40 * 30) {
        b.step();
    }
    let isd_speed = b.ships[isd as usize].speed;
    println!("isd speed: {:.1}", isd_speed);
    // Order MC-80 to attack ISD
    b.command(1, Command::Attack { ships: vec![mc80], target: isd }).unwrap();
    for _ in 0..(20 * 30) {
        b.step();
    }
    let mc80_ship = &b.ships[mc80 as usize];
    let to_isd = b.ships[isd as usize].pos - mc80_ship.pos;
    let bearing_to_isd = to_isd.y.atan2(to_isd.x);
    let mc80_rel = (bearing_to_isd - mc80_ship.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    println!("MC-80 relative angle to target at standoff: {:.1} deg", mc80_rel.to_degrees());
    assert!(mc80_rel.abs().to_degrees() > 50.0, "MC-80 presents broadside: angle is {:.1} deg", mc80_rel.to_degrees());
    assert!(isd_speed < 80.0, "ISD brakes into standoff: speed {}", isd_speed);
}

