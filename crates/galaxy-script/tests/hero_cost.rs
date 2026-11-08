//! Scripts (and so the AI) read unit prices through `Get_Build_Cost`; a hero ship that the mod prices at 1 credit costs its ship class there as well.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::SimConfig;

#[test]
fn scripts_see_the_class_price_of_a_hero_ship() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        return;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).unwrap());
    let data = Arc::new(load_galaxy(&*fs).0);
    let c = data.campaign("GC_Full_Web_Way_Rebel").unwrap().clone();
    let sim = ScriptedSim::new(fs, data, &c, SimConfig { victory_enabled: false, ..Default::default() }, ScriptedConfig { human: Some("Rebel".into()), remake_ai: false, difficulty: None, ..Default::default() }).unwrap();
    let price = |name: &str| sim.runtime().with_host(|h| h.type_number(name, "build_cost"));
    assert_eq!(price("Strikefast"), Some(12000.0));
    assert_eq!(price("Profundity"), Some(16000.0));
    assert_eq!(price("Star_Destroyer"), sim.with_sim(|s| s.world().unit_id("Star_Destroyer").map(|u| s.world().units[u as usize].cost)));
}
