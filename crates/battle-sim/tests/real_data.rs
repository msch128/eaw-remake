//! Tests against the real Remake data; skipped when the game is not installed.

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

fn side(cat: &Catalog, name: &str, units: &[(&str, u32)], structures: &[&str], ai: bool) -> SideSetup {
    SideSetup {
        name: name.into(),
        units: units.iter().map(|&(n, c)| UnitSetup { ship: cat.ship_id(n).unwrap_or_else(|| panic!("{n}")), count: c, tag: 1 }).collect(),
        structures: structures.iter().map(|n| cat.ship_id(n).unwrap_or_else(|| panic!("{n}"))).collect(),
        ai,
    }
}

#[test]
fn catalog_uses_model_bones() {
    let Some((data, fs)) = load() else { return };
    let cat = Catalog::build(&data, Some(&fs), &["Star_Destroyer", "X-Wing_Squadron"]);
    let isd = cat.ship(cat.ship_id("Star_Destroyer").unwrap());
    assert_eq!(isd.class, TargetClass::Capital);
    assert!(isd.hardpoints.len() > 20);
    let positioned = isd.hardpoints.iter().filter(|h| h.from_bone).count();
    assert!(positioned * 10 >= isd.hardpoints.len() * 7, "{positioned}/{} hardpoints placed from bones", isd.hardpoints.len());
    // The bow lies along -Y in model space, i.e. +X in battle space: the bow cannon is ahead of the bridge.
    let bow = isd.hardpoints.iter().find(|h| h.name == "HP_ISD_1_New_TL_05").unwrap();
    let shield = isd.hardpoints.iter().find(|h| h.kind == HpKind::ShieldGenerator).unwrap();
    let engine = isd.hardpoints.iter().find(|h| h.kind == HpKind::Engine).expect("ISD must have engine hardpoint");
    assert!(bow.offset.x > 300.0 && shield.offset.x < -100.0, "bow {:?} shield {:?}", bow.offset, shield.offset);
    assert!(engine.offset.x < -200.0, "engine hardpoint should be at the rear: {:?}", engine.offset);
    assert!(isd.abilities.iter().any(|a| a.salvo));
    let xw = cat.ship(cat.ship_id("X-Wing_Squadron").unwrap());
    assert!(xw.is_squadron && xw.members.iter().map(|m| m.1).sum::<u32>() == 6);
    assert!(cat.damage.multiplier(cat.projectiles.iter().find_map(|p| p.damage_type), cat.ship(0).armor) > 0.0);
}

#[test]
fn fleet_battle_runs_deterministically_to_a_winner() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Star_Destroyer", "Nebulon_B_Frigate", "TIE_Fighter_Squadron", "X-Wing_Squadron"]));
    let run = || {
        let sides = vec![
            side(&cat, "Empire", &[("Star_Destroyer", 3), ("TIE_Fighter_Squadron", 3)], &[], true),
            side(&cat, "Rebel", &[("Nebulon_B_Frigate", 4), ("X-Wing_Squadron", 3)], &[], true),
        ];
        let setup = BattleSetup { catalog: cat.clone(), sides, config: BattleConfig { record_events: false, seed: 5, ..BattleConfig::default() } };
        let mut b = Battle::new(setup);
        let steps = b.run_to_end(30 * 900);
        (b.result(), steps, b.state_hash())
    };
    let (r1, s1, h1) = run();
    let (r2, s2, h2) = run();
    assert_eq!((s1, h1), (s2, h2), "deterministic");
    assert_eq!(r1, r2);
    assert_eq!(r1.winner, Some(0), "three Star Destroyers beat four Nebulons: {r1:?}");
    let empire_isd = r1.groups.iter().find(|g| g.side == 0 && g.ship == cat.ship_id("Star_Destroyer").unwrap()).unwrap();
    assert!(empire_isd.survived >= 2);
}

#[test]
fn star_base_defends_with_garrison_and_hardpoints_break() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Star_Destroyer", "Rebel_Star_Base_2"]));
    let sides = vec![side(&cat, "Empire", &[("Star_Destroyer", 10)], &[], true), side(&cat, "Rebel", &[], &["Rebel_Star_Base_2"], true)];
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config: BattleConfig::default() });
    let base = b.ships.iter().find(|s| cat.ship(s.def).is_static).unwrap().id;
    let garrison = b.ships.iter().filter(|s| s.side == 1 && !cat.ship(s.def).is_static).count();
    assert!(garrison > 0, "star base launches its garrison squadrons");
    let mut hp_destroyed = 0;
    let mut hardpoint_hits = 0;
    let mut destroyed_base = false;
    for _ in 0..30 * 1500 {
        for e in b.step() {
            match e {
                Event::HardpointDestroyed { ship, .. } if *ship == base => hp_destroyed += 1,
                Event::Hit { ship, kind: HitKind::Hardpoint(_), .. } if *ship == base => hardpoint_hits += 1,
                Event::ShipDestroyed { ship, .. } if *ship == base => destroyed_base = true,
                _ => {}
            }
        }
        if b.is_over() {
            break;
        }
    }
    assert!(hardpoint_hits > 0 && hp_destroyed > 0, "hardpoints take damage and break ({hardpoint_hits} hits, {hp_destroyed} destroyed)");
    assert!(destroyed_base && b.result().winner == Some(0));
}

