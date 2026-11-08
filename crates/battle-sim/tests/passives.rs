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

/// A hero squadron: leader (id 0, hull 1000) with `Redirect_Damage_To_Teammates`, two wingmen (id 1, hull 1000 each), and a target-less
/// shooter. Returns the battle, the leader and the wingmen.
fn hero_squadron(redirect: bool) -> (Battle, u32, Vec<u32>) {
    let fighter = |name: &str| ShipDef { class: TargetClass::Fighter, radius: 10.0, ..ship(name, 1000.0) };
    let leader = ShipDef { redirect_to_teammates: redirect, ..fighter("leader") };
    let squad = ShipDef {
        name: "hero_squad".into(),
        is_squadron: true,
        class: TargetClass::Fighter,
        members: vec![(0, 1), (1, 2)],
        offsets: vec![Vec3::ZERO, Vec3::new(0.0, 400.0, 0.0), Vec3::new(0.0, -400.0, 0.0)],
        ..ShipDef::default()
    };
    let shot = ProjDef { name: "laser".into(), category: "laser".into(), ..missile() };
    let b = Battle::new(setup(Catalog::from_defs(vec![leader, fighter("wingman"), squad, ship("shooter", 1.0e6)], vec![shot]), "hero_squad", "shooter"));
    let lead = b.ships.iter().find(|s| s.def == 0).unwrap().id;
    let wings = b.ships.iter().filter(|s| s.def == 1).map(|s| s.id).collect();
    (b, lead, wings)
}

/// Fires one bolt of `proj` at `target` from 30 units in front of it (fighters bob up and down) and runs 2 s; returns the ships it hit.
fn bolt_at(b: &mut Battle, target: u32, proj: ProjId) -> Vec<u32> {
    let shooter = b.ships.iter().find(|s| s.side == 1).unwrap().id;
    let tp = b.ships[target as usize].pos;
    b.projectiles.push(Projectile { pos: tp + Vec3::new(30.0, 0.0, 0.0), vel: Vec3::new(-500.0, 0.0, 0.0), def: proj, side: 1, owner: shooter, target, aim_hp: u16::MAX, range_left: 9000.0 });
    let mut hits = Vec::new();
    for _ in 0..60 {
        hits.extend(b.step().iter().filter_map(|e| if let Event::Hit { ship, .. } = e { Some(*ship) } else { None }));
    }
    hits
}

#[test]
fn hero_leader_hits_go_to_the_wingmen_until_they_are_gone() {
    let (mut b, lead, wings) = hero_squadron(true);
    assert!(wings.iter().all(|&w| b.ships[w as usize].pos.dist(b.ships[lead as usize].pos) > 200.0), "the wingmen fly apart from the leader");
    bolt_at(&mut b, lead, 0);
    let wing_hull: f32 = wings.iter().map(|&w| b.ships[w as usize].hull).sum();
    assert_eq!((b.ships[lead as usize].hull, wing_hull), (1000.0, 1900.0), "the leader's hit lands on a wingman");
    for &w in &wings {
        b.ships[w as usize].alive = false;
    }
    bolt_at(&mut b, lead, 0);
    assert_eq!(b.ships[lead as usize].hull, 900.0, "without wingmen the leader is hit");

    let (mut b, lead, _) = hero_squadron(false);
    bolt_at(&mut b, lead, 0);
    assert_eq!(b.ships[lead as usize].hull, 900.0, "a plain leader takes its own hits");
}

#[test]
fn hero_leaders_redirect_damage_in_the_data() {
    let Some((data, fs)) = load() else { return };
    let leaders = ["TIE_Advanced", "Rogue_1", "Green_Leader", "Defender_Leader_Y", "Defender_Leader_K", "Sipal_StarViper", "Luke_XWing"];
    let wingmen = ["Black_TIE_Fighter", "Rogue_2", "Defender_Y", "Peggat_2"];
    let names: Vec<&str> = leaders.iter().chain(&wingmen).copied().collect();
    let cat = Catalog::build(&data, Some(&fs), &names);
    for n in leaders {
        assert!(cat.ship(cat.ship_id(n).unwrap_or_else(|| panic!("{n}"))).redirect_to_teammates, "{n}");
    }
    for n in wingmen {
        assert!(!cat.ship(cat.ship_id(n).unwrap_or_else(|| panic!("{n}"))).redirect_to_teammates, "{n}");
    }
}

