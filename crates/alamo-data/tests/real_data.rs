//! Tests against the real Empire at War Remake mod data; skipped when it is not installed.

use alamo_data::{TextTable, load_galaxy};
use alamo_fs::LooseDir;

const MOD_DATA: &str = r"F:\SteamLibrary\steamapps\workshop\content\32470\1770851727\Data";

fn open() -> Option<LooseDir> {
    if std::path::Path::new(MOD_DATA).is_dir() {
        LooseDir::new(MOD_DATA).ok()
    } else {
        eprintln!("skipping: {MOD_DATA} not found");
        None
    }
}

#[test]
fn text_table_loads() {
    let Some(src) = open() else { return };
    let t = TextTable::load(&src, "Text/MasterTextFile_ENGLISH.DAT").expect("dat parses");
    assert!(t.len() > 10_000, "only {} strings", t.len());
    assert!(t.get("GC_WEB_WAY_FULL").is_some());
}

#[test]
fn galaxy_loads() {
    let Some(src) = open() else { return };
    let (data, warnings) = load_galaxy(&src);
    eprintln!("{} warnings", warnings.len());
    assert!(data.campaigns.len() >= 15, "{} campaigns", data.campaigns.len());

    let grand = data.campaign("GC_Full_Web_Way_Rebel").expect("grand campaign");
    assert!(grand.has_galaxy());
    let planets = data.campaign_planets(grand);
    assert!(planets.len() >= grand.locations.len() - 5, "{} of {}", planets.len(), grand.locations.len());
    assert!(planets.iter().filter(|p| p.is_real_planet()).count() > 100);
    assert!(data.campaign_trade_routes(grand).len() > 100);

    let cor = data.planet("coruscant_ww").expect("Coruscant");
    assert_eq!(cor.position, [1.0, 106.0, 10.0]);
    assert_eq!(cor.credit_value, 607);
    assert_eq!(cor.max_space_base, 3);
    assert_eq!(cor.space_tactical_map.as_deref(), Some("_Space_Planet_Coruscant_02.ted"));
    assert!(data.planet_display_name(cor).to_lowercase().contains("coruscant"));

    let rebel = grand.faction("Rebel").unwrap();
    assert_eq!(rebel.starting_credits, Some(15000));
    assert_eq!(rebel.home_location.as_deref(), Some("Yavin_WW"));
    assert!(data.faction("Rebel").unwrap().is_playable);
    assert!(data.faction("Empire").is_some());
}
