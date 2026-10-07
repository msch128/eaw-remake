use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
fn main() {
    let install = GameInstall::detect().unwrap();
    let fs = GameFs::for_mod(&install, Some("1770851727")).unwrap();
    let (data, _) = load_galaxy(&fs);
    for k in std::env::args().skip(1) {
        println!("== {k}\n{}\n", data.text(&k).unwrap_or("<missing>"));
    }
}
