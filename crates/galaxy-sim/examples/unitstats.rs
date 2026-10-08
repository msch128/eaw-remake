//! Prints the combat numbers of units: `unitstats Name1 Name2 ...`
use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn main() {
    let install = GameInstall::detect().expect("game install not found");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("game files");
    let (data, _) = load_galaxy(&fs);
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let sim = Sim::new(&data, campaign, SimConfig::default()).expect("sim");
    let w = sim.world();
    for name in std::env::args().skip(1) {
        let Some(u) = w.unit_id(&name) else { println!("{name}: unknown"); continue };
        let ut = &w.units[u as usize];
        let c = &ut.combat;
        println!("{name}: {:?} hull {:.0} shield {:.0} regen {:.1} dps {:.1} durability {:.0} weapons {}", ut.category, c.hull, c.shield, c.shield_regen, c.raw_dps(), c.durability(), c.weapons.len());
        for wp in c.weapons.iter().take(6) {
            println!("   {:?}", wp);
        }
    }
}
