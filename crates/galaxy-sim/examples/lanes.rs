//! Lane speed factors of a campaign and how often the fastest route differs from the fewest-hops route.
use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn main() {
    let install = GameInstall::detect().expect("game install");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("game files");
    let (data, _) = load_galaxy(&fs);
    let c = data.campaign(&std::env::args().nth(1).unwrap_or("GC_Full_Web_Way_Rebel".into())).unwrap();
    let sim = Sim::new(&data, c, SimConfig::default()).unwrap();
    let w = sim.world();
    let mut hist = std::collections::BTreeMap::new();
    for r in &w.routes {
        *hist.entry((r.hs_factor * 100.0) as i32).or_insert(0) += 1;
    }
    println!("{} routes, HS_Speed_Factor x100 -> count: {hist:?}", w.routes.len());
    let mut by_line = std::collections::BTreeMap::new();
    for r in data.campaign_trade_routes(c) {
        *by_line.entry((r.visible_line_name.clone().unwrap_or_default(), (r.hs_speed_factor * 100.0) as i32)).or_insert(0) += 1;
    }
    println!("(visible line, HS factor x100) -> routes: {by_line:?}");
    let n = w.planets.len() as u32;
    let (mut pairs, mut differ) = (0, 0);
    let mut worst: (f32, u32, u32) = (1.0, 0, 0);
    for a in (0..n).filter(|&p| w.real[p as usize]) {
        // Fewest hops by BFS.
        let mut hops = vec![u32::MAX; n as usize];
        hops[a as usize] = 0;
        let mut q = std::collections::VecDeque::from([a]);
        while let Some(u) = q.pop_front() {
            for &(v, _) in &w.adjacency[u as usize] {
                if hops[v as usize] == u32::MAX {
                    hops[v as usize] = hops[u as usize] + 1;
                    q.push_back(v);
                }
            }
        }
        for b in (a + 1..n).filter(|&p| w.real[p as usize] && hops[p as usize] != u32::MAX) {
            let Some(path) = w.find_path(a, b) else { continue };
            pairs += 1;
            if path.len() as u32 - 1 > hops[b as usize] {
                differ += 1;
                let _ = &mut worst;
            }
        }
    }
    println!("{pairs} planet pairs, {differ} where the fastest route takes more hops than the shortest");
}
