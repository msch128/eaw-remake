//! Soak test of the scripted galactic game, headless (no Bevy app, no window).
//!
//! ```text
//! soak [--campaign GC_Full_Web_Way_Rebel] [--weeks 100] [--seed 1] [--human Rebel|Empire|none]
//!      [--tier easy|normal|hard|brutal|fair] [--classic] [--battles auto|tactical] [--ai-tactical]
//!      [--save-every 25] [--twice]
//! ```
//! The player side is driven by a simple bot (builds ships at planets with a shipyard, merges fleets and sends big ones at the weakest
//! neighbouring enemy or neutral planet); everyone else is the AI. Battles with the player are settled with `ResolveMode::Auto` or by the headless
//! tactical simulation (`--battles tactical`); with `--ai-tactical` the AI's own battles use the tactical resolver too. Every week the run checks
//! for anomalies (credits negative or exploding, script errors, stuck fleets, absurd fleet sizes, a dead worker) and every 25 weeks it writes a save
//! and reports its size and the process memory. Exit code 1 if anything was found. `--twice` plays the same game again and compares the final state.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Instant;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_play::{BuildMenu, BuildTarget, PlayCatalog, PlayConfig, PlayInit, ResolveMode, SimDriver, Snapshot, TacticalRuntime, build_command};
use galaxy_script::DifficultyTier;
use galaxy_sim::{BattleResolver, Command, FleetLocation, SimConfig, run_headless};

struct Opts {
    campaign: String,
    weeks: u64,
    seed: u64,
    human: String,
    tier: DifficultyTier,
    classic: bool,
    tactical_battles: bool,
    ai_tactical: bool,
    ai_tactical_max: Option<u32>,
    save_every: u64,
    twice: bool,
}

fn parse() -> Opts {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let has = |k: &str| args.iter().any(|a| a == k);
    Opts {
        campaign: get("--campaign").unwrap_or_else(|| "GC_Full_Web_Way_Rebel".into()),
        weeks: get("--weeks").and_then(|v| v.parse().ok()).unwrap_or(100),
        seed: get("--seed").and_then(|v| v.parse().ok()).unwrap_or(1),
        human: get("--human").unwrap_or_else(|| "active".into()),
        tier: match get("--tier").as_deref() {
            Some("easy") => DifficultyTier::Easy,
            Some("hard") => DifficultyTier::Hard,
            Some("brutal") => DifficultyTier::Brutal,
            Some("fair") => DifficultyTier::Fair,
            _ => DifficultyTier::Normal,
        },
        classic: has("--classic"),
        tactical_battles: get("--battles").as_deref() == Some("tactical"),
        ai_tactical: has("--ai-tactical"),
        ai_tactical_max: get("--ai-tactical-max").and_then(|v| v.parse().ok()),
        save_every: get("--save-every").and_then(|v| v.parse().ok()).unwrap_or(25),
        twice: has("--twice"),
    }
}

fn working_set_mb() -> f64 {
    let pid = std::process::id();
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &format!("(Get-Process -Id {pid}).WorkingSet64 / 1MB")])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().replace(',', ".").parse().ok())
        .unwrap_or(0.0)
}

/// Result of one run.
struct Outcome {
    hash: u64,
    anomalies: Vec<String>,
    wall: f64,
}

