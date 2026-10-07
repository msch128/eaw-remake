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
    assert!(bow.offset.x > 300.0 && shield.offset.x < -100.0, "bow {:?} shield {:?}", bow.offset, shield.offset);
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

