//! Space-battle environment against the real Remake data. Skipped when the game is not installed.
//!
//! The old classifier guessed a radius from the name (huge 600, large 350, medium 180, small 80,
//! anything else 200) and treated every name containing "asteroid" as a solid rock.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_map::{MapObject, SpaceMap};
use battle_sim::*;

fn load() -> Option<alamo_data::GameData> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    Some(load_galaxy(&fs).0)
}

fn load_fs() -> Option<(alamo_data::GameData, GameFs)> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    let data = load_galaxy(&fs).0;
    Some((data, fs))
}

#[test]
fn prop_radii_come_from_the_object_not_the_name() {
    let Some(data) = load() else { return };
    let huge = prop_zones(&data, "Asteroid_Huge");
    assert_eq!(huge.rock, Some(280.0), "Space_Obstacle_Radius, was name-guess 600");
    assert_eq!(huge.field, None);

    let field = prop_zones(&data, "Asteroid_Field_Huge_Obstacle_Damage_8000");
    assert_eq!(field.field, Some(3500.0));
    assert_eq!(field.rock, None, "a damage field is not a rock (was name-guess 600)");

    let small = prop_zones(&data, "Asteroid Field Small");
    assert_eq!(small.rock, None, "radius is commented out; was name-guess 80");
    assert_eq!(small.field, None);

    let yard = prop_zones(&data, "Star_Destroyer_Construction");
    assert_eq!(yard.rock, Some(300.0), "name has no 'asteroid'; was not an obstacle");
    assert_eq!(yard.offset, [100.0, 0.0, 0.0]);

    let neb = prop_zones(&data, "Nebula_Small");
    assert_eq!(neb.nebula, Some(135.0));
    assert_eq!(neb.rock, None);
    assert_eq!(neb.ion, None);

    let named_ion = prop_zones(&data, "Nebula_Friggin_Huge_Ion");
    assert_eq!(named_ion.nebula, Some(2800.0));
    assert_eq!(named_ion.ion, None, "Is_Ion_Storm is No");

    let small_ion = prop_zones(&data, "Nebula_Small_Ion");
    assert_eq!((small_ion.nebula, small_ion.ion), (Some(135.0), Some(135.0)));

    // Is_Nebula and Is_Ion_Storm with radius 40000, but Behavior IDLE only (no SPACE_OBSTACLE) and In_Background.
    let backdrop = prop_zones(&data, "Nebula_Ultra_Huge");
    assert_eq!(backdrop.nebula, None, "was nebula 40000");
    assert_eq!(backdrop.ion, None, "was ion storm 40000");

    let destroyable = prop_zones(&data, "Destroyable_Asteroid_Huge");
    assert_eq!(destroyable.rock, None, "footprint 320 is not Space_Obstacle_Radius; was name-guess 600");

    let mut map = SpaceMap::default();
    map.bounds.min = [-1000.0, -1000.0, 0.0];
    map.bounds.max = [1000.0, 1000.0, 0.0];
    map.objects.push(MapObject {
        type_name: Some("Star_Destroyer_Construction".into()),
        position: [10.0, 20.0, 0.0],
        rotation_deg: [0.0, 0.0, 90.0],
        ..MapObject::default()
    });
    map.objects.push(MapObject {
        type_name: Some("Asteroid_Field_Huge_Obstacle_Damage_8000".into()),
        position: [0.0, 0.0, 0.0],
        ..MapObject::default()
    });
    let arena = Arena::from_space_map(&data, &map);
    assert_eq!(arena.obstacles.len(), 1);
    let rock = &arena.obstacles[0];
    assert!((rock.radius - 300.0).abs() < 1e-3);
    assert!((rock.pos.x - 10.0).abs() < 1e-3, "yaw 90 leaves x at {}", rock.pos.x);
    assert!((rock.pos.y - 120.0).abs() < 1e-3, "offset 100 rotated to +Y, got {}", rock.pos.y);
    assert_eq!(arena.asteroid_fields.len(), 1);
    assert!((arena.asteroid_fields[0].radius - 3500.0).abs() < 1e-3);
    assert!(arena.nebulae.is_empty() && arena.ion_storms.is_empty());
}

