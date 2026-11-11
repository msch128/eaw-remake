//! The words of the galactic HUD against the real data.

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};

/// The Remake names its fiscal cycle a month: the header counter, the cycle log and the upkeep texts all say so. The HUD, the advisor
/// lines and the tooltips use that word for the simulation's week (one `Fiscal_Cycle_Time_In_Secs`).
#[test]
fn the_fiscal_cycle_is_called_a_month() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return;
    }
    let data = load_galaxy(&GameFs::for_mod(&install, Some("1770851727")).unwrap()).0;
    assert_eq!(data.text("TEXT_CYCLES"), Some("Month"));
    assert_eq!(data.text("TEXT_LOG_CYCLE_01"), Some("Month ##0"));
    assert_eq!(data.text("TEXT_TOOLTIP_TOTAL_INCOME"), Some("Total credit income per month"));
    assert!(data.text("TEXT_TOOLTIP_UPKEEP_800").is_some_and(|t| t.contains("800 credits per month")), "{:?}", data.text("TEXT_TOOLTIP_UPKEEP_800"));
}

/// Capturing a faction's holding planet (`Rebel_Dummy_WW`: no name, not on the map) makes no notice; a map planet is named.
#[test]
fn notices_name_their_planet_and_skip_unnamed_holding_planets() {
    use galaxy_play::notify::describe_event;
    use galaxy_play::PlayCatalog;
    use galaxy_sim::{Sim, SimConfig};
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return;
    }
    let data = load_galaxy(&GameFs::for_mod(&install, Some("1770851727")).unwrap()).0;
    let campaign = data.campaign("GC_Full_Web_Way_Imperial").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let names = PlayCatalog::from_data(&w, &data);
    let rebel = w.faction_id("Rebel").unwrap();
    let empire = w.faction_id("Empire").unwrap();
    let dummy = w.planet_id("Rebel_Dummy_WW").unwrap();
    assert_eq!(w.planets[dummy as usize].display_name.trim(), "", "TEXT_PLANET_NONE is blank");
    // A hero's return names the planet the script gives by object name.
    assert_eq!(galaxy_play::notify::planet_label(&w, "Coruscant_WW"), "The Coruscant System");
    let byss = w.planet_id("Byss_WW").unwrap();
    for (p, named) in [(dummy, false), (byss, true)] {
        sim.set_planet_owner(p, None);
        let events = sim.set_planet_owner(p, Some(rebel));
        let lines: Vec<String> = events.iter().filter_map(|e| describe_event(&w, &names, sim.galaxy(), Some(empire), e, &Default::default())).map(|l| l.1).collect();
        if named {
            assert!(lines.iter().any(|l| l.starts_with("Byss: Control changed from")), "{lines:?}");
        } else {
            assert!(lines.is_empty(), "{lines:?}");
        }
    }
}
