//! Group moves (user rule 2026-10-17: 20 Star Destroyers stand in a tight row, if needed in 2-3 rows): a Move or
//! Attack-move order for several units gives every unit its own slot of a formation at the destination, every ship flies its own route
//! there, and the group keeps the pace of its slowest member (`formation.rs`, RULES.md 9d). Real Remake data where it is installed.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use battle_sim::*;

fn real_data() -> Option<(alamo_data::GameData, GameFs)> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    let data = load_galaxy(&fs).0;
    Some((data, fs))
}

/// Ships whose centres lie within this distance along the move axis count as one row.
const ROW_BAND: f32 = 200.0;

/// A group move: the fleet of side 0, the moves it gets one after the other (degrees off the direction towards the map centre, distance
/// from the fleet's centre), the seconds each move runs, attack-move or move, whether the craft its carriers launched are selected too,
/// and an enemy that stands on the way of the first move (`enemy_at` units along it) or far away; an MC-80 far away keeps the battle going.
struct Scenario<'a> {
    units: &'a [(&'a str, u32)],
    moves: &'a [(f32, f32)],
    secs: f32,
    seed: u64,
    attack: bool,
    carried: bool,
    enemy: (&'a str, u32),
    enemy_at: Option<f32>,
}

impl Default for Scenario<'_> {
    fn default() -> Self {
        Scenario { units: &[], moves: &[(0.0, 6000.0)], secs: 240.0, seed: 1, attack: false, carried: false, enemy: ("MC-80_Star_Cruiser", 1), enemy_at: None }
    }
}

/// How the last move ended (the ordered fleet's ships: corvettes, frigates, capital ships).
#[derive(Debug, Default, Clone)]
#[allow(dead_code)] // all fields are printed (Debug) for the record
struct GroupMove {
    /// Ships per row across the move, front row first, and the most common class of each row.
    rows: Vec<usize>,
    row_class: Vec<TargetClass>,
    /// Centre distance of neighbours in a row: smallest and largest.
    spacing: (f32, f32),
    /// Gap between the hull sides of neighbours in a row (centre distance minus both half widths): smallest and largest.
    side_gap: (f32, f32),
    /// Largest spread of the ship centres of one row along the move.
    row_ragged: f32,
    /// Extent of the parked fleet across and along the move (ship centres).
    width: f32,
    depth: f32,
    /// Largest heading difference to the move direction at the end, degrees.
    facing_err: f32,
    /// Distance of the parked fleet's centre from the ordered point.
    centre_err: f32,
    /// Seconds from the last order until every ship stood still (speed under 1) for good.
    settled: f32,
    /// Largest hull capsule overlap of two ships at any half-second sample of the whole run, as a share of their half widths summed, and where.
    overlap: f32,
    overlap_at: String,
    /// Over the last 20 s: the farthest a ship moved and its largest heading change (degrees); parked ships hold still.
    drift: f32,
    turn: f32,
    /// Ordered ships alive at the end (of those at the order) and enemy ships alive.
    alive: (usize, usize),
    enemies: usize,
    /// Ordered fighter squadrons at the end, and the pairs of them whose members' boxes (across and along the move) reach into each other.
    squads: usize,
    squads_mixed: usize,
    /// Largest heading difference of the squadron leaders to the move at the end, degrees (a squadron keeps its leader's heading at rest).
    squad_facing: f32,
    /// State hash at the end.
    hash: u64,
}

