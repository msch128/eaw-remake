use alamo_fs::{GameFs, GameInstall};
fn main() {
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("fs");
    let pats: Vec<String> = std::env::args().skip(1).map(|a| a.to_lowercase()).collect();
    for m in fs.megs() {
        for e in m.entries() {
            let n = e.name.to_lowercase();
            if pats.iter().any(|p| n.contains(p)) {
                println!("{} ({} B)", e.name, e.size);
            }
        }
    }
}
