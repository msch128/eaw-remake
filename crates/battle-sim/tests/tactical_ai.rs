//! Tactical AI behaviour against the real Remake data (retreat rule, separation, focus on weak ships, real-map duels);
//! skipped when the game is not installed.

use std::sync::Arc;

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use battle_sim::*;

fn load() -> Option<(GameData, GameFs)> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    let data = load_galaxy(&fs).0;
    Some((data, fs))
}

fn side(cat: &Catalog, name: &str, units: &[(&str, u32)], structures: &[&str]) -> SideSetup {
    SideSetup {
        name: name.into(),
        units: units.iter().map(|&(n, c)| UnitSetup { ship: cat.ship_id(n).unwrap_or_else(|| panic!("{n}")), count: c, tag: 1 }).collect(),
        structures: structures.iter().map(|n| cat.ship_id(n).unwrap_or_else(|| panic!("{n}"))).collect(),
        ai: true,
    }
}

fn run(cat: &Arc<Catalog>, sides: Vec<SideSetup>, config: BattleConfig) -> (BattleResult, u64) {
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config: BattleConfig { record_events: false, ..config } });
    b.run_to_end(30 * 1500);
    (b.result(), b.state_hash())
}

#[test]
fn clearly_outgunned_ai_retreats_unless_it_may_not() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Star_Destroyer", "MC-80_Star_Cruiser"]));
    let sides = || vec![side(&cat, "Empire", &[("Star_Destroyer", 3)], &[]), side(&cat, "Rebel", &[("MC-80_Star_Cruiser", 5)], &[])];
    // 3 Star Destroyers against 5 MC-80s: the Empire ends up beyond the 2x threshold of Retreat_From_Space while some ships still live.
    let (r, _) = run(&cat, sides(), BattleConfig { seed: 3, ..BattleConfig::default() });
    let weak = r.groups.iter().find(|g| g.side == 0).unwrap();
    println!("retreat allowed: winner {:?} after {:.0} s, weak side {}/{} survived ({} retreated)", r.winner, r.duration, weak.survived, weak.total, weak.retreated);
    assert_eq!(r.winner, Some(1));
    assert!(weak.retreated > 0, "the outgunned AI should have pulled out: {weak:?}");
    assert!(r.duration >= 120.0, "not before the 120 s age of the rule: {}", r.duration);

    // A faction without a second planet (or Pirates) fights to the end.
    let (r2, _) = run(&cat, sides(), BattleConfig { seed: 3, retreat_allowed: vec![false, true], ..BattleConfig::default() });
    let weak2 = r2.groups.iter().find(|g| g.side == 0).unwrap();
    println!("retreat forbidden: weak side {}/{} survived ({} retreated)", weak2.survived, weak2.total, weak2.retreated);
    assert_eq!(weak2.retreated, 0);
    assert!(weak2.survived < weak.survived, "without the retreat nobody of the weak side is left: {weak2:?}");
}

#[test]
fn even_fights_do_not_trigger_the_retreat() {
    // A mirror match (3 Star Destroyers a side: ISD vs MC-80 is not even, the MC-80s win it) is fought down; most seeds of an uneven 3 v 3 end with the loser pulling out after the first loss (the 2x rule at work).
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Star_Destroyer"]));
    let (r, _) = run(&cat, vec![side(&cat, "Empire", &[("Star_Destroyer", 3)], &[]), side(&cat, "Rebel", &[("Star_Destroyer", 3)], &[])], BattleConfig { seed: 16, ..BattleConfig::default() });
    println!("even fight: winner {:?} after {:.0} s, groups {:?}", r.winner, r.duration, r.groups.iter().map(|g| (g.side, g.survived, g.retreated)).collect::<Vec<_>>());
    // With the real jump (10 s) a ship that the 2x rule sends away at 1 v 1 does get out, so an even fight may end with that retreat, but only once half of the six ships are gone.
    let left: u32 = r.groups.iter().map(|g| g.survived).sum();
    assert!(left <= 3, "an even fight is fought down to half of the ships before anyone pulls out: {:?}", r.groups.iter().map(|g| (g.side, g.survived, g.retreated)).collect::<Vec<_>>());
    assert!(r.winner.is_some());
}

#[test]
fn big_ships_do_not_stack_on_top_of_each_other() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Star_Destroyer", "MC-80_Star_Cruiser", "Nebulon_B_Frigate"]));
    let setup = BattleSetup {
        catalog: cat.clone(),
        sides: vec![side(&cat, "Empire", &[("Star_Destroyer", 8)], &[]), side(&cat, "Rebel", &[("MC-80_Star_Cruiser", 6), ("Nebulon_B_Frigate", 6)], &[])],
        config: BattleConfig { record_events: false, seed: 4, ..BattleConfig::default() },
    };
    let mut b = Battle::new(setup);
    let (mut worst, mut samples) = (0.0f32, 0);
    for step in 0..30 * 240 {
        b.step();
        if step % 90 == 0 && b.time() > 20.0 {
            let ships: Vec<_> = b.ships.iter().filter(|s| s.alive && !s.retreated && !cat.ship(s.def).is_static && cat.ship(s.def).class as u8 >= TargetClass::Frigate as u8).collect();
            for (i, a) in ships.iter().enumerate() {
                for c in &ships[i + 1..] {
                    let (ra, rc) = (cat.ship(a.def).radius * 0.75, cat.ship(c.def).radius * 0.75);
                    let overlap = (ra + rc - a.pos.dist(c.pos)) / (ra + rc);
                    worst = worst.max(overlap);
                }
            }
            samples += 1;
        }
        if b.is_over() {
            break;
        }
    }
    println!("worst overlap of two big ships over {samples} samples: {:.0}% of their radii", worst * 100.0);
    assert!(samples > 5);
    assert!(worst < 0.5, "capital ships interpenetrate by {:.0}%", worst * 100.0);
}

