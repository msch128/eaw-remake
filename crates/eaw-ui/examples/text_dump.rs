use alamo_data::text::TextTable;
use alamo_fs::{GameFs, GameInstall};
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let install = GameInstall::detect().unwrap();
    let fs = GameFs::for_mod(&install, Some("1770851727")).unwrap();
    let t = TextTable::load(&fs, "Text/MasterTextFile_ENGLISH.DAT").unwrap();
    println!("{} entries", t.len());
    let pat = a[0].to_uppercase();
    for (k, v) in t.iter() { if k.to_uppercase().contains(&pat) || v.to_uppercase().contains(&pat) { println!("{k} = {v}"); } }
}
