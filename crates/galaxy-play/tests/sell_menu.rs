//! A planet without a free slot of a kind lists no structure entries of that kind; selling frees a slot (real data, skipped without the game).

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use galaxy_play::{BuildMenu, BuildTarget, PlayCatalog};
use galaxy_sim::{Command, Sim, SimConfig};

#[test]
fn full_planets_list_no_structures_of_that_kind() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return;
    }
    let data = load_galaxy(&GameFs::for_mod(&install, Some("1770851727")).unwrap()).0;
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let names = PlayCatalog::from_data(&w, &data);
    let empire = w.faction_id("Empire").unwrap();
    let p = w.planet_id("Coruscant_WW").unwrap();
    sim.set_planet_owner(p, None);
    sim.set_planet_owner(p, Some(empire));
    sim.add_credits(empire, 1.0e9);
    sim.set_tech_level(empire, 9);
    let ground_entries = |sim: &Sim| {
        let menu = BuildMenu::for_planet(&w, sim.galaxy(), &names, empire, p).unwrap();
        menu.options.iter().filter(|o| matches!(o.target, BuildTarget::Structure(s) if !w.structures[s as usize].orbital)).count()
    };
    // Fill the ground slots with any ground structure (the data does not care here).
    let filler = (0..w.structures.len() as u32).find(|&s| !w.structures[s as usize].orbital && w.structures[s as usize].galactic).unwrap();
    let total = w.planets[p as usize].ground_slots;
    for _ in 0..total {
        sim.galaxy_mut().planets[p as usize].structures.push(filler);
    }
    assert_eq!(sim.world().planet_capacity(sim.galaxy(), p).ground.free(), 0);
    assert_eq!(ground_entries(&sim), 0, "no ground structure entries on a full planet");
    sim.apply(Command::SellStructure { faction: empire, planet: p, structure: filler }).unwrap();
    assert!(sim.world().planet_capacity(sim.galaxy(), p).ground.free() == 1);
    assert!(ground_entries(&sim) > 0, "a free slot brings the entries back");
}