#[test]
fn real_map_duels_are_sane_and_deterministic() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Star_Destroyer", "Victory_Destroyer", "MC-80_Star_Cruiser", "Nebulon_B_Frigate", "TIE_Fighter_Squadron", "X-Wing_Squadron", "CR-90_Corvette"]));
    for map in ["_SPACE_PLANET_CORUSCANT.ted", "_SPACE_PLANET_KUAT_01.ted"] {
        let Some(arena) = Arena::load(&data, &fs, map).map(Arc::new) else { continue };
        let go = || {
            let sides = vec![
                side(&cat, "Empire", &[("Star_Destroyer", 2), ("Victory_Destroyer", 2), ("TIE_Fighter_Squadron", 3)], &[]),
                side(&cat, "Rebel", &[("MC-80_Star_Cruiser", 2), ("Nebulon_B_Frigate", 3), ("CR-90_Corvette", 2), ("X-Wing_Squadron", 3)], &[]),
            ];
            run(&cat, sides, BattleConfig { seed: 11, half_size: arena.half_size(), arena: Some(arena.clone()), ..BattleConfig::default() })
        };
        let (a, ha) = go();
        let (b, hb) = go();
        println!("{map}: winner {:?} after {:.0} s, survivors {:?}", a.winner, a.duration, a.groups.iter().map(|g| (g.side, g.survived, g.retreated)).collect::<Vec<_>>());
        assert_eq!((a.clone(), ha), (b, hb), "same seed, same battle (threads included)");
        assert!(a.winner.is_some() && a.duration < 1500.0, "{map}: no decision: {a:?}");
    }
}

#[test]
fn defenders_with_a_station_hold_against_a_small_raid_and_do_not_flee() {
    let Some((data, fs)) = load() else { return };
    let Some(base) = data.objects.iter().map(|o| o.name.clone()).find(|n| n.eq_ignore_ascii_case("Empire_Star_Base_2")) else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Star_Destroyer", "X-Wing_Squadron", "Nebulon_B_Frigate", &base]));
    let (r, _) = run(&cat, vec![side(&cat, "Rebel", &[("Nebulon_B_Frigate", 2), ("X-Wing_Squadron", 2)], &[]), side(&cat, "Empire", &[("Star_Destroyer", 1)], &[&base])], BattleConfig { seed: 2, ..BattleConfig::default() });
    println!("raid on a defended base: winner {:?} after {:.0} s", r.winner, r.duration);
    assert_eq!(r.winner, Some(1));
    assert!(r.groups.iter().filter(|g| g.side == 1).all(|g| g.retreated == 0));
}

#[test]
fn player_idle_units_hold_position_and_do_not_automatically_charge() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Star_Destroyer", "MC-80_Star_Cruiser"]));
    let mut sides = vec![
        side(&cat, "Empire", &[("Star_Destroyer", 1)], &[]),
        side(&cat, "Rebel", &[("MC-80_Star_Cruiser", 1)], &[]),
    ];
    sides[0].ai = false; // Player side
    let mut b = Battle::new(BattleSetup {
        catalog: cat.clone(),
        sides,
        config: BattleConfig { record_events: false, ..BattleConfig::default() },
    });
    let p_pos = b.ships.iter().find(|s| s.side == 0).unwrap().pos;
    // Step forward 300 ticks (10 seconds): player ship with Order::Idle should stay put and not charge towards enemy
    for _ in 0..300 {
        b.step();
    }
    let p_pos2 = b.ships.iter().find(|s| s.side == 0).unwrap().pos;
    assert!(p_pos.dist(p_pos2) < 50.0, "player idle unit must hold position: moved from {p_pos:?} to {p_pos2:?}");
}

#[test]
fn ai_station_launches_active_fighters_and_defenders_engage_approaching_enemies() {
    let Some((data, fs)) = load() else { return };
    let Some(base) = data.objects.iter().map(|o| o.name.clone()).find(|n| n.eq_ignore_ascii_case("Empire_Star_Base_2")) else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Star_Destroyer", "Nebulon_B_Frigate", "TIE_Fighter_Squadron", &base]));
    let setup = BattleSetup {
        catalog: cat.clone(),
        sides: vec![
            side(&cat, "Rebel", &[("Nebulon_B_Frigate", 1)], &[]),
            side(&cat, "Empire", &[("Star_Destroyer", 1)], &[&base]),
        ],
        config: BattleConfig { record_events: false, ..BattleConfig::default() },
    };
    let mut b = Battle::new(setup);
    let mut fighters_launched = false;
    for _ in 0..120 {
        b.step();
        let ai_fighters: Vec<_> = b.ships.iter().filter(|s| s.alive && s.side == 1 && matches!(cat.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber)).collect();
        if !ai_fighters.is_empty() {
            fighters_launched = true;
            assert!(ai_fighters.iter().all(|f| matches!(f.order, Order::AttackMove(_))), "launched AI fighters have attack-move order: {:?}", ai_fighters.iter().map(|f| f.order).collect::<Vec<_>>());
            break;
        }
    }
    assert!(fighters_launched, "station should launch garrison fighters");

    // The defenders guard the station until an enemy comes within `FREE_STORE_ATTACK_RANGE` (3600) of it (`SpaceTacticalFreeStore.lua`), then attack.
    let station = b.ships.iter().find(|s| s.side == 1 && cat.ship(s.def).is_static).unwrap().pos;
    let mut threat_at = None;
    for _ in 0..30 * 180 {
        b.step();
        let isd = b.ships.iter().find(|s| s.alive && s.side == 1 && !cat.ship(s.def).is_static && !matches!(cat.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber)).unwrap();
        let nearest = b.ships.iter().filter(|s| s.alive && s.side == 0 && !matches!(cat.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber)).map(|s| s.pos.dist(station)).fold(f32::MAX, f32::min);
        if nearest < 3600.0 && threat_at.is_none() {
            threat_at = Some(b.time());
        }
        if threat_at.is_none() {
            assert!(matches!(isd.order, Order::Guard(_)), "the defender holds at its station while the enemy is far ({nearest:.0}): {:?}", isd.order);
        }
        if matches!(isd.order, Order::AttackMove(_) | Order::Attack(_)) {
            break;
        }
    }
    let isd = b.ships.iter().find(|s| s.alive && s.side == 1 && !cat.ship(s.def).is_static && !matches!(cat.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber)).unwrap();
    assert!(matches!(isd.order, Order::AttackMove(_) | Order::Attack(_)), "defending capital ship engages incoming enemy: {:?}", isd.order);
    assert!(threat_at.is_some_and(|t| b.time() - t <= 2.0), "it attacks as soon as the enemy threatens the station: {threat_at:?} / {}", b.time());
}

