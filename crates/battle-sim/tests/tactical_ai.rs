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
    // Seed 16: most seeds of a 3 v 3 end with the last ship pulling out at 1 v 2 (the 2x rule at work); this one is fought out.
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Star_Destroyer", "MC-80_Star_Cruiser"]));
    let (r, _) = run(&cat, vec![side(&cat, "Empire", &[("Star_Destroyer", 3)], &[]), side(&cat, "Rebel", &[("MC-80_Star_Cruiser", 3)], &[])], BattleConfig { seed: 16, ..BattleConfig::default() });
    println!("even fight: winner {:?} after {:.0} s, groups {:?}", r.winner, r.duration, r.groups.iter().map(|g| (g.side, g.survived, g.retreated)).collect::<Vec<_>>());
    assert!(r.groups.iter().all(|g| g.retreated == 0), "an even fight is fought out");
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