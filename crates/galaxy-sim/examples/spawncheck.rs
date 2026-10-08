//! Distances between the attacker entry marker and the defender / station markers of every GC space map.
use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use battle_sim::Arena;

fn main() {
    let install = GameInstall::detect().expect("game install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("game files");
    let (data, _) = load_galaxy(&fs);
    let c = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let sim = galaxy_sim::Sim::new(&data, c, galaxy_sim::SimConfig::default()).unwrap();
    let mut names: Vec<String> = sim.world().planets.iter().filter_map(|p| p.space_map.clone()).collect();
    names.sort();
    names.dedup();
    let mut ds = Vec::new();
    let mut mins: Vec<f32> = Vec::new();
    for n in &names {
        let Some(a) = Arena::load(&data, &fs, n) else { continue };
        let Some(att) = a.attacker_entries.first().map(|x| x.0) else { println!("{n}: no attacker marker"); continue };
        let def = a.defender_positions.first().map(|x| att.dist(x.0));
        let st = a.station.map(|x| att.dist(x.0));
        println!("{n}: bounds {:.0}x{:.0}, attacker markers {}, defender markers {}, att->def {:?}, att->station {:?}", a.max[0] - a.min[0], a.max[1] - a.min[1], a.attacker_entries.len(), a.defender_positions.len(), def.map(|d| d as i32), st.map(|d| d as i32));
        if let Some(d) = st.or(def) {
            ds.push(d);
        }
        let mut best = f32::MAX;
        for (p, _) in &a.attacker_entries {
            for (q, _) in a.defender_positions.iter().chain(a.station.iter()) {
                best = best.min(p.dist(*q));
            }
        }
        mins.push(best);
    }
    ds.sort_by(f32::total_cmp);
    mins.sort_by(f32::total_cmp);
    println!("closest attacker-defender marker pair per map: median {:.0}, min {:.0}", mins[mins.len() / 2], mins[0]);
    println!("{} maps, median distance {:.0}, min {:.0}, max {:.0}", ds.len(), ds[ds.len() / 2], ds[0], ds[ds.len() - 1]);
}
