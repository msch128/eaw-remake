//! `NEW_POWER_FOR_ALL` from the real Rebel shield plot. Skipped when the Remake mod is not installed.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{SaveFile, ScriptedConfig, ScriptedSim};
use galaxy_sim::SimConfig;

fn build() -> Option<(ScriptedSim, Arc<dyn FileSource>, Arc<alamo_data::GameData>)> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipped: Remake mod not installed");
        return None;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
    let (data, _) = load_galaxy(&*fs);
    let data = Arc::new(data);
    let campaign = data.campaign("GC_Full_Web_Way_Rebel")?.clone();
    let sim = ScriptedSim::new(
        fs.clone(),
        data.clone(),
        &campaign,
        SimConfig {
            victory_enabled: false,
            ..SimConfig::default()
        },
        ScriptedConfig {
            remake_ai: false,
            ..ScriptedConfig::default()
        },
    )
    .ok()?;
    Some((sim, fs, data))
}

#[test]
fn rebel_shield_upgrade_grants_reflector_shield_and_saves() {
    let Some((mut sim, fs, data)) = build() else {
        return;
    };
    let rebel = sim.with_sim(|s| s.world().faction_id("Rebel").unwrap());
    let empire = sim.with_sim(|s| s.world().faction_id("Empire").unwrap());
    let has = |sim: &ScriptedSim, faction, unit: &str| {
        sim.with_sim(|s| s.has_power(faction, unit, "Reflector_Shield"))
    };
    assert!(!has(&sim, rebel, "CR-90_Corvette"));

    sim.runtime_mut()
        .story_notify("STORY_CONSTRUCT", "Rebel_Shield_Upgrade");

    assert!(has(&sim, rebel, "CR-90_Corvette"));
    assert!(has(&sim, rebel, "DP-20_Corvette"));
    assert!(!has(&sim, rebel, "X-Wing"));
    assert!(!has(&sim, empire, "CR-90_Corvette"));
    let n = sim.with_sim(|s| s.galaxy().factions[rebel as usize].powers.len());
    assert_eq!(
        n, 25,
        "one grant per type in Story_SandBox_Rebel_Shield_Upgrade.xml"
    );
    assert_eq!(
        sim.host_stats()
            .rewards_applied
            .get("NEW_POWER_FOR_ALL")
            .copied(),
        Some(25)
    );
    // Notify also fires other due story rewards, so compare the hash with and without one extra grant.
    let hash = sim.with_sim(|s| s.galaxy().state_hash());
    sim.with_sim_mut(|s| s.grant_power(rebel, "Not_A_Ship", "Reflector_Shield"));
    assert_eq!(sim.with_sim(|s| s.galaxy().state_hash()), hash);
    sim.with_sim_mut(|s| {
        s.galaxy_mut().factions[rebel as usize]
            .powers
            .remove(&("not_a_ship".into(), "reflector_shield".into()));
    });
    assert_eq!(
        sim.with_sim(|s| s.galaxy().factions[rebel as usize].powers.len()),
        25
    );

    let loaded =
        ScriptedSim::from_save(sim.to_save().unwrap(), fs.clone(), data.clone(), |_| {}).unwrap();
    assert!(has(&loaded, rebel, "CR-90_Corvette"));
    assert!(has(&loaded, rebel, "DP-20_Corvette"));
    assert_eq!(
        loaded.with_sim(|s| s.galaxy().factions[rebel as usize].powers.len()),
        25
    );

    let mut v = serde_json::to_value(sim.to_save().unwrap()).unwrap();
    for f in v["state"]["sim"]["galaxy"]["factions"]
        .as_array_mut()
        .unwrap()
    {
        f.as_object_mut().unwrap().remove("powers");
    }
    let old: SaveFile = serde_json::from_value(v).unwrap();
    let loaded_old = ScriptedSim::from_save(old, fs, data, |_| {}).unwrap();
    assert!(loaded_old.with_sim(|s| s.galaxy().factions.iter().all(|f| f.powers.is_empty())));
}
