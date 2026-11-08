//! Chase ranges, target stickiness and guard orders against the real Remake data. Skipped when the game is not installed.
//!
//! Before these rules an idle squadron never left its place, an attack-move took up any enemy it saw, a target was replaced by a
//! better one at the next scan (every 0.4 s) and a guard pursued enemies up to 4500 from the guarded place.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use battle_sim::*;

fn load() -> Option<(alamo_data::GameData, GameFs)> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    let data = load_galaxy(&fs).0;
    Some((data, fs))
}

/// A battle without AI and fog: side 0 holds `own`, side 1 `enemy`, one of each listed unit.
fn battle(data: &alamo_data::GameData, fs: &GameFs, own: &[&str], enemy: &[&str]) -> Battle {
    let names: Vec<&str> = own.iter().chain(enemy).copied().collect();
    let cat = Arc::new(Catalog::build(data, Some(fs as &dyn FileSource), &names));
    let unit = |n: &&str| UnitSetup { ship: cat.ship_id(n).unwrap_or_else(|| panic!("{n}")), count: 1, tag: 1 };
    let sides = vec![
        SideSetup { name: "Empire".into(), units: own.iter().map(unit).collect(), structures: vec![], ai: false },
        SideSetup { name: "Rebel".into(), units: enemy.iter().map(unit).collect(), structures: vec![], ai: false },
    ];
    Battle::new(BattleSetup { catalog: cat, sides, config: BattleConfig { seed: 3, fog_of_war: false, ..BattleConfig::default() } })
}

/// Puts every ship of `def` (squadron members and launched craft included) at `at`, keeping their offsets to the first one.
fn place(b: &mut Battle, side: u8, def: &str, at: Vec3) -> Vec<u32> {
    let id = b.catalog().ship_id(def).unwrap();
    let cat = b.catalog().clone();
    let members: Vec<ShipId> = cat.ship(id).members.iter().map(|m| m.0).collect();
    let ids: Vec<u32> = b.ships.iter().filter(|s| s.side == side && (s.def == id || members.contains(&s.def))).map(|s| s.id).collect();
    let first = b.ships[ids[0] as usize].pos;
    for &i in &ids {
        let s = &mut b.ships[i as usize];
        s.pos = at + (s.pos - first);
        s.rest_at = s.pos;
    }
    ids
}

#[test]
fn chase_rules_come_from_vessel_base_and_the_squadron() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs as &dyn FileSource), &["Victory_Destroyer", "TIE_Fighter_Squadron"]);
    let vsd = cat.ship(cat.ship_id("Victory_Destroyer").unwrap()).chase;
    assert_eq!((vsd.idle, vsd.guard, vsd.attack_move, vsd.stickiness), (Some(0.0), Some(1000.0), Some(400.0), Some(5.0)), "Vessel_Base");
    let squadron = cat.ship(cat.ship_id("TIE_Fighter_Squadron").unwrap());
    let tie = cat.ship(squadron.members[0].0).chase;
    // The squadron's 1500 ranges, the craft's own stickiness (`Spacecraft_Base` 5).
    assert_eq!((tie.idle, tie.guard, tie.attack_move, tie.stickiness), (Some(1500.0), Some(1500.0), Some(1500.0), Some(5.0)));
}

#[test]
fn a_target_is_kept_five_seconds_before_a_closer_one_takes_over() {
    let Some((data, fs)) = load() else { return };
    let mut b = battle(&data, &fs, &["Victory_Destroyer"], &["Nebulon_B_Frigate", "Nebulon_B_Frigate"]);
    let me = place(&mut b, 0, "Victory_Destroyer", Vec3::new(0.0, 0.0, 0.0))[0];
    let enemies: Vec<u32> = b.ships.iter().filter(|s| s.side == 1).map(|s| s.id).collect();
    let (far, near) = (enemies[0], enemies[1]);
    b.ships[far as usize].pos = Vec3::new(2500.0, 0.0, 0.0);
    b.ships[near as usize].pos = Vec3::new(0.0, 30000.0, 0.0);
    for s in b.ships.iter_mut() {
        s.stance = if s.side == 0 { Stance::Aggressive } else { Stance::HoldFire };
    }
    for _ in 0..30 {
        b.step();
    }
    assert_eq!(b.ships[me as usize].target, far, "the only enemy in reach");
    // A second, equally ranked enemy appears much closer.
    b.ships[near as usize].pos = Vec3::new(-900.0, 0.0, 0.0);
    let mut kept = 0;
    while b.ships[me as usize].target == far && kept < 30 * 20 {
        b.step();
        kept += 1;
    }
    eprintln!("kept the first target {kept} frames after a closer one appeared");
    assert_eq!(b.ships[me as usize].target, near);
    // The first target was taken up after about 1 s: it is kept until it has been the target for 5 s (about 4 s more), up to one 0.4 s scan later.
    assert!((100..=150).contains(&kept), "{kept} frames, was at most 12 (one scan)");
}

