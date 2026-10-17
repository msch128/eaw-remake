//! Tactical AI probes: scripted battles against the real Remake data that print what the AI's retreat rule sees and how the battle went.
//!
//! Usage: `cargo run --release -p battle-sim --example ai_probe -- [scenario filter] [seeds=N] [trace]`

use std::collections::HashMap;
use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use battle_sim::*;

type Fleet = &'static [(&'static str, u32)];

struct Scenario {
    name: &'static str,
    /// Side 0, the attacker.
    attacker: Fleet,
    /// Side 1, the defender, with its structures.
    defender: Fleet,
    base: &'static [&'static str],
    /// The side the AI plays; the other one is the scripted player (an attacking player attacks the station with everything, then attack-moves on the nearest
    /// enemy warship; a defending player holds and fires at what comes in range).
    ai: u8,
}

const SCENARIOS: &[Scenario] = &[
    // An Executor fleet attacks Rebel defences of two sizes (the user's finding of 2026-10-17), then the Executor alone, an Executor against a
    // Golan III, and an even fight in which the station falls.
    Scenario {
        name: "ssd-vs-big-defence",
        attacker: &[("Executor_Star_Dreadnought", 1), ("Star_Destroyer", 3), ("Victory_Destroyer", 3), ("TIE_Fighter_Squadron", 4)],
        defender: &[("MC-80_Star_Cruiser", 4), ("Nebulon_B_Frigate", 4), ("Assault_Frigate_Mark_II", 3), ("CR-90_Corvette", 4), ("X-Wing_Squadron", 4)],
        base: &["Rebel_Star_Base_3"],
        ai: 1,
    },
    Scenario {
        name: "ssd-vs-huge-defence",
        attacker: &[("Executor_Star_Dreadnought", 1), ("Star_Destroyer", 3), ("Victory_Destroyer", 3), ("TIE_Fighter_Squadron", 4)],
        defender: &[("MC-80_Star_Cruiser", 6), ("Nebulon_B_Frigate", 6), ("Assault_Frigate_Mark_II", 4), ("CR-90_Corvette", 6), ("X-Wing_Squadron", 6)],
        base: &["Rebel_Star_Base_3"],
        ai: 1,
    },
    Scenario {
        name: "ssd-alone-vs-defence",
        attacker: &[("Executor_Star_Dreadnought", 1)],
        defender: &[("MC-80_Star_Cruiser", 6), ("Assault_Frigate_Mark_II", 6), ("Nebulon_B_Frigate", 4)],
        base: &["Rebel_Star_Base_3"],
        ai: 1,
    },
    Scenario {
        name: "ssd-vs-golan",
        attacker: &[("Executor_Star_Dreadnought", 1)],
        defender: &[("MC-80_Star_Cruiser", 6), ("Nebulon_B_Frigate", 4)],
        base: &["Golan_3_Orbital"],
        ai: 1,
    },
    Scenario {
        name: "fair-station-falls",
        attacker: &[("Star_Destroyer", 6), ("Victory_Destroyer", 4)],
        defender: &[("MC-80_Star_Cruiser", 6), ("Nebulon_B_Frigate", 4)],
        base: &["Rebel_Star_Base_1"],
        ai: 1,
    },
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seeds: u64 = args.iter().find_map(|a| a.strip_prefix("seeds=")).and_then(|v| v.parse().ok()).unwrap_or(1);
    let trace = args.iter().any(|a| a == "trace");
    let filter = args.iter().find(|a| !a.contains('=') && *a != "trace").cloned().unwrap_or_default();
    let install = GameInstall::detect().expect("game install not found");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("cannot open game files");
    let (data, _) = load_galaxy(&fs);
    let names: Vec<&str> = SCENARIOS.iter().flat_map(|s| s.attacker.iter().chain(s.defender.iter()).map(|x| x.0).chain(s.base.iter().copied())).collect();
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &names));
    // `AI_Combat_Power` per definition (0 where the data has none).
    let cp: HashMap<ShipId, f32> = (0..cat.ships.len() as ShipId)
        .map(|id| {
            let n = &cat.ship(id).name;
            let v = data.objects.space_unit(n).map(|u| u.ai_combat_power).or_else(|| data.objects.star_base(n).map(|b| b.ai_combat_power)).or_else(|| data.objects.structure(n).and_then(|s| s.fields.get("ai_combat_power").and_then(|v| v.trim().parse().ok()))).unwrap_or(0.0);
            (id, v)
        })
        .collect();
    if filter == "units" {
        for id in 0..cat.ships.len() as ShipId {
            let d = cat.ship(id);
            println!("{:40} {:?} hull {:7.0} shield {:7.0} dps {:7.1} estimate {:7.0} AI_Combat_Power {:6.0} speed {:5.0} range {:5.0} standoff {:5.0} radius {:4.0}", d.name, d.class, d.hull, d.shield, dps(d, &cat), ((d.hull + d.shield) * dps(d, &cat)).sqrt(), cp[&id], d.max_speed, d.max_weapon_range, d.preferred_range, d.radius);
        }
        return;
    }
    for s in SCENARIOS.iter().filter(|s| s.name.contains(&filter)) {
        for seed in 1..=seeds {
            run(&cat, &cp, s, seed, trace);
        }
    }
}