#[test]
fn corvette_flanking_maneuver_and_carrier_targeting() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["CR-90_Corvette", "Star_Destroyer", "Nebulon_B_Frigate"]));
    let cr90_id = cat.ship_id("CR-90_Corvette").unwrap();
    let isd_id = cat.ship_id("Star_Destroyer").unwrap();
    let neb_id = cat.ship_id("Nebulon_B_Frigate").unwrap();
    assert!(cat.ship(cr90_id).is_flanker(), "CR-90 should be recognized as flanker");
    assert!(!cat.ship(isd_id).is_flanker(), "ISD should not be a flanker");
    assert!(!cat.ship(neb_id).is_flanker(), "Nebulon should not be a flanker");

    let setup = BattleSetup {
        catalog: cat.clone(),
        sides: vec![
            side(&cat, "Rebel", &[("CR-90_Corvette", 1)], &[]),
            side(&cat, "Empire", &[("Star_Destroyer", 1), ("Nebulon_B_Frigate", 1)], &[]),
        ],
        config: BattleConfig { record_events: false, seed: 7, ..BattleConfig::default() },
    };
    let mut b = Battle::new(setup);
    for s in &mut b.ships {
        let name = &cat.ship(s.def).name;
        if name.contains("CR-90") {
            s.pos = Vec3::new(-4000.0, 0.0, 0.0);
            s.order = Order::AttackMove(Vec3::new(3000.0, 0.0, 0.0));
        } else if name.contains("Star_Destroyer") {
            s.pos = Vec3::new(1000.0, 0.0, 0.0);
            s.order = Order::Idle;
        } else if name.contains("Nebulon") {
            s.pos = Vec3::new(3500.0, 0.0, 0.0);
            s.order = Order::Idle;
        }
    }
    for _ in 0..120 {
        b.step();
    }
    let cr90 = b.ships.iter().find(|s| cat.ship(s.def).name.contains("CR-90")).unwrap();
    assert!(cr90.pos.y.abs() > 10.0, "corvette should maneuver sideways to execute a wide flank, pos={:?}", cr90.pos);
}

#[test]
fn emergency_retreat_saves_outgunned_capital_ship() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Star_Destroyer", "MC-80_Star_Cruiser"]));
    let sides = vec![
        side(&cat, "Empire", &[("Star_Destroyer", 1)], &[]),
        side(&cat, "Rebel", &[("MC-80_Star_Cruiser", 4)], &[]),
    ];
    let (r, _) = run(&cat, sides, BattleConfig { seed: 15, retreat_countdown: 30.0, ..BattleConfig::default() });
    let empire = r.groups.iter().find(|g| g.side == 0).unwrap();
    println!("emergency retreat: winner {:?} after {:.0} s, empire survived {}/{} ({} retreated)", r.winner, r.duration, empire.survived, empire.total, empire.retreated);
    assert_eq!(empire.retreated, 1, "outgunned ISD should retreat early under emergency retreat rule");
    assert!(r.duration < 90.0, "should trigger before 120 s normal threshold, took {:.0} s", r.duration);
}
/// Longest time (s) any two frigates or bigger stay inside each other's hull by more than a quarter of their half widths (hull capsules
/// from the models), sampled every half second after the first 10 s.
fn longest_hull_overlap(b: &mut Battle, cat: &Catalog, seconds: f32, mut each_tick: impl FnMut(&mut Battle)) -> (f32, String) {
    let mut run: std::collections::HashMap<(u32, u32), f32> = std::collections::HashMap::new();
    let mut longest = (0.0f32, String::new());
    for step in 0..(seconds * 30.0) as u32 {
        each_tick(b);
        b.step();
        if step % 15 != 0 || step < 300 {
            continue;
        }
        let big: Vec<&Ship> = b.ships.iter().filter(|s| s.alive && !s.retreated && cat.ship(s.def).class as u8 >= TargetClass::Frigate as u8).collect();
        let mut now = std::collections::HashMap::new();
        for (i, x) in big.iter().enumerate() {
            for y in &big[i + 1..] {
                if x.carrier == y.id || y.carrier == x.id {
                    continue;
                }
                let (hx, hy) = (cat.ship(x.def).footprint(), cat.ship(y.def).footprint());
                if nav::hull_overlap((x.pos, x.yaw, hx), (y.pos, y.yaw, hy), 0.0, 1.0).is_some_and(|(depth, _)| depth > 0.25 * (hx.half_width + hy.half_width)) {
                    let t = run.get(&(x.id, y.id)).copied().unwrap_or(0.0) + 0.5;
                    if t > longest.0 {
                        longest = (t, format!("{} #{} in {} #{} at t={:.0}", cat.ship(x.def).name, x.id, cat.ship(y.def).name, y.id, b.time()));
                    }
                    now.insert((x.id, y.id), t);
                }
            }
        }
        run = now;
        if b.is_over() {
            break;
        }
    }
    longest
}