#[test]
fn space_maps_with_fields_nebulae_or_ion_storms() {
    let Some((data, fs)) = load_fs() else { return };
    let mut ted = 0u32;
    let mut parsed = 0u32;
    let mut lines = Vec::new();
    for path in fs.list("Art/Maps") {
        let Some(name) = path.strip_prefix("ART/MAPS/") else { continue };
        if !name.ends_with(".TED") {
            continue;
        }
        ted += 1;
        let Some(arena) = Arena::load(&data, &fs, name) else { continue };
        parsed += 1;
        if arena.nebulae.is_empty() && arena.ion_storms.is_empty() && arena.asteroid_fields.is_empty() {
            continue;
        }
        let mut props: Vec<String> = arena
            .nebulae
            .iter()
            .chain(&arena.ion_storms)
            .chain(&arena.asteroid_fields)
            .map(|o| o.name.clone())
            .collect();
        props.sort();
        props.dedup();
        lines.push(format!(
            "{name} nebula={} ion={} field={} rocks={} props={}",
            arena.nebulae.len(),
            arena.ion_storms.len(),
            arena.asteroid_fields.len(),
            arena.obstacles.len(),
            props.join(",")
        ));
    }
    lines.sort();
    eprintln!("ted files {ted}, parsed {parsed}, with a zone {}", lines.len());
    for line in &lines {
        eprintln!("  {line}");
    }
    assert_eq!((ted, parsed, lines.len()), (372, 372, 16));
    assert_eq!(
        lines,
        [
            "MENUSPACEVULCANICPLANET.TED nebula=0 ion=0 field=3 rocks=1 props=Asteroid_Field_Huge_Obstacle_Damage_4000,Asteroid_Field_Huge_Obstacle_Damage_6000,Asteroid_Field_Huge_Obstacle_Damage_8000",
            "_SPACE_PLANET_AETENII_01.TED nebula=0 ion=0 field=5 rocks=1 props=Asteroid_Field_Huge_Obstacle_Damage_4000,Asteroid_Field_Huge_Obstacle_Damage_6000,Asteroid_Field_Huge_Obstacle_Damage_8000",
            "_SPACE_PLANET_ERAIDU_01.TED nebula=0 ion=0 field=4 rocks=1 props=Asteroid_Field_Huge_Obstacle_Damage_4000,Asteroid_Field_Huge_Obstacle_Damage_6000,Asteroid_Field_Huge_Obstacle_Damage_8000",
            "_SPACE_PLANET_GALAXY_ASTEROIDS.TED nebula=0 ion=0 field=6 rocks=2 props=Asteroid_Field_Huge_Obstacle_Damage_4000,Asteroid_Field_Huge_Obstacle_Damage_6000,Asteroid_Field_Huge_Obstacle_Damage_8000",
            "_SPACE_PLANET_GEONOSIS_01.TED nebula=0 ion=0 field=6 rocks=2 props=Asteroid_Field_Huge_Obstacle_Damage_4000,Asteroid_Field_Huge_Obstacle_Damage_6000,Asteroid_Field_Huge_Obstacle_Damage_8000",
            "_SPACE_PLANET_ILUM_01.TED nebula=0 ion=0 field=6 rocks=0 props=Asteroid_Field_Huge_Obstacle_Damage_4000,Asteroid_Field_Huge_Obstacle_Damage_6000,Asteroid_Field_Huge_Obstacle_Damage_8000",
            "_SPACE_PLANET_ILUM_02.TED nebula=0 ion=0 field=6 rocks=0 props=Asteroid_Field_Huge_Obstacle_Damage_4000,Asteroid_Field_Huge_Obstacle_Damage_6000,Asteroid_Field_Huge_Obstacle_Damage_8000",
            "_SPACE_PLANET_JABIIM_01.TED nebula=0 ion=0 field=3 rocks=0 props=Asteroid_Field_Huge_Obstacle_Damage_4000,Asteroid_Field_Huge_Obstacle_Damage_8000",
            "_SPACE_PLANET_KESSEL_02.TED nebula=0 ion=0 field=4 rocks=1 props=Asteroid_Field_Huge_Obstacle_Damage_4000,Asteroid_Field_Huge_Obstacle_Damage_8000",
            "_SPACE_PLANET_MUSTAFAR_01.TED nebula=0 ion=0 field=2 rocks=1 props=Asteroid_Field_Huge_Obstacle_Damage_8000",
            "_SPACE_PLANET_MUUNILINST_01.TED nebula=0 ion=0 field=3 rocks=1 props=Asteroid_Field_Huge_Obstacle_Damage_4000,Asteroid_Field_Huge_Obstacle_Damage_6000,Asteroid_Field_Huge_Obstacle_Damage_8000",
            "_SPACE_PLANET_SULLUST_01.TED nebula=0 ion=0 field=4 rocks=1 props=Asteroid_Field_Huge_Obstacle_Damage_4000,Asteroid_Field_Huge_Obstacle_Damage_6000,Asteroid_Field_Huge_Obstacle_Damage_8000",
            "_SPACE_PLANET_VERGESSOASTEROIDS_01.TED nebula=0 ion=0 field=1 rocks=0 props=Asteroid_Field_Huge_Obstacle_Damage_4000",
            "_SPACE_PLANET_VERGESSOASTEROIDS_02.TED nebula=0 ion=0 field=1 rocks=0 props=Asteroid_Field_Huge_Obstacle_Damage_4000",
            "_SPACE_PLANET_VERGESSOASTEROIDS_03.TED nebula=0 ion=0 field=1 rocks=0 props=Asteroid_Field_Huge_Obstacle_Damage_4000",
            "_SPACE_PLANET_VERGESSOASTEROIDS_04.TED nebula=0 ion=0 field=1 rocks=0 props=Asteroid_Field_Huge_Obstacle_Damage_4000",
        ]
    );
}