fn side(cat: &Catalog, name: &str, units: Fleet, structures: &[&str], ai: bool) -> SideSetup {
    SideSetup {
        name: name.into(),
        units: units.iter().map(|&(n, c)| UnitSetup { ship: cat.ship_id(n).unwrap_or_else(|| panic!("{n}")), count: c, tag: 1 }).collect(),
        structures: structures.iter().map(|n| cat.ship_id(n).unwrap_or_else(|| panic!("{n}"))).collect(),
        ai,
    }
}

/// Our old strength estimate of a unit: `sqrt((hull + shield) x best-bearing damage rate)`.
fn estimate(cat: &Catalog, s: &Ship) -> f32 {
    let d = cat.ship(s.def);
    ((s.hull.max(0.0) + s.shield.max(0.0)) * dps(d, cat)).sqrt()
}

fn dps(d: &ShipDef, cat: &Catalog) -> f32 {
    let hp = d.bearing_dps.iter().copied().fold(0.0f32, f32::max);
    if hp > 0.0 {
        return hp;
    }
    if let Some(w) = &d.direct
        && let Some(p) = cat.projectiles.get(w.projectile as usize)
    {
        return p.damage * w.pulses as f32 / (w.pulses as f32 * w.pulse_delay + w.recharge).max(0.2);
    }
    0.0
}

