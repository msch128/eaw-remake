//! A detonation particle that names another projectile is a second-stage warhead.
//!
//! The Remake has three: `Proj_Proton_Torpedo_TS` (Torpedo Sphere), `Proj_SBJ_Ion` (Subjugator ion bolt and its super laser) and `Proj_Boom_Cannon_Ultra_Mortar` (no live unit; the only reference, on Tyber Zann, is commented out).

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use battle_sim::*;

fn load() -> Option<(alamo_data::GameData, GameFs)> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    Some((load_galaxy(&fs).0, fs))
}

fn craft(name: &str, hull: f32, shield: f32) -> ShipDef {
    ShipDef { name: name.into(), class: TargetClass::Fighter, hull, shield, shield_regen: 0.0, radius: 40.0, ..ShipDef::default() }
}

fn bolt(name: &str, damage: f32, shield: bool, hull: bool) -> ProjDef {
    ProjDef { name: name.into(), damage, shield_damage: shield, hull_damage: hull, speed: 300.0, max_distance: 5000.0, ..ProjDef::default() }
}

fn hold(b: &mut Battle) {
    for s in &mut b.ships {
        s.stance = Stance::HoldFire;
        s.order = Order::Idle;
        s.speed = 0.0;
        s.target = NONE;
        s.squad = NONE;
        s.flyout_left = 0.0;
        s.flyout_exit = Vec3::ZERO;
        for hp in &mut s.hps {
            hp.cooldown = 1.0e6;
            hp.pulses_left = 0;
        }
    }
}

/// One straight shot into `target` from just outside its hit sphere. Other guns are held.
fn fire_one(b: &mut Battle, cat: &Catalog, proj: ProjId, owner: u32, target: u32) -> Vec<Event> {
    hold(b);
    let ship = &b.ships[target as usize];
    let def = cat.ship(ship.def);
    let hit_r = if def.class as u8 >= TargetClass::Frigate as u8 { def.radius * 0.75 } else { def.radius };
    let pos = ship.pos - Vec3::new(hit_r + 8.0, 0.0, 0.0);
    b.projectiles.push(Projectile {
        pos,
        vel: Vec3::new((hit_r + 80.0) * 30.0, 0.0, 0.0),
        def: proj,
        side: b.ships[owner as usize].side,
        owner,
        target: NONE,
        aim_hp: NO_HP,
        range_left: 20_000.0,
    });
    b.step().to_vec()
}

fn lost(before: &Ship, after: &Ship) -> (f32, f32) {
    let shield = (before.shield - after.shield).max(0.0);
    let hp: f32 = before.hps.iter().zip(&after.hps).map(|(a, c)| (a.health - c.health).max(0.0)).sum();
    let hull = (before.hull - after.hull).max(0.0);
    (shield, if hp > 0.5 { hp } else { hull })
}

#[test]
fn a_detonation_warhead_hits_the_ship_and_blasts_other_enemies() {
    // Parent 20 (shield bypass, the Torpedo Sphere carrier). Stage 400 direct and 400 blast over 250, 20 victims.
    let mut parent = bolt("carrier", 20.0, false, true);
    parent.detonate = [Some(1), Some(1), Some(1), None];
    let mut stage = bolt("warhead", 400.0, true, true);
    stage.blast_damage = 400.0;
    stage.blast_range = 250.0;
    stage.blast_victims = 20;
    let ships = vec![craft("shooter", 1.0e6, 0.0), craft("target", 1.0e6, 0.0)];
    let cat = Catalog::from_defs(ships, vec![parent, stage]);
    let mut b = Battle::new(BattleSetup {
        catalog: Arc::new(cat.clone()),
        sides: vec![
            SideSetup { name: "a".into(), units: vec![UnitSetup { ship: 0, count: 2, tag: 1 }], structures: vec![], ai: false },
            SideSetup { name: "b".into(), units: vec![UnitSetup { ship: 1, count: 3, tag: 1 }], structures: vec![], ai: false },
        ],
        config: BattleConfig { half_size: 20_000.0, fog_of_war: false, record_events: false, ..BattleConfig::default() },
    });
    let mut ours: Vec<u32> = b.ships.iter().filter(|s| s.side == 0).map(|s| s.id).collect();
    let mut theirs: Vec<u32> = b.ships.iter().filter(|s| s.side == 1).map(|s| s.id).collect();
    ours.sort();
    theirs.sort();
    let (owner, friend, target, neighbor, far) = (ours[0], ours[1], theirs[0], theirs[1], theirs[2]);
    b.ships[target as usize].pos = Vec3::ZERO;
    // Impact is on the -X skin (radius 40). Neighbor and friend sit 60 off that point: inside the 250 blast, outside each other's hull.
    b.ships[neighbor as usize].pos = Vec3::new(-40.0, 60.0, 0.0);
    b.ships[friend as usize].pos = Vec3::new(-40.0, -60.0, 0.0);
    b.ships[far as usize].pos = Vec3::new(0.0, 4000.0, 0.0);
    b.ships[owner as usize].pos = Vec3::new(-8000.0, 0.0, 0.0);
    let before: Vec<Ship> = b.ships.clone();
    fire_one(&mut b, &cat, 0, owner, target);
    let (shield, body) = lost(&before[target as usize], &b.ships[target as usize]);
    let (_, near) = lost(&before[neighbor as usize], &b.ships[neighbor as usize]);
    let (_, pal) = lost(&before[friend as usize], &b.ships[friend as usize]);
    let (_, away) = lost(&before[far as usize], &b.ships[far as usize]);
    // Before the fix the carrier's 20 was the whole hit and the neighbor lost nothing.
    assert!((shield + body - 420.0).abs() < 1.0, "struck ship shield {shield} body {body}");
    assert!(near > 200.0 && near < 400.0, "neighbor blast {near}");
    assert_eq!(pal, 0.0, "a friend in the blast");
    assert_eq!(away, 0.0, "a ship outside the blast");
}