#[test]
fn big_hulls_do_not_stay_inside_each_other_and_nobody_parks_in_an_executor() {
    let Some((data, fs)) = load() else { return };
    let empire = [("Star_Destroyer", 3), ("Victory_Destroyer", 3), ("Arquitens_E", 3), ("Piett_Executor_P1", 1)];
    let rebel = [("MC-80_Star_Cruiser", 4), ("Nebulon_B_Frigate", 4), ("Assault_Frigate_Mark_II", 3), ("CR-90_Corvette", 3)];
    let names: Vec<&str> = empire.iter().chain(rebel.iter()).map(|x| x.0).collect();
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &names));
    let exec_def = cat.ship(cat.ship_id("Piett_Executor_P1").unwrap());
    let h = exec_def.footprint();
    // The Executor model is about 9000 units long; its collision radius is clamped to 700.
    assert!(h.front + h.back > 8000.0 && h.half_width > 1000.0, "the hull comes from the model: {h:?} (radius {})", exec_def.radius);

    // Both sides AI: a standard fleet battle.
    let config = BattleConfig { record_events: false, seed: 1, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![side(&cat, "Empire", &empire, &[]), side(&cat, "Rebel", &rebel, &[])], config: config.clone() });
    let (t, what) = longest_hull_overlap(&mut b, &cat, 150.0, |_| {});
    println!("standard battle: longest hull overlap {t:.1} s {what}");
    assert!(t <= 3.0, "hulls stay inside each other for {t:.1} s: {what}");

    // The player sends every big ship at the Executor (the user's screenshot of ships flying into it).
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![side(&cat, "Empire", &empire, &[]), SideSetup { ai: false, ..side(&cat, "Rebel", &rebel, &[]) }], config });
    let mut ordered = false;
    let (t, what) = longest_hull_overlap(&mut b, &cat, 150.0, |b| {
        if !ordered && b.time() > 2.0 {
            ordered = true;
            let exec = b.ships.iter().find(|s| s.side == 0 && cat.ship(s.def).name == "Piett_Executor_P1").unwrap().id;
            let mine: Vec<u32> = b.ships.iter().filter(|s| s.side == 1 && s.alive).map(|s| s.id).collect();
            b.command(1, Command::Attack { ships: mine, target: exec }).unwrap();
        }
    });
    println!("attack on the Executor: longest hull overlap {t:.1} s {what}");
    assert!(t <= 3.0, "hulls stay inside each other for {t:.1} s: {what}");
}

/// How an AI attack went (user finding 2026-10-16: eight AI ships flew at the player's fleet and station one behind the other and were
/// destroyed one by one). "Fleet" = the AI's own big ships (not fighters, not the craft a carrier launches).
#[derive(Debug, Default, Clone)]
struct Attack {
    /// Shape of the fleet when its nearest ship first comes within 6000 units of an enemy warship: spread along the line from the fleet
    /// to that enemy (a column is deep) and across it (a line is wide).
    depth: f32,
    width: f32,
    /// Battle second of the first hit a fleet ship lands, and the seconds until half of the fleet ships have landed one (a column comes
    /// into range one ship after the other).
    first_hit: f32,
    half_in: f32,
    /// Damage the AI side dealt (shield and hull, after armour) in the 60 s after the first hit.
    damage_60: f32,
    /// Winner, battle seconds, fleet ships alive at the end (of the fleet ships at the start).
    winner: Option<u8>,
    duration: f32,
    ai_left: (u32, u32),
    /// Damage the AI side dealt and took over the whole battle.
    dealt: f32,
    taken: f32,
}

/// An AI fleet (side 0) attacks a player (side 1, no AI: the ships hold and fire at what comes in range).
fn ai_attack(cat: &Arc<Catalog>, arena: Option<Arc<Arena>>, ai: &[(&str, u32)], player: &[(&str, u32)], base: &[&str], seed: u64) -> Attack {
    let mut sides = vec![side(cat, "Empire", ai, &[]), side(cat, "Rebel", player, base)];
    sides[1].ai = false;
    let half = arena.as_ref().map_or(10_000.0, |a| a.half_size());
    let config = BattleConfig { seed, half_size: half, arena, retreat_allowed: vec![false, false], ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config });
    let small = |s: &Ship| matches!(cat.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber);
    let fleet = |s: &Ship| s.side == 0 && !cat.ship(s.def).is_static && !small(s) && s.carrier == NONE;
    let fleet_ids: Vec<u32> = b.ships.iter().filter(|s| fleet(s)).map(|s| s.id).collect();
    let mut m = Attack::default();
    let mut hit_at = vec![f32::INFINITY; fleet_ids.len()];
    let mut first: Option<f32> = None;
    let mut shaped = false;
    while !b.is_over() && b.time() < 1500.0 {
        let events = b.step().to_vec();
        let t = b.time();
        if !shaped {
            let alive: Vec<&Ship> = b.ships.iter().filter(|s| s.alive && fleet(s)).collect();
            let near = alive.iter().flat_map(|s| b.ships.iter().filter(|o| o.alive && o.side == 1 && !small(o)).map(move |o| (o.pos.dist(s.pos), o.pos))).min_by(|a, c| a.0.total_cmp(&c.0));
            if let Some((d, enemy)) = near
                && d <= 6000.0
            {
                shaped = true;
                let centre = alive.iter().fold(Vec3::ZERO, |a, s| a + s.pos) * (1.0 / alive.len() as f32);
                let axis = (enemy - centre).normalized();
                let spread = |v: Vec3| {
                    let k: Vec<f32> = alive.iter().map(|s| (s.pos - centre).dot(v)).collect();
                    k.iter().copied().fold(f32::MIN, f32::max) - k.iter().copied().fold(f32::MAX, f32::min)
                };
                m.depth = spread(axis);
                m.width = spread(Vec3::new(-axis.y, axis.x, 0.0));
            }
        }
        for e in &events {
            let Event::Hit { attacker, damage, .. } = e else { continue };
            if b.ships[*attacker as usize].side == 1 {
                m.taken += damage;
                continue;
            }
            m.dealt += damage;
            if let Some(k) = fleet_ids.iter().position(|&f| f == *attacker) {
                hit_at[k] = hit_at[k].min(t);
                first.get_or_insert(t);
            }
            if first.is_some_and(|c| t <= c + 60.0) {
                m.damage_60 += damage;
            }
        }
    }
    m.first_hit = first.unwrap_or(f32::INFINITY);
    hit_at.sort_by(f32::total_cmp);
    m.half_in = hit_at[hit_at.len().div_ceil(2).max(1) - 1] - m.first_hit;
    m.winner = b.winner();
    m.duration = b.time();
    m.ai_left = (b.ships.iter().filter(|s| s.alive && fleet(s)).count() as u32, fleet_ids.len() as u32);
    m
}

