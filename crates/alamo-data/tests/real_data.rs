//! Tests against the real Empire at War Remake mod data; skipped when it is not installed.

use alamo_data::{TextTable, load_galaxy};
use alamo_fs::{GameFs, GameInstall};

/// Layered mod + base game files; `None` (test skipped) when the game is not installed.
fn open() -> Option<GameFs> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    GameFs::for_mod(&install, Some("1770851727")).ok()
}

#[test]
fn text_table_loads() {
    let Some(src) = open() else { return };
    let t = TextTable::load(&src, "Text/MasterTextFile_ENGLISH.DAT").expect("dat parses");
    assert!(t.len() > 10_000, "only {} strings", t.len());
    assert!(t.get("GC_WEB_WAY_FULL").is_some());
}

#[test]
fn galaxy_loads() {
    let Some(src) = open() else { return };
    let (data, warnings) = load_galaxy(&src);
    eprintln!("{} warnings", warnings.len());
    assert!(
        data.campaigns.len() >= 15,
        "{} campaigns",
        data.campaigns.len()
    );

    let grand = data
        .campaign("GC_Full_Web_Way_Rebel")
        .expect("grand campaign");
    assert!(grand.has_galaxy());
    let planets = data.campaign_planets(grand);
    assert!(
        planets.len() >= grand.locations.len() - 5,
        "{} of {}",
        planets.len(),
        grand.locations.len()
    );
    assert!(planets.iter().filter(|p| p.is_real_planet()).count() > 100);
    assert!(data.campaign_trade_routes(grand).len() > 100);

    let cor = data.planet("coruscant_ww").expect("Coruscant");
    assert_eq!(cor.position, [1.0, 106.0, 10.0]);
    assert_eq!(cor.credit_value, 607);
    assert_eq!(cor.max_space_base, 3);
    assert_eq!(
        cor.space_tactical_map.as_deref(),
        Some("_Space_Planet_Coruscant_02.ted")
    );
    assert!(
        data.planet_display_name(cor)
            .to_lowercase()
            .contains("coruscant")
    );

    let rebel = grand.faction("Rebel").unwrap();
    assert_eq!(rebel.starting_credits, Some(15000));
    assert_eq!(rebel.home_location.as_deref(), Some("Yavin_WW"));
    assert!(data.faction("Rebel").unwrap().is_playable);
    assert!(data.faction("Empire").is_some());
}

#[test]
fn objects_resolve() {
    let Some(src) = open() else { return };
    let (data, _) = load_galaxy(&src);
    assert!(
        data.objects.len() > 10_000,
        "{} objects",
        data.objects.len()
    );

    let isd = data.objects.space_unit("Star_Destroyer").expect("ISD");
    assert_eq!(isd.category, alamo_data::ShipCategory::Capital);
    assert!(isd.hull > 10_000.0 && isd.shield_points > 1000.0);
    assert_eq!(isd.armor_type.as_deref(), Some("Armor_Starship"));
    assert!(
        isd.upkeep() < 0.0,
        "upkeep comes from an overriding variant"
    );
    let hps = data.objects.hardpoints_of(&isd.hardpoints);
    assert_eq!(
        hps.len(),
        isd.hardpoints.len(),
        "all ISD hardpoints resolve"
    );
    let weapon = hps
        .iter()
        .find(|h| h.is_weapon() && h.fire_range > 1000.0)
        .expect("a weapon");
    let proj = data
        .objects
        .projectile(weapon.projectile.as_deref().unwrap())
        .expect("projectile");
    assert!(proj.damage > 0.0 && proj.max_speed > 0.0);

    let sq = data
        .objects
        .space_unit("X-Wing_Squadron")
        .expect("squadron");
    assert!(sq.is_squadron() && sq.squadron_units.len() == 6);
    assert_eq!(sq.category, alamo_data::ShipCategory::Fighter);

    let base = data
        .objects
        .star_base("Empire_Star_Base_2")
        .expect("starbase");
    assert_eq!(base.base_level, 2);
    assert_eq!(base.next_level.as_deref(), Some("Empire_Star_Base_3"));

    let income_structures = data
        .objects
        .iter()
        .filter_map(|o| data.objects.structure(&o.name))
        .filter(|s| !s.income_modifiers.is_empty())
        .count();
    assert!(income_structures > 50, "{income_structures}");
    assert!(data.objects.structure("Mining_Facility_E").is_some());

    assert!(data.combat.len() > 500);
    let m = data
        .combat
        .multiplier("Damage_Heavy_Laser", "Armor_Starship");
    assert!((m - 0.3).abs() < 1e-6, "{m}");
}

#[test]
fn ownership_is_consistent() {
    let Some(src) = open() else { return };
    let (data, _) = load_galaxy(&src);
    for c in data.campaigns.iter().filter(|c| c.has_galaxy()) {
        let owners = data.campaign_planet_owners(c);
        assert!(owners.len() > 50, "{}: {}", c.name, owners.len());
        for (planet, faction) in &owners {
            assert!(
                data.planet(planet).is_some(),
                "{}: unknown planet {planet}",
                c.name
            );
            assert!(
                data.faction(faction).is_some(),
                "{}: unknown faction {faction}",
                c.name
            );
        }
        // Every home location is owned by someone (a structure owner wins over the home label).
        for f in &c.factions {
            if let Some(home) = &f.home_location {
                assert!(
                    owners.contains_key(home),
                    "{}: home of {} unowned",
                    c.name,
                    f.name
                );
            }
        }
    }
    let grand = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let owners = data.campaign_planet_owners(grand);
    assert_eq!(
        owners.get("Coruscant_WW").map(String::as_str),
        Some("Empire")
    );
    assert_eq!(owners.get("Yavin_WW").map(String::as_str), Some("Rebel"));
}
