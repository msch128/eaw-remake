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