#[test]
fn attack_hardpoint_and_abilities() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &["Star_Destroyer", "Victory_Destroyer"]));
    let mk = || {
        let sides = vec![side(&cat, "Empire", &[("Star_Destroyer", 1)], &[], false), side(&cat, "Rebel", &[("Victory_Destroyer", 1)], &[], false)];
        let config = BattleConfig { half_size: 3000.0, ..BattleConfig::default() };
        Battle::new(BattleSetup { catalog: cat.clone(), sides, config })
    };

    // Targeting a shield generator.
    let mut b = mk();
    let target_def = cat.ship(b.ships[1].def);
    let sg = target_def.hardpoints.iter().position(|h| h.kind == HpKind::ShieldGenerator && h.targetable);
    if let Some(sg) = sg {
        b.command(0, Command::AttackHardpoint { ships: vec![0], target: 1, hardpoint: sg as u16 }).unwrap();
        assert_eq!(b.command(0, Command::AttackHardpoint { ships: vec![0], target: 1, hardpoint: 9999 }), Err(CommandError::InvalidHardpoint));
    }
    assert_eq!(b.command(1, Command::Attack { ships: vec![0], target: 1 }), Err(CommandError::InvalidShip), "cannot order the enemy's ship");

    // Full Salvo speeds up the hardpoints that define a salvo multiplier (heavy turbolaser turrets).
    let isd_def = cat.ship_id("Star_Destroyer").unwrap();
    let count = |salvo: bool| {
        let mut b = mk();
        b.command(0, Command::Attack { ships: vec![0], target: 1 }).unwrap();
        b.command(1, Command::SetStance { ships: vec![1], stance: Stance::HoldFire }).unwrap();
        // Close the distance first.
        for _ in 0..30 * 90 {
            b.step();
        }
        let idx = cat.ship(b.ships[0].def).abilities.iter().position(|a| a.salvo).expect("FULL_SALVO");
        if salvo {
            b.command(0, Command::UseAbility { ship: 0, ability: idx as u16 }).unwrap();
            assert_eq!(b.command(0, Command::UseAbility { ship: 0, ability: idx as u16 }), Err(CommandError::AbilityUnavailable));
        }
        let mut fired = 0;
        for _ in 0..30 * 15 {
            fired += b
                .step()
                .iter()
                .filter(|e| matches!(e, Event::ProjectileFired { ship: 0, hardpoint: Some(h), .. } if cat.ship(isd_def).hardpoints[*h as usize].full_salvo_mult < 1.0))
                .count();
        }
        fired
    };
    let (normal, salvo) = (count(false), count(true));
    assert!(normal >= 4, "the salvo turrets fire ({normal})");
    assert!(salvo as f32 > normal as f32 * 1.2, "full salvo fires faster: {normal} vs {salvo}");
}


