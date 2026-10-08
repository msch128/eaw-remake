use alamo_fs::{FileSource, GameFs, GameInstall};
fn main() {
    let install = GameInstall::detect().expect("install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("fs");
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|s| s.as_str()) == Some("--cat") {
        let d = fs.read(&args[1]).expect("read");
        println!("{}", String::from_utf8_lossy(&d));
        return;
    }
    let pats: Vec<String> = args.iter().map(|a| a.to_lowercase()).collect();
    for m in fs.megs() {
        for e in m.entries() {
            let n = e.name.to_lowercase();
            if pats.iter().any(|p| n.contains(p)) {
                println!("{} ({} B)", e.name, e.size);
            }
        }
    }
}
