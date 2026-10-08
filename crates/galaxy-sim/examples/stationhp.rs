//! Hardpoints of a ship or station as battle-sim sees them: `stationhp Empire_Star_Base_2`
use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use battle_sim::Catalog;

fn main() {
    let install = GameInstall::detect().expect("game install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("game files");
    let (data, _) = load_galaxy(&fs);
    let names: Vec<String> = std::env::args().skip(1).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let cat = Catalog::build(&data, Some(&fs), &refs);
    let mut todo: Vec<(String, u32)> = names.iter().map(|n| (n.clone(), 1)).collect();
    while let Some((n, count)) = todo.pop() {
        let d = cat.ship(cat.ship_id(&n).expect("unit"));
        if d.is_squadron {
            for &(m, c) in &d.members {
                todo.push((cat.ship(m).name.clone(), c * count));
            }
            continue;
        }
        let n = &format!("{n} x{count}");
        println!("{n}: hull {} shield {} regen {} armor {:?} radius {} hardpoints {}", d.hull, d.shield, d.shield_regen, d.armor, d.radius, d.hardpoints.len());
        let mut total = 0.0;
        for h in &d.hardpoints {
            let Some(p) = h.projectile else { continue };
            let pd = &cat.projectiles[p as usize];
            let rate = (h.recharge_min + h.recharge_max) / 2.0;
            let dps = pd.damage * h.pulses as f32 / (rate + h.pulses.saturating_sub(1) as f32 * h.pulse_delay).max(0.05);
            total += dps;
            println!("  {:28} proj {:22} cat {:8} dmg {:6.0} type {:?} pulses {} recharge {:.1}-{:.1} range {:.0} yaw {:.0}deg hp {} dps {:.0}", h.name, pd.name, pd.category, pd.damage, pd.damage_type, h.pulses, h.recharge_min, h.recharge_max, h.range, h.half_yaw.to_degrees() * 2.0, h.health, dps);
        }
        println!("  total raw dps {total:.0}");
    }
}
