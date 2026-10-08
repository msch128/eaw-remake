//! Compares the statistical auto-resolve with the tactical simulation on a sample of battles.
//!
//! ```text
//! cargo run --release -p galaxy-sim --example calibrate -- [scenarios=48] [tactical seeds=3] [auto seeds=40] [rng seed=1]
//! ```
//!
//! Scenarios: random compositions of buildable ships (tech 1-3) of two major factions, scaled to near-even strength (so the winner is open), from 2 to ~40
//! ships a side, half of them at a defended planet (the defender owns a star base), half in open space. Each scenario is resolved by `run_headless` (several
//! battle seeds) and by `Sim::preview_auto_resolve` (many seeds). Reported: winner agreement (side with the higher win share; draws count half), mean
//! absolute difference of the attacker's win share, and the mean fraction of ships lost per side, by star base and by size.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::combat::Source;
use galaxy_sim::rng::Rng;
use galaxy_sim::*;

#[derive(Default, Clone)]
struct Acc {
    n: f64,
    agree: f64,
    abs_diff: f64,
    signed_diff: f64,
    loss_att: [f64; 2],
    loss_def: [f64; 2],
    big_side_win: [f64; 2],
    loser_surv: [f64; 2],
    decisive: [f64; 2],
}

impl Acc {
    fn add(&mut self, tac: &Row, auto: &Row, big_is_att: bool) {
        self.n += 1.0;
        self.agree += f64::from((tac.att_win - 0.5).signum() == (auto.att_win - 0.5).signum() || (tac.att_win - 0.5).abs() < 0.1);
        self.abs_diff += (tac.att_win - auto.att_win).abs();
        self.signed_diff += auto.att_win - tac.att_win;
        self.loss_att[0] += tac.loss_att;
        self.loss_att[1] += auto.loss_att;
        self.loss_def[0] += tac.loss_def;
        self.loss_def[1] += auto.loss_def;
        let big = |r: &Row| if big_is_att { r.att_win } else { 1.0 - r.att_win };
        self.big_side_win[0] += big(tac);
        self.big_side_win[1] += big(auto);
        self.loser_surv[0] += tac.loser_survived;
        self.loser_surv[1] += auto.loser_survived;
        self.decisive[0] += tac.decisive;
        self.decisive[1] += auto.decisive;
    }

    fn print(&self, label: &str) {
        let n = self.n.max(1.0);
        println!(
            "{label:<16} n={:<3} agree {:>5.1}%  |dwin| {:.2}  auto-tac attacker win {:+.2}  loss att tac/auto {:.2}/{:.2}  def {:.2}/{:.2}  bigger side wins {:.2}/{:.2}  loser keeps {:.2}/{:.2} decisive {:.2}/{:.2}",
            self.n,
            100.0 * self.agree / n,
            self.abs_diff / n,
            self.signed_diff / n,
            self.loss_att[0] / n,
            self.loss_att[1] / n,
            self.loss_def[0] / n,
            self.loss_def[1] / n,
            self.big_side_win[0] / n,
            self.big_side_win[1] / n,
            self.loser_surv[0] / self.decisive[0].max(1e-9),
            self.loser_surv[1] / self.decisive[1].max(1e-9),
            self.decisive[0] / n,
            self.decisive[1] / n,
        );
    }
}

/// Mean over seeds: attacker win share (draw = 0.5) and the fraction of ships each side lost.
#[derive(Default, Clone, Copy)]
struct Row {
    att_win: f64,
    loss_att: f64,
    loss_def: f64,
    /// Tactical only: share of the losing side's ships still alive at the end (retreaters).
    loser_survived: f64,
    decisive: f64,
}