#[test]
fn an_idle_squadron_chases_within_its_idle_chase_range_and_a_capital_holds() {
    let Some((data, fs)) = load() else { return };
    let mut b = battle(&data, &fs, &["TIE_Fighter_Squadron", "Victory_Destroyer"], &["Nebulon_B_Frigate", "Nebulon_B_Frigate"]);
    let ties = place(&mut b, 0, "TIE_Fighter_Squadron", Vec3::new(0.0, 0.0, 0.0));
    let vsd = place(&mut b, 0, "Victory_Destroyer", Vec3::new(0.0, -20000.0, 0.0))[0];
    let enemies: Vec<u32> = b.ships.iter().filter(|s| s.side == 1).map(|s| s.id).collect();
    // One frigate within 1500 + the TIE's range of the squadron's place, one 5000 from the Victory (beyond its 0 idle chase plus range).
    b.ships[enemies[0] as usize].pos = Vec3::new(1500.0, 0.0, 0.0);
    b.ships[enemies[1] as usize].pos = Vec3::new(0.0, -25000.0, 0.0);
    for &e in &enemies {
        b.ships[e as usize].stance = Stance::HoldFire;
    }
    let vsd_start = b.ships[vsd as usize].pos;
    let lead_start = b.ships[ties[0] as usize].pos;
    let mut closest = f32::MAX;
    for _ in 0..30 * 20 {
        b.step();
        closest = closest.min(b.ships[ties[0] as usize].pos.dist(b.ships[enemies[0] as usize].pos));
    }
    let moved = b.ships[vsd as usize].pos.dist(vsd_start);
    eprintln!("TIE leader came within {closest:.0} of the frigate (started {:.0} away); Victory moved {moved:.0}", lead_start.dist(Vec3::new(1500.0, 0.0, 0.0)));
    assert!(closest < 800.0, "the idle squadron attacks the frigate in its chase range");
    assert!(moved < 50.0, "Idle_Chase_Range 0: the Victory holds");
}

#[test]
fn an_attack_move_takes_up_only_enemies_within_its_response_range() {
    let Some((data, fs)) = load() else { return };
    let mut b = battle(&data, &fs, &["Victory_Destroyer"], &["Nebulon_B_Frigate"]);
    let me = place(&mut b, 0, "Victory_Destroyer", Vec3::new(0.0, 0.0, 0.0))[0];
    let enemy = b.ships.iter().find(|s| s.side == 1).unwrap().id;
    let cat = b.catalog().clone();
    let d = cat.ship(b.ships[me as usize].def);
    let reach = d.max_weapon_range.max(d.targeting_range);
    // 1500 beyond the Victory's reach, to the side of its way.
    let off = reach + 1500.0;
    b.ships[enemy as usize].pos = Vec3::new(0.0, off, 0.0);
    b.ships[enemy as usize].stance = Stance::HoldFire;
    b.command(0, Command::AttackMove { ships: vec![me], to: Vec3::new(-20000.0, 0.0, 0.0) }).unwrap();
    for _ in 0..30 * 10 {
        b.step();
    }
    eprintln!("reach {reach}, enemy {off} away: candidates {:?}", b.ships[me as usize].candidates);
    assert!(!b.ships[me as usize].candidates.contains(&enemy), "beyond Attack_Move_Response_Range 400 + reach");
    assert!(b.ships[me as usize].pos.y < 500.0, "it does not turn towards the frigate");
    // Within reach + 400 it is taken up.
    let at = b.ships[me as usize].pos + Vec3::new(0.0, reach + 300.0, 0.0);
    b.ships[enemy as usize].pos = at;
    for _ in 0..30 {
        b.step();
    }
    assert!(b.ships[me as usize].candidates.contains(&enemy), "within reach + 400");
}