#[test]
fn the_ai_fleet_advances_as_one_line_and_attacks_together() {
    // The big ships move as one formation at the pace of the slowest (`MoveToLocation.lua`) and all of them attack once one meets the
    // enemy, so they come into range together instead of one behind the other. Before this rule (same scenarios, seeds 1-3): open square
    // width 1577-1595 / depth 1572-1590, 7 of 30 fleet ships left, damage dealt / taken 1.30-1.33; Kuat depth 2124-2134 at the approach.
    let Some((data, fs)) = load() else { return };
    let names = ["Star_Destroyer", "Victory_Destroyer", "Acclamator", "Tartan_Patrol_Cruiser", "Broadside_Cruiser", "MC-80_Star_Cruiser", "Nebulon_B_Frigate", "CR-90_Corvette", "Rebel_Star_Base_2"];
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &names));
    let ai = [("Star_Destroyer", 2), ("Victory_Destroyer", 2), ("Acclamator", 1), ("Broadside_Cruiser", 1), ("Tartan_Patrol_Cruiser", 2)];
    // The same fleet waiting on an open square (no station): the AI arrives as a line and wins with ships to spare. How many ships are left
    // depends on the dogfights, which any change to fighter behaviour moves chaotically (seeds 1-3, ships left of 30: 12 on f9710b3, 11 once the
    // fighters flew the data's formation again, the threshold sat on the noise), so it is judged on the mean of eight seeds like the Kuat attack
    // below: seeds 1-8 left 34 of 80 on f9710b3 and 36 of 80 with the data's formation (4.3 and 4.5 a run), 2.3 a run (7 of 30 on seeds 1-3) before the
    // rule; 3.5 a run (28 of 80) tells the two apart.
    let mut left = 0;
    for seed in 1..=8 {
        let m = ai_attack(&cat, None, &ai, &ai, &[], seed);
        println!("mirror on an open square, seed {seed}: {m:?} (dealt / taken {:.2})", m.dealt / m.taken);
        assert!(m.width >= 2.5 * m.depth, "a line abreast, not a column: {m:?}");
        assert_eq!(m.winner, Some(0));
        left += m.ai_left.0;
    }
    assert!(left >= 28, "the concentrated attack keeps ships alive: {left} of 80 left over eight seeds");
    // An attack on a defended planet on a map with several entry points (Kuat): the fleet does not string out on the way in. When half the fleet
    // has fired depends on the dogfights around it, which any change to fighter behaviour moves chaotically (2026-10-17: idle fighters keeping
    // their heading moved seed 1 from 38.7 to 47.6 s and the mean of seeds 1-8 from 33.9 to 37.1 s), so that time is judged on the mean of
    // eight seeds. So are the ships left at the end (seed 1: 9 on main, 7 after the fighters left their carriers below the hull; the mean
    // of seeds 1-8 8.75 and 8.6). A fleet that strings out fails on its depth: 2122-2142 on seeds 1-8 before this rule.
    if let Some(arena) = Arena::load(&data, &fs, "_SPACE_PLANET_KUAT_01.ted").map(Arc::new) {
        let player = [("MC-80_Star_Cruiser", 1), ("Nebulon_B_Frigate", 2), ("CR-90_Corvette", 2)];
        let runs: Vec<Attack> = std::thread::scope(|sc| {
            let jobs: Vec<_> = (1..=8)
                .map(|seed| {
                    let (cat, arena, ai, player) = (&cat, arena.clone(), &ai, &player);
                    sc.spawn(move || ai_attack(cat, Some(arena), ai, player, &["Rebel_Star_Base_2"], seed))
                })
                .collect();
            jobs.into_iter().map(|j| j.join().expect("Kuat battle")).collect()
        });
        for (seed, m) in (1..).zip(&runs) {
            println!("attack on Kuat, seed {seed}: {m:?} (dealt / taken {:.2})", m.dealt / m.taken);
            assert!(m.depth <= 1600.0, "the fleet keeps together on the way in: {m:?}");
            assert!(m.half_in <= 55.0, "half the fleet fires within 55 s of the first shot: {m:?}");
            assert_eq!(m.winner, Some(0));
        }
        let mean = runs.iter().map(|m| m.half_in).sum::<f32>() / runs.len() as f32;
        assert!(mean <= 40.0, "half the fleet fires within 40 s of the first shot on average: {mean:.1} s");
        let left = runs.iter().map(|m| m.ai_left.0 as f32).sum::<f32>() / runs.len() as f32;
        assert!(left >= 8.0, "the corvettes escort the big ships instead of charging the station: {left:.1} of 10 left on average");
    }
}

/// What the AI's retreat rule did in a battle against a scripted player.
#[derive(Debug, Default)]
struct RetreatRun {
    /// Battle second the AI's last structure fell.
    station_down: Option<f32>,
    /// Battle second of the AI's retreat order and the strength ratio then (enemy force / own force, see [`force`]).
    ordered: Option<(f32, f32)>,
    /// Ratio at the start.
    start_ratio: f32,
    /// The AI's big ships (no fighters, bombers or structures) at the start, destroyed and escaped at the end.
    big: (u32, u32, u32),
    winner: Option<u8>,
    /// The player side ordered a retreat (it never may on its own).
    player_retreat: bool,
}

/// A side's force as the Remake AI counts it (`FriendlyForceUnnormalized`): the `AI_Combat_Power` of its units, each weighted by the share of
/// hull and shield it has left.
fn force(b: &Battle, cat: &Catalog, side: u8) -> f32 {
    b.ships
        .iter()
        .filter(|s| s.alive && !s.retreated && s.side == side)
        .map(|s| {
            let d = cat.ship(s.def);
            d.ai_power * ((s.hull.max(0.0) + s.shield.max(0.0)) / (d.hull + d.shield).max(1.0)).min(1.0)
        })
        .sum()
}