#[test]
fn a_shot_without_a_warhead_stays_at_its_own_damage() {
    let parent = bolt("carrier", 20.0, false, true);
    let cat = Catalog::from_defs(vec![craft("shooter", 1.0e6, 0.0), craft("target", 1.0e6, 0.0)], vec![parent]);
    let mut b = Battle::new(BattleSetup {
        catalog: Arc::new(cat.clone()),
        sides: vec![
            SideSetup { name: "a".into(), units: vec![UnitSetup { ship: 0, count: 1, tag: 1 }], structures: vec![], ai: false },
            SideSetup { name: "b".into(), units: vec![UnitSetup { ship: 1, count: 1, tag: 1 }], structures: vec![], ai: false },
        ],
        config: BattleConfig { half_size: 20_000.0, fog_of_war: false, record_events: false, ..BattleConfig::default() },
    });
    let owner = b.ships.iter().find(|s| s.side == 0).unwrap().id;
    let target = b.ships.iter().find(|s| s.side == 1).unwrap().id;
    b.ships[target as usize].pos = Vec3::ZERO;
    let hull = b.ships[target as usize].hull;
    fire_one(&mut b, &cat, 0, owner, target);
    assert!((hull - b.ships[target as usize].hull - 20.0).abs() < 1.0, "{}", hull - b.ships[target as usize].hull);
}

#[test]
fn blast_stops_after_the_victim_cap() {
    let mut parent = bolt("carrier", 0.0, false, true);
    parent.detonate = [Some(1), None, None, None];
    let mut stage = bolt("warhead", 0.0, false, true);
    stage.blast_damage = 100.0;
    stage.blast_range = 500.0;
    stage.blast_victims = 2;
    let cat = Catalog::from_defs(vec![craft("shooter", 1.0e6, 0.0), craft("target", 1.0e6, 0.0)], vec![parent, stage]);
    let mut b = Battle::new(BattleSetup {
        catalog: Arc::new(cat.clone()),
        sides: vec![
            SideSetup { name: "a".into(), units: vec![UnitSetup { ship: 0, count: 1, tag: 1 }], structures: vec![], ai: false },
            SideSetup { name: "b".into(), units: vec![UnitSetup { ship: 1, count: 4, tag: 1 }], structures: vec![], ai: false },
        ],
        config: BattleConfig { half_size: 20_000.0, fog_of_war: false, record_events: false, ..BattleConfig::default() },
    });
    let owner = b.ships.iter().find(|s| s.side == 0).unwrap().id;
    let mut theirs: Vec<u32> = b.ships.iter().filter(|s| s.side == 1).map(|s| s.id).collect();
    theirs.sort();
    b.ships[theirs[0] as usize].pos = Vec3::ZERO;
    // Distances from the impact (about -40, 0): 70, 90, 140, 200. The cap keeps the two closest.
    for (i, y) in [70.0, 90.0, 140.0, 200.0].into_iter().enumerate() {
        b.ships[theirs[i] as usize].pos = if i == 0 { Vec3::ZERO } else { Vec3::new(-40.0, y, 0.0) };
    }
    let before: Vec<f32> = b.ships.iter().map(|s| s.hull).collect();
    fire_one(&mut b, &cat, 0, owner, theirs[0]);
    let hit: Vec<bool> = (1..4).map(|i| before[theirs[i] as usize] - b.ships[theirs[i] as usize].hull > 1.0).collect();
    assert_eq!(hit, vec![true, true, false], "two closest victims, {hit:?}");
}

