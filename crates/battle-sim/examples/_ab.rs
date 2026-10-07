use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use battle_sim::*;
fn main() {
    let install = GameInstall::detect().unwrap();
    let fs = GameFs::for_mod(&install, Some("1770851727")).unwrap();
    let (data, _) = load_galaxy(&fs);
    let u = data.objects.space_unit("Dominator_Star_Destroyer").unwrap();
    for a in &u.abilities { println!("raw ability {} rec {:?} exp {:?} fields {:?}", a.kind, a.recharge_seconds, a.expiration_seconds, a.fields.keys().collect::<Vec<_>>()); }
    let cat = Catalog::build(&data, Some(&fs), &["Dominator_Star_Destroyer"]);
    for a in &cat.ship(cat.ship_id("Dominator_Star_Destroyer").unwrap()).abilities { println!("{:?}", a); }
}