#[test]
fn vaders_wingmen_take_his_hits() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Black_Squadron", "Star_Destroyer"]));
    let side = |n: &str, ai| SideSetup { name: "s".into(), units: vec![UnitSetup { ship: cat.ship_id(n).unwrap(), count: 1, tag: 1 }], structures: vec![], ai };
    let config = BattleConfig { half_size: 8000.0, fog_of_war: false, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![side("Black_Squadron", false), side("Star_Destroyer", false)], config });
    let vader = b.ships.iter().find(|s| cat.ship(s.def).name == "TIE_Advanced").expect("Vader").id;
    let wings: Vec<u32> = b.ships.iter().filter(|s| cat.ship(s.def).name == "Black_TIE_Fighter").map(|s| s.id).collect();
    assert_eq!(wings.len(), 8);
    assert!(wings.iter().all(|&w| b.ships[w as usize].squad == b.ships[vader as usize].squad), "one squadron");
    let full = |b: &Battle, id: u32| b.ships[id as usize].hull + b.ships[id as usize].shield;
    let vader_hp = full(&b, vader);
    // A laser that hurts the Black TIEs (shield and hull).
    let wing = cat.ship(b.ships[wings[0] as usize].def);
    let hurts = |p: &ProjDef| p.category == "laser" && p.damage > 0.0 && p.shield_damage && p.hull_damage && cat.damage.multiplier(p.damage_type, wing.armor) > 0.0 && cat.damage.multiplier(p.damage_type, wing.shield_armor) > 0.0;
    let laser = cat.projectiles.iter().position(hurts).expect("a laser") as ProjId;
    let hits = bolt_at(&mut b, vader, laser);
    assert!(!hits.is_empty() && hits.iter().all(|h| wings.contains(h)), "the bolt hits a wingman, not Vader: {hits:?}");
    assert_eq!(full(&b, vader), vader_hp, "Vader is not damaged while his wingmen live");
}

#[test]
fn retreat_prevention_keeps_its_own_side_in_the_battle() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Titan_Star_Destroyer", "Death_Star_II", "Star_Destroyer", "MC-80_Star_Cruiser"]));
    let prevents = |n: &str| cat.ship(cat.ship_id(n).unwrap_or_else(|| panic!("{n}"))).prevents_retreat;
    assert_eq!([prevents("Titan_Star_Destroyer"), prevents("Death_Star_II"), prevents("Star_Destroyer")], [true, true, false]);
    let side = |n: &str| SideSetup { name: "s".into(), units: vec![UnitSetup { ship: cat.ship_id(n).unwrap(), count: 1, tag: 1 }], structures: vec![], ai: false };
    let config = BattleConfig { retreat_countdown: 0.0, ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![side("Titan_Star_Destroyer"), side("MC-80_Star_Cruiser")], config });
    assert_eq!(b.command(0, Command::Retreat), Err(CommandError::RetreatPrevented), "the Titan's own side cannot retreat");
    assert_eq!(b.command(1, Command::Retreat), Ok(()), "its enemy can");
    let titan = b.ships.iter().find(|s| s.side == 0 && cat.ship(s.def).prevents_retreat).unwrap().id;
    b.ships[titan as usize].alive = false;
    assert_eq!(b.command(0, Command::Retreat), Ok(()), "with the Titan gone the side may retreat");
}

