//! Large fleet battle benchmark: Empire (ISDs, Victories, Arquitens, Tartans, TIEs) against Rebels
//! (MC-80, Nebulon-B, Assault Frigates, CR-90, X-wings), 40 big ships + 15 squadrons per side.
//!
//! Usage: `cargo run --release -p battle-sim --example battle_bench -- [max_seconds] [seed] [sequential] [stress[N]] [storm] [map]` (N = fleet size multiplier, default 3)

use std::sync::Arc;
use std::time::Instant;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use battle_sim::*;

const EMPIRE: [(&str, u32); 5] = [("Star_Destroyer", 10), ("Victory_Destroyer", 14), ("Arquitens_E", 10), ("Tartan_Patrol_Cruiser", 6), ("TIE_Fighter_Squadron", 15)];
const REBEL: [(&str, u32); 5] = [("MC-80_Star_Cruiser", 10), ("Nebulon_B_Frigate", 14), ("Assault_Frigate_Mark_II", 8), ("CR-90_Corvette", 8), ("X-Wing_Squadron", 15)];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let max_seconds: f32 = args.first().and_then(|a| a.parse().ok()).unwrap_or(240.0);
    let seed: u64 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(1);
    let parallel = !args.iter().any(|a| a == "sequential");

    let install = GameInstall::detect().expect("game install not found");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("cannot open game files");
    let (data, _) = load_galaxy(&fs);

    let t = Instant::now();
    let names: Vec<&str> = EMPIRE.iter().chain(REBEL.iter()).map(|x| x.0).collect();
    let catalog = Arc::new(Catalog::build(&data, Some(&fs), &names));
    println!("catalog: {} ships, {} projectiles built in {:?}", catalog.ships.len(), catalog.projectiles.len(), t.elapsed());
    for (n, _) in EMPIRE.iter().chain(REBEL.iter()) {
        let id = catalog.ship_id(n).unwrap_or_else(|| panic!("{n} missing"));
        let d = catalog.ship(id);
        let from_bone = d.hardpoints.iter().filter(|h| h.from_bone).count();
        println!(
            "  {:<26} {:?} hull {:>7.0} shield {:>7.0} radius {:>5.0} speed {:>5.1} hardpoints {:>3} ({} from bones) carried {:?} members {}",
            d.name,
            d.class,
            d.hull,
            d.shield,
            d.radius,
            d.max_speed,
            d.hardpoints.len(),
            from_bone,
            d.carried.iter().map(|c| c.1).sum::<u32>(),
            d.members.iter().map(|m| m.1).sum::<u32>()
        );
    }

    for n in ["TIE_Fighter", "X-Wing"] {
        let d = catalog.ship(catalog.ship_id(n).unwrap());
        println!("  {:<26} {:?} hull {:.0} shield {:.0} speed {:.1} turn {:.2} direct {:?} hardpoints {}", d.name, d.class, d.hull, d.shield, d.max_speed, d.turn_rate, d.direct.as_ref().map(|w| (catalog.projectiles[w.projectile as usize].name.clone(), w.range)), d.hardpoints.len());
    }
    let stress_arg = args.iter().find_map(|a| a.strip_prefix("stress"));
    let stress = stress_arg.is_some();
    let mult: u32 = stress_arg.and_then(|n| n.parse().ok()).unwrap_or(if stress { 3 } else { 1 });
    let side = |name: &str, list: &[(&str, u32)], tag: u32| SideSetup {
        name: name.into(),
        units: list.iter().map(|&(n, c)| UnitSetup { ship: catalog.ship_id(n).unwrap(), count: c * mult, tag }).collect(),
        structures: vec![],
        ai: true,
    };
    let arena = args.iter().any(|a| a == "map").then(|| Arc::new(Arena::load(&data, &fs, "_Space_Planet_Coruscant_02.ted").expect("map")));
    let config = BattleConfig { seed, record_events: false, parallel, arena, unit_cap: 100_000, half_size: if stress { 5_000.0 } else { 10_000.0 }, ..BattleConfig::default() };
    let mut battle = Battle::new(BattleSetup { catalog: catalog.clone(), sides: vec![side("Empire", &EMPIRE, 1), side("Rebel", &REBEL, 2)], config });
    println!("{} ships (incl. launched fighters), parallel = {parallel}\n", battle.ships.len());

    let steps_total = (max_seconds / battle.config().dt) as u64;
    let mut times: Vec<f64> = Vec::with_capacity(steps_total as usize);
    let (mut peak_proj, mut window) = (0usize, 0.0f64);
    let wall = Instant::now();
    // "storm" keeps 10,000 extra projectiles in flight to measure the projectile stage.
    let storm = args.iter().any(|a| a == "storm");
    let laser = catalog.projectile_id("Proj_Ship_Turbolaser_Green").or_else(|| catalog.projectile_id("Proj_Ship_Laser_Heavy_G")).expect("projectile");
    let mut lcg = 12345u64;
    let mut rnd = move || {
        lcg = lcg.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((lcg >> 33) as f32) / (1u64 << 31) as f32
    };
    for i in 0..steps_total {
        if storm {
            while battle.projectiles.len() < 10_000 {
                let from = Vec3::new(-6000.0 + rnd() * 3000.0, (rnd() - 0.5) * 3000.0, 0.0);
                let to = Vec3::new(1500.0 + rnd() * 2500.0, (rnd() - 0.5) * 3000.0, 0.0);
                battle.projectiles.push(Projectile {
                    pos: from,
                    vel: (to - from).normalized() * 900.0,
                    def: laser,
                    side: 0,
                    owner: 0,
                    target: NONE,
                    aim_hp: u16::MAX,
                    range_left: 9000.0,
                });
            }
        }
        let t = Instant::now();
        battle.step();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        times.push(ms);
        window += ms;
        peak_proj = peak_proj.max(battle.projectiles.len());
        if (i + 1) % 300 == 0 {
            println!(
                "t={:>5.0}s ships {:>3}/{:>3} health {:>9.0}/{:>9.0} projectiles {:>6} avg step {:.3} ms",
                battle.time(),
                battle.alive_count(0),
                battle.alive_count(1),
                battle.side_health(0),
                battle.side_health(1),
                battle.projectiles.len(),
                window / 300.0
            );
            window = 0.0;
        }
        if battle.is_over() {
            break;
        }
    }
    let mut sorted = times.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let avg = times.iter().sum::<f64>() / times.len().max(1) as f64;
    let pct = |p: f64| sorted[((sorted.len() as f64 * p) as usize).min(sorted.len().saturating_sub(1))];
    println!(
        "\n{} steps ({:.0} s battle time) in {:?}: avg {avg:.3} ms, p50 {:.3}, p99 {:.3}, max {:.3} ms, peak projectiles {peak_proj}",
        times.len(),
        battle.time(),
        wall.elapsed(),
        pct(0.5),
        pct(0.99),
        sorted.last().copied().unwrap_or(0.0)
    );
    let r = battle.result();
    println!("over: {} winner: {:?} duration {:.0} s", battle.is_over(), r.winner.map(|w| ["Empire", "Rebel"][w as usize]), r.duration);
    for g in &r.groups {
        println!("  side {} {:<26} {}/{} survived", g.side, catalog.ship(g.ship).name, g.survived, g.total);
    }
}