fn run(sc: &Scenario) -> Option<GroupMove> {
    let (data, fs) = real_data()?;
    let mut names: Vec<&str> = sc.units.iter().map(|u| u.0).collect();
    names.extend([sc.enemy.0, "MC-80_Star_Cruiser"]);
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &names));
    let mk = |name: &str, u: &[(&str, u32)]| SideSetup { name: name.into(), units: u.iter().map(|&(n, c)| UnitSetup { ship: cat.ship_id(n).unwrap_or_else(|| panic!("{n}")), count: c, tag: 1 }).collect(), structures: vec![], ai: false };
    let config = BattleConfig { record_events: false, seed: sc.seed, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![mk("Empire", sc.units), mk("Rebel", &[sc.enemy, ("MC-80_Star_Cruiser", 1)])], config });
    for _ in 0..30 {
        b.step();
    }
    let small = |s: &Ship| matches!(cat.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber);
    // The player selects the fleet: its ships and squadrons, and with `carried` the craft its carriers launched.
    let ordered: Vec<u32> = b.ships.iter().filter(|s| s.side == 0 && s.alive && (sc.carried || s.carrier == NONE)).map(|s| s.id).collect();
    let big: Vec<u32> = ordered.iter().copied().filter(|&i| !small(&b.ships[i as usize]) && b.ships[i as usize].carrier == NONE).collect();
    let centre_of = |b: &Battle| {
        let alive: Vec<Vec3> = big.iter().filter(|&&i| b.ships[i as usize].alive).map(|&i| b.ships[i as usize].pos).collect();
        alive.iter().fold(Vec3::ZERO, |a, &p| a + p) * (1.0 / alive.len().max(1) as f32)
    };
    let start = centre_of(&b);
    let base = if start.len() > 1.0 { (start * -1.0).normalized() } else { Vec3::new(1.0, 0.0, 0.0) };
    // The enemy on the way of the first move (`enemy_at`), everything else of its side far away.
    let enemies: Vec<u32> = b.ships.iter().filter(|s| s.side == 1 && sc.enemy_at.is_some() && cat.ship(s.def).name == sc.enemy.0).map(|s| s.id).collect();
    for s in b.ships.iter_mut().filter(|s| s.side == 1) {
        s.pos = match enemies.iter().position(|&e| e == s.id) {
            Some(k) => start + base.rotate_yaw(sc.moves[0].0.to_radians()) * sc.enemy_at.unwrap_or(0.0) + Vec3::new(0.0, 300.0 * k as f32, 0.0),
            None => s.pos + Vec3::new(60000.0, 0.0, 0.0),
        };
    }
    let mut m = GroupMove::default();
    let (mut to, mut dir, mut t0) = (start, base, 0.0);
    let mut moving_at = 0.0;
    let mut parked: Option<Vec<(Vec3, f32)>> = None;
    let steps = (sc.secs * 30.0) as u32;
    for (n, &(turn_deg, distance)) in sc.moves.iter().enumerate() {
        dir = base.rotate_yaw(turn_deg.to_radians());
        to = centre_of(&b) + dir * distance;
        let ships: Vec<u32> = ordered.iter().copied().filter(|&i| b.ships[i as usize].alive).collect();
        b.command(0, if sc.attack { Command::AttackMove { ships, to } } else { Command::Move { ships, to } }).unwrap();
        t0 = b.time();
        moving_at = t0;
        for step in 0..steps {
            b.step();
            if big.iter().any(|&i| b.ships[i as usize].alive && b.ships[i as usize].speed >= 1.0) {
                moving_at = b.time();
            }
            if n + 1 == sc.moves.len() && step + 20 * 30 == steps {
                parked = Some(big.iter().map(|&i| (b.ships[i as usize].pos, b.ships[i as usize].yaw)).collect());
            }
            if step % 15 != 0 {
                continue;
            }
            for (k, &x) in big.iter().enumerate() {
                for &y in &big[k + 1..] {
                    let (sx, sy) = (&b.ships[x as usize], &b.ships[y as usize]);
                    if !sx.alive || !sy.alive {
                        continue;
                    }
                    let (hx, hy) = (cat.ship(sx.def).footprint(), cat.ship(sy.def).footprint());
                    if let Some((depth, _)) = nav::hull_overlap((sx.pos, sx.yaw, hx), (sy.pos, sy.yaw, hy), 0.0, 1.0) {
                        let share = depth / (hx.half_width + hy.half_width);
                        if share > m.overlap {
                            m.overlap = share;
                            m.overlap_at = format!("{} #{x} in {} #{y} at {:.0} s", cat.ship(sx.def).name, cat.ship(sy.def).name, b.time());
                        }
                    }
                }
            }
        }
    }
    m.settled = moving_at - t0;
    let live: Vec<u32> = big.iter().copied().filter(|&i| b.ships[i as usize].alive).collect();
    m.alive = (live.len(), big.len());
    m.enemies = enemies.iter().filter(|&&e| b.ships[e as usize].alive).count();
    let left = Vec3::new(-dir.y, dir.x, 0.0);
    // Fighter squadrons: the box their members span across and along the move.
    let mut boxes: std::collections::BTreeMap<u32, (f32, f32, f32, f32)> = std::collections::BTreeMap::new();
    for &i in &ordered {
        let s = &b.ships[i as usize];
        if s.alive && small(s) && s.squad != NONE {
            let (a, c) = ((s.pos - to).dot(left), (s.pos - to).dot(dir));
            let e = boxes.entry(s.squad).or_insert((f32::MAX, f32::MIN, f32::MAX, f32::MIN));
            *e = (e.0.min(a), e.1.max(a), e.2.min(c), e.3.max(c));
        }
    }
    let move_yaw = dir.y.atan2(dir.x);
    m.squad_facing = boxes.keys().filter_map(|&q| b.squad_leader(q)).map(|l| ((b.ships[l as usize].yaw - move_yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI).abs().to_degrees()).fold(0.0, f32::max);
    let bx: Vec<(f32, f32, f32, f32)> = boxes.into_values().collect();
    m.squads = bx.len();
    m.squads_mixed = (0..bx.len()).flat_map(|i| (i + 1..bx.len()).map(move |j| (i, j))).filter(|&(i, j)| bx[i].0 < bx[j].1 && bx[j].0 < bx[i].1 && bx[i].2 < bx[j].3 && bx[j].2 < bx[i].3).count();
    // Where each ship stands: (along, across, half width, class, yaw).
    type Place = (f32, f32, f32, TargetClass, f32);
    let mut at: Vec<Place> = live
        .iter()
        .map(|&i| {
            let s = &b.ships[i as usize];
            let d = cat.ship(s.def);
            ((s.pos - to).dot(dir), (s.pos - to).dot(left), d.footprint().half_width, d.class, s.yaw)
        })
        .collect();
    at.sort_by(|a, c| c.0.total_cmp(&a.0));
    let mut rows: Vec<Vec<Place>> = Vec::new();
    for p in at.iter().copied() {
        match rows.last_mut() {
            Some(r) if r.last().is_some_and(|q| q.0 - p.0 <= ROW_BAND) => r.push(p),
            _ => rows.push(vec![p]),
        }
    }
    m.spacing = (f32::MAX, 0.0);
    m.side_gap = (f32::MAX, f32::MIN);
    for r in &mut rows {
        let along: Vec<f32> = r.iter().map(|p| p.0).collect();
        m.row_ragged = m.row_ragged.max(along.iter().copied().fold(f32::MIN, f32::max) - along.iter().copied().fold(f32::MAX, f32::min));
        r.sort_by(|a, c| a.1.total_cmp(&c.1));
        for w in r.windows(2) {
            let d = w[1].1 - w[0].1;
            m.spacing = (m.spacing.0.min(d), m.spacing.1.max(d));
            m.side_gap = (m.side_gap.0.min(d - w[0].2 - w[1].2), m.side_gap.1.max(d - w[0].2 - w[1].2));
        }
        let mut count = std::collections::BTreeMap::new();
        for p in r.iter() {
            *count.entry(p.3 as u8).or_insert(0) += 1;
        }
        m.rows.push(r.len());
        m.row_class.push(r.iter().map(|p| p.3).max_by_key(|c| (count[&(*c as u8)], *c as u8)).expect("a row"));
    }
    let spread = |f: &dyn Fn(&Place) -> f32| at.iter().map(f).fold(f32::MIN, f32::max) - at.iter().map(f).fold(f32::MAX, f32::min);
    m.width = spread(&|p| p.1);
    m.depth = spread(&|p| p.0);
    let heading = dir.y.atan2(dir.x);
    let off = |yaw: f32| ((yaw - heading + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI).abs().to_degrees();
    m.facing_err = at.iter().map(|p| off(p.4)).fold(0.0, f32::max);
    m.centre_err = (centre_of(&b) - to).len();
    if let Some(p) = parked {
        for (k, &i) in big.iter().enumerate() {
            let s = &b.ships[i as usize];
            if !s.alive {
                continue;
            }
            m.drift = m.drift.max(s.pos.dist(p[k].0));
            m.turn = m.turn.max(((s.yaw - p[k].1 + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI).abs().to_degrees());
        }
    }
    m.hash = b.state_hash();
    Some(m)
}

#[test]
fn twenty_star_destroyers_sent_to_one_point_stand_in_two_tight_rows() {
    // Before (main 892edaf, seed 1): the Star Destroyers flew to the one point and jostled there in a blob 3450 wide and 4900 deep, neighbours
    // 530-2810 apart, still moving after 240 s (926 units and 63 degrees in the last 20 s), headings up to 171 degrees off the move.
    let Some(m) = run(&Scenario { units: &[("Star_Destroyer", 20)], ..Scenario::default() }) else { return };
    println!("20 Star Destroyers, 6000 ahead: {m:?}");
    assert!((2..=3).contains(&m.rows.len()) && m.rows.iter().all(|&n| n >= 5), "2-3 rows across the move: {m:?}");
    // Side by side: 0.85 of the two snapshot radii keeps the hulls apart (539), plus `BetweenFormationSpacing` 50.
    assert!(m.spacing.0 >= 539.0 && m.spacing.1 <= 650.0, "neighbours stand tight and apart: {m:?}");
    assert!(m.row_ragged <= 60.0, "the rows are straight: {m:?}");
    assert!(m.facing_err <= 6.0, "every ship faces the move (FinalFormationFacingMinimumAngle 5): {m:?}");
    assert!(m.centre_err <= 300.0, "the formation stands at the ordered point: {m:?}");
    assert!(m.overlap <= 0.10, "no hull overlaps another by more than 10%: {m:?}");
    assert!(m.settled <= 180.0, "the fleet has formed up: {m:?}");
    // Multi-frame stability: parked ships neither creep nor turn back and forth.
    assert!(m.drift <= 1.0 && m.turn <= 0.5, "the parked formation holds still: {m:?}");
}

#[test]
fn a_mixed_fleet_keeps_its_rows_and_turns_them_with_the_next_move() {
    // Before (main 892edaf, seed 1): a blob 1610 wide and 1500 deep with the corvettes between the capital ships, still moving after 240 s
    // (717 units and 171 degrees in the last 20 s), headings up to 178 degrees off the move.
    let fleet = [("Star_Destroyer", 3), ("Victory_Destroyer", 4), ("CR-90_Corvette", 4), ("Tartan_Patrol_Cruiser", 2)];
    let Some(m) = run(&Scenario { units: &fleet, moves: &[(40.0, 5000.0)], ..Scenario::default() }) else { return };
    println!("mixed fleet, 5000 at 40 degrees: {m:?}");
    assert_eq!(m.rows, vec![8, 7], "4 CR-90s and 4 Tartans in a row ahead of 3 Star Destroyers and 4 Victorys: {m:?}");
    assert_eq!(m.row_class[0], TargetClass::Corvette, "the corvettes stand ahead: {m:?}");
    assert!(m.row_ragged <= 60.0 && m.facing_err <= 6.0 && m.overlap <= 0.10, "{m:?}");
    assert!(m.drift <= 1.0 && m.turn <= 0.5, "the parked formation holds still: {m:?}");
    // The next order sends it off at a right angle: the rows turn to face the new move (`Rotate_Formation_Facing_Moves`).
    let Some(n) = run(&Scenario { units: &fleet, moves: &[(40.0, 5000.0), (130.0, 4000.0)], ..Scenario::default() }) else { return };
    println!("then 4000 at 130 degrees: {n:?}");
    assert_eq!(n.rows, vec![8, 7], "{n:?}");
    assert_eq!(n.row_class[0], TargetClass::Corvette, "{n:?}");
    assert!(n.row_ragged <= 60.0 && n.facing_err <= 6.0 && n.overlap <= 0.10, "{n:?}");
    assert!(n.drift <= 1.0 && n.turn <= 0.5, "{n:?}");
}

#[test]
fn an_attack_move_fights_what_it_meets_and_forms_up_after() {
    let fleet = [("Star_Destroyer", 2), ("Victory_Destroyer", 4)];
    let Some(m) = run(&Scenario { units: &fleet, moves: &[(0.0, 7000.0)], secs: 300.0, attack: true, enemy: ("CR-90_Corvette", 2), enemy_at: Some(3500.0), ..Scenario::default() }) else { return };
    println!("attack-move through 2 CR-90s: {m:?}");
    assert_eq!(m.enemies, 0, "the corvettes on the way are destroyed: {m:?}");
    assert_eq!(m.alive.0, m.alive.1, "{m:?}");
    assert_eq!(m.rows.len(), 1, "then the fleet stands in its line: {m:?}");
    assert!(m.row_ragged <= 60.0 && m.facing_err <= 6.0 && m.overlap <= 0.10 && m.centre_err <= 300.0, "{m:?}");
}

#[test]
fn a_fleet_sent_behind_itself_turns_round_and_forms_up() {
    let Some(m) = run(&Scenario { units: &[("Star_Destroyer", 6), ("Victory_Destroyer", 4)], moves: &[(180.0, 3000.0)], ..Scenario::default() }) else { return };
    println!("10 ships, 3000 behind them: {m:?}");
    assert_eq!(m.rows, vec![5, 5], "the 10 ships (wider than 6000 in one row) stand in two rows of 5: {m:?}");
    assert!(m.row_ragged <= 60.0 && m.facing_err <= 6.0 && m.overlap <= 0.10 && m.centre_err <= 300.0, "{m:?}");
    assert!(m.drift <= 1.0 && m.turn <= 0.5, "{m:?}");
}

#[test]
fn a_selection_with_the_launched_craft_forms_up_too() {
    // Selecting everything takes the fighters and shuttles the Star Destroyers launched along: they fly in the escort row ahead.
    let Some(m) = run(&Scenario { units: &[("Star_Destroyer", 6)], carried: true, ..Scenario::default() }) else { return };
    println!("6 Star Destroyers with their launched craft: {m:?}");
    assert_eq!(m.rows, vec![6], "{m:?}");
    assert!(m.row_ragged <= 60.0 && m.facing_err <= 6.0 && m.overlap <= 0.10, "{m:?}");
    // Each squadron is one unit with the room of its spread members, each within `FIGHTER_HOLD` of its place: no squadron reaches into
    // another (counted as round units of radius 90, 4 pairs of the 12 squadrons did; with their places but not the hold distance, 1 pair).
    assert!(m.squads >= 6 && m.squads_mixed == 0, "the squadrons stand side by side: {m:?}");
    // The leaders came in behind their slots, so the squadrons face the move (flying straight to the slot they stood up to 12.7 degrees off).
    assert!(m.squad_facing <= 6.0, "the squadrons face the move: {m:?}");
}

#[test]
fn group_moves_are_deterministic() {
    let fleet = [("Star_Destroyer", 3), ("Victory_Destroyer", 4), ("CR-90_Corvette", 4), ("Tartan_Patrol_Cruiser", 2)];
    let sc = Scenario { units: &fleet, moves: &[(40.0, 5000.0)], secs: 120.0, seed: 7, attack: true, ..Scenario::default() };
    let Some(a) = run(&sc) else { return };
    let Some(b) = run(&sc) else { return };
    assert_eq!(a.hash, b.hash, "same seed, same battle (threads included)");
}

// Hand-made units: the data tags.

fn unit(name: &str, class: TargetClass, speed: f32) -> ShipDef {
    ShipDef { name: name.into(), class, hull: 1000.0, radius: 50.0, max_speed: speed, accel: speed, turn_rate: 1.5, population: 1, ..ShipDef::default() }
}

fn battle(defs: Vec<ShipDef>, units: &[(ShipId, u32)]) -> Battle {
    let cat = Arc::new(Catalog::from_defs(defs, vec![]));
    let side = |name: &str, units: Vec<UnitSetup>| SideSetup { name: name.into(), units, structures: vec![], ai: false };
    let mine = units.iter().map(|&(ship, count)| UnitSetup { ship, count, tag: 1 }).collect();
    let far = side("b", vec![UnitSetup { ship: 0, count: 1, tag: 1 }]);
    let mut b = Battle::new(BattleSetup { catalog: cat, sides: vec![side("a", mine), far], config: BattleConfig { half_size: 20000.0, record_events: false, ..BattleConfig::default() } });
    for s in b.ships.iter_mut().filter(|s| s.side == 1) {
        s.pos = Vec3::new(60000.0, 0.0, 0.0);
    }
    b
}

#[test]
fn units_wait_for_the_slowest_unless_the_data_says_they_do_not() {
    // Slow (50) and fast (200) frigates ordered 3000 ahead: the fast ones keep the slow ones' pace; with `WaitsForFormationFormup` false they
    // fly at their own speed.
    for no_wait in [false, true] {
        let mut fast = unit("fast", TargetClass::Frigate, 200.0);
        fast.formation_no_wait = no_wait;
        let mut b = battle(vec![unit("slow", TargetClass::Frigate, 50.0), fast], &[(0, 2), (1, 2)]);
        let mine: Vec<u32> = b.ships.iter().filter(|s| s.side == 0).map(|s| s.id).collect();
        let start: Vec<Vec3> = mine.iter().map(|&i| b.ships[i as usize].pos).collect();
        let to = mine.iter().fold(Vec3::ZERO, |a, &i| a + b.ships[i as usize].pos) * 0.25 + Vec3::new(3000.0, 0.0, 0.0);
        b.command(0, Command::Move { ships: mine.clone(), to }).unwrap();
        for _ in 0..30 * 10 {
            b.step();
        }
        let gone = |k: usize| b.ships[mine[k] as usize].pos.dist(start[k]);
        let (slow, fast) = (gone(0).max(gone(1)), gone(2).min(gone(3)));
        println!("waits {}: slow ships flew {slow:.0}, fast ones {fast:.0} in 10 s", !no_wait);
        if no_wait {
            assert!(fast > 1500.0, "a unit that does not wait flies at its own speed: {fast}");
        } else {
            assert!(fast < 1.5 * slow + 100.0, "the fast ships keep the slow ones' pace: {fast} vs {slow}");
        }
    }
}

#[test]
fn formation_order_and_spacing_come_from_the_data() {
    // `FormationOrder` 2 stands behind order 1 (same class), `FormationSpacing` 3 widens the gap next to the unit (`Solo`: a unit test of formation.rs).
    let mut back = unit("back", TargetClass::Frigate, 100.0);
    back.formation_order = 2;
    let mut wide = unit("wide", TargetClass::Frigate, 100.0);
    wide.formation_spacing = 3.0;
    let defs = vec![unit("front", TargetClass::Frigate, 100.0), back, wide];
    let mut b = battle(defs, &[(0, 2), (1, 2), (2, 1)]);
    let mine: Vec<u32> = b.ships.iter().filter(|s| s.side == 0).map(|s| s.id).collect();
    let to = mine.iter().fold(Vec3::ZERO, |a, &i| a + b.ships[i as usize].pos) * (1.0 / mine.len() as f32) + Vec3::new(4000.0, 0.0, 0.0);
    b.command(0, Command::Move { ships: mine.clone(), to }).unwrap();
    for _ in 0..30 * 120 {
        b.step();
    }
    let of = |name: &str| -> Vec<Vec3> { mine.iter().filter(|&&i| b.catalog().ship(b.ships[i as usize].def).name == name).map(|&i| b.ships[i as usize].pos).collect() };
    let (front, back, wide) = (of("front"), of("back"), of("wide"));
    println!("to {to:?}\nfront {front:?}\nback {back:?}\nwide {wide:?}");
    let ahead = |v: &[Vec3]| v.iter().map(|p| p.x).fold(0.0, |a, x| a + x) / v.len() as f32;
    assert!(ahead(&back) < ahead(&front) - 50.0, "FormationOrder 2 stands behind order 1");
    // The ordered units of order 1 stand in one row: the wide one keeps 3 x 50 from its neighbours instead of 50.
    let mut row: Vec<Vec3> = front.iter().chain(&wide).copied().collect();
    row.sort_by(|a, c| a.y.total_cmp(&c.y));
    let gaps: Vec<f32> = row.windows(2).map(|w| w[1].y - w[0].y).collect();
    println!("order-1 row gaps {gaps:?}");
    assert!(gaps.iter().copied().fold(0.0, f32::max) > gaps.iter().copied().fold(f32::MAX, f32::min) + 80.0, "the wide unit gets more room: {gaps:?}");
}