/// A battle on an empty test map with one `Asteroid_Field_Huge_Obstacle_Damage_8000` (radius 3500) at the origin. Side 0 holds `units`,
/// side 1 one `Star_Destroyer` far outside the field (so the battle does not end).
fn field_battle(data: &alamo_data::GameData, fs: &GameFs, units: &[&str]) -> Battle {
    let mut names: Vec<&str> = units.to_vec();
    names.push("Star_Destroyer");
    let cat = Arc::new(Catalog::build(data, Some(fs as &dyn FileSource), &names));
    let mut map = SpaceMap::default();
    map.bounds.min = [-30000.0, -30000.0, 0.0];
    map.bounds.max = [30000.0, 30000.0, 0.0];
    map.objects.push(MapObject { type_name: Some("Asteroid_Field_Huge_Obstacle_Damage_8000".into()), ..MapObject::default() });
    let arena = Arena::from_space_map(data, &map);
    let unit = |n: &str| UnitSetup { ship: cat.ship_id(n).unwrap_or_else(|| panic!("{n}")), count: 1, tag: 1 };
    let sides = vec![
        SideSetup { name: "Empire".into(), units: units.iter().map(|n| unit(n)).collect(), structures: vec![], ai: false },
        SideSetup { name: "Rebel".into(), units: vec![unit("Star_Destroyer")], structures: vec![], ai: false },
    ];
    let config = BattleConfig { seed: 7, arena: Some(Arc::new(arena)), ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat, sides, config });
    let mut x = -1500.0;
    for s in b.ships.iter_mut().filter(|s| s.carrier == NONE) {
        s.pos = if s.side == 0 { Vec3::new(x, 0.0, 0.0) } else { Vec3::new(25000.0, 25000.0, 0.0) };
        x += 100.0;
    }
    b
}

#[test]
fn asteroid_field_damages_ships_with_the_behaviour() {
    let Some((data, fs)) = load_fs() else { return };
    let mut b = field_battle(&data, &fs, &["Victory_Destroyer", "TIE_Fighter_Squadron"]);
    let cat = b.catalog().clone();
    assert_eq!((cat.asteroid_field_damage, cat.asteroid_field_damage_rate), (200.0, 0.05), "Gameconstants.xml");
    let vsd = cat.ship_id("Victory_Destroyer").unwrap();
    assert!(cat.ship(vsd).asteroid_field_damage, "Vessel_Base SpaceBehavior");
    let fighters: Vec<u32> = b.ships.iter().filter(|s| s.side == 0 && s.carrier == NONE && cat.ship(s.def).class == TargetClass::Fighter).map(|s| s.id).collect();
    assert!(!fighters.is_empty());
    assert!(fighters.iter().all(|&f| !cat.ship(b.ships[f as usize].def).asteroid_field_damage), "Spacecraft_Base has no ASTEROID_FIELD_DAMAGE");
    let victory = b.ships.iter().find(|s| s.def == vsd).unwrap().id;
    let enemies: Vec<u32> = b.ships.iter().filter(|s| s.side == 1).map(|s| s.id).collect();
    let (shield0, hull0) = (b.ships[victory as usize].shield, b.ships[victory as usize].hull);

    let steps = 30 * 20;
    let (mut hits, mut entered, mut damage) = (Vec::new(), Vec::new(), 0.0f32);
    for _ in 0..steps {
        for e in b.step() {
            match *e {
                Event::AsteroidFieldHit { ship, .. } => hits.push(ship),
                Event::EnteredAsteroidField { ship } => entered.push(ship),
                Event::Hit { ship, projectile, damage: d, .. } if projectile == NO_PROJ => {
                    assert_eq!(ship, victory);
                    damage += d;
                }
                _ => {}
            }
        }
    }
    let lost = (shield0 - b.ships[victory as usize].shield) + (hull0 - b.ships[victory as usize].hull);
    eprintln!("{steps} steps: {} hits, {damage} damage reported, shield+hull lost {lost}, entered {}", hits.len(), entered.len());
    assert!(hits.iter().all(|&s| s == victory), "only the ASTEROID_FIELD_DAMAGE ship is hit");
    // 0.05 per frame over 600 frames: 30 expected.
    assert!((15..=45).contains(&hits.len()), "{} hits", hits.len());
    assert!((damage - 200.0 * hits.len() as f32).abs() < 1.0, "200 per hit, Damage_Asteroid has no armor modifier");
    assert!(lost > 0.0);
    assert!(entered.contains(&victory) && fighters.iter().all(|f| entered.contains(f)), "every ship inside reports the entry");
    assert!(enemies.iter().all(|e| !entered.contains(e)), "the ships outside the field");
    assert!(fighters.iter().all(|f| entered.iter().filter(|e| *e == f).count() == 1), "once per entry");
}