/// Whether side 0's seer (sensor 1000, static) sees side 1's ship 1150 units away (centre to centre, radius 100: 1050 to its edge).
fn seen_at_1150(seer_mods: Vec<FieldModDef>, granted: Vec<FieldModDef>) -> bool {
    let seer = ShipDef { sensor_range: 1000.0, field_mods: seer_mods, ..ship("seer", 1.0e6) };
    // Structures are always known: the far ship is mobile (it does not move without orders).
    let far = ShipDef { sensor_range: 1000.0, is_static: false, ..ship("far", 1.0e6) };
    let mut s = setup(Catalog::from_defs(vec![seer, far], vec![missile()]), "seer", "far");
    s.config.fog_of_war = true;
    s.config.side_field_mods = vec![granted];
    let mut b = Battle::new(s);
    let (s0, s1) = (b.ships.iter().find(|s| s.side == 0).unwrap().id, b.ships.iter().find(|s| s.side == 1).unwrap().id);
    b.ships[s1 as usize].pos = b.ships[s0 as usize].pos + Vec3::new(1150.0, 0.0, 0.0);
    for _ in 0..20 {
        b.step();
    }
    b.is_visible_to(0, s1)
}

#[test]
fn battlefield_modifiers_lengthen_sensor_range() {
    let sensors = FieldModDef { fow_reveal: 1.2, stealth_duration: 1.0, stacking: 3 };
    assert!(!seen_at_1150(vec![], vec![]), "1050 is beyond the 1000 sensor range");
    assert!(seen_at_1150(vec![], vec![sensors]), "an upgrade's x1.2 reaches 1200");
    assert!(seen_at_1150(vec![sensors], vec![]), "a unit that carries it does the same");
    let weak = FieldModDef { fow_reveal: 1.01, ..sensors };
    assert!(!seen_at_1150(vec![], vec![weak, weak]), "one stacking category does not multiply");
}

#[test]
fn battlefield_modifiers_lengthen_timed_cloaks() {
    let cloak = AbilityDef { kind: "STEALTH".into(), stealth: true, duration: 10.0, recharge: 30.0, speed_mult: 1.0, shield_regen_mult: 1.0, weapon_delay_mult: 1.0, take_damage_mult: 1.0, ..Default::default() };
    let run = |granted: Vec<FieldModDef>| {
        let ghost = ShipDef { abilities: vec![cloak.clone()], ..ship("ghost", 1.0e6) };
        let mut s = setup(Catalog::from_defs(vec![ghost, ship("other", 1.0e6)], vec![missile()]), "ghost", "other");
        s.config.side_field_mods = vec![granted];
        let mut b = Battle::new(s);
        let g = b.ships.iter().find(|s| s.side == 0).unwrap().id;
        b.command(0, Command::UseAbility { ship: g, ability: 0 }).unwrap();
        (b.ships[g as usize].abilities[0].active, b.ships[g as usize].abilities[0].cooldown)
    };
    assert_eq!(run(vec![]), (10.0, 40.0));
    assert_eq!(run(vec![FieldModDef { fow_reveal: 1.0, stealth_duration: 1.5, stacking: 3 }]), (15.0, 45.0), "US_Cloaking_Generator_L2: x1.5");
}

#[test]
fn battlefield_modifiers_are_read_from_the_upgrades() {
    let Some((data, _fs)) = load() else { return };
    let mods = |n: &str| FieldModDef::of_object(&data.objects, n);
    assert_eq!(mods("Advanced_Ship_Sensors_Empire"), vec![FieldModDef { fow_reveal: 1.2, stealth_duration: 1.0, stacking: 3 }]);
    assert_eq!(mods("US_Cloaking_Generator_L1_Upgrade"), vec![FieldModDef { fow_reveal: 1.0, stealth_duration: 1.25, stacking: 3 }]);
    assert_eq!(mods("US_Cloaking_Generator_L2_Upgrade"), vec![FieldModDef { fow_reveal: 1.0, stealth_duration: 1.5, stacking: 3 }]);
    assert!(mods("Advanced_Targeting_Systems_Fighters_Empire").is_empty(), "a combat bonus is no battlefield modifier");
}