#[test]
fn mortar_warhead_is_the_whole_hit() {
    // `Proj_Boom_Cannon_Ultra_Mortar` comments out its own damage. The stage is 0 direct and 8000 blast, so the struck ship takes 8000 and shields are skipped (`Does_Shield_Damage` No on the stage).
    let Some((data, _)) = load() else { return };
    let parent_xml = data.objects.projectile("Proj_Boom_Cannon_Ultra_Mortar").expect("mortar");
    let stage_xml = data.objects.projectile("Proj_Boom_Cannon_Ultra_Mortar_Damage").expect("mortar stage");
    assert!(data.objects.projectile(parent_xml.fields.get("projectile_object_detonation_particle").unwrap().trim()).is_some());
    assert_eq!(stage_xml.damage, 0.0);
    assert!((stage_xml.blast_damage - 8000.0).abs() < 1.0 && (stage_xml.blast_range - 1500.0).abs() < 1.0);
    let mut parent = bolt("Proj_Boom_Cannon_Ultra_Mortar", parent_xml.damage.max(parent_xml.blast_damage), parent_xml.does_shield_damage, parent_xml.does_hitpoint_damage);
    parent.detonate = [Some(1); 4];
    let mut stage = bolt("Proj_Boom_Cannon_Ultra_Mortar_Damage", stage_xml.damage.max(stage_xml.blast_damage), stage_xml.does_shield_damage, stage_xml.does_hitpoint_damage);
    stage.blast_damage = stage_xml.blast_damage;
    stage.blast_range = stage_xml.blast_range;
    stage.blast_victims = stage_xml.blast_max_victims.unwrap_or(10).max(1) as u32;
    let cat = Catalog::from_defs(vec![craft("shooter", 1.0e6, 0.0), craft("target", 1.0e6, 5000.0)], vec![parent, stage]);
    let mut b = Battle::new(BattleSetup {
        catalog: Arc::new(cat.clone()),
        sides: vec![
            SideSetup { name: "a".into(), units: vec![UnitSetup { ship: 0, count: 1, tag: 1 }], structures: vec![], ai: false },
            SideSetup { name: "b".into(), units: vec![UnitSetup { ship: 1, count: 1, tag: 1 }], structures: vec![], ai: false },
        ],
        config: BattleConfig { half_size: 20_000.0, fog_of_war: false, record_events: false, ..BattleConfig::default() },
    });
    let owner = b.ships.iter().find(|s| s.side == 0).unwrap().id;
    let target = b.ships.iter().find(|s| s.side == 1).unwrap().id;
    b.ships[target as usize].pos = Vec3::ZERO;
    let (shield, hull) = (b.ships[target as usize].shield, b.ships[target as usize].hull);
    fire_one(&mut b, &cat, 0, owner, target);
    let s = &b.ships[target as usize];
    assert!((shield - s.shield).abs() < 1.0, "stage does not damage shields: {}", shield - s.shield);
    assert!((hull - s.hull - 8000.0).abs() < 1.0, "hull loss {}", hull - s.hull);
}

fn proj<'a>(cat: &'a Catalog, name: &str) -> &'a ProjDef {
    &cat.projectiles[cat.projectile_id(name).unwrap_or_else(|| panic!("{name}")) as usize]
}

#[test]
fn three_projectiles_detonate_into_a_projectile() {
    let Some((data, _)) = load() else { return };
    let tags = [
        "projectile_object_detonation_particle",
        "projectile_object_armor_reduced_detonation_particle",
        "projectile_lifetime_detonation_particle",
        "projectile_ground_detonation_particle",
        "projectile_absorbed_by_shields_particle",
    ];
    let mut carriers: Vec<String> = data
        .objects
        .of_kind("Projectile")
        .filter_map(|o| data.objects.projectile(&o.name))
        .filter(|p| tags.iter().any(|t| p.fields.get(*t).is_some_and(|n| data.objects.projectile(n.trim()).is_some())))
        .map(|p| p.name)
        .collect();
    carriers.sort();
    assert_eq!(carriers, vec!["Proj_Boom_Cannon_Ultra_Mortar".to_string(), "Proj_Proton_Torpedo_TS".to_string(), "Proj_SBJ_Ion".to_string()], "{carriers:?}");
}