fn run(cat: &Arc<Catalog>, cp: &HashMap<ShipId, f32>, s: &Scenario, seed: u64, trace: bool) {
    let (ai, player) = (s.ai, 1 - s.ai);
    let sides = vec![side(cat, "Attacker", s.attacker, &[], ai == 0), side(cat, "Defender", s.defender, s.base, ai == 1)];
    let config = BattleConfig { seed, defenders: vec![false, true], ..BattleConfig::default() };
    let mut b = Battle::new(BattleSetup { catalog: cat.clone(), sides, config });
    let small = |s: &Ship| matches!(cat.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber);
    let force = |b: &Battle, side: u8| -> (f32, f32) {
        b.ships.iter().filter(|x| x.alive && !x.retreated && x.side == side).fold((0.0, 0.0), |a, x| {
            let d = cat.ship(x.def);
            let frac = ((x.hull.max(0.0) + x.shield.max(0.0)) / (d.hull + d.shield).max(1.0)).min(1.0);
            (a.0 + estimate(cat, x), a.1 + cp[&x.def] * frac)
        })
    };
    let ratio = |b: &Battle| {
        let (ae, ac) = force(b, ai);
        let (pe, pc) = force(b, player);
        (pe / ae.max(1.0), pc / ac.max(1.0))
    };
    let ai_big: Vec<u32> = b.ships.iter().filter(|x| x.side == ai && !small(x) && !cat.ship(x.def).is_static).map(|x| x.id).collect();
    let mut station_dead: Option<(f32, (f32, f32))> = None;
    let mut ordered: Option<(f32, (f32, f32))> = None;
    let mut goal: Option<Vec3> = None;
    let mut last_print = -100.0f32;
    while !b.is_over() && b.time() < 1200.0 {
        let t = b.time();
        // The scripted attacking player: everything attacks the station, then attack-moves on the nearest enemy warship once the area is clear.
        if player == 0 && b.tick().is_multiple_of(30) {
            let enemy_big: Vec<Vec3> = b.ships.iter().filter(|x| x.alive && !x.retreated && x.side == ai && !small(x)).map(|x| x.pos).collect();
            let station = b.ships.iter().find(|x| x.alive && x.side == ai && cat.ship(x.def).is_static).map(|x| (x.pos, x.id));
            let mine: Vec<u32> = b.ships.iter().filter(|x| x.alive && !x.retreated && x.side == player && !cat.ship(x.def).is_static).map(|x| x.id).collect();
            if let Some((p, id)) = station {
                if goal.is_none() {
                    goal = Some(p);
                    let _ = b.command(player, Command::Attack { ships: mine, target: id });
                }
            } else if goal.is_none_or(|g| !enemy_big.iter().any(|p| p.dist(g) < 3000.0)) {
                let (sum, n) = b.ships.iter().filter(|x| x.alive && x.side == player && !small(x)).fold((Vec3::ZERO, 0.0f32), |a, x| (a.0 + x.pos, a.1 + 1.0));
                let centre = if n > 0.0 { sum * (1.0 / n) } else { Vec3::ZERO };
                if let Some(p) = enemy_big.iter().copied().min_by(|a, c| a.dist(centre).total_cmp(&c.dist(centre))) {
                    goal = Some(p);
                    let _ = b.command(player, Command::AttackMove { ships: mine, to: p });
                }
            }
        }
        let events = b.step().to_vec();
        if ordered.is_none() && events.iter().any(|e| matches!(e, Event::RetreatOrdered { side, .. } if *side == ai)) {
            ordered = Some((t, ratio(&b)));
        }
        if station_dead.is_none() && !s.base.is_empty() && !b.ships.iter().any(|x| x.alive && x.side == 1 && cat.ship(x.def).is_static) {
            station_dead = Some((t, ratio(&b)));
        }
        if trace && t - last_print >= 30.0 {
            last_print = t;
            let (ae, ac) = force(&b, ai);
            let (pe, pc) = force(&b, player);
            let big = |side: u8| b.ships.iter().filter(|x| x.alive && !x.retreated && x.side == side && !small(x) && !cat.ship(x.def).is_static).count();
            println!("  t={t:4.0} AI big {:2} est {ae:7.0} cp {ac:7.0} | player big {:2} est {pe:7.0} cp {pc:7.0} | player / AI: est {:.2} cp {:.2}", big(ai), big(player), pe / ae.max(1.0), pc / ac.max(1.0));
        }
    }
    let r = b.result();
    let count = |side: u8, f: fn(&GroupResult) -> u32| -> u32 { r.groups.iter().filter(|g| g.side == side).map(f).sum() };
    let fmt = |x: Option<(f32, (f32, f32))>| x.map_or("never".to_string(), |(t, (e, c))| format!("{t:.0} s (player / AI: estimate x{e:.2}, AI_Combat_Power x{c:.2})"));
    let big_lost = ai_big.iter().filter(|&&i| !b.ships[i as usize].alive).count();
    let big_escaped = ai_big.iter().filter(|&&i| b.ships[i as usize].alive && b.ships[i as usize].retreated).count();
    println!(
        "{} seed {seed}: winner {:?} after {:.0} s; station destroyed {}; AI retreat ordered {}; AI big ships {} ({big_lost} lost, {big_escaped} escaped), AI units {}/{} left ({} escaped), player units {}/{} left",
        s.name,
        r.winner,
        r.duration,
        if s.base.is_empty() { "-".to_string() } else { fmt(station_dead) },
        fmt(ordered),
        ai_big.len(),
        count(ai, |g| g.survived),
        count(ai, |g| g.total),
        count(ai, |g| g.retreated),
        count(player, |g| g.survived),
        count(player, |g| g.total),
    );
}