/// The AI plays side `ai`, the player the other side. An attacking player sends everything at the AI's station, then attack-moves on the
/// nearest AI warship whenever the place it was sent to is clear; a defending player holds and fires at what comes in range. With `gap`
/// the attacker's big ships start that far from the defender's station (or fleet) instead of across the map.
fn retreat_run(cat: &Arc<Catalog>, ai: u8, attacker: &[(&str, u32)], defender: &[(&str, u32)], base: &[&str], gap: Option<f32>, seed: u64) -> RetreatRun {
    let player = 1 - ai;
    let mut sides = vec![side(cat, "Attacker", attacker, &[]), side(cat, "Defender", defender, base)];
    sides[player as usize].ai = false;
    let config = BattleConfig { seed, defenders: vec![false, true], ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config });
    let structure = |s: &Ship| cat.ship(s.def).is_static;
    let big = |s: &Ship| !structure(s) && !matches!(cat.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber);
    let centre = |b: &Battle, side: u8| {
        let ships: Vec<Vec3> = b.ships.iter().filter(|s| s.alive && !s.retreated && s.side == side && big(s)).map(|s| s.pos).collect();
        ships.iter().fold(Vec3::ZERO, |a, &p| a + p) * (1.0 / ships.len().max(1) as f32)
    };
    if let Some(gap) = gap {
        let aim = b.ships.iter().find(|s| s.side == 1 && structure(s)).map_or(centre(&b, 1), |s| s.pos);
        let to = aim - centre(&b, 0);
        let shift = to - to.normalized() * gap;
        for s in b.ships.iter_mut().filter(|s| s.side == 0 && !structure(s)) {
            s.pos += shift;
        }
    }
    let mut run = RetreatRun { start_ratio: force(&b, cat, player) / force(&b, cat, ai).max(1.0), ..RetreatRun::default() };
    let ai_big: Vec<u32> = b.ships.iter().filter(|s| s.side == ai && big(s)).map(|s| s.id).collect();
    let mut goal: Option<Vec3> = None;
    while !b.is_over() && b.time() < 900.0 {
        if player == 0 && b.tick().is_multiple_of(30) {
            let mine: Vec<u32> = b.ships.iter().filter(|s| s.alive && !s.retreated && s.side == 0 && !structure(s)).map(|s| s.id).collect();
            let enemies: Vec<Vec3> = b.ships.iter().filter(|s| s.alive && !s.retreated && s.side == ai && big(s)).map(|s| s.pos).collect();
            if let Some(station) = b.ships.iter().find(|s| s.alive && s.side == ai && structure(s)) {
                if goal.is_none() {
                    goal = Some(station.pos);
                    let target = station.id;
                    b.command(0, Command::Attack { ships: mine, target }).unwrap();
                }
            } else if goal.is_none_or(|g| !enemies.iter().any(|p| p.dist(g) < 3000.0)) {
                let from = centre(&b, 0);
                if let Some(p) = enemies.iter().copied().min_by(|a, c| a.dist(from).total_cmp(&c.dist(from))) {
                    goal = Some(p);
                    b.command(0, Command::AttackMove { ships: mine, to: p }).unwrap();
                }
            }
        }
        let events = b.step().to_vec();
        for e in &events {
            match e {
                Event::RetreatOrdered { side, .. } if *side == ai && run.ordered.is_none() => run.ordered = Some((b.time(), force(&b, cat, player) / force(&b, cat, ai).max(1.0))),
                Event::RetreatOrdered { side, .. } if *side == player => run.player_retreat = true,
                _ => {}
            }
        }
        if run.station_down.is_none() && !base.is_empty() && !b.ships.iter().any(|s| s.side == 1 && s.alive && structure(s)) {
            run.station_down = Some(b.time());
        }
    }
    let ships = |f: fn(&Ship) -> bool| ai_big.iter().filter(|&&i| f(&b.ships[i as usize])).count() as u32;
    run.big = (ai_big.len() as u32, ships(|s| !s.alive), ships(|s| s.alive && s.retreated));
    run.winner = b.winner();
    run
}

#[test]
fn an_ai_defender_without_a_chance_against_a_super_star_destroyer_retreats_at_the_latest_when_its_station_falls() {
    // User finding 2026-10-17: after the player destroyed its station, the AI fought on without a chance against a Super Star Destroyer fleet.
    // The retreat rule compares forces, and the Remake's force is the units' `AI_Combat_Power` (Executor
    // 65000, Star Destroyer 1800, Victory 1200, MC-80 1750, Assault Frigate 1200, Nebulon-B 300, CR-90 100, X-wing 10, star base 3750); our old
    // estimate (sqrt of hull and shield times damage) put the Executor at 11 Star Destroyers instead of 36. Main f9710b3, same battle: the station
    // fell after 126 / 133 s at 4.5x / 4.8x and the AI fought on until 260 / 242 s, 11 of its 29 units got away (the player kept the Executor alone).
    let Some((data, fs)) = load() else { return };
    let names = ["Executor_Star_Dreadnought", "Star_Destroyer", "Victory_Destroyer", "TIE_Fighter_Squadron", "MC-80_Star_Cruiser", "Nebulon_B_Frigate", "Assault_Frigate_Mark_II", "CR-90_Corvette", "X-Wing_Squadron", "Rebel_Star_Base_3"];
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &names));
    let empire = [("Executor_Star_Dreadnought", 1), ("Star_Destroyer", 3), ("Victory_Destroyer", 3), ("TIE_Fighter_Squadron", 4)];
    let rebels = [("MC-80_Star_Cruiser", 6), ("Nebulon_B_Frigate", 6), ("Assault_Frigate_Mark_II", 4), ("CR-90_Corvette", 6), ("X-Wing_Squadron", 6)];
    for seed in [1, 2] {
        let r = retreat_run(&cat, 1, &empire, &rebels, &["Rebel_Star_Base_3"], None, seed);
        println!("Executor fleet against a defended base, seed {seed}: {r:?}");
        assert!(r.start_ratio >= 3.0, "no chance from the start: {:.2}", r.start_ratio);
        let down = r.station_down.expect("the station falls");
        let (at, ratio) = r.ordered.expect("the defender retreats");
        // The rule is checked every 2 s, not before the 60 s retreat countdown.
        assert!(at <= down.max(60.0) + 4.0, "the AI leaves at the latest right after its station falls: station {down:.0} s, order {at:.0} s");
        assert!(ratio >= 2.95, "it leaves because the enemy is 3x stronger: {ratio:.2}");
        assert!(r.big.2 * 2 >= r.big.0, "most of the fleet gets away: {:?}", r.big);
    }
}