#[test]
fn torpedo_sphere_and_subjugator_warheads_are_loaded() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Torpedo_Sphere", "Star_Destroyer", "Empire_Star_Base_3", "Subjugator", "TIE_Fighter"]));
    let ts = proj(&cat, "Proj_Proton_Torpedo_TS");
    let boom = proj(&cat, "Proj_Proton_Torpedo_TS_Damage");
    let id = cat.projectile_id("Proj_Proton_Torpedo_TS_Damage");
    assert_eq!(ts.detonate[0], id);
    assert_eq!(ts.detonate[2], id);
    assert!((boom.damage - 400.0).abs() < 1.0 && (boom.blast_damage - 400.0).abs() < 1.0);
    assert!((boom.blast_range - 250.0).abs() < 1.0 && boom.blast_victims == 20);
    assert!(boom.shield_damage && boom.hull_damage);
    assert!(!ts.shield_damage && ts.hull_damage, "the carrier still bypasses shields");
    let ion = proj(&cat, "Proj_SBJ_Ion");
    let ion_boom = proj(&cat, "Proj_SBJ_Ion_Damage");
    let ion_id = cat.projectile_id("Proj_SBJ_Ion_Damage");
    assert_eq!(ion.detonate[0], ion_id);
    assert_eq!(ion.detonate[3], ion_id);
    assert!((ion.damage - 99900.0).abs() < 1.0);
    assert!((ion_boom.damage - 200_000.0).abs() < 1.0 && (ion_boom.blast_damage - 200_000.0).abs() < 1.0);
    assert!((ion_boom.blast_range - 100.0).abs() < 1.0 && ion_boom.blast_victims == 5);
}

fn shoot(cat: &Arc<Catalog>, shooter: &str, target: &str, structure: bool, projectile: &str, prime_shield: Option<f32>) -> (f32, f32, f32) {
    let unit = |n: &str| UnitSetup { ship: cat.ship_id(n).unwrap(), count: 1, tag: 1 };
    let mut enemy_units = vec![unit("TIE_Fighter"), unit("TIE_Fighter"), unit("TIE_Fighter")];
    let structures = if structure {
        vec![cat.ship_id(target).unwrap()]
    } else {
        enemy_units.insert(0, unit(target));
        vec![]
    };
    let mut b = Battle::new(BattleSetup {
        catalog: cat.clone(),
        sides: vec![
            SideSetup { name: "a".into(), units: vec![unit(shooter), unit("TIE_Fighter")], structures: vec![], ai: false },
            SideSetup { name: "b".into(), units: enemy_units, structures, ai: false },
        ],
        // Navigation would push the blast witness out of the capital's hull before the warhead lands.
        config: BattleConfig { half_size: 40_000.0, fog_of_war: false, navigation: false, record_events: true, ..BattleConfig::default() },
    });
    for s in &mut b.ships {
        s.pos = Vec3::new(25_000.0, 200.0 * s.id as f32, 0.0);
    }
    let of = |n: &str| {
        let id = cat.ship_id(n).unwrap();
        b.ships.iter().filter(|s| s.def == id).map(|s| s.id).collect::<Vec<_>>()
    };
    let owner = of(shooter)[0];
    let friend = of("TIE_Fighter").into_iter().find(|&i| b.ships[i as usize].side == 0).unwrap();
    let target_id = if structure { b.ships.iter().find(|s| s.def == cat.ship_id(target).unwrap()).unwrap().id } else { of(target)[0] };
    let ties: Vec<u32> = b.ships.iter().filter(|s| s.def == cat.ship_id("TIE_Fighter").unwrap() && s.side == 1).map(|s| s.id).collect();
    b.ships[owner as usize].pos = Vec3::new(-30_000.0, 0.0, 0.0);
    b.ships[target_id as usize].pos = Vec3::ZERO;
    b.ships[target_id as usize].yaw = 0.0;
    // The shot crosses the target's middle, so the detonation is there. The witness sits 80 off that point:
    // clear of the shot line, inside a 100-unit ion blast and a 250-unit torpedo blast.
    b.ships[ties[0] as usize].pos = Vec3::new(0.0, 80.0, 0.0);
    // The ion warhead damages shields only. A fighter has none, so the witness is given one.
    b.ships[ties[0] as usize].shield = 80_000.0;
    b.ships[friend as usize].pos = Vec3::new(0.0, -80.0, 0.0);
    b.ships[ties[1] as usize].pos = Vec3::new(0.0, 8_000.0, 0.0);
    if let Some(v) = prime_shield {
        b.ships[target_id as usize].shield = v;
    }
    let before = b.ships.clone();
    let events = fire_one(&mut b, &cat, cat.projectile_id(projectile).unwrap(), owner, target_id);
    // The damage of the hits themselves (a shield also recharges within the step).
    let dealt = |to: u32, on_shield: bool| -> f32 {
        events.iter().map(|e| match e { Event::Hit { ship, kind, damage, .. } if *ship == to && (*kind == HitKind::Shield) == on_shield => *damage, _ => 0.0 }).sum()
    };
    let (shield, body) = (dealt(target_id, true), dealt(target_id, false));
    let (ns, nb) = (dealt(ties[0], true), dealt(ties[0], false));
    let (fs, fb) = lost(&before[friend as usize], &b.ships[friend as usize]);
    let (as_, ab) = lost(&before[ties[1] as usize], &b.ships[ties[1] as usize]);
    assert_eq!(fs + fb, 0.0, "{target}: friend took blast");
    assert_eq!(as_ + ab, 0.0, "{target}: far ship took blast");
    (shield, body, ns + nb)
}