fn run(o: &Opts, fs: &Arc<dyn FileSource>, data: &Arc<alamo_data::GameData>) -> Outcome {
    let campaign = data.campaign(&o.campaign).unwrap_or_else(|| panic!("campaign {}", o.campaign)).clone();
    let mut cfg = PlayConfig { sim: SimConfig { seed: o.seed, victory_enabled: false, ..Default::default() }, difficulty: Some(o.tier), classic_ai: o.classic, ..PlayConfig::default() };
    if o.ai_tactical {
        cfg.sim.resolver = BattleResolver::Tactical;
        if let Some(m) = o.ai_tactical_max {
            cfg.sim.tactical_resolver_max_ships = m;
        }
    }
    let mut init = PlayInit::scripted(data.clone(), fs.clone(), &campaign, cfg).expect("init");
    let script = init.script.as_mut().expect("scripted");
    match o.human.as_str() {
        "active" => {}
        other => script.config.human = Some(other.to_string()),
    }
    let human_name = script.config.human.clone().unwrap_or_default();
    let human = init.sim.world().faction_id(&human_name);
    let world = init.sim.world().clone();
    let tactical = if o.tactical_battles || o.ai_tactical {
        let t = Instant::now();
        let names = init.sim.tactical_names();
        let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let catalog = Arc::new(battle_sim::Catalog::build(data, Some(&**fs), &name_refs));
        let arenas = init.sim.tactical_map_names().into_iter().filter_map(|n| battle_sim::Arena::load(data, &**fs, &n).map(|a| (n, Arc::new(a)))).collect();
        println!("tactical catalog and arenas built in {:?}", t.elapsed());
        Some(TacticalRuntime { catalog, arenas })
    } else {
        None
    };
    init.script.as_mut().unwrap().tactical = tactical.clone();
    let script = init.script.take().unwrap();
    let mut d = SimDriver::spawn_scripted(init.sim, script, human);
    let names = PlayCatalog::plain(&world);
    let t0 = Instant::now();
    let mut last = d.wait_initial();
    println!(
        "== {} seed {} human {} tier {:?}{} battles {}{}",
        o.campaign,
        o.seed,
        human_name,
        o.tier,
        if o.classic { " classic AI" } else { "" },
        if o.tactical_battles { "tactical" } else { "auto" },
        if o.ai_tactical { " + AI battles tactical" } else { "" }
    );
    let mut anomalies: Vec<String> = Vec::new();
    let flag = |week: u64, text: String, an: &mut Vec<String>| {
        if an.len() < 40 {
            println!("  !! week {week}: {text}");
        }
        an.push(format!("week {week}: {text}"));
    };
    let nf = world.factions.len();
    let (mut ships_min, mut ships_max, mut planets_min, mut planets_max) = (vec![u32::MAX; nf], vec![0u32; nf], vec![u32::MAX; nf], vec![0u32; nf]);
    let mut moving_seen: HashMap<u32, (u64, u64)> = HashMap::new(); // fleet -> (progress bits, week first seen with it)
    let (mut worst_batch, mut work_total, mut battles_human, mut battles_ai, mut captures) = (0u64, 0u64, 0u32, 0u32, 0u32);
    let mut errors_reported = 0;
    let mut saved: Vec<(u64, u64, f64)> = Vec::new();
    let mut mem: Vec<(u64, f64)> = vec![(0, working_set_mb())];
    let save_path = std::env::temp_dir().join(format!("soak_{}_{}_{}.json", o.seed, o.campaign, std::process::id()));
    let mut week = 0u64;
    let mut reported = u64::MAX;
    while week < o.weeks {
      // One game week in 1 s requests (a multiple of the tick); a player battle holds the clock until it is settled, then the rest of the week is requested again.
      let mut attempts = 0;
      while last.week <= week && attempts < 30 {
        attempts += 1;
        for _ in 0..60 {
            d.advance(1.0);
        }
        loop {
            let snaps = d.wait_idle();
            let Some(s) = snaps.last().cloned() else {
                flag(week, "the simulation worker died (panic?)".into(), &mut anomalies);
                return Outcome { hash: 0, anomalies, wall: t0.elapsed().as_secs_f64() };
            };
            for sn in &snaps {
                worst_batch = worst_batch.max(sn.work_micros);
                work_total += sn.work_micros;
                for e in &sn.events {
                    match e {
                        galaxy_sim::Event::PlanetCaptured { .. } => captures += 1,
                        galaxy_sim::Event::BattleResolved { .. } => battles_ai += 1,
                        _ => {}
                    }
                }
            }
            last = s;
            if last.human_battles.is_empty() {
                break;
            }
            // The player's battles.
            for b in last.human_battles.clone() {
                battles_human += 1;
                if !o.tactical_battles {
                    d.resolve_battle(b.id, ResolveMode::Auto);
                    continue;
                }
                d.tactical_setup(b.id);
                let snaps = d.wait_idle();
                let req = snaps.iter().flat_map(|s| s.tactical.iter()).find(|r| r.battle_id == b.id).cloned();
                match req.and_then(|r| {
                    let t = tactical.as_ref().unwrap();
                    let arena = r.map.as_ref().and_then(|m| t.arenas.get(m)).cloned();
                    run_headless(&r, &t.catalog, arena)
                }) {
                    Some(result) => d.apply_tactical(b.id, result),
                    None => d.cancel_tactical(b.id),
                }
            }
        }
      }
        if last.week <= week {
            flag(week, "game time does not advance (stalled)".into(), &mut anomalies);
            break;
        }
        week = last.week;
        let g = &last.galaxy;

        // ---- checks -------------------------------------------------------------------------------------
        for (i, f) in g.factions.iter().enumerate() {
            if f.credits < -1.0 || f.credits > 5.0e7 || !f.credits.is_finite() {
                flag(week, format!("{} credits {:.0}", world.factions[i].name, f.credits), &mut anomalies);
            }
            let ships: u32 = g.fleets.values().filter(|x| x.owner == i as u32).map(|x| x.unit_count()).sum();
            if ships > 4000 {
                flag(week, format!("{} has {ships} ships", world.factions[i].name), &mut anomalies);
            }
            ships_min[i] = ships_min[i].min(ships);
            ships_max[i] = ships_max[i].max(ships);
            planets_min[i] = planets_min[i].min(f.planets_owned);
            planets_max[i] = planets_max[i].max(f.planets_owned);
        }
        if let Some(dg) = &last.diagnostics
            && dg.script_errors > errors_reported
        {
            flag(week, format!("script errors: {} (last: {:?})", dg.script_errors, dg.last_script_error), &mut anomalies);
            errors_reported = dg.script_errors;
        }
        // Fleets that sit in hyperspace without progress for 4 weeks.
        let mut now_moving = HashMap::new();
        for f in g.fleets.values() {
            if let FleetLocation::Moving { progress, .. } = &f.location {
                let sig = u64::from(progress.to_bits());
                let first = match moving_seen.get(&f.id) {
                    Some(&(s, w)) if s == sig => w,
                    _ => week,
                };
                if week - first >= 4 {
                    flag(week, format!("fleet {} of {} has not moved for {} weeks in hyperspace: {:?}", f.id, world.factions[f.owner as usize].name, week - first, f.location), &mut anomalies);
                }
                now_moving.insert(f.id, (sig, first));
            }
        }
        moving_seen = now_moving;

        // ---- the player bot -----------------------------------------------------------------------------
        if let Some(h) = human
            && week % 2 == 0
        {
            bot(&mut d, &last, &world, &names, h);
        }

        if week != reported && week % o.save_every == 0 {
            d.save(&save_path);
            d.wait_idle();
            let size = std::fs::metadata(&save_path).map(|m| m.len()).unwrap_or(0);
            let steps = last.diagnostics.as_ref().map_or(0, |x| x.journal_steps);
            saved.push((week, size, steps as f64));
            let m = working_set_mb();
            mem.push((week, m));
            println!("  week {week}: save {} KB ({} journal steps), memory {m:.0} MB, {} fleets, {} planets changed hands so far", size / 1024, steps, g.fleets.len(), captures);
        }
        let first_time = week != reported;
        reported = week;
        if first_time && week % 20 == 0 {
            let total: Vec<String> = g.factions.iter().enumerate().filter(|(_, f)| f.planets_owned > 0).map(|(i, f)| format!("{}:{}p/{}k/pop{}of{}/T{}", &world.factions[i].name[..world.factions[i].name.len().min(4)], f.planets_owned, (f.credits / 1000.0) as i64, f.population_used, f.population_cap, f.tech)).collect();
            println!("  week {week}: {}", total.join(" "));
        }
    }
    let wall = t0.elapsed().as_secs_f64();
    let g = &last.galaxy;
    // Factions that never did anything: no change in planets and ships over the whole game although they started with something.
    for i in 0..nf {
        let name = &world.factions[i].name;
        if (planets_max[i] > 0 || ships_max[i] > 0) && planets_min[i] == planets_max[i] && ships_min[i] == ships_max[i] && !name.eq_ignore_ascii_case("pirates") && Some(i as u32) != human && o.weeks >= 40 {
            flag(week, format!("{name} never acted (planets {}, ships {}, credits {:.0})", planets_max[i], ships_max[i], g.factions[i].credits), &mut anomalies);
        }
    }
    // Growth: journal and memory must not blow up.
    if let (Some(first), Some(lastp)) = (saved.first(), saved.last())
        && saved.len() >= 3
    {
        let per_step = |p: &(u64, u64, f64)| p.1 as f64 / p.2.max(1.0);
        println!("  save size per journal step: {:.1} B at week {} -> {:.1} B at week {}", per_step(first), first.0, per_step(lastp), lastp.0);
        if per_step(lastp) > 3.0 * per_step(first) + 50.0 {
            flag(week, "journal size grows faster than the number of steps".into(), &mut anomalies);
        }
    }
    if mem.len() >= 3 {
        let (m0, m1) = (mem[1].1.max(1.0), mem[mem.len() - 1].1);
        println!("  memory: {m0:.0} MB (week {}) -> {m1:.0} MB (week {})", mem[1].0, mem[mem.len() - 1].0);
        if m1 > m0 * 2.0 + 300.0 {
            flag(week, format!("memory grew from {m0:.0} to {m1:.0} MB"), &mut anomalies);
        }
    }
    let planets: BTreeMap<&str, u32> = g.factions.iter().enumerate().filter(|(_, f)| f.planets_owned > 0).map(|(i, f)| (world.factions[i].name.as_str(), f.planets_owned)).collect();
    println!("  final planets {planets:?}");
    println!("  {week} weeks in {wall:.1} s ({:.0} ms/week wall, {:.0} ms/week worker, worst batch {:.0} ms), {captures} captures, {battles_human} player battles, {battles_ai} resolved battles", wall * 1000.0 / week as f64, work_total as f64 / 1000.0 / week as f64, worst_batch as f64 / 1000.0);
    let _ = std::fs::remove_file(&save_path);
    Outcome { hash: g.state_hash(), anomalies, wall }
}

