//! Prints the star base of a planet: `starbase_at Celanon`
use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn main() {
    let install = GameInstall::detect().expect("game install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("game files");
    let (data, _) = load_galaxy(&fs);
    let c = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let sim = Sim::new(&data, c, SimConfig::default()).unwrap();
    let w = sim.world();
    if std::env::var("UNITS").is_ok() {
        for f in ["Rebel"] {
            let fid = w.faction_id(f).unwrap();
            println!("{}", w.buildable_units[fid as usize].iter().map(|&u| w.units[u as usize].name.clone()).collect::<Vec<_>>().join(", "));
        }
    }
    for name in std::env::args().skip(1) {
        let Some(p) = w.planet_id(&format!("{name}_WW")).or_else(|| w.planet_id(&name)) else { println!("{name}: unknown"); continue };
        let ps = &sim.galaxy().planets[p as usize];
        println!("{} owner {:?} structures {:?}", w.planets[p as usize].name, ps.owner.map(|o| w.factions[o as usize].name.clone()), ps.structures.iter().map(|&s| w.structures[s as usize].name.clone()).collect::<Vec<_>>());
        if let Some(sb) = ps.starbase {
            let b = &w.starbases[sb as usize];
            let cb = &b.combat;
            println!("  star base {} level {} hull {:.0} shield {:.0} regen {:.0} armor {:?} shield_armor {:?} weapons {} garrison {:?}", b.name, b.level, cb.hull, cb.shield, cb.shield_regen, cb.armor, cb.shield_armor, cb.weapons.len(), b.garrison);
        }
    }
}
