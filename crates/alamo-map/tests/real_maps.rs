//! Parses every space map referenced by the Remake mod's planets; skipped without an install.

use alamo_data::{GameObjects, load_galaxy};
use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_map::SpaceMap;

const REMAKE: &str = "1770851727";

#[test]
fn parses_all_remake_space_maps() {
    let Some(install) = GameInstall::detect() else {
        eprintln!("skipped: Empire at War not installed");
        return;
    };
    if !install.mod_data(REMAKE).is_dir() {
        eprintln!("skipped: Remake mod not installed");
        return;
    }
    let fs = GameFs::for_mod(&install, Some(REMAKE)).unwrap();
    let (galaxy, _) = load_galaxy(&fs);
    let mut warnings = Vec::new();
    let objects = GameObjects::load(&fs, &mut warnings);
    let names: Vec<&str> = objects.iter().map(|o| o.name.as_str()).collect();

    let (mut referenced, mut missing, mut failed, mut ok) = (0, 0, 0, 0);
    let (mut objs, mut resolved) = (0, 0);
    for map_name in galaxy
        .planets
        .iter()
        .filter_map(|p| p.space_tactical_map.as_deref())
    {
        referenced += 1;
        let Ok(data) = fs.read(&format!("Art/Maps/{map_name}")) else {
            missing += 1;
            eprintln!("missing: {map_name}");
            continue;
        };
        match SpaceMap::parse(&data) {
            Ok(mut map) => {
                ok += 1;
                objs += map.objects.len();
                resolved += map.resolve_names(names.iter().copied());
                assert!(
                    map.bounds.max[0] > map.bounds.min[0],
                    "{map_name}: empty bounds"
                );
                assert!(
                    map.unknown.is_empty(),
                    "{map_name}: unknown chunks {:?}",
                    map.unknown.iter().map(|u| &u.path).collect::<Vec<_>>()
                );
            }
            Err(e) => {
                failed += 1;
                eprintln!("parse error in {map_name}: {e}");
            }
        }
    }
    eprintln!(
        "referenced {referenced}, missing {missing}, parse errors {failed}, ok {ok}; objects {objs}, names resolved {resolved}"
    );
    assert!(referenced > 100, "expected many referenced maps");
    assert_eq!(failed, 0);
    assert!(missing <= 2, "too many missing maps");
}
