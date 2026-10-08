use alamo_fs::{GameFs, GameInstall};
use eaw_ui::shell::ShellLayout;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let install = GameInstall::detect().unwrap();
    let fs = GameFs::for_mod(&install, Some("1770851727")).unwrap();
    let l = ShellLayout::load(&fs, &a[0]).unwrap();
    for n in &l.order {
        if let Some(i) = l.item(n) { println!("{n} center=({:.0},{:.0}) size=({:.0},{:.0}) z={:.1}", i.center.x, i.center.y, i.size.x, i.size.y, i.z); }
    }
    for art in &l.art { let (lo, hi) = art.bounds(); println!("ART {} alt={:?} tex={} lo=({:.0},{:.0}) hi=({:.0},{:.0})", art.name, art.alt, art.texture, lo.x, lo.y, hi.x, hi.y); }
}