fn main() {
    let args: Vec<usize> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let scenarios = args.first().copied().unwrap_or(48);
    let tac_seeds = args.get(1).copied().unwrap_or(3);
    let auto_seeds = args.get(2).copied().unwrap_or(40);
    let tac_seeds_big = args.get(4).copied().unwrap_or(tac_seeds);
    let env = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<f32>().ok());
    // Defaults are the configured values; BASE, EF, EC, EFR, FOCUS override them for sweeps (`OLD=1` = the formula before calibration).
    let c0 = SimConfig::default();
    let mut tuning = galaxy_sim::combat::Tuning { focus: c0.combat_focus, base_factor: c0.combat_base_factor, efficiency: c0.combat_efficiency, crowd: c0.combat_crowd, waves: c0.combat_waves };
    if std::env::var("OLD").is_ok() {
        tuning = galaxy_sim::combat::Tuning { focus: 1, base_factor: 1.0, efficiency: [1.0; 4], crowd: (25.0, 0.0), waves: false };
    }
    tuning.focus = env("FOCUS").map_or(tuning.focus, |v| v as u32);
    tuning.base_factor = env("BASE").unwrap_or(tuning.base_factor);
    tuning.waves = env("WAVES").map_or(tuning.waves, |v| v > 0.0);
    tuning.crowd = (env("CK").unwrap_or(tuning.crowd.0), env("CG").unwrap_or(tuning.crowd.1));
    tuning.efficiency = [env("EF").unwrap_or(tuning.efficiency[0]), env("EC").unwrap_or(tuning.efficiency[1]), env("EFR").unwrap_or(tuning.efficiency[2]), tuning.efficiency[3]];
    let mut rng = Rng::new(args.get(3).copied().unwrap_or(1) as u64);

    let install = GameInstall::detect().expect("game install not found");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("cannot open game files");
    let (data, _) = load_galaxy(&fs);
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let mut sim = Sim::new(&data, campaign, SimConfig { auto_resolve_battles: false, victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let names = sim.tactical_names();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let catalog = Arc::new(battle_sim::Catalog::build(&data, Some(&fs), &names));
    let w = sim.world().clone();

    let majors: Vec<FactionId> = ["Empire", "Rebel", "Underworld"].iter().filter_map(|n| w.faction_id(n)).collect();
    let pool = |f: FactionId| -> Vec<UnitTypeId> {
        let key = w.factions[f as usize].name.to_ascii_lowercase();
        (0..w.units.len() as u32)
            .filter(|&u| {
                let ut = &w.units[u as usize];
                matches!(ut.role, UnitRole::Ship) && ut.cost > 0.0 && ut.tech <= 3 && ut.affiliation.contains(&key) && !ut.hero && ut.combat.hull > 0.0 && ut.combat.raw_dps() > 0.0 && ut.category != alamo_data::ShipCategory::Transport
            })
            .collect()
    };
    let open_planet = (0..w.planets.len() as u32)
        .find(|&p| w.real[p as usize] && w.planets[p as usize].space_map.is_some() && sim.galaxy().planets[p as usize].owner.is_none())
        .expect("an open planet");

    // Tactical results are slow: they are cached by scenario in `target/calibrate_cache.txt`.
    let cache_file = std::env::var("CACHE").unwrap_or_else(|_| "target/calibrate_cache.txt".into());
    let cache_path = std::path::Path::new(&cache_file);
    let mut cache: std::collections::HashMap<String, Row> = std::fs::read_to_string(cache_path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            Some((f.first()?.to_string(), Row { att_win: f.get(1)?.parse().ok()?, loss_att: f.get(2)?.parse().ok()?, loss_def: f.get(3)?.parse().ok()?, loser_survived: f.get(4)?.parse().ok()?, decisive: f.get(5)?.parse().ok()? }))
        })
        .collect();
    let mut all = Acc::default();
    let (mut with_sb, mut open) = (Acc::default(), Acc::default());
    let mut by_size = [Acc::default(), Acc::default(), Acc::default()];
    let mut skipped = 0;
    for sc in 0..scenarios {
        // Two distinct major factions; the first is the attacker.
        let a = majors[rng.below(majors.len() as u64) as usize];
        let d = loop {
            let d = majors[rng.below(majors.len() as u64) as usize];
            if d != a {
                break d;
            }
        };
        let defended = sc % 2 == 0;
        let planet = if defended { w.factions[d as usize].home.expect("home") } else { open_planet };
        if defended && sim.galaxy().planets[planet as usize].starbase.is_none() {
            skipped += 1;
            continue;
        }
        let scale = if std::env::var("BIGONLY").is_ok() { [20.0f32, 30.0, 40.0, 60.0][rng.below(4) as usize] } else { [2.0f32, 5.0, 10.0, 20.0, 40.0][rng.below(5) as usize] };
        let compose = |f: FactionId, rng: &mut Rng| -> Vec<(UnitTypeId, f32)> {
            let p = pool(f);
            (0..1 + rng.below(4)).map(|_| (p[rng.below(p.len() as u64) as usize], 0.3 + rng.unit())).collect()
        };
        let (ca, cd) = (compose(a, &mut rng), compose(d, &mut rng));
        let strength = |c: &[(UnitTypeId, f32)], k: f32| w.strength(c.iter().map(|&(u, x)| (Source::Unit(u), (x * k).max(0.0).round() as u32)));
        // Scale side A to ~`scale` ships on average, then side D to a random fraction of A's strength (0.6..1.6) counting the star base on defense.
        let per = ca.iter().map(|(_, x)| x).sum::<f32>().max(0.1);
        if std::env::var("DBG").is_ok() { eprintln!("scale {scale} ca {ca:?} per {per}"); }
        let ka = scale / per;
        let counts_a: Vec<(UnitTypeId, u32)> = ca.iter().map(|&(u, x)| (u, (x * ka).round().max(1.0) as u32)).collect();
        let want = strength(&ca, ka) * (0.6 + rng.unit());
        let mut kd = 0.1f32;
        while strength(&cd, kd) < want && kd < 200.0 {
            kd *= 1.1;
        }
        let counts_d: Vec<(UnitTypeId, u32)> = cd.iter().map(|&(u, x)| (u, (x * kd).round().max(if x * kd >= 0.5 { 1.0 } else { 0.0 }) as u32)).filter(|&(_, n)| n > 0).collect();
        if counts_d.is_empty() {
            skipped += 1;
            continue;
        }

        let g = sim.galaxy_mut();
        g.fleets.clear();
        g.battles.clear();
        for (id, (owner, counts)) in (1..).zip([(d, &counts_d), (a, &counts_a)]) {
            g.fleets.insert(id, Fleet { id, owner, units: counts.iter().copied().collect(), location: FleetLocation::AtPlanet(planet), previous_planet: planet });
        }
        g.planets[planet as usize].owner = if defended { Some(d) } else { None };
        let battle = sim.step(1.0).into_iter().find_map(|e| if let Event::BattlePending(b) = e { Some(b) } else { None });
        let Some(battle) = battle else {
            skipped += 1;
            continue;
        };
        let req = sim.tactical_setup(battle.id).expect("request");
        let ships_a: u32 = counts_a.iter().map(|x| x.1).sum();
        let ships_d: u32 = counts_d.iter().map(|x| x.1).sum();

        // Tactical.
        let side_of = |f: FactionId| req.sides.iter().position(|s| s.faction == f).unwrap();
        let (sa, sd) = (side_of(a), side_of(d));
        // Alive = deployed survivors + reserve never called + called reserve that survived.
        let alive = |r: &TacticalResult, s: usize| -> u32 {
            let deployed: u32 = req.sides[s].groups.iter().zip(&r.survived[s]).filter(|(g, _)| g.kind == GroupKind::Unit).map(|(_, n)| *n).sum();
            let reserve: u32 = req.sides[s]
                .reserve
                .iter()
                .enumerate()
                .map(|(ri, g)| {
                    let called = r.reserve_called.get(s).and_then(|v| v.get(ri)).copied().unwrap_or(0).min(g.count);
                    g.count - called + r.reserve_survived.get(s).and_then(|v| v.get(ri)).copied().unwrap_or(0).min(called)
                })
                .sum();
            deployed + reserve
        };
        let tac_seeds = if ships_a + ships_d >= 40 { tac_seeds_big } else { tac_seeds };
        let mut srng = Rng::new(0x5EED ^ (sc as u64) << 20);
        let key = format!("{planet}|{defended}|{tac_seeds}|{a}:{counts_a:?}|{d}:{counts_d:?}");
        let mut tac = cache.get(&key).copied().unwrap_or_default();
        let cached = cache.contains_key(&key);
        for s in 0..if cached { 0 } else { tac_seeds } {
            let mut r = req.clone();
            r.seed = srng.next_u64() ^ s as u64;
            let Some(res) = run_headless(&r, &catalog, None) else { continue };
            tac.att_win += match res.winner {
                Some(x) if x == sa => 1.0,
                Some(_) => 0.0,
                None => 0.5,
            } / tac_seeds as f64;
            tac.loss_att += f64::from(ships_a - alive(&res, sa).min(ships_a)) / f64::from(ships_a) / tac_seeds as f64;
            tac.loss_def += f64::from(ships_d - alive(&res, sd).min(ships_d)) / f64::from(ships_d) / tac_seeds as f64;
            if let Some(wi) = res.winner {
                let (ls, n0) = if wi == sa { (sd, ships_d) } else { (sa, ships_a) };
                tac.loser_survived += f64::from(alive(&res, ls).min(n0)) / f64::from(n0) / tac_seeds as f64;
                tac.decisive += 1.0 / tac_seeds as f64;
            }
        }
        if !cached {
            cache.insert(key.clone(), tac);
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(cache_path) {
                let _ = writeln!(f, "{key}	{}	{}	{}	{}	{}", tac.att_win, tac.loss_att, tac.loss_def, tac.loser_survived, tac.decisive);
            }
        }
        if std::env::var("DBG").is_ok() { eprintln!("tactical done"); }
        // Auto-resolve.
        let mut auto = Row::default();
        for s in 0..auto_seeds {
            let p = sim.preview_auto_resolve_with(battle.id, srng.next_u64() ^ s as u64, tuning).expect("preview");
            if let Some(wf) = p.winner {
                let (l0, l1) = p.sides.iter().find(|x| x.0 != wf).map_or((1, 1), |x| (x.1.max(1), x.2));
                auto.loser_survived += f64::from(l1.min(l0)) / f64::from(l0) / auto_seeds as f64;
                auto.decisive += 1.0 / auto_seeds as f64;
            }
            auto.att_win += match p.winner {
                Some(x) if x == a => 1.0,
                Some(_) => 0.0,
                None => 0.5,
            } / auto_seeds as f64;
            let of = |f| p.sides.iter().find(|x| x.0 == f).map_or((1, 1), |x| (x.1.max(1), x.2));
            auto.loss_att += f64::from(of(a).0 - of(a).1.min(of(a).0)) / f64::from(of(a).0) / auto_seeds as f64;
            auto.loss_def += f64::from(of(d).0 - of(d).1.min(of(d).0)) / f64::from(of(d).0) / auto_seeds as f64;
        }
        let comp = |c: &[(UnitTypeId, u32)]| c.iter().map(|(u, n)| format!("{n}{}", w.units[*u as usize].name.chars().take(10).collect::<String>())).collect::<Vec<_>>().join(",");
        let total = ships_a + ships_d;
        println!(
            "{sc:>3} {} {}{}x{ships_a:<3} vs {}{}x{ships_d:<3}  att win tac {:.2} auto {:.2}  loss att {:.2}/{:.2} def {:.2}/{:.2}",
            if defended { "SB " } else { "open" },
            w.factions[a as usize].name.chars().take(3).collect::<String>(),
            comp(&counts_a),
            w.factions[d as usize].name.chars().take(3).collect::<String>(),
            comp(&counts_d),
            tac.att_win,
            auto.att_win,
            tac.loss_att,
            auto.loss_att,
            tac.loss_def,
            auto.loss_def
        );
        let big_is_att = strength(&ca, ka) >= want;
        all.add(&tac, &auto, big_is_att);
        if defended { &mut with_sb } else { &mut open }.add(&tac, &auto, big_is_att);
        by_size[if total < 12 { 0 } else if total < 40 { 1 } else { 2 }].add(&tac, &auto, big_is_att);
    }
    println!("\nskipped {skipped}");
    all.print("all");
    with_sb.print("with star base");
    open.print("open space");
    by_size[0].print("< 12 ships");
    by_size[1].print("12-39 ships");
    by_size[2].print(">= 40 ships");
}
