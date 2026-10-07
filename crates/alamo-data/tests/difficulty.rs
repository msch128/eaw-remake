//! Real-data check of the difficulty table; skipped without the Remake install.

use alamo_data::DifficultyTable;
use alamo_fs::{GameFs, GameInstall};

#[test]
fn difficulty_table_has_the_three_tiers() {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        return;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).unwrap();
    let t = DifficultyTable::load(&fs, &mut Vec::new());
    let (e, n, h) = (t.get("Easy_Default").unwrap(), t.get("Normal_Default").unwrap(), t.get("Hard_Default").unwrap());
    assert_eq!((e.credit_multiplier, n.credit_multiplier, h.credit_multiplier), (1.0, 2.0, 3.0));
    assert!(h.galactic_contrast > n.galactic_contrast && n.galactic_contrast > e.galactic_contrast);
    assert!(h.damage > 1.0 && h.galactic_build_time < 1.0);
    assert!(t.get("Hard_Minor").is_some());
}