#[test]
fn ships_ordered_to_attack_stop_at_their_weapon_range() {
    let Some((data, fs)) = load() else { return };
    let candidates = ["Star_Destroyer", "Victory_Destroyer", "Nebulon_B_Frigate", "MC-80_Star_Cruiser", "CR-90_Corvette", "Assault_Frigate", "Acclamator_Assault_Ship", "Broadside_Class_Cruiser", "Quasar_Fire_Cruiser", "Imperial_Class_Star_Destroyer"];
    let mut names: Vec<&str> = candidates.to_vec();
    names.push("Rebel_Star_Base_2");
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &names));
    let mut checked = 0;
    for name in candidates {
        let Some(id) = cat.ship_id(name) else { continue };
        let def = cat.ship(id);
        if def.preferred_range <= 0.0 || def.is_static {
            continue;
        }
        let sides = vec![side(&cat, "Attacker", &[(name, 1)], &[], false), side(&cat, "Defender", &[], &["Rebel_Star_Base_2"], false)];
        // The attacker is made very tough so that the star base cannot end the run before the stand-off shows.
        let config = BattleConfig { record_events: false, seed: 3, fog_of_war: false, side_mults: vec![SideMult { damage: 1.0, health: 1000.0, shield: 1000.0 }, SideMult::default()], ..BattleConfig::default() };
        let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config });
        let (me, target) = (b.ships.iter().find(|s| s.side == 0).unwrap().id, b.ships.iter().find(|s| s.side == 1).unwrap().id);
        b.command(0, Command::Attack { ships: vec![me], target }).unwrap();
        let mut closest = f32::MAX;
        for _ in 0..30 * 900 {
            b.step();
            if b.is_over() {
                break;
            }
            let (s, t) = (&b.ships[me as usize], &b.ships[target as usize]);
            closest = closest.min(s.pos.dist(t.pos));
        }
        let (s, t) = (&b.ships[me as usize], &b.ships[target as usize]);
        let tdef = cat.ship(t.def);
        let stand = def.preferred_range + tdef.radius * 0.5;
        let d = s.pos.dist(t.pos);
        println!("{name}: over {} at {:.0}s result {:?} order {:?} speed {:.1}", b.is_over(), b.time(), b.result().winner, s.order, s.speed);
        println!("{name}: weapons reach {:.0}, stand-off {:.0}, stopped at {:.0}, closest {:.0}", def.max_weapon_range, stand, d, closest);
        assert!(closest >= stand * 0.8, "{name} flew in to {closest:.0} (stand-off {stand:.0})");
        assert!(d <= stand * 1.3 + 200.0, "{name} never got within range: {d:.0} vs {stand:.0}");
        checked += 1;
    }
    assert!(checked > 0, "no candidate ship found");
}


#[test]
fn hardpoint_offsets_follow_the_scaled_model_bones() {
    let Some((data, fs)) = load() else { return };
    let mut names: Vec<String> = ["Star_Destroyer", "MC-80_Star_Cruiser", "Pirate_Starbase_Lv1"].iter().map(|s| s.to_string()).collect();
    for f in ["Rebel", "Empire", "Underworld", "Pirate"] {
        for l in 1..=5 {
            names.push(format!("{f}_Star_Base_{l}"));
        }
    }
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let cat = Catalog::build(&data, Some(&fs), &names);
    let mut scaled_checked = 0;
    for name in names {
        let Some(id) = cat.ship_id(name) else { continue };
        let def = cat.ship(id);
        let scale = data.objects.resolve(name).and_then(|r| r.num::<f32>("scale_factor")).filter(|s| *s > 0.0).unwrap_or(1.0);
        let model_path = def.model.clone().expect("model");
        let model = alamo_model::Model::parse(&alamo_fs::FileSource::read(&fs, &format!("Art/Models/{model_path}")).unwrap()).unwrap();
        // Bone positions in battle space (model (x, y, z) -> (-y, x, z)), times the Scale_Factor the renderer applies.
        let bones: Vec<Vec3> = (0..model.bones.len()).map(|i| {
            let t = model.world_transform(i);
            Vec3::new(-t[1][3] * scale, t[0][3] * scale, t[2][3] * scale)
        }).collect();
        let placed: Vec<_> = def.hardpoints.iter().filter(|h| h.from_bone).collect();
        if placed.is_empty() {
            println!("{name}: no hardpoint placed from a bone");
            continue;
        }
        for h in &placed {
            let nearest = bones.iter().map(|b| b.dist(h.offset)).fold(f32::MAX, f32::min);
            assert!(nearest < 1.0, "{name} {}: offset {:?} is {nearest:.1} from the nearest scaled bone (scale {scale})", h.name, h.offset);
        }
        println!("{name}: scale {scale}, {} of {} hardpoints on scaled bones", placed.len(), def.hardpoints.len());
        if (scale - 1.0).abs() > 1e-3 {
            scaled_checked += 1;
        }
    }
    assert!(scaled_checked > 0, "none of the checked objects has a Scale_Factor, the test would not prove the scaling");
}

#[test]
fn shots_at_an_outlying_hardpoint_of_a_tall_station_can_hit_it() {
    let Some((data, fs)) = load() else { return };
    for n in ["Rebel_Star_Base_2", "Empire_Star_Base_3", "Pirate_Starbase_Lv1"] {
        let cat = Catalog::build(&data, Some(&fs), &[n]);
        let d = cat.ship(cat.ship_id(n).unwrap());
        let far = d.hardpoints.iter().filter(|h| h.destroyable).map(|h| h.offset.len()).fold(0.0f32, f32::max);
        // The collision radius is `radius * 0.75` for frigates and up; the hit radius is at least the farthest hardpoint, which lies beyond it on these stations.
        assert!(far > d.radius * 0.75, "{n}: premise: farthest hardpoint {far:.0} lies beyond the collision radius {:.0}", d.radius * 0.75);
        assert!(d.hardpoint_reach >= far, "{n}: hit reach {:.0} covers the farthest hardpoint {far:.0}", d.hardpoint_reach);
    }
}
