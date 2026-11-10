//! The 181st Fighter Wing loads from `<181st>` and enters the galaxy catalog with the build flags the file gives it.

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn load() -> Option<alamo_data::GameData> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    Some(load_galaxy(&fs).0)
}

#[test]
fn squadron_and_heroes_match_the_file() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Empire").or_else(|| data.campaign("GC_Full_Web_Way_Rebel")).expect("campaign");
    let sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let w = sim.world();
    let empire = w.faction_id("Empire").expect("Empire");

    let fel_id = w.unit_id("TIE_Fighter_181st_Soontir").expect("Soontir in catalog");
    let fel = &w.units[fel_id as usize];
    // The craft variants `TIE_Fighter`, which has no price and no build tab (those tags are on `TIE_Fighter_Squadron`). He is a squadron member, not a build-bar item.
    assert!(fel.hero);
    assert!(!fel.buildable);
    assert_eq!((fel.tech, fel.cost), (0, 0.0));
    assert!(!w.buildable_units[empire as usize].contains(&fel_id));

    let sq_id = w.unit_id("181st_Fighter_Squadron").expect("fighter squadron");
    let sq = &w.units[sq_id as usize];
    // `Tech_Level` 99, `Affiliation` Neutral, no `Build_Tab_Space_Units`.
    assert!(!sq.buildable);
    assert_eq!((sq.tech, sq.cost), (99, 1750.0));
    assert!(sq.affiliation.iter().any(|a| a == "neutral"));
    assert!(!w.buildable_units[empire as usize].contains(&sq_id));

    let def_id = w.unit_id("181st_Defender_Squadron").expect("defender squadron");
    let def = &w.units[def_id as usize];
    // Price and Empire affiliation are set; `Build_Tab_Space_Units` is not, so it is not a build-bar item.
    assert!(!def.buildable);
    assert_eq!((def.tech, def.cost), (4, 9800.0));
    assert!(def.affiliation.iter().any(|a| a == "empire"));
    assert!(!w.buildable_units[empire as usize].contains(&def_id));

    assert!(w.unit_id("TIE_Fighter_181st").is_some());
    assert!(w.unit_id("TIE_Interceptor_181st_Soontir").is_some());
    assert!(w.unit_id("TIE_Defender_181st_Soontir").is_some());
    assert!(w.unit_id("181st_Interceptor_Squadron").is_some());
}
