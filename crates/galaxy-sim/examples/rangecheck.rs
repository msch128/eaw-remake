//! Lists ships whose battle definition would not hold a standoff range (no usable weapons, zero preferred range).
use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn main() {
    let install = GameInstall::detect().expect("game install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("game files");
    let (data, _) = load_galaxy(&fs);
    let c = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let sim = Sim::new(&data, c, SimConfig::default()).unwrap();
    let names = sim.tactical_names();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let cat = battle_sim::Catalog::build(&data, Some(&fs), &names);
    let (mut total, mut bad) = (0, 0);
    for n in &names {
        let Some(id) = cat.ship_id(n) else { continue };
        let d = cat.ship(id);
        if d.is_static || matches!(d.class, battle_sim::TargetClass::Fighter | battle_sim::TargetClass::Bomber) || d.is_squadron {
            continue;
        }
        let armed = sim.world().unit_id(n).is_some_and(|u| !sim.world().units[u as usize].combat.weapons.is_empty() && sim.world().units[u as usize].combat.hull > 0.0 && !n.starts_with("Death_Clone"));
        if !armed {
            continue;
        }
        total += 1;
        let facing_sum: f32 = d.bearing_dps.iter().sum();
        if d.preferred_range <= 0.0 || facing_sum <= 0.0 {
            bad += 1;
            println!("{n}: class {:?} preferred range {:.0}, max weapon range {:.0}, weapon dps by bearing {:.0}, hull {:.0}", d.class, d.preferred_range, d.max_weapon_range, facing_sum, d.hull);
        }
    }
    println!("{bad} of {total} mobile non-fighter ships have no preferred range or no turret/battery dps");
}