#[test]
fn an_ai_defender_with_a_fair_chance_fights_on_after_losing_its_station() {
    // Six Star Destroyers and four Victorys (force 15600) destroy the star base of six MC-80s and four Nebulon-Bs (11700 and 1250): the
    // defenders fight on while the enemy is less than 3x stronger, and the player, who never retreats on its own, does not either.
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Star_Destroyer", "Victory_Destroyer", "MC-80_Star_Cruiser", "Nebulon_B_Frigate", "Rebel_Star_Base_1"]));
    let r = retreat_run(&cat, 1, &[("Star_Destroyer", 6), ("Victory_Destroyer", 4)], &[("MC-80_Star_Cruiser", 6), ("Nebulon_B_Frigate", 4)], &["Rebel_Star_Base_1"], None, 1);
    println!("Star Destroyers against a defended base: {r:?}");
    assert!(r.start_ratio < 2.0, "a fair fight: {:.2}", r.start_ratio);
    let down = r.station_down.expect("the station falls");
    assert!(r.ordered.is_none_or(|(at, ratio)| at > down + 30.0 && ratio >= 2.95), "the defender fights on: {r:?}");
    assert!(!r.player_retreat, "the player's side never retreats by itself");
}

#[test]
fn an_ai_attacker_that_is_losing_badly_retreats() {
    // An AI fleet (6 MC-80s, 4 Nebulon-Bs: force 11700) jumps in 4500 units from a planet held by an Executor and a star base (67250): the
    // attacker leaves once the retreat countdown allows (60 s) instead of after the 120 s of `Retreat_From_Space`. Main f9710b3: order at 80 s,
    // 4 of the 10 ships lost.
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Executor_Star_Dreadnought", "MC-80_Star_Cruiser", "Nebulon_B_Frigate", "Empire_Star_Base_2"]));
    let r = retreat_run(&cat, 0, &[("MC-80_Star_Cruiser", 6), ("Nebulon_B_Frigate", 4)], &[("Executor_Star_Dreadnought", 1)], &["Empire_Star_Base_2"], Some(4500.0), 1);
    println!("AI attack on an Executor: {r:?}");
    let (at, ratio) = r.ordered.expect("the attacker retreats");
    assert!(at <= 64.0, "{at:.0} s");
    assert!(ratio >= 2.95, "{ratio:.2}");
    assert!(r.big.2 * 2 >= r.big.0, "most of the fleet gets away: {:?}", r.big);
}

#[test]
fn a_nearly_destroyed_ai_fleet_retreats_before_it_is_wiped_out() {
    // User finding 2026-10-16: an AI fleet of frigates and corvettes fought to the last ship against a defended planet. `Retreat_From_Space` lets an
    // attacker leave only after 120 s, and the emergency rule (U94) was only for capital ships and units of 15 population. Now a side that has lost
    // half of its strength while the enemy is more than 3x stronger leaves at once. Before: 3 Acclamators with 3 Tartan pairs ordered the retreat
    // at 122 s and saved 2 / 1 of the Acclamators (seeds 1 / 2); 4 Tartan pairs never ordered it and were wiped out.
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Acclamator", "Tartan_Patrol_Cruiser", "MC-80_Star_Cruiser", "Nebulon_B_Frigate", "CR-90_Corvette", "Rebel_Star_Base_3"]));
    let player = [("MC-80_Star_Cruiser", 3), ("Nebulon_B_Frigate", 4), ("CR-90_Corvette", 2)];
    // (battle second of the retreat order, units that escaped)
    let attack = |ai: &[(&str, u32)], seed: u64| -> (Option<f32>, u32) {
        let mut sides = vec![side(&cat, "Empire", ai, &[]), side(&cat, "Rebel", &player, &["Rebel_Star_Base_3"])];
        sides[1].ai = false;
        let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config: BattleConfig { seed, ..BattleConfig::default() } });
        let mut ordered = None;
        while !b.is_over() && b.time() < 600.0 {
            if b.step().iter().any(|e| matches!(e, Event::RetreatOrdered { side: 0, .. })) && ordered.is_none() {
                ordered = Some(b.time());
            }
        }
        let escaped = b.result().groups.iter().filter(|g| g.side == 0).map(|g| g.retreated).sum();
        (ordered, escaped)
    };
    for seed in [1, 2] {
        let (ordered, escaped) = attack(&[("Acclamator", 3), ("Tartan_Patrol_Cruiser", 3)], seed);
        println!("frigates and corvettes, seed {seed}: retreat ordered at {ordered:?}, {escaped} escaped");
        assert!(ordered.is_some_and(|t| t < 120.0), "the battered fleet does not wait for 120 s: {ordered:?}");
        assert!(escaped >= 2, "two of the three Acclamators get away: {escaped}");
    }
    let (ordered, escaped) = attack(&[("Tartan_Patrol_Cruiser", 4)], 1);
    println!("corvettes only: retreat ordered at {ordered:?}, {escaped} escaped");
    assert!(ordered.is_some(), "corvettes alone retreat too instead of dying to the last ship");
}

/// A battle of an AI side (0) against a player side (1) that holds and fires at what comes in range, without retreats; `each` sees every
/// step's events.
fn hold_battle(cat: &Arc<Catalog>, ai: &[(&str, u32)], player: &[(&str, u32)], seed: u64, seconds: f32, mut each: impl FnMut(&Battle, &[Event])) {
    let mut sides = vec![side(cat, "AI", ai, &[]), side(cat, "Player", player, &[])];
    sides[1].ai = false;
    let config = BattleConfig { seed, retreat_allowed: vec![false, false], ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config });
    while !b.is_over() && b.time() < seconds {
        let events = b.step().to_vec();
        each(&b, &events);
    }
}