/// A deliberately simple player: spend money on ships, gather, attack weak neighbours.
fn bot(d: &mut SimDriver, s: &Snapshot, w: &Arc<galaxy_sim::World>, names: &PlayCatalog, h: u32) {
    let g = &s.galaxy;
    // Build: at every owned planet that offers units, queue the dearest affordable one (the sim refuses when the queue is full).
    let credits = g.factions[h as usize].credits as f32;
    let mut budget = credits;
    for (pl, ps) in g.planets.iter().enumerate() {
        if ps.owner != Some(h) || budget < 500.0 {
            continue;
        }
        let Some(menu) = BuildMenu::for_planet(w, g, names, h, pl as u32) else { continue };
        let pick = menu.options.iter().filter(|o| o.availability.is_available() && matches!(o.target, BuildTarget::Unit(_)) && o.cost <= budget).max_by(|a, b| a.cost.total_cmp(&b.cost));
        if let Some(o) = pick {
            budget -= o.cost;
            d.send_command(build_command(h, pl as u32, o.target));
        }
    }
    // Merge idle fleets that share a planet, then send big ones at the weakest neighbouring non-owned planet.
    let mut by_planet: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for f in g.fleets.values().filter(|f| f.owner == h) {
        if let Some(p) = f.planet() {
            by_planet.entry(p).or_default().push(f.id);
        }
    }
    for (p, ids) in by_planet {
        for &other in ids.iter().skip(1) {
            d.send_command(Command::MergeFleets { faction: h, into: ids[0], from: other });
        }
        let Some(main) = g.fleets.get(&ids[0]) else { continue };
        let units: u32 = ids.iter().filter_map(|i| g.fleets.get(i)).map(|f| f.unit_count()).sum();
        if units + main.unit_count() < 8 {
            continue;
        }
        let target = w.adjacency[p as usize]
            .iter()
            .map(|&(n, _)| n)
            .filter(|&n| w.real[n as usize] && g.planets[n as usize].owner != Some(h))
            .min_by_key(|&n| g.fleets_at(n).map(|f| f.unit_count()).sum::<u32>() + 5 * u32::from(g.planets[n as usize].starbase.is_some()));
        if let Some(t) = target {
            d.send_command(Command::MoveFleet { faction: h, fleet: ids[0], to: t });
        }
    }
}

fn main() {
    let o = parse();
    let install = GameInstall::detect().expect("Empire at War install");
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).expect("game files"));
    let (data, _) = load_galaxy(&*fs);
    let data = Arc::new(data);
    let a = run(&o, &fs, &data);
    let mut anomalies = a.anomalies.clone();
    if o.twice {
        let b = run(&o, &fs, &data);
        if a.hash != b.hash {
            anomalies.push(format!("NOT DETERMINISTIC: final hashes {:x} vs {:x}", a.hash, b.hash));
        } else {
            println!("determinism: both runs ended in state {:x}", a.hash);
        }
        anomalies.extend(b.anomalies);
    }
    println!("{}", if anomalies.is_empty() { "SOAK OK".to_string() } else { format!("SOAK FOUND {} anomalies (first: {})", anomalies.len(), anomalies[0]) });
    let _ = a.wall;
    std::process::exit(i32::from(!anomalies.is_empty()));
}
