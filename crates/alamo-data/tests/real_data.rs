//! Tests against the real Empire at War Remake mod data; skipped when it is not installed.

use alamo_data::{TextTable, load_galaxy, parse_attrition_credits};
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

#[test]
fn campaign_sets_list_the_playable_factions() {
    let Some(src) = open() else { return };
    let (data, _) = load_galaxy(&src);
    let sets = data.campaign_sets();
    for s in &sets {
        let names: Vec<String> = s.campaigns.iter().map(|p| format!("{} -> {} ({})", p.faction.name, p.campaign.name, p.slot.and_then(|c| c.home_location.clone()).unwrap_or_default())).collect();
        println!("set {}: {}", s.name, names.join(", "));
    }
    assert!(sets.len() >= 4, "Mini, Standard, Full and the casual / observer sets: {}", sets.len());
    let full = sets.iter().find(|s| s.name.eq_ignore_ascii_case("Web_Way4")).expect("the Full galaxy set");
    let factions: Vec<&str> = full.campaigns.iter().map(|p| p.faction.name.as_str()).collect();
    assert_eq!(factions.len(), 3);
    for f in ["Rebel", "Empire", "Underworld"] {
        assert!(factions.contains(&f), "{f} in {factions:?}");
    }
    assert!(full.campaigns.iter().all(|p| p.faction.is_playable));
    let playable: Vec<&str> = data.playable_factions().iter().map(|f| f.name.as_str()).collect();
    let flagged: Vec<&str> = data.flagged_playable_without_campaign().iter().map(|f| f.name.as_str()).collect();
    println!("playable in GC: {playable:?}\nflagged Is_Playable but only AI: {flagged:?}");
    let rebel = full.campaigns.iter().find(|p| p.faction.name == "Rebel").unwrap();
    let opponents: Vec<&str> = data.campaign_opponents(rebel.campaign).iter().map(|(f, _)| f.name.as_str()).collect();
    println!("opponents in {}: {opponents:?}", rebel.campaign.name);
    assert!(opponents.contains(&"Empire") && opponents.contains(&"Underworld") && !opponents.contains(&"Rebel"));
}

#[test]
fn attrition_credits_from_real_data() {
    let Some(src) = open() else { return };
    let (data, _) = load_galaxy(&src);

    // 1. Units with a penalty dummy:
    let tie = data.objects.space_unit("TIE_Fighter_G").expect("TIE_Fighter_G");
    assert_eq!(tie.attrition_credits(&data.objects), -20);

    let defender = data.objects.space_unit("TIE_Defender_G").expect("TIE_Defender_G");
    assert_eq!(defender.attrition_credits(&data.objects), -110);

    let tartan = data.objects.space_unit("Tartan_Gunship_G").expect("Tartan_Gunship_G");
    assert_eq!(tartan.attrition_credits(&data.objects), -550);

    // Hero units with custom named penalty dummies:
    let zaarin = data.objects.space_unit("Vindicator_Cruiser_Kas").expect("Vindicator_Cruiser_Kas");
    assert_eq!(zaarin.attrition_credits(&data.objects), -9000);

    let harkov = data.objects.space_unit("Neb_Akaga").expect("Neb_Akaga");
    assert_eq!(harkov.attrition_credits(&data.objects), -3000);

    // 2. Hero with a refund dummy:
    let tarkin = data.objects.space_unit("Havelon").expect("Havelon");
    assert_eq!(tarkin.attrition_credits(&data.objects), 2400);

    let ackbar = data.objects.space_unit("Home_One").expect("Home_One");
    assert_eq!(ackbar.attrition_credits(&data.objects), 10125);

    // 3. Super ship phase object that must give 0:
    let viscount = data.objects.space_unit("Viscount_Star_Defender").expect("Viscount_Star_Defender");
    assert_eq!(viscount.attrition_credits(&data.objects), 0);

    // 4. A unit listing several pairs:
    let grav = data.objects.structure("E_Gravity_Well_Station").expect("E_Gravity_Well_Station");
    assert_eq!(grav.attrition_credits(&data.objects), 0);

    // Multi-pair string resolving against real dummy objects from _Attrition.xml:
    let multi = "20_Death_Penalty_Dummy, 2\n110_Death_Penalty_Dummy, 1.0\nViscount_Star_Defender_P2, 1.0\n2400_Reverse_Death_Penalty_Dummy, 1.0";
    assert_eq!(parse_attrition_credits(Some(multi), &data.objects), 2 * -20 + -110 + 0 + 2400);
}

/// `Projectile_Ion_Stun_*_Percent` are fractions in the data; three are written with a float suffix (`0.999f`).
#[test]
fn ion_stun_fractions_parse_with_a_float_suffix() {
    let Some(src) = open() else { return };
    let (data, _) = load_galaxy(&src);
    let planetary = data.objects.projectile("Proj_Ion_Cannon_Planetary").expect("planetary ion cannon shot");
    assert_eq!(planetary.ion_stun_speed_reduction, 0.999);
    let large = data.objects.projectile("Proj_Ship_Ion_Cannon_Large_Dual").expect("large dual ion cannon shot");
    assert_eq!((large.ion_stun_speed_reduction, large.ion_stun_shot_rate_reduction, large.ion_stun_duration), (0.4, 0.4, 4.5));
}