#[test]
fn a_guard_pursues_no_further_than_its_guard_chase_range() {
    let Some((data, fs)) = load() else { return };
    let mut b = battle(&data, &fs, &["Victory_Destroyer"], &["Nebulon_B_Frigate"]);
    let me = place(&mut b, 0, "Victory_Destroyer", Vec3::new(0.0, 0.0, 0.0))[0];
    let enemy = b.ships.iter().find(|s| s.side == 1).unwrap().id;
    b.ships[enemy as usize].pos = Vec3::new(4000.0, 0.0, 0.0);
    b.ships[enemy as usize].stance = Stance::HoldFire;
    b.ships[me as usize].order = Order::Guard(Vec3::new(0.0, 0.0, 0.0));
    let mut farthest = 0.0f32;
    for _ in 0..30 * 60 {
        b.step();
        if b.ships[enemy as usize].alive {
            farthest = farthest.max(b.ships[me as usize].pos.len());
        }
    }
    eprintln!("guard went {farthest:.0} from its place, target {}", b.ships[me as usize].target);
    assert!(farthest > 300.0, "it closes on the enemy");
    assert!(farthest <= 1000.0 + 150.0, "Guard_Chase_Range 1000 (plus the arrival tolerance), was up to the standoff of an enemy 4500 out");
}

#[test]
fn a_guard_follows_its_ship_and_holds_where_it_was_lost() {
    let Some((data, fs)) = load() else { return };
    let mut b = battle(&data, &fs, &["Victory_Destroyer", "Star_Destroyer"], &["Nebulon_B_Frigate"]);
    assert_eq!(b.catalog().space_guard_range, 2000.0, "Gameconstants.xml Space_Guard_Range");
    let guard = place(&mut b, 0, "Victory_Destroyer", Vec3::new(-4000.0, 0.0, 0.0))[0];
    let isd = place(&mut b, 0, "Star_Destroyer", Vec3::new(0.0, 0.0, 0.0))[0];
    let enemy = b.ships.iter().find(|s| s.side == 1).unwrap().id;
    b.ships[enemy as usize].pos = Vec3::new(0.0, 30000.0, 0.0);
    b.ships[enemy as usize].stance = Stance::HoldFire;
    assert_eq!(b.command(0, Command::Guard { ships: vec![guard], target: GuardTarget::Ship(enemy) }), Err(CommandError::InvalidShip), "not an enemy");
    assert_eq!(b.command(0, Command::Guard { ships: vec![isd], target: GuardTarget::Ship(isd) }), Err(CommandError::InvalidShip), "not itself");
    b.command(0, Command::Guard { ships: vec![guard], target: GuardTarget::Ship(isd) }).unwrap();
    assert_eq!(b.ships[guard as usize].order, Order::GuardShip(isd));
    // The guarded ship moves away; the guard follows it.
    b.command(0, Command::Move { ships: vec![isd], to: Vec3::new(0.0, -8000.0, 0.0) }).unwrap();
    for _ in 0..30 * 120 {
        b.step();
    }
    let apart = b.ships[guard as usize].pos.dist(b.ships[isd as usize].pos);
    eprintln!("guard {apart:.0} from the Star Destroyer after it moved 8000");
    assert!(apart < 2500.0, "follows: {apart:.0}");
    // The guarded ship dies: the guard holds where it was.
    let at = b.ships[isd as usize].pos;
    b.ships[isd as usize].alive = false;
    b.step();
    assert_eq!(b.ships[guard as usize].order, Order::Guard(at));
}

#[test]
fn a_guarding_squadron_takes_up_an_enemy_beyond_its_reach_within_space_guard_range() {
    let Some((data, fs)) = load() else { return };
    let mut b = battle(&data, &fs, &["TIE_Fighter_Squadron", "Star_Destroyer"], &["Nebulon_B_Frigate"]);
    let ties = place(&mut b, 0, "TIE_Fighter_Squadron", Vec3::new(-1500.0, 0.0, 0.0));
    let isd = place(&mut b, 0, "Star_Destroyer", Vec3::new(0.0, 0.0, 0.0))[0];
    let enemy = b.ships.iter().find(|s| s.side == 1).unwrap().id;
    b.ships[enemy as usize].pos = Vec3::new(1900.0, 0.0, 0.0);
    b.ships[enemy as usize].stance = Stance::HoldFire;
    let cat = b.catalog().clone();
    let d = cat.ship(b.ships[ties[0] as usize].def);
    let reach = (d.max_weapon_range.max(d.targeting_range) * 1.25).max(400.0);
    eprintln!("TIE reach {reach}, frigate 3400 from the squadron, 1900 from the Star Destroyer");
    assert!(3400.0 > reach + 1000.0);
    b.command(0, Command::Guard { ships: ties.clone(), target: GuardTarget::Ship(isd) }).unwrap();
    for _ in 0..30 {
        b.step();
    }
    assert!(ties.iter().all(|&t| b.ships[t as usize].candidates.contains(&enemy)), "within Space_Guard_Range 2000 of the guarded ship");
}
