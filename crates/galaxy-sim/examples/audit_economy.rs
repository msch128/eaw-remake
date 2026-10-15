//! Economy audit: `audit_economy [mod id] [campaign substring]`.
//! For every campaign start and main faction: the sim's weekly income against an independent recomputation straight from the XML
//! (planet `Planet_Credit_Value`, `Planet_Income_Bonus_Ability` of the star base, structures and ships), under two readings of an
//! `<Abilities SubObjectList>` that a variant redefines: replace the parent's list (what `alamo-data` does) or merge by ability name.
use std::collections::BTreeMap;

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

/// (absolute, percentage, mining multiplier) of every income ability of `name`.
fn abilities(data: &GameData, name: &str, merge: bool) -> Vec<(String, f32, f32, Option<f32>)> {
    let mut chain = Vec::new();
    let mut cur = data.objects.get(name);
    while let Some(o) = cur {
        if chain.iter().any(|c: &&alamo_data::objects::GameObject| std::ptr::eq(*c, o)) {
            break;
        }
        chain.push(o);
        cur = o.variant_of.as_deref().and_then(|p| data.objects.get(p));
    }
    let mut out: Vec<(String, f32, f32, Option<f32>)> = Vec::new();
    for o in &chain {
        let mut found = false;
        for ab in o.element.children.iter().filter(|c| c.name == "abilities") {
            found = true;
            for a in ab.children.iter().filter(|c| c.name == "planet_income_bonus_ability") {
                let n = a.attr("name").unwrap_or_default().trim().to_ascii_lowercase();
                let num = |k: &str| a.child_text(k).and_then(|v| v.trim().parse::<f32>().ok());
                if !out.iter().any(|e| e.0 == n) || !merge {
                    out.push((n, num("absolute_income_modifier").unwrap_or(0.0), num("percentage_income_modifier").unwrap_or(0.0), num("additional_multiplier_on_mining_colonies")));
                }
            }
        }
        if found && !merge {
            break; // the leaf-most object with an Abilities list wins
        }
    }
    out
}