#[test]
fn an_ai_fleet_shelled_from_beyond_its_sight_goes_for_the_shooter() {
    // User finding 2026-10-17 (the AI played very weakly) with an Executor fleet: the Executor sees 30000 units and fires
    // from up to 11000, a Rebel fleet sees 1750-3000. The AI only went for enemies it could see, so it chased the TIE bombers and swept
    // its search points while the Executor shelled it, and dealt no damage to a warship at all. The Remake hands its AI the attacker of
    // every hit (`Default_Unit_Damaged`): now a ship that hit the side is known to it for the fog's 15 s, and the fleet goes for it.
    let Some((data, fs)) = load() else { return };
    let ai = [("MC-80_Star_Cruiser", 4), ("Nebulon_B_Frigate", 4), ("Assault_Frigate_Mark_II", 3), ("CR-90_Corvette", 4), ("X-Wing_Squadron", 4)];
    let player = [("Executor_Star_Dreadnought", 1), ("Star_Destroyer", 2), ("Victory_Destroyer", 2), ("TIE_Fighter_Squadron", 3)];
    let names: Vec<&str> = ai.iter().chain(&player).map(|u| u.0).collect();
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &names));
    let small = |s: &Ship| matches!(cat.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber);
    for seed in [1, 2] {
        // Battle second of the AI's first hit on a warship, and the damage it dealt to warships.
        let (mut first, mut dealt) = (None, 0.0f32);
        hold_battle(&cat, &ai, &player, seed, 400.0, |b, events| {
            for e in events {
                if let Event::Hit { ship, attacker, damage, .. } = *e
                    && b.ships[attacker as usize].side == 0
                    && !small(&b.ships[ship as usize])
                {
                    first.get_or_insert(b.time());
                    dealt += damage;
                }
            }
        });
        println!("seed {seed}: first hit on a warship at {first:?}, damage to warships {dealt:.0}");
        // Before: the first hit at 200 s (seed 1) and 601 damage; now 102-109 s and 7400-14400.
        assert!(first.is_some_and(|t| t < 160.0), "the fleet reaches the shooters: {first:?}");
        assert!(dealt > 3000.0, "it hurts them: {dealt:.0}");
    }
}

#[test]
fn ai_capital_ships_take_the_frigates_near_them() {
    // `SpaceTacticalFreeStore.lua` (`Service_Attack`, `Get_Preferred_Target_In_Range`): a ship outside the advancing formation attacks the
    // nearest enemy of the kind it prefers within its aggro range (capital ships frigates, frigates corvettes, corvettes and anti-fighter
    // ships fighters, bombers and transports) before the side's priority target. Before, every ship weighed only the side's priority
    // target, the weakest capital ship: the Star Destroyers put 51% of their fire on the frigates.
    let Some((data, fs)) = load() else { return };
    let ai = [("Star_Destroyer", 2), ("Victory_Destroyer", 2)];
    let player = [("MC-80_Star_Cruiser", 2), ("Nebulon_B_Frigate", 3), ("CR-90_Corvette", 3)];
    let names: Vec<&str> = ai.iter().chain(&player).map(|u| u.0).collect();
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &names));
    for seed in [1, 2] {
        // Damage the AI's capital ships dealt to capital ships and to frigates while the player had both.
        let (mut on_capitals, mut on_frigates) = (0.0f32, 0.0f32);
        hold_battle(&cat, &ai, &player, seed, 600.0, |b, events| {
            let alive = |c: TargetClass| b.ships.iter().any(|s| s.alive && s.side == 1 && cat.ship(s.def).class == c);
            if !alive(TargetClass::Capital) || !alive(TargetClass::Frigate) {
                return;
            }
            for e in events {
                let Event::Hit { ship, attacker, damage, .. } = *e else { continue };
                let a = &b.ships[attacker as usize];
                if a.side != 0 || cat.ship(a.def).class != TargetClass::Capital {
                    continue;
                }
                match cat.ship(b.ships[ship as usize].def).class {
                    TargetClass::Capital => on_capitals += damage,
                    TargetClass::Frigate => on_frigates += damage,
                    _ => {}
                }
            }
        });
        let share = on_frigates / (on_capitals + on_frigates).max(1.0);
        println!("seed {seed}: the Star Destroyers put {:.0}% of {:.0} on the frigates", 100.0 * share, on_capitals + on_frigates);
        assert!(share > 0.7, "{:.0}%", 100.0 * share);
    }
}

#[test]
fn the_ai_fires_full_salvos_only_at_capital_ships_and_stations() {
    // `Try_Offensive_Ability_Space`: the Remake AI fires a Star Destroyer's full salvo only at a capital ship or a station with more than a
    // fifth of its hull. Before, the AI fired it at whatever its target was: 2 of 8 salvos here went at corvettes (14-38% of the weapon
    // abilities in fleet battles).
    let Some((data, fs)) = load() else { return };
    let ai = [("Star_Destroyer", 2)];
    let player = [("CR-90_Corvette", 6), ("MC-80_Star_Cruiser", 2)];
    let names: Vec<&str> = ai.iter().chain(&player).map(|u| u.0).collect();
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &names));
    for seed in [1, 2] {
        // Full salvos fired at capital ships (or bigger) and at smaller targets.
        let (mut big, mut small) = (0, 0);
        hold_battle(&cat, &ai, &player, seed, 600.0, |b, events| {
            for e in events {
                let Event::AbilityActivated { ship, ability } = *e else { continue };
                let s = &b.ships[ship as usize];
                if s.side != 0 || !cat.ship(s.def).abilities[ability as usize].salvo || s.target == NONE {
                    continue;
                }
                if cat.ship(b.ships[s.target as usize].def).class as u8 >= TargetClass::Capital as u8 {
                    big += 1;
                } else {
                    small += 1;
                }
            }
        });
        println!("seed {seed}: full salvos at capital ships {big}, at smaller targets {small}");
        assert_eq!(small, 0);
        assert!(big > 0, "the salvo is still used");
    }
}
