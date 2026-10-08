//! Starting fleets per faction for every campaign set: `startforces [campaign-substring]`
use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn main() {
    let install = GameInstall::detect().expect("game install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("game files");
    let (data, _) = load_galaxy(&fs);
    let filter = std::env::args().nth(1).unwrap_or_default();
    for c in data.campaigns.iter().filter(|c| c.name.contains(&filter)) {
        let Ok(sim) = Sim::new(&data, c, SimConfig::default()) else { continue };
        let (w, g) = (sim.world(), sim.galaxy());
        let per: Vec<String> = ["Rebel", "Empire", "Underworld"]
            .iter()
            .filter_map(|n| w.faction_id(n))
            .map(|f| format!("{} units {} ships/fleets {} planets {}", w.factions[f as usize].name, g.fleets.values().filter(|x| x.owner == f).map(|x| x.unit_count()).sum::<u32>(), g.fleets.values().filter(|x| x.owner == f).count(), g.factions[f as usize].planets_owned))
            .collect();
        println!("{}: {}", c.name, per.join(" | "));
    }
}
