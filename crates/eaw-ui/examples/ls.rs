use alamo_fs::{FileSource, GameFs, GameInstall};
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let install = GameInstall::detect().unwrap();
    let fs = GameFs::for_mod(&install, Some("1770851727")).unwrap();
    let pat = a[0].to_uppercase();
    let lim: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);
    let all = fs.list("");
    let m: Vec<_> = all.iter().filter(|p| p.contains(&pat)).collect();
    println!("{} matches", m.len());
    for p in m.iter().take(lim) { println!("{p}"); }
}
