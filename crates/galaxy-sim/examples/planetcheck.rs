//! Checks how a planet name resolves: `planetcheck Allanteen_WW`
use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};

fn main() {
    let install = GameInstall::detect().expect("game install not found");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("game files");
    let (data, _) = load_galaxy(&fs);
    let c = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    for camp in &data.campaigns {
        let planets = data.campaign_planets(camp);
        let has = |n: &str| planets.iter().any(|p| p.name.eq_ignore_ascii_case(n));
        let routes = data.campaign_trade_routes(camp);
        let bad = routes.iter().filter(|r| !has(&r.point_a) || !has(&r.point_b)).count();
        if !planets.is_empty() {
            println!("{:<40} {} planets, {} routes, {} routes with an end outside the planet list (Allanteen_WW in list: {})", camp.name, planets.len(), routes.len(), bad, has("Allanteen_WW"));
        }
    }
    for name in std::env::args().skip(1) {
        let lower = name.to_ascii_lowercase();
        let all: Vec<_> = data.planets.iter().filter(|p| p.name.eq_ignore_ascii_case(&name)).collect();
        println!("{name}: {} definitions in the data", all.len());
        let _ = (&lower, c);
        for p in all {
            println!("  from {} behavior {:?} model {:?} position {:?}", p.source_file, p.behavior, p.galactic_model, p.position);
        }
        println!("  in campaign_planets: {}", data.campaign_planets(c).iter().any(|p| p.name.eq_ignore_ascii_case(&name)));
    }
}
