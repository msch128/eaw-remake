use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall, FileSource};
use alamo_map::SpaceMap;
fn main() {
    let name = std::env::args().nth(1).unwrap_or("_Space_Planet_Coruscant_02.ted".into());
    let install = GameInstall::detect().unwrap();
    let fs = GameFs::for_mod(&install, Some("1770851727")).unwrap();
    let (data, _) = load_galaxy(&fs);
    let bytes = fs.read(&format!("Art/Maps/{name}")).expect("map");
    let mut map = SpaceMap::parse(&bytes).unwrap();
    let names: Vec<&str> = data.objects.iter().map(|o| o.name.as_str()).collect();
    let n = map.resolve_names(names.iter().copied());
    println!("{} objects, {} resolved; bounds {:?}..{:?} header size {:?} defender slots {}", map.objects.len(), n, map.bounds.min, map.bounds.max, map.header.size, map.defender_slots.len());
    let mut counts = std::collections::BTreeMap::new();
    for o in &map.objects { *counts.entry(o.type_name.clone().unwrap_or(format!("#{:08x}", o.type_hash))).or_insert(0) += 1; }
    for (k, v) in &counts { println!("  {v:>3} {k}"); }
    for o in map.objects.iter().filter(|o| o.class() == alamo_map::ObjectClass::Marker).take(30) {
        println!("marker {:<40} pos ({:7.0},{:7.0},{:6.0}) yaw {:5.0} slot {:?}", o.type_name.clone().unwrap_or_default(), o.position[0], o.position[1], o.position[2], o.rotation_deg[2], o.player_slot);
    }
}
