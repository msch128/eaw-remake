use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use battle_sim::*;
fn main() {
    let name = std::env::args().nth(1).unwrap();
    let install = GameInstall::detect().unwrap();
    let fs = GameFs::for_mod(&install, Some("1770851727")).unwrap();
    let (data, _) = load_galaxy(&fs);
    let cat = Catalog::build(&data, Some(&fs), &[name.as_str()]);
    let d = cat.ship(cat.ship_id(&name).unwrap());
    println!("{} radius {} pref {} maxrange {} bow {} speed {} turn {}", d.name, d.radius, d.preferred_range, d.max_weapon_range, d.bow_weapons, d.max_speed, d.turn_rate);
    println!("bearing dps: {:?}", d.bearing_dps.iter().map(|v| *v as i32).collect::<Vec<_>>());
    for h in &d.hardpoints {
        let p = h.projectile.map(|p| cat.projectiles[p as usize].name.clone()).unwrap_or_default();
        println!("{:<22} {:?} bone={} turret={} ext({:3.0},{:3.0}) off=({:5.0},{:5.0},{:4.0}) fwd=({:5.2},{:5.2},{:5.2}) nrm=({:5.2},{:5.2},{:5.2}) arc({:3.0},{:3.0}) fb={} y=({:5.2},{:5.2},{:5.2}) range {:5.0} proj {}", h.name, h.kind, h.from_bone, h.turret, h.turret_yaw.to_degrees(), h.turret_pitch.to_degrees(), h.offset.x, h.offset.y, h.offset.z, h.forward.x, h.forward.y, h.forward.z, h.normal.x, h.normal.y, h.normal.z, h.arc_yaw.to_degrees(), h.arc_pitch.to_degrees(), h.arc_fallback, h.bone_y.x, h.bone_y.y, h.bone_y.z, h.range, p);
    }
}
