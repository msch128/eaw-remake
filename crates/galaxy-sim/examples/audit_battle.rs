//! Space battle audit: `audit_battle <defender> <attacker> <attacker unit> <attacker count> <starbase level 0-3> [structure,structure...]`
//! Builds a Rebel/Empire/Underworld campaign, gives the defender's planet that star base level and those structures, lets the attacker
//! arrive and runs the tactical battle headless. Prints the request groups, what spawned (hull, shield, hardpoints, special weapons)
//! and the result.
use std::collections::BTreeMap;
use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use battle_sim::{Arena, Catalog};
use galaxy_sim::*;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (defender, attacker, unit, count, level) = (&a[0], &a[1], &a[2], a[3].parse::<u32>().unwrap(), a[4].parse::<i32>().unwrap());
    let structures: Vec<&str> = a.get(5).map(|s| s.split(',').filter(|x| !x.is_empty()).collect()).unwrap_or_default();
    let modid = std::env::var("MOD").unwrap_or_else(|_| "1770851727".into());
    let campaign = std::env::var("CAMPAIGN").unwrap_or_else(|_| format!("GC_Full_Web_Way_{defender}"));
    let install = GameInstall::detect().expect("game install");
    let fs = GameFs::for_mod(&install, Some(&modid)).expect("game files");
    let data = load_galaxy(&fs).0;
    let c = data.campaign(&campaign).expect("campaign");
    let mut sim = Sim::new(&data, c, SimConfig { auto_resolve_battles: false, victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let w = sim.world().clone();
    let (def_f, att_f) = (w.faction_id(defender).unwrap(), w.faction_id(attacker).unwrap());
    let target = (0..w.planets.len() as u32)
        .find(|&p| w.real[p as usize] && w.planets[p as usize].space_map.is_some() && sim.galaxy().planets[p as usize].owner == Some(def_f))
        .expect("a planet of the defender with a map");
    let neighbour = w.routes.iter().find_map(|r| (r.a == target).then_some(r.b).or((r.b == target).then_some(r.a))).expect("a route");
    println!("planet {} (max star base {})", w.planets[target as usize].name, w.max_starbase_level(target));
    {
        let g = sim.galaxy_mut();
        let mut sb = None;
        if level > 0 {
            let mut cur = w.first_starbase[def_f as usize];
            for _ in 1..level {
                cur = cur.and_then(|c| w.starbases[c as usize].next);
            }
            sb = cur;
            println!("star base {:?}", sb.map(|s| w.starbases[s as usize].name.clone()));
        }
        g.planets[target as usize].starbase = sb;
        g.planets[target as usize].structures = structures.iter().map(|n| w.structure_id(n).unwrap_or_else(|| panic!("no structure {n}"))).collect();
        let u = w.unit_id(unit).unwrap_or_else(|| panic!("no unit {unit}"));
        let id = g.fleets.keys().max().copied().unwrap_or(0) + 1;
        g.fleets.insert(id, Fleet { id, owner: att_f, units: [(u, count)].into(), location: FleetLocation::AtPlanet(target), previous_planet: neighbour, slot: 0 });
    }
    let battle = sim.step(1.0).into_iter().find_map(|e| if let Event::BattlePending(b) = e { Some(b) } else { None }).expect("pending battle");
    let req = sim.tactical_setup(battle.id).expect("request");
    let names: Vec<&str> = req.sides.iter().flat_map(|s| s.groups.iter().map(|g| g.name.as_str())).collect();
    for s in &req.sides {
        println!("side {} defender {} groups:", s.faction_name, s.defender);
        for g in &s.groups {
            println!("  {:?} {} x{}", g.kind, g.name, g.count);
        }
    }
    let catalog = Arc::new(Catalog::build(&data, Some(&fs), &names));
    let arena = req.map.as_ref().and_then(|m| Arena::load(&data, &fs, m)).map(Arc::new);
    let mut first = true;
    let attackers_side = req.sides.iter().position(|s| !s.defender).unwrap_or(0) as u8;
    let res = run_headless_observed(&req, &catalog, arena, &mut |b| {
        let hull: f32 = b.ships.iter().filter(|s| s.alive && s.side == attackers_side && !catalog.ship(s.def).is_static).map(|s| s.hull + s.shield).sum();
        let sw: Vec<String> = b
            .ships
            .iter()
            .filter(|s| catalog.ship(s.def).special_weapon.is_some())
            .map(|s| format!("{}:cd{:.0}/pulses{}", catalog.ship(s.def).name, s.special_cd, s.special_pulses))
            .collect();
        let big: Vec<String> = b.ships.iter().filter(|s| s.alive && s.side == attackers_side && catalog.ship(s.def).hull > 1000.0).map(|s| format!("{}:{:.0}+{:.0}", catalog.ship(s.def).name, s.hull, s.shield)).collect();
        println!("t={:.0}s attacker hull+shield {hull:.0} big {big:?} special {sw:?}", b.time());
        if !first {
            return;
        }
        first = false;
        let mut by: BTreeMap<(u8, String), (u32, f32, f32, usize, bool)> = BTreeMap::new();
        for s in b.ships.iter().filter(|s| s.alive) {
            let d = catalog.ship(s.def);
            let e = by.entry((s.side, d.name.clone())).or_insert((0, 0.0, 0.0, 0, false));
            e.0 += 1;
            e.1 = s.hull;
            e.2 = s.shield;
            e.3 = s.hps.len();
            e.4 = d.special_weapon.is_some();
        }
        println!("--- spawned at t=10 s");
        for ((side, name), (n, hull, shield, hps, special)) in by {
            println!("  side {side} {name} x{n}: hull {hull:.0} shield {shield:.0} hardpoints {hps} special {special}");
        }
    });
    match res {
        Some(r) => println!("result: winner {:?} survived {:?} duration {:.0}s", r.winner, r.survived, r.duration),
        None => println!("no result"),
    }
}
