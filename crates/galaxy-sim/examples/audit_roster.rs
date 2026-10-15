//! Roster and build-limit audit: `audit_roster <mod id or fft> <campaign>`.
//! For every faction: each ship/squadron/station-class unit with its build requirements and the planets that can ever build it,
//! the shipyard structures and star base caps per planet, and ground-to-space structures.
use alamo_data::{ShipCategory, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn main() {
    let mut args = std::env::args().skip(1);
    let modid = args.next().unwrap_or_else(|| "1770851727".into());
    let campaign = args.next().unwrap_or_else(|| "GC_Full_Web_Way_Rebel".into());
    let install = GameInstall::detect().expect("game install");
    let fs = GameFs::for_mod(&install, Some(&modid)).expect("game files");
    let (data, _) = load_galaxy(&fs);
    let filter = args.next().map(|f| f.to_ascii_lowercase());
    let c = data.campaign(&campaign).expect("campaign");
    let sim = Sim::new(&data, c, SimConfig::default()).expect("sim");
    let w = sim.world();
    let real: Vec<u32> = (0..w.planets.len() as u32).filter(|&p| w.real[p as usize]).collect();
    println!("# mod {modid} campaign {campaign}: {} real planets, {} units, {} structures", real.len(), w.units.len(), w.structures.len());

    // Star base cap histogram.
    let mut hist = std::collections::BTreeMap::<i32, Vec<&str>>::new();
    for &p in &real {
        hist.entry(w.max_starbase_level(p)).or_default().push(&w.planets[p as usize].name);
    }
    for (lvl, names) in &hist {
        println!("max_space_base {lvl}: {} planets{}", names.len(), if names.len() <= 12 { format!(" {names:?}") } else { String::new() });
    }
    let slots = real.iter().map(|&p| (w.planets[p as usize].space_slots, w.planets[p as usize].ground_slots)).collect::<std::collections::BTreeSet<_>>();
    println!("slot combinations (space, ground): {slots:?}");

    let sname = |s: StructureTypeId| w.structures[s as usize].name.clone();
    let can_host = |p: u32, req_planets: &[u32], req_sb: i32, groups: &[Vec<StructureTypeId>]| -> bool {
        (req_planets.is_empty() || req_planets.contains(&p))
            && w.planet_takes_starbase(p, req_sb)
            && groups.iter().all(|g| g.iter().any(|&s| w.planet_takes_starbase(p, w.structures[s as usize].req_starbase) && (w.structures[s as usize].req_planets.is_empty() || w.structures[s as usize].req_planets.contains(&p))))
    };

    if let Some(f) = &filter {
        for s in w.structures.iter().filter(|s| if f == "*" { s.tech < 90 && (s.combat.is_some() || s.space_reach) } else { s.name.to_ascii_lowercase().contains(f) }) {
            if f == "*" {
                let c = s.combat.as_ref();
                println!(
                    "FIGHTER {} | aff {:?} | tech {} | cost {} | buildable {} | sb {} | planets {} | hull {:?} shield {:?} weapons {:?} dps {:?}",
                    s.name, s.affiliation, s.tech, s.cost, s.buildable, s.req_starbase, s.req_planets.len(),
                    c.map(|c| c.hull), c.map(|c| c.shield), c.map(|c| c.weapons.len()), c.map(|c| c.raw_dps().round())
                );
                continue;
            }
            println!(
                "ANY {} | tech {} | cost {} | sb {} | aff {:?} | planets {} | buildable {} | orbital {} | combat {:?} | reach {} | ancestors {:?}",
                s.name, s.tech, s.cost, s.req_starbase, s.affiliation, s.req_planets.len(), s.buildable, s.orbital,
                s.combat.as_ref().map(|c| (c.hull, c.shield, c.weapons.len())), s.space_reach,
                s.ancestors.iter().map(|&a| w.structures[a as usize].name.clone()).collect::<Vec<_>>()
            );
        }
        return;
    }
    for (fi, f) in w.factions.iter().enumerate() {
        let fname = f.name.to_ascii_lowercase();
        println!("\n## faction {} (start tech {}, max {})", f.name, f.start_tech, f.max_tech);
        let mut rows = Vec::new();
        for (ui, u) in w.units.iter().enumerate() {
            if !u.affiliation.contains(&fname) || u.role != UnitRole::Ship {
                continue;
            }
            if !matches!(u.category, ShipCategory::Capital | ShipCategory::Super | ShipCategory::Frigate | ShipCategory::Corvette | ShipCategory::Transport | ShipCategory::Fighter | ShipCategory::Bomber | ShipCategory::SpaceStructure | ShipCategory::Other) {
                continue;
            }
            let in_bar = w.buildable_units[fi].binary_search(&(ui as u32)).is_ok();
            let hosts: Vec<&str> = real.iter().filter(|&&p| can_host(p, &u.req_planets, u.req_starbase, &u.req_structures)).map(|&p| w.planets[p as usize].name.as_str()).collect();
            rows.push((u.category as u8, u.name.clone(), u.category, u.tech, u.cost, u.req_starbase, u.req_planets.iter().map(|&p| w.planets[p as usize].name.clone()).collect::<Vec<_>>(),
                u.req_structures.iter().map(|g| g.iter().map(|&s| sname(s)).collect::<Vec<_>>().join("|")).collect::<Vec<_>>(), u.locked, u.buildable, in_bar, u.hero, u.limit_current, u.limit_lifetime, hosts.len(),
                if hosts.len() <= 6 { hosts.join(",") } else { String::new() }, u.combat.hull));
        }
        rows.sort_by(|a, b| (b.0, &b.1).cmp(&(a.0, &a.1)));
        for r in rows {
            if r.4 <= 0.0 && !r.9 {
                continue;
            }
            println!(
                "UNIT {} | {:?} | tech {} | cost {} | sb {} | planets {:?} | structs {:?} | locked {} | buildflag {} | inbar {} | hero {} | lim {:?}/{:?} | hosts {} {} | hull {:.0}",
                r.1, r.2, r.3, r.4, r.5, r.6, r.7, r.8, r.9, r.10, r.11, r.12, r.13, r.14, r.15, r.16
            );
        }
        println!("-- structures with space reach / shipyard-ish for {}", f.name);
        for (si, s) in w.structures.iter().enumerate() {
            if !s.affiliation.contains(&fname) {
                continue;
            }
            let interesting = s.space_reach || s.name.to_ascii_lowercase().contains("shipyard") || s.name.to_ascii_lowercase().contains("starforge") || s.name.to_ascii_lowercase().contains("planet_shield");
            if !interesting {
                continue;
            }
            let hosts: Vec<&str> = real.iter().filter(|&&p| can_host(p, &s.req_planets, s.req_starbase, &s.req_structures)).map(|&p| w.planets[p as usize].name.as_str()).collect();
            println!(
                "STRUCT {} | tech {} | cost {} | sb {} | planets {:?} | structs {:?} | buildable {} | inbar {} | orbital {} | max {} | hosts {} {}",
                s.name, s.tech, s.cost, s.req_starbase,
                s.req_planets.iter().map(|&p| w.planets[p as usize].name.clone()).collect::<Vec<_>>(),
                s.req_structures.iter().map(|g| g.iter().map(|&x| sname(x)).collect::<Vec<_>>().join("|")).collect::<Vec<_>>(),
                s.buildable, w.buildable_structures[fi].binary_search(&(si as u32)).is_ok(), s.orbital, s.max_per_planet, hosts.len(),
                if hosts.len() <= 6 { hosts.join(",") } else { String::new() }
            );
        }
        println!("-- star bases for {}", f.name);
        for b in w.starbases.iter().filter(|b| b.affiliation.contains(&fname)) {
            println!("BASE {} | level {} | tech {} | cost {} | hull {} shield {} | weapons {} | garrison {:?}", b.name, b.level, b.tech, b.cost, b.combat.hull, b.combat.shield, b.combat.weapons.len(), b.garrison.iter().map(|(u, n)| format!("{}x{}", n, w.units[*u as usize].name)).collect::<Vec<_>>());
        }
    }
}
