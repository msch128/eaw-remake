//! Reflector shields start disabled and turn on only for a unit type the story granted.

use std::collections::BTreeSet;
use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use battle_sim::*;

fn load() -> Option<(alamo_data::GameData, GameFs)> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    Some((load_galaxy(&fs).0, fs))
}

fn side(cat: &Catalog, name: &str, units: &[(&str, u32)]) -> SideSetup {
    SideSetup {
        name: name.into(),
        units: units.iter().map(|&(n, c)| UnitSetup { ship: cat.ship_id(n).expect(n), count: c, tag: 1 }).collect(),
        structures: vec![],
        ai: false,
    }
}

/// Hull and shield left on `ship` after a Star Destroyer shoots it. `grant` is the unit type whose reflector is on.
fn remain(cat: &Arc<Catalog>, ship: &str, grant: Option<&str>) -> (f32, f32) {
    let mut rebel = BTreeSet::new();
    if let Some(unit) = grant {
        rebel.insert((unit.to_ascii_lowercase(), "reflector_shield".into()));
    }
    let mut b = Battle::new(BattleSetup {
        catalog: cat.clone(),
        sides: vec![side(cat, "Empire", &[("Star_Destroyer", 1)]), side(cat, "Rebel", &[(ship, 1)])],
        config: BattleConfig {
            fog_of_war: false,
            shield_regen_delay: 10_000.0,
            record_events: false,
            seed: 1,
            retreat_allowed: vec![false, false],
            side_mults: vec![SideMult { damage: 1.0, health: 1.0, shield: 1.0 }, SideMult { damage: 0.0, health: 80.0, shield: 80.0 }],
            powers: vec![BTreeSet::new(), rebel],
            ..BattleConfig::default()
        },
    });
    let isd = b.ships.iter().find(|s| s.side == 0).unwrap().id;
    let corvette = b.ships.iter().find(|s| s.side == 1).unwrap().id;
    b.ships[isd as usize].pos = Vec3::new(0.0, 0.0, 0.0);
    b.ships[corvette as usize].pos = Vec3::new(600.0, 0.0, 0.0);
    b.command(0, Command::Attack { ships: vec![isd], target: corvette }).unwrap();
    for _ in 0..30 * 12 {
        b.step();
    }
    let s = &b.ships[corvette as usize];
    assert!(s.alive, "{ship} died; hull {hull} shield {shield}", hull = s.hull, shield = s.shield);
    (s.hull, s.shield)
}

#[test]
fn cr90_and_dp20_reflector_shields_follow_the_grant() {
    let Some((data, fs)) = load() else { return };
    let cat = Arc::new(Catalog::build(
        &data,
        Some(&fs),
        &["CR-90_Corvette", "DP-20_Corvette", "CR-90_Corvette_CSA", "Star_Destroyer"],
    ));
    for name in ["CR-90_Corvette", "DP-20_Corvette"] {
        let d = cat.ship(cat.ship_id(name).unwrap());
        assert!(d.redirects.is_empty(), "{name} starts with the shield off");
        let (ability, r) = d.locked_redirects.iter().find(|(n, _)| n == "reflector_shield").unwrap_or_else(|| panic!("{name}: {:?}", d.locked_redirects));
        assert_eq!(ability, "reflector_shield");
        assert!((r.block - 0.15).abs() < 1e-4 && r.redirect == 0.0, "{r:?}");
        assert!((r.half_arc.to_degrees() - 175.0).abs() < 0.05, "350 degree arc, got {}", r.half_arc.to_degrees() * 2.0);
        assert!(r.classes & TargetClass::Capital.bit() != 0);

        let bare = remain(&cat, name, None);
        let other = if name.starts_with("CR-90") { "DP-20_Corvette" } else { "CR-90_Corvette" };
        assert_eq!(remain(&cat, name, Some(other)), bare, "a grant for {other} does not cover {name}");
        let granted = remain(&cat, name, Some(name));
        assert!(granted.0 + granted.1 > bare.0 + bare.1 + 1.0, "{name} grant {:?} vs none {bare:?}", granted);
    }
    let csa = cat.ship(cat.ship_id("CR-90_Corvette_CSA").unwrap());
    assert!(csa.redirects.is_empty() && csa.locked_redirects.is_empty(), "the CSA variant has no reflector");
}
