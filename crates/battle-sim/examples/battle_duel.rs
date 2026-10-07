//! Balance diagnostics: controlled duels with per-ship damage channels, hardpoint usage, hit rates
//! and a comparison with the ideal time-to-kill derived from the data.
//!
//! Usage: `cargo run --release -p battle-sim --example battle_duel -- [scenario filter]`

use std::collections::BTreeMap;
use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use battle_sim::*;

type Fleet<'a> = &'a [(&'a str, u32)];

const SCENARIOS: &[(&str, &[(&str, u32)], &[(&str, u32)])] = &[
    ("ISD vs MC80", &[("Star_Destroyer", 1)], &[("MC-80_Star_Cruiser", 1)]),
    ("VSD vs Nebulon-B", &[("Victory_Destroyer", 1)], &[("Nebulon_B_Frigate", 1)]),
    ("CR90 vs Tartan", &[("CR-90_Corvette", 1)], &[("Tartan_Patrol_Cruiser", 1)]),
    ("X-wing vs TIE", &[("X-Wing_Squadron", 1)], &[("TIE_Fighter_Squadron", 1)]),
    ("ISD vs 3 CR90", &[("Star_Destroyer", 1)], &[("CR-90_Corvette", 3)]),
    ("2 ISD vs 2 MC80", &[("Star_Destroyer", 2)], &[("MC-80_Star_Cruiser", 2)]),
    ("5 ISD vs 5 MC80", &[("Star_Destroyer", 5)], &[("MC-80_Star_Cruiser", 5)]),
    ("10 ISD vs 10 MC80", &[("Star_Destroyer", 10)], &[("MC-80_Star_Cruiser", 10)]),
    ("10 VSD vs 14 Nebulon-B", &[("Victory_Destroyer", 10)], &[("Nebulon_B_Frigate", 14)]),
    ("Arquitens vs CR90", &[("Arquitens_E", 1)], &[("CR-90_Corvette", 1)]),
];

#[derive(Default)]
struct SideStats {
    fired: u64,
    hits: u64,
    dmg_shield: f64,
    dmg_hull: f64,
    dmg_hardpoint: f64,
    hp_fired: BTreeMap<(u32, u16), u64>,
    shield_up_steps: BTreeMap<u32, u64>,
    alive_steps: BTreeMap<u32, u64>,
    death_time: BTreeMap<u32, f32>,
}

fn main() {
    let filter = std::env::args().nth(1).unwrap_or_default().to_lowercase();
    let install = GameInstall::detect().expect("game install not found");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("cannot open game files");
    let (data, _) = load_galaxy(&fs);
    let names: Vec<&str> = SCENARIOS.iter().flat_map(|s| s.1.iter().chain(s.2.iter()).map(|x| x.0)).collect();
    let cat = Arc::new(Catalog::build(&data, Some(&fs), &names));
    for (title, a, b) in SCENARIOS {
        if !filter.is_empty() && !title.to_lowercase().contains(&filter) {
            continue;
        }
        duel(&cat, title, a, b);
    }
}

fn ideal(cat: &Catalog, attacker: ShipId, victim: ShipId) -> (f32, f32, f32) {
    let (a, v) = (cat.ship(attacker), cat.ship(victim));
    let (mut sd, mut hd) = (0.0, 0.0);
    for h in a.hardpoints.iter().filter(|h| h.kind == HpKind::Weapon) {
        let Some(p) = h.projectile else { continue };
        let pd = &cat.projectiles[p as usize];
        let cycle = ((h.recharge_min + h.recharge_max) * 0.5 + (h.pulses - 1) as f32 * h.pulse_delay).max(0.2);
        let dps = pd.damage * h.pulses as f32 / cycle;
        if pd.shield_damage {
            sd += dps * cat.damage.multiplier(pd.damage_type, v.shield_armor);
        }
        if pd.hull_damage {
            hd += dps * cat.damage.multiplier(pd.damage_type, v.armor);
        }
    }
    let ttk = if sd > 0.0 { (v.shield / (sd - v.shield_regen).max(sd * 0.05)).max(0.0) } else { 0.0 } + if hd > 0.0 { v.hull / hd } else { f32::INFINITY };
    (sd, hd, ttk)
}

fn duel(cat: &Arc<Catalog>, title: &str, a: Fleet, b: Fleet) {
    let side = |name: &str, f: Fleet| SideSetup {
        name: name.into(),
        units: f.iter().map(|&(n, c)| UnitSetup { ship: cat.ship_id(n).unwrap_or_else(|| panic!("{n}")), count: c, tag: 0 }).collect(),
        structures: vec![],
        ai: true,
    };
    let config = BattleConfig { half_size: 5000.0, ..BattleConfig::default() };
    let mut battle = Battle::new(BattleSetup { catalog: cat.clone(), sides: vec![side("A", a), side("B", b)], config });
    let mut stats = [SideStats::default(), SideStats::default()];
    let mut first_contact = None;
    for _ in 0..30 * 900 {
        let events: Vec<Event> = battle.step().to_vec();
        for e in &events {
            match e {
                Event::ProjectileFired { ship, hardpoint, .. } => {
                    let s = &battle.ships[*ship as usize];
                    let st = &mut stats[s.side as usize];
                    st.fired += 1;
                    *st.hp_fired.entry((s.def, hardpoint.unwrap_or(u16::MAX))).or_default() += 1;
                }
                Event::Hit { attacker, kind, damage, .. } => {
                    let st = &mut stats[battle.ships[*attacker as usize].side as usize];
                    st.hits += 1;
                    match kind {
                        HitKind::Shield => st.dmg_shield += *damage as f64,
                        HitKind::Hull => st.dmg_hull += *damage as f64,
                        HitKind::Hardpoint(_) => st.dmg_hardpoint += *damage as f64,
                    }
                    first_contact.get_or_insert(battle.time());
                }
                Event::ShipDestroyed { ship, .. } => {
                    let side = battle.ships[*ship as usize].side as usize;
                    stats[side].death_time.insert(*ship, battle.time());
                }
                _ => {}
            }
        }
        for s in &battle.ships {
            if s.alive && cat.ship(s.def).shield > 0.0 && !matches!(cat.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber) {
                let st = &mut stats[s.side as usize];
                *st.alive_steps.entry(s.id).or_default() += 1;
                if s.shield > cat.ship(s.def).shield * 0.02 {
                    *st.shield_up_steps.entry(s.id).or_default() += 1;
                }
            }
        }
        if battle.is_over() {
            break;
        }
    }
    let r = battle.result();
    let t0 = first_contact.unwrap_or(0.0);
    println!("\n=== {title}: winner {:?} after {:.0} s (first hit at {:.0} s, fight lasted {:.0} s)", r.winner.map(|w| ["A", "B"][w as usize]), r.duration, t0, r.duration - t0);
    for (si, (name, fleet)) in [("A", a), ("B", b)].into_iter().enumerate() {
        let st = &stats[si];
        let survivors: u32 = r.groups.iter().filter(|g| g.side as usize == si).map(|g| g.survived).sum();
        let total: u32 = r.groups.iter().filter(|g| g.side as usize == si).map(|g| g.total).sum();
        println!(
            "  side {name} {:?}: survivors {survivors}/{total}; fired {} hits {} ({:.0}% hit rate); damage dealt: shield {:.0} hull {:.0} hardpoint {:.0}",
            fleet,
            st.fired,
            st.hits,
            100.0 * st.hits as f64 / st.fired.max(1) as f64,
            st.dmg_shield,
            st.dmg_hull,
            st.dmg_hardpoint
        );
        let dur = (r.duration - t0).max(1.0) as f64;
        println!("      average dealt DPS over the fight: {:.0}", (st.dmg_shield + st.dmg_hull + st.dmg_hardpoint) / dur);
        for (id, alive) in &st.alive_steps {
            let up = st.shield_up_steps.get(id).copied().unwrap_or(0);
            println!(
                "      ship #{id} ({}) shield up {:.0}% of its life, died at {:?}",
                cat.ship(battle.ships[*id as usize].def).name,
                100.0 * up as f64 / *alive as f64,
                st.death_time.get(id)
            );
        }
        // Ideal numbers against the first enemy unit type.
        let enemy = if si == 0 { b } else { a };
        let (ev, _) = (cat.ship_id(enemy[0].0).unwrap(), 0);
        let mut seen = std::collections::BTreeSet::new();
        for (&(def, hp), n) in &st.hp_fired {
            if hp == u16::MAX || !seen.insert(def) {
                continue;
            }
            let d = cat.ship(def);
            let (sd, hd, ttk) = ideal(cat, def, ev);
            let weapons: Vec<usize> = (0..d.hardpoints.len()).filter(|&i| d.hardpoints[i].kind == HpKind::Weapon).collect();
            let never: Vec<&str> = weapons.iter().filter(|&&i| !st.hp_fired.contains_key(&(def, i as u16))).map(|&i| d.hardpoints[i].name.as_str()).collect();
            println!(
                "      {}: {} weapons, {} never fired{}; ideal DPS vs enemy shield {:.0} hull {:.0}, ideal TTK {:.0} s (all guns, no regen, no misses)",
                d.name,
                weapons.len(),
                never.len(),
                if never.is_empty() { String::new() } else { format!(" ({})", never.iter().take(6).copied().collect::<Vec<_>>().join(", ")) },
                sd,
                hd,
                ttk
            );
            let fight = (r.duration - t0).max(1.0);
            let n_ships = fleet.iter().filter(|f| cat.ship_id(f.0) == Some(def)).map(|f| f.1).sum::<u32>().max(1) as f32;
            let util: Vec<String> = weapons
                .iter()
                .map(|&i| {
                    let h = &d.hardpoints[i];
                    let cycle = ((h.recharge_min + h.recharge_max) * 0.5 + (h.pulses - 1) as f32 * h.pulse_delay).max(0.2);
                    let expected = fight / cycle * h.pulses as f32 * n_ships;
                    let fired = st.hp_fired.get(&(def, i as u16)).copied().unwrap_or(0) as f32;
                    format!("{}:{:.0}%@{:.0}", h.name.trim_start_matches("HP_"), 100.0 * fired / expected.max(1.0), h.range)
                })
                .collect();
            println!("        utilization: {}", util.join(" "));
            let mut top: Vec<_> = weapons.iter().map(|&i| (st.hp_fired.get(&(def, i as u16)).copied().unwrap_or(0), i)).collect();
            top.sort_by(|a, b| b.cmp(a));
            let _ = n;
            let line: Vec<String> = top.iter().take(5).map(|(c, i)| format!("{}x{}", d.hardpoints[*i].name, c)).collect();
            println!("        most fired: {}", line.join(", "));
        }
    }
}