#[test]
fn one_torpedo_sphere_shot_damages_a_capital_and_a_star_base() {
    // Per torpedo, shields up, on a Star Destroyer and on a level 3 star base alike. The torpedo itself (20, `Damage_Torpedo_Proton`, no
    // shield damage) goes through the shield onto the hull: 20 x 2.65 = 53. That was the whole hit. Its warhead (400, `Damage_Proton_Torp`,
    // shield and hull damage) now lands on the shield: 400 x 0.33 = 132. 53 -> 185 per torpedo.
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Torpedo_Sphere", "Star_Destroyer", "Empire_Star_Base_3", "TIE_Fighter"]));
    let torpedo = proj(&cat, "Proj_Proton_Torpedo_TS");
    let warhead = proj(&cat, "Proj_Proton_Torpedo_TS_Damage");
    for (target, structure, total) in [("Star_Destroyer", false, 185.0), ("Empire_Star_Base_3", true, 185.0)] {
        let (shield, body, near) = shoot(&cat, "Torpedo_Sphere", target, structure, "Proj_Proton_Torpedo_TS", None);
        let ship = cat.ship(cat.ship_id(target).unwrap());
        let on_hull = torpedo.damage * cat.damage.multiplier(torpedo.damage_type, ship.armor);
        let on_shield = warhead.damage * cat.damage.multiplier(warhead.damage_type, ship.shield_armor);
        assert!((body - on_hull).abs() < 1.0, "{target}: hull {body}, the torpedo's own damage is {on_hull}");
        assert!((shield - on_shield).abs() < 1.0, "{target}: shield {shield}, the warhead's damage is {on_shield}");
        assert!((shield + body - total).abs() < 1.0, "{target}: {} per torpedo", shield + body);
        assert!(near > 0.0, "{target}: the blast missed the neighbor");
    }
}

#[test]
fn one_subjugator_ion_bolt_adds_the_area_warhead() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Subjugator", "Star_Destroyer", "TIE_Fighter"]));
    // Shield damage only, on a Star Destroyer given 2000000 shield points: the bolt 99900 x 0.33 = 32967 (that was the whole hit), and its
    // warhead `Proj_SBJ_Ion_Damage` 200000 x 0.33 = 66000 more. 32967 -> 98967.
    let bolt = proj(&cat, "Proj_SBJ_Ion");
    let warhead = proj(&cat, "Proj_SBJ_Ion_Damage");
    let (shield, body, near) = shoot(&cat, "Subjugator", "Star_Destroyer", false, "Proj_SBJ_Ion", Some(2_000_000.0));
    let ship = cat.ship(cat.ship_id("Star_Destroyer").unwrap());
    let expected = (bolt.damage + warhead.damage) * cat.damage.multiplier(bolt.damage_type, ship.shield_armor);
    assert!((shield - expected).abs() < 1.0 && (shield - 98_967.0).abs() < 1.0, "shield {shield}, bolt and warhead are {expected}");
    assert!(body < 1.0, "the ion warhead does no hull damage: {body}");
    assert!(near > 0.0, "the blast missed the neighbor");
}
