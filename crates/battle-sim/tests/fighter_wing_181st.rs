//! Spawning the 181st squadrons pulls in Soontir and his wingmen, with damage redirect only on the leaders.

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use battle_sim::Catalog;

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
fn squadrons_spawn_their_heroes() {
    let Some(data) = load() else { return };
    let cat = Catalog::build(
        &data,
        None,
        &[
            "181st_Fighter_Squadron",
            "181st_Interceptor_Squadron",
            "181st_Defender_Squadron",
        ],
    );

    let squadron = cat.ship(cat.ship_id("181st_Fighter_Squadron").expect("squadron"));
    assert!(squadron.is_squadron);
    let count: u32 = squadron.members.iter().map(|(_, n)| *n).sum();
    assert_eq!(count, 36);
    let fel_id = cat.ship_id("TIE_Fighter_181st_Soontir").expect("Soontir");
    let wing_id = cat.ship_id("TIE_Fighter_181st").expect("wingman");
    assert_eq!(squadron.members.iter().find(|(id, _)| *id == fel_id).map(|(_, n)| *n), Some(1));
    assert_eq!(squadron.members.iter().find(|(id, _)| *id == wing_id).map(|(_, n)| *n), Some(35));

    let fel = cat.ship(fel_id);
    assert!(fel.redirect_to_teammates);
    assert_eq!((fel.hull, fel.shield), (33.0, 15.0));
    assert!(!cat.ship(wing_id).redirect_to_teammates);

    let int_id = cat.ship_id("TIE_Interceptor_181st_Soontir").expect("interceptor leader");
    let def_id = cat.ship_id("TIE_Defender_181st_Soontir").expect("defender leader");
    assert!(cat.ship(int_id).redirect_to_teammates);
    assert!(cat.ship(def_id).redirect_to_teammates);
    assert!(!cat.ship(cat.ship_id("TIE_Interceptor_181st").unwrap()).redirect_to_teammates);
    assert!(!cat.ship(cat.ship_id("TIE_Defender_181st").unwrap()).redirect_to_teammates);
}
