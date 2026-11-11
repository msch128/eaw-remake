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
