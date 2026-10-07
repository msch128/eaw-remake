use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use battle_sim::*;
fn main() {
    let install = GameInstall::detect().unwrap();
    let fs = GameFs::for_mod(&install, Some("1770851727")).unwrap();
    let (data, _) = load_galaxy(&fs);
    let names: Vec<String> = data.objects.space_units().into_iter().filter(|u| !u.is_squadron() && u.hull > 0.0 && u.build_cost > 0.0).map(|u| u.name).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let cat = Catalog::build(&data, Some(&fs), &refs);
    let (mut fwd_ok, mut fwd_bad, mut unknown) = (0, 0, 0);
    let mut bad = Vec::new();
    for s in &cat.ships {
        let eng: Vec<&HpDef> = s.hardpoints.iter().filter(|h| h.kind == HpKind::Engine && h.from_bone).collect();
        if eng.is_empty() { unknown += 1; continue; }
        let mx = eng.iter().map(|h| h.offset.x).sum::<f32>() / eng.len() as f32;
        let my = eng.iter().map(|h| h.offset.y).sum::<f32>() / eng.len() as f32;
        if mx < -s.radius * 0.1 && mx.abs() > my.abs() { fwd_ok += 1 } else { fwd_bad += 1; bad.push(format!("{} eng=({:.0},{:.0}) r={:.0}", s.name, mx, my, s.radius)); }
    }
    println!("engines behind (-X): {fwd_ok}, not: {fwd_bad}, no engine hardpoint: {unknown}");
    for b in bad.iter().take(25) { println!("  {b}"); }
}
