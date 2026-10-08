//! Size and top speed of ships as the battle sim sees them: `speeds Star_Destroyer TIE_Fighter_Squadron ...`
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
    let mut todo: Vec<String> = names.clone();
    while let Some(n) = todo.pop() {
        let d = cat.ship(cat.ship_id(&n).expect("unit"));
        if d.is_squadron {
            todo.extend(d.members.iter().map(|m| cat.ship(m.0).name.clone()));
            continue;
        }
        println!("{n}: radius {:.0}, model extent {:.0}, max speed {:.1} u/s at scale {}, accel {:.1}, turn {:.2} rad/s", d.radius, d.model_extent, d.max_speed, cat.speed_scale, d.accel, d.turn_rate);
    }
}
