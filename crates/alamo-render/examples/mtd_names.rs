use alamo_fs::{FileSource, GameFs, GameInstall};
fn main() {
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("fs");
    let d = fs.read("Art/Textures/MT_CommandBar.mtd").expect("mtd");
    let pats: Vec<String> = std::env::args().skip(1).map(|a| a.to_lowercase()).collect();
    let mut cur = String::new();
    for &b in &d {
        if (32..127).contains(&b) {
            cur.push(b as char);
        } else {
            if cur.len() > 5 && pats.iter().any(|p| cur.to_lowercase().contains(p)) {
                println!("{cur}");
            }
            cur.clear();
        }
    }
}
