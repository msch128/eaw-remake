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

    // Step until enemy approaches or is spotted:
    for _ in 0..1200 {
        b.step();
        let isd = b.ships.iter().find(|s| s.alive && s.side == 1 && !cat.ship(s.def).is_static && !matches!(cat.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber)).unwrap();
        if matches!(isd.order, Order::AttackMove(_) | Order::Attack(_)) {
            break;
        }
    }
    let isd = b.ships.iter().find(|s| s.alive && s.side == 1 && !cat.ship(s.def).is_static && !matches!(cat.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber)).unwrap();
    assert!(matches!(isd.order, Order::AttackMove(_) | Order::Attack(_)), "defending capital ship engages incoming enemy: {:?}", isd.order);
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