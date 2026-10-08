use alamo_fs::{FileSource, GameFs, GameInstall};
fn main() {
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("fs");
    for a in std::env::args().skip(1) {
        let d = fs.read(&a).expect("read");
        println!("== {a} ({} B)", d.len());
        let mut cur = String::new();
        for &b in &d {
            if (32..127).contains(&b) {
                cur.push(b as char);
            } else {
                if cur.len() >= 4 {
                    println!("{cur}");
                }
                cur.clear();
            }
        }
    }
}
