//! Orbital structures follow their star base: when the star base is lost (e.g. destroyed in battle),
//! orbital structures are suspended (no income/upkeep, capabilities suspended, but slots remain reserved).
//! When the star base is rebuilt, they are restored. When the planet changes hands, they are cleared.

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn load() -> Option<GameData> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    Some(load_galaxy(&fs).0)
}

#[test]
fn orbital_structures_suspended_when_starbase_lost_and_restored_when_rebuilt() {
    let Some(data) = load() else { return };
    let c = data.campaign("GC_Full_Web_Way_Imperial").unwrap();
    let mut sim = Sim::new(&data, c, SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let kuat = w.planet_id("Kuat_WW").unwrap();

    let yard1 = w.structure_id("E_Shipyard_1").expect("E_Shipyard_1");
    assert!(w.structures[yard1 as usize].orbital);

    // Kuat starts with starbase level 3 and E_Shipyard_1 (and 2 and 3)
    let ps = &sim.galaxy().planets[kuat as usize];
    assert!(ps.starbase.is_some());
    assert!(ps.structures.contains(&yard1));

    // Destroy/remove the star base
    let income_before = w.planet_income(sim.galaxy(), kuat);
    sim.set_starbase(kuat, None);

    let ps = &sim.galaxy().planets[kuat as usize];
    assert!(ps.starbase.is_none());
    // Orbital structures moved to suspended_structures
    assert!(!ps.structures.contains(&yard1));
    assert!(ps.suspended_structures.contains(&yard1));

    // Suspended shipyard no longer satisfies structure requirements
    assert!(!w.has_structure(ps, yard1));

    // Space slots in planet_capacity still account for suspended structures
    let cap = w.planet_capacity(sim.galaxy(), kuat);
    assert_eq!(cap.space.built, ps.suspended_structures.len() as i32, "suspended structures still reserve space slots");

    // Income changes because suspended orbital structures (like shipyards and Golan) do not charge upkeep
    let income_suspended = w.planet_income(sim.galaxy(), kuat);
    assert!(income_suspended > income_before, "suspended orbital structures do not charge upkeep");

    // Ground structures still stand
    assert!(ps.structures.iter().any(|&s| !w.structures[s as usize].orbital));

    // Rebuild the star base
    let first_sb = w.first_starbase[empire as usize].unwrap();
    sim.set_starbase(kuat, Some(first_sb));

    let ps = &sim.galaxy().planets[kuat as usize];
    assert_eq!(ps.starbase, Some(first_sb));
    assert!(ps.structures.contains(&yard1));
    assert!(ps.suspended_structures.is_empty());
    assert!(w.has_structure(ps, yard1));

    // Suspend again and test planet owner change clears suspended structures
    sim.set_starbase(kuat, None);
    assert!(!sim.galaxy().planets[kuat as usize].suspended_structures.is_empty());
    let rebel = w.faction_id("Rebel").unwrap();
    sim.set_planet_owner(kuat, Some(rebel));
    assert!(sim.galaxy().planets[kuat as usize].suspended_structures.is_empty());
}
