//! Clone Wars stack (`fft`): the Republic and CIS story plots start without errors and the Remake AI plays the other side.
//! Skipped when a workshop folder is missing.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::SimConfig;

#[test]
fn republic_and_cis_story_plots_start() {
    let Some(install) = GameInstall::detect() else { return };
    if ["3229239424", "2794270450", "1770851727"].iter().any(|id| !install.mod_data(id).is_dir()) {
        eprintln!("skipping: Clone Wars stack not installed");
        return;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("fft")).unwrap());
    let data = Arc::new(load_galaxy(&*fs).0);
    for human in ["Republic", "CIS"] {
        let campaign = data.campaign(&format!("GC_Full_Web_Way_{human}")).expect("campaign").clone();
        let sim = ScriptedSim::new(
            fs.clone(),
            data.clone(),
            &campaign,
            SimConfig { victory_enabled: false, ..SimConfig::default() },
            ScriptedConfig { game_scoring: false, ..ScriptedConfig::default() },
        )
        .expect("scripted sim");
        println!("{human}: start errors {:?}", sim.start_errors);
        assert!(!sim.start_errors.iter().any(|e| e.contains("story")), "{human}: {:?}", sim.start_errors);
    }
}

/// The Clone Wars plots lock the GCW-era units (`LOCK_UNIT` at `Universal_Story_Start` in `Story_SandBox_Minors_{Republic,CIS}_WW_CW.xml`); the build rules must refuse them.
#[test]
fn clone_wars_era_locks_make_gcw_units_unbuildable() {
    let Some(install) = GameInstall::detect() else { return };
    if ["3229239424", "2794270450", "1770851727"].iter().any(|id| !install.mod_data(id).is_dir()) {
        eprintln!("skipping: Clone Wars stack not installed");
        return;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("fft")).unwrap());
    let data = Arc::new(load_galaxy(&*fs).0);
    for (campaign, side, gcw, cw) in [
        ("GC_Full_Web_Way_Shatterpoint_Republic", "Republic", ["Retaliation", "Uliah_Krayt", "Servator"], "Servator_CW"),
        ("GC_Full_Web_Way_Shatterpoint_CIS", "CIS", ["Intemptor", "Defiance_Banner", "Great_Rapina"], "Invisible_Hand"),
    ] {
        let campaign = data.campaign(campaign).expect("campaign").clone();
        let mut sim = ScriptedSim::new(
            fs.clone(),
            data.clone(),
            &campaign,
            SimConfig { victory_enabled: false, ..SimConfig::default() },
            ScriptedConfig { game_scoring: false, ..ScriptedConfig::default() },
        )
        .expect("scripted sim");
        for _ in 0..10 {
            sim.step(1.0);
        }
        sim.with_sim(|s| {
            let w = s.world();
            let f = w.faction_id(side).unwrap();
            let locked = &s.galaxy().factions[f as usize].story_locked;
            for n in gcw {
                assert!(locked.contains(&n.to_ascii_lowercase()), "{side}: {n} must be story-locked, locked = {locked:?}");
            }
            assert!(!locked.contains(&cw.to_ascii_lowercase()), "{side}: {cw} is a Clone Wars unit");
            // The build rule refuses a locked unit that is otherwise buildable (tech 1 hero, owned home planet).
            let home = w.factions[f as usize].home.expect("home");
            let u = w.unit_id(gcw[0]).expect("unit");
            let r = w.check_build_unit(s.galaxy(), f, home, u);
            assert!(matches!(&r, Err(galaxy_sim::CommandError::RequirementsNotMet(m)) if m == "locked"), "{side}: {r:?}");
        });
    }
}
