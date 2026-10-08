//! Lists the live script instances of a match (diagnostics for the save format).
use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::SimConfig;

fn main() {
    let install = GameInstall::detect().expect("game install not found");
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).expect("game files"));
    let data = Arc::new(load_galaxy(&*fs).0);
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign").clone();
    let mut sim = ScriptedSim::new(fs, data, &campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }, ScriptedConfig { human: Some("Rebel".into()), ..ScriptedConfig::default() }).expect("sim");
    for _ in 0..(60 * 8) {
        sim.step(1.0);
    }
    println!("alive scripts: {}", sim.runtime().alive_scripts());
    for (name, t, n) in sim.runtime().profile().iter().take(60) {
        println!("{name:<50} {t:?} {n}");
    }
}
