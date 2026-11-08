//! Passive effects in space battles: passive missile shields and jamming pods. Rule tests with hand-made definitions, plus checks
//! against the real Remake data (skipped when the game is not installed).

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

fn missile() -> ProjDef {
    ProjDef {
        name: "missile".into(),
        category: "missile".into(),
        damage: 100.0,
        speed: 500.0,
        max_distance: 9000.0,
        shield_damage: true,
        hull_damage: true,
        ..ProjDef::default()
    }
}

fn ship(name: &str, hull: f32) -> ShipDef {
    ShipDef {
        name: name.into(),
        class: TargetClass::Frigate,
        is_static: true,
        hull,
        radius: 100.0,
        population: 5,
        ..ShipDef::default()
    }
}

fn setup(catalog: Catalog, a: &str, b: &str) -> BattleSetup {
    let catalog = Arc::new(catalog);
    let side = |n: &str| SideSetup { name: n.into(), units: vec![UnitSetup { ship: catalog.ship_id(n).unwrap(), count: 1, tag: 7 }], structures: vec![], ai: false };
    BattleSetup { sides: vec![side(a), side(b)], catalog, config: BattleConfig { half_size: 6000.0, fog_of_war: false, ..BattleConfig::default() } }
}

/// Hull plus shield of the target after a missile flew straight at it from 1500 units for `secs` seconds (`delay` seconds after
/// `prepare` ran).
fn missile_at(target: ShipDef, prepare: impl Fn(&mut Battle, u32), delay: f32, secs: f32) -> f32 {
    let shooter = ship("shooter", 1.0e6);
    let mut b = Battle::new(setup(Catalog::from_defs(vec![shooter, target], vec![missile()]), "shooter", "target"));
    let (s, t) = (b.ships.iter().find(|x| x.side == 0).unwrap().id, b.ships.iter().find(|x| x.side == 1).unwrap().id);
    prepare(&mut b, t);
    for _ in 0..(delay * 30.0) as u32 {
        b.step();
    }
    let tp = b.ships[t as usize].pos;
    b.projectiles.push(Projectile { pos: tp + Vec3::new(0.0, 1500.0, 0.0), vel: Vec3::new(0.0, -500.0, 0.0), def: 0, side: 0, owner: s, target: t, aim_hp: u16::MAX, range_left: 9000.0 });
    for _ in 0..(secs * 30.0) as u32 {
        b.step();
    }
    b.ships[t as usize].shield + b.ships[t as usize].hull
}

#[test]
fn passive_missile_shield_destroys_enemy_missiles() {
    let hp = 1.0e5f32;
    assert!(missile_at(ship("target", hp), |_, _| {}, 0.0, 6.0) < hp, "without a passive shield the missile hits");
    let shielded = ShipDef { passive_missile_radius: 300.0, ..ship("target", hp) };
    assert_eq!(missile_at(shielded, |_, _| {}, 0.0, 6.0), hp, "a passive shield destroys the missile, no ability needed");
}

#[test]
fn jamming_pods_jam_missiles_for_their_life() {
    let hp = 1.0e5f32;
    let pods = AbilityDef {
        kind: "BUZZ_DROIDS".into(),
        zone: Some(ZoneDef { radius: 500.0, enemy_dps: 0.0, max_range: 500.0, missile_radius: 500.0 }),
        duration: 60.0,
        recharge: 450.0,
        speed_mult: 1.0,
        shield_regen_mult: 1.0,
        weapon_delay_mult: 1.0,
        take_damage_mult: 1.0,
        ..Default::default()
    };
    let target = ShipDef { abilities: vec![pods], ..ship("target", hp) };
    let drop = |b: &mut Battle, t: u32| b.command(1, Command::UseAbility { ship: t, ability: 0 }).unwrap();
    assert_eq!(missile_at(target.clone(), drop, 0.0, 6.0), hp, "a missile entering the pods' area is destroyed");
    assert!(missile_at(target.clone(), |_, _| {}, 0.0, 6.0) < hp, "without the pods the missile hits");
    assert!(missile_at(target, drop, 61.0, 6.0) < hp, "the pods are gone after their 60 s");
}

#[test]
fn passive_missile_shields_and_jamming_pods_are_read_from_the_data() {
    let Some((data, fs)) = load() else { return };
    let names = ["Star_Home", "Reaver", "A-Wing", "Xyston_Star_Destroyer_Hero", "TIE_Interdictor", "Star_Destroyer"];
    let cat = Catalog::build(&data, Some(&fs), &names);
    let radius = |n: &str| cat.ship(cat.ship_id(n).unwrap_or_else(|| panic!("{n}"))).passive_missile_radius;
    assert_eq!([radius("Star_Home"), radius("Reaver"), radius("A-Wing"), radius("Xyston_Star_Destroyer_Hero"), radius("Star_Destroyer")], [500.0, 300.0, 50.0, 100.0, 0.0]);
    let tie = cat.ship(cat.ship_id("TIE_Interdictor").unwrap());
    let pods = tie.abilities.iter().find(|a| a.kind.eq_ignore_ascii_case("BUZZ_DROIDS")).expect("jamming pods");
    let z = pods.zone.as_ref().expect("pods zone");
    assert_eq!((z.missile_radius, z.radius, z.enemy_dps, z.max_range), (500.0, 500.0, 0.0, 500.0), "{z:?}");
    assert_eq!((pods.duration, pods.recharge), (60.0, 450.0), "Tactical_Health 60 / Own_Damage_Per_Second 1; Recharge_Seconds 450");
}
