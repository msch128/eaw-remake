//! Tests for spawner rules and building family duplicate prevention (finding 1 from economy audit).
//!
//! Spawner structures (e.g. `TAXATION_AGENCY_URBAN_SPAWNER`, `E_GOLAN_1_SPAWNER`) represent
//! building families. Once an instance of the family exists (whether base or upgraded),
//! the spawner cannot be built again on the same planet. If completed while a family member
//! exists, it aborts with an 80% refund. On successful completion, it places the base structure.

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn load() -> Option<GameData> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    Some(load_galaxy(&fs).0)
}

fn sim_of(data: &GameData, campaign_name: &str) -> Sim {
    let campaign = data.campaign(campaign_name).expect("campaign");
    Sim::new(data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim")
}

#[test]
fn income_buildings_do_not_stack_on_coruscant() {
    let Some(data) = load() else { return };
    let mut sim = sim_of(&data, "GC_Full_Web_Way_Imperial");
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let cor = w.planet_id("Coruscant_WW").unwrap();

    let spawner = w.structure_id("TAXATION_AGENCY_URBAN_SPAWNER").expect("spawner exists");
    let agency = w.structure_id("Taxation_Agency_Urban").expect("agency exists");
    let office = w.structure_id("Administration_Office_Urban").expect("office exists");

    // Initially Coruscant has no taxation agency or administration office
    assert!(!sim.galaxy().planets[cor as usize].structures.contains(&agency));
    assert!(!sim.galaxy().planets[cor as usize].structures.contains(&office));

    // Player can build the spawner
    assert!(w.check_build_structure(sim.galaxy(), empire, cor, spawner).is_ok());

    // Queue and complete the spawner
    sim.apply(Command::BuildStructure { faction: empire, planet: cor, structure: spawner }).expect("queue spawner");
    sim.galaxy_mut().planets[cor as usize].queues[Lane::Ground as usize][0].remaining = 0.01;
    sim.step(1.0);

    // On completion, the base structure (Taxation_Agency_Urban) is placed, NOT the spawner
    assert!(sim.galaxy().planets[cor as usize].structures.contains(&agency), "base structure placed");
    assert!(!sim.galaxy().planets[cor as usize].structures.contains(&spawner), "spawner not left on planet");

    // While Taxation_Agency_Urban is standing, spawner build is refused
    assert_eq!(
        w.check_build_structure(sim.galaxy(), empire, cor, spawner),
        Err(CommandError::NoSlot),
        "spawner cannot be queued when base structure is already present"
    );

    // Upgrade Taxation_Agency_Urban to Administration_Office_Urban
    let upgrade_unit = w.unit_id("UPGRADE_TAXATION_AGENCY_URBAN").expect("upgrade dummy unit");
    sim.apply(Command::BuildUnit { faction: empire, planet: cor, unit: upgrade_unit }).expect("queue upgrade");
    sim.galaxy_mut().planets[cor as usize].queues[Lane::Ground as usize][0].remaining = 0.01;
    sim.step(1.0);

    assert!(sim.galaxy().planets[cor as usize].structures.contains(&office), "office placed by upgrade");
    assert!(!sim.galaxy().planets[cor as usize].structures.contains(&agency), "agency replaced");

    // After upgrading to office, spawner build is STILL refused! (Finding 1: income buildings stack)
    assert_eq!(
        w.check_build_structure(sim.galaxy(), empire, cor, spawner),
        Err(CommandError::NoSlot),
        "spawner cannot be queued when upgraded office is already present"
    );

    // If forcefully queued and completed while office exists, it aborts and refunds 80% of cost (2800 * 0.8 = 2240)
    let spawner_cost = f64::from(w.structures[spawner as usize].cost);
    assert_eq!(spawner_cost, 2800.0);
    sim.galaxy_mut().planets[cor as usize].queues[Lane::Ground as usize].push_back(galaxy_sim::ProductionItem {
        kind: ProductionKind::Structure(spawner),
        remaining: 0.01,
        total: 60.0,
        cost: spawner_cost as f32,
    });
    let credits_before = sim.galaxy().factions[empire as usize].credits;
    let (per_tick, _) = {
        let fs = &sim.galaxy().factions[empire as usize];
        let income = fs.income_per_week;
        let scaled = if income > 0.0 { income * f64::from(fs.difficulty.credit) } else { income };
        (scaled / f64::from(sim.world().fiscal_cycle_seconds), fs.credits)
    };
    sim.step(1.0);
    let refund = sim.galaxy().factions[empire as usize].credits - credits_before - per_tick;
    assert!((refund - (spawner_cost * 0.8)).abs() < 1e-3, "refunded 80% on abort: got {refund}");
    assert!(!sim.galaxy().planets[cor as usize].structures.contains(&spawner), "spawner not added on abort");
    assert_eq!(
        sim.galaxy().planets[cor as usize].structures.iter().filter(|&&s| s == office).count(),
        1,
        "still only one office"
    );
}

#[test]
fn building_families_prevent_duplicate_spawner_across_factions() {
    let Some(data) = load() else { return };
    let sim = sim_of(&data, "GC_Full_Web_Way_Imperial");
    let w = sim.world();

    let families = [
        // (Spawner, Base, Higher)
        // Empire Gov
        ("TAXATION_AGENCY_URBAN_SPAWNER", "Taxation_Agency_Urban", "Administration_Office_Urban"),
        ("TAXATION_AGENCY_TEMPERATE_SPAWNER", "Taxation_Agency_Temperate", "Administration_Office_Temperate"),
        ("TAXATION_AGENCY_BARREN_SPAWNER", "Taxation_Agency_Barren", "Administration_Office_Barren"),
        // Agricultural
        ("AGRICULTURAL_COMMUNITY_E_TEMPERATE_SPAWNER", "Agricultural_Community_E_Temperate", "Agricultural_Community_Lv2_E_Temperate"),
        ("AGRICULTURAL_COMMUNITY_2_E_TEMPERATE_SPAWNER", "Agricultural_Community_2_E_Temperate", "Agricultural_Community_Lv2_2_E_Temperate"),
        ("AGRICULTURAL_COMMUNITY_R_TEMPERATE_SPAWNER", "Agricultural_Community_R_Temperate", "Agricultural_Community_Lv2_R_Temperate"),
        ("AGRICULTURAL_COMMUNITY_2_R_TEMPERATE_SPAWNER", "Agricultural_Community_2_R_Temperate", "Agricultural_Community_Lv2_2_R_Temperate"),
        ("AGRICULTURAL_COMMUNITY_U_TEMPERATE_SPAWNER", "Agricultural_Community_U_Temperate", "Agricultural_Community_Lv2_U_Temperate"),
        ("AGRICULTURAL_COMMUNITY_2_U_TEMPERATE_SPAWNER", "Agricultural_Community_2_U_Temperate", "Agricultural_Community_Lv2_2_U_Temperate"),
        // Mining Facility
        ("MINING_FACILITY_E_URBAN_SPAWNER", "Mining_Facility_E_Urban", "Mining_Facility_Lv2_E_Urban"),
        ("MINING_FACILITY_2_E_URBAN_SPAWNER", "Mining_Facility_2_E_Urban", "Mining_Facility_Lv2_2_E_Urban"),
        ("MINING_FACILITY_R_URBAN_SPAWNER", "Mining_Facility_R_Urban", "Mining_Facility_Lv2_R_Urban"),
        ("MINING_FACILITY_2_R_URBAN_SPAWNER", "Mining_Facility_2_R_Urban", "Mining_Facility_Lv2_2_R_Urban"),
        // Mining Company
        ("MINING_COMPANY_E_BARREN_SPAWNER", "Mining_Company_E_Barren", "Mining_Company_Lv2_E_Barren"),
        ("MINING_COMPANY_R_BARREN_SPAWNER", "Mining_Company_R_Barren", "Mining_Company_Lv2_R_Barren"),
        // Trade Hub
        ("TRADE_HUB_E_SPAWNER", "E_Trade_Hub", "E_Trade_Hub_Lv2"),
        ("TRADE_HUB_R_SPAWNER", "R_Trade_Hub", "R_Trade_Hub_Lv2"),
        ("TRADE_HUB_U_SPAWNER", "U_Trade_Hub", "U_Trade_Hub_Lv2"),
        // Golan
        ("E_GOLAN_1_SPAWNER", "E_Golan_1", "E_Golan_2"),
        ("R_GOLAN_1_SPAWNER", "R_Golan_1", "R_Golan_2"),
        ("U_GOLAN_1_SPAWNER", "U_Golan_1", "U_Golan_2"),
        ("CIS_GOLAN_1_SPAWNER", "CIS_Golan_1", "CIS_Golan_2"),
        ("Republic_GOLAN_1_SPAWNER", "Republic_Golan_1", "Republic_Golan_2"),
        ("CSA_GOLAN_1_SPAWNER", "CSA_Golan_1", "CSA_Golan_2"),
        ("Warlords_GOLAN_1_SPAWNER", "Warlords_Golan_1", "Warlords_Golan_2"),
        ("Hapan_GOLAN_1_SPAWNER", "Hapan_Golan_1", "Hapan_Golan_2"),
        ("Mandalorians_GOLAN_1_SPAWNER", "Mandalorians_Golan_1", "Mandalorians_Golan_2"),
        ("Cartels_GOLAN_1_SPAWNER", "Cartels_Golan_1", "Cartels_Golan_2"),
        ("Sith_GOLAN_1_SPAWNER", "Sith_Golan_1", "Sith_Golan_2"),
        // Mercenary
        ("E_MERCENARY_OUTPOST_SPAWNER", "E_Mercenary_Outpost", "E_Mercenary_Settlement"),
        ("R_MERCENARY_OUTPOST_SPAWNER", "R_Mercenary_Outpost", "R_Mercenary_Settlement"),
        ("U_MERCENARY_OUTPOST_SPAWNER", "U_Mercenary_Outpost", "U_Mercenary_Settlement"),
    ];

    for (spawner_name, base_name, higher_name) in families {
        let sp_id = w.structure_id(spawner_name).unwrap_or_else(|| panic!("missing {spawner_name}"));
        let base_id = w.structure_id(base_name).unwrap_or_else(|| panic!("missing {base_name}"));
        let higher_id = w.structure_id(higher_name).unwrap_or_else(|| panic!("missing {higher_name}"));

        let st = &w.structures[sp_id as usize];
        assert!(st.family.contains(&sp_id), "{spawner_name} family must contain spawner");
        assert!(st.family.contains(&base_id), "{spawner_name} family must contain base {base_name}");
        assert!(st.family.contains(&higher_id), "{spawner_name} family must contain higher {higher_name}");

        let rule = st.spawner.as_ref().unwrap_or_else(|| panic!("{spawner_name} must have SpawnerRule"));
        assert_eq!(rule.base, base_id, "{spawner_name} rule base should be {base_name}");
        assert!(rule.higher.contains(&higher_id), "{spawner_name} rule higher should contain {higher_name}");
    }
}