fn main() {
    let mut args = std::env::args().skip(1);
    let modid = args.next().unwrap_or_else(|| "1770851727".into());
    let filter = args.next().unwrap_or_else(|| "Web_Way".into());
    let install = GameInstall::detect().expect("game install");
    let fs = GameFs::for_mod(&install, Some(&modid)).expect("game files");
    let (data, _) = load_galaxy(&fs);
    for c in data.campaigns.iter().filter(|c| c.has_galaxy() && c.name.contains(&filter)) {
        let Ok(sim) = Sim::new(&data, c, SimConfig::default()) else { continue };
        let (w, g) = (sim.world(), sim.galaxy());
        println!("# campaign {}", c.name);
        for &f in &w.main_factions {
            let fname = &w.factions[f as usize].name;
            let factor = data.faction(fname).map_or(1.0, |d| d.credits_accumulation_factor);
            let mut tot = [0.0f64; 2]; // replace, merge
            let (mut base_sum, mut planets) = (0.0f64, 0);
            let mut diffs: BTreeMap<String, f32> = BTreeMap::new();
            for (i, ps) in g.planets.iter().enumerate().filter(|(_, ps)| ps.owner == Some(f)) {
                planets += 1;
                let pname = &w.planets[i].name;
                let base = data.planet(pname).map_or(0.0, |p| p.credit_value as f32);
                if (base - w.planets[i].credit_value).abs() > 0.5 {
                    println!("  DEVIATION {fname}: planet {pname} value {} in the data, {} in the sim", base, w.planets[i].credit_value);
                }
                base_sum += base as f64;
                let mining = w.planets[i].mining_colony;
                for (k, merge) in [false, true].into_iter().enumerate() {
                    let mut names: Vec<String> = ps.structures.iter().map(|&s| w.structures[s as usize].name.clone()).collect();
                    if let Some(sb) = ps.starbase {
                        names.push(w.starbases[sb as usize].name.clone());
                    }
                    let (mut pct, mut abs) = (0.0f32, 0.0f32);
                    for n in &names {
                        for (_, a, p, m) in abilities(&data, n, merge) {
                            abs += a;
                            pct += p;
                            if mining && let Some(m) = m {
                                abs += a * m;
                                pct += p * m;
                            }
                        }
                    }
                    tot[k] += (base * (1.0 + pct + ps.gamble) + abs) as f64;
                    if k == 1 {
                        // which objects read differently
                        for n in names {
                            let (r, m): (f32, f32) = (abilities(&data, &n, false).iter().map(|a| a.1).sum(), abilities(&data, &n, true).iter().map(|a| a.1).sum());
                            if (r - m).abs() > 0.5 {
                                *diffs.entry(format!("{n}: replace {r}, merge {m}")).or_default() += 1.0;
                            }
                        }
                    }
                }
            }
            let mut ships = [0.0f64; 2];
            for fl in g.fleets.values().filter(|fl| fl.owner == f) {
                for (&u, &n) in &fl.units {
                    let name = &w.units[u as usize].name;
                    for (k, merge) in [false, true].into_iter().enumerate() {
                        let up: f32 = abilities(&data, name, merge).iter().map(|a| a.1).sum();
                        ships[k] += (up * n as f32) as f64;
                    }
                    let (r, m): (f32, f32) = (abilities(&data, name, false).iter().map(|a| a.1).sum(), abilities(&data, name, true).iter().map(|a| a.1).sum());
                    if (r - m).abs() > 0.5 {
                        *diffs.entry(format!("{name}: replace {r}, merge {m}")).or_default() += n as f32;
                    }
                }
            }
            let (replace, merge) = ((tot[0] + ships[0]) * factor as f64, (tot[1] + ships[1]) * factor as f64);
            let sim_income = g.factions[f as usize].income_per_week;
            let flag = if (replace - sim_income).abs() > 1.0 { "  <-- sim differs from the data" } else { "" };
            // Starting objects with an income effect in the data that the sim does not hold.
            for sf in c.starting_forces.iter().filter(|s| s.faction.eq_ignore_ascii_case(fname)) {
                let eff: f32 = abilities(&data, &sf.object, false).iter().map(|a| a.1.abs() + a.2.abs()).sum();
                if eff == 0.0 {
                    continue;
                }
                let Some(p) = w.planet_id(&sf.planet) else { continue };
                let ps = &g.planets[p as usize];
                let want = c.starting_forces.iter().filter(|o| o.faction.eq_ignore_ascii_case(fname) && o.planet == sf.planet && o.object.eq_ignore_ascii_case(&sf.object)).count();
                let have = if let Some(s) = w.structure_id(&sf.object) {
                    ps.structures.iter().filter(|&&x| x == s).count()
                } else if let Some(u) = w.unit_id(&sf.object) {
                    g.fleets.values().filter(|fl| fl.owner == f && fl.location == FleetLocation::AtPlanet(p)).map(|fl| fl.units.get(&u).copied().unwrap_or(0) as usize).sum()
                } else {
                    ps.starbase.map_or(0, |b| usize::from(w.starbases[b as usize].name.eq_ignore_ascii_case(&sf.object)))
                };
                if have < want && ps.owner == Some(f) {
                    println!("  MISSING {fname}: {} on {} ({have} of {want} in the sim, income effect {eff})", sf.object, sf.planet);
                }
            }
            let cf = c.faction(fname);
            if let Some(sc) = cf.and_then(|f| f.starting_credits) {
                let have = g.factions[f as usize].credits;
                if (have - sc as f64).abs() > 0.5 {
                    println!("  DEVIATION {fname}: starting credits {sc} in the data, {have} in the sim");
                }
            }
            println!("{fname:>11}: planets {planets:>3} base {base_sum:>7.0} | ship upkeep {:>8.0} | factor {factor:.2} | data(replace) {replace:>8.0} data(merge) {merge:>8.0} | sim {sim_income:>8.0} | credits {:.0}{flag}", ships[0], g.factions[f as usize].credits);
            for (d, n) in &diffs {
                println!("       replace/merge differs: {d} (x{n})");
            }
        }
    }
}
