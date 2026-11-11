//! The Pirates grow with the human player (docs/RULES.md 1.9), on the real Remake data in "From the Ground Up" games with the story scripts running.
//! Skipped without the game and the mod.

use std::sync::Arc;

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::{FactionId, Sim, SimConfig};

struct Ctx {
    fs: Arc<dyn FileSource>,
    data: Arc<GameData>,
}

fn ctx() -> Option<Ctx> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
    let (data, _) = load_galaxy(&*fs);
    Some(Ctx { fs, data: Arc::new(data) })
}

fn scripted(c: &Ctx, campaign: &str, human: &str) -> ScriptedSim {
    let campaign = c.data.campaign(campaign).expect("campaign").clone();
    ScriptedSim::new(
        c.fs.clone(),
        c.data.clone(),
        &campaign,
        SimConfig { victory_enabled: false, ground_up: true, seed: 5, ..SimConfig::default() },
        ScriptedConfig { remake_ai: false, human: Some(human.into()), game_scoring: false, ..ScriptedConfig::default() },
    )
    .expect("sim")
}

/// Pirate planets by their star base level.
fn base_levels(s: &Sim) -> std::collections::BTreeMap<i32, usize> {
    let (w, g) = (s.world(), s.galaxy());
    let pirates = w.passive_faction().unwrap();
    let mut m = std::collections::BTreeMap::new();
    for p in (0..w.planets.len() as u32).filter(|&p| w.real[p as usize] && g.planets[p as usize].owner == Some(pirates)) {
        *m.entry(w.starbase_level(g, p)).or_default() += 1;
    }
    m
}

fn faction(s: &Sim, name: &str) -> FactionId {
    s.world().faction_id(name).unwrap()
}

#[test]
fn an_all_ai_game_has_no_human_and_the_pirates_follow_the_best_major_faction() {
    let Some(c) = ctx() else { return };
    let mut sim = scripted(&c, "GC_Full_Web_Way_Rebel", "none");
    assert_eq!(sim.with_sim(|s| s.world().victory.active_player), None, "human \"none\": nobody is the human player");
    // Ground Up gives every main faction three planets when nobody is human.
    sim.with_sim(|s| {
        for f in ["Rebel", "Empire", "Underworld"] {
            assert_eq!(s.galaxy().owned_planets(faction(s, f)).count(), 3, "{f} keeps three planets");
        }
    });
    // The Empire reaches tech 4 while the Rebels stay at 1: the Pirates follow the best major faction (their bases rise to level 2).
    sim.with_sim_mut(|s| {
        let e = faction(s, "Empire");
        s.galaxy_mut().factions[e as usize].tech = 4;
    });
    let week = sim.with_sim(|s| s.world().config.week_seconds);
    for _ in 0..(week as u32 + 1) {
        sim.step(1.0);
    }
    let levels = sim.with_sim(base_levels);
    println!("pirate base levels after a week with the Empire at tech 4: {levels:?}");
    let total: usize = levels.values().sum();
    let raised: usize = levels.iter().filter(|(l, _)| **l >= 2).map(|(_, n)| n).sum();
    assert!(raised * 10 >= total * 9, "bases follow the best major faction's tech: {levels:?}");
}

#[test]
fn a_game_with_a_human_keeps_that_human() {
    let Some(c) = ctx() else { return };
    for (campaign, human) in [("GC_Full_Web_Way_Rebel", "Rebel"), ("GC_Full_Web_Way_Imperial", "Empire"), ("GC_Full_Web_Way_Underworld", "Underworld")] {
        let sim = scripted(&c, campaign, human);
        sim.with_sim(|s| {
            assert_eq!(s.world().victory.active_player, Some(faction(s, human)), "{campaign}");
            assert_eq!(s.galaxy().owned_planets(faction(s, human)).count(), 1, "{campaign}: the human starts with one planet");
        });
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Growth with the human's tech: the Pirate fleet of every planet grows toward start value + PASSIVE_GROWTH_CREDITS[T].
// ---------------------------------------------------------------------------------------------------------------------

use std::collections::BTreeMap;

use alamo_data::ShipCategory;
use galaxy_sim::{Event, Fleet, FleetLocation, PlanetId, SimSnapshot, UnitTypeId};

/// Weeks the growth runs in the tests: at 2 units a week every planet reaches its tech 5 value (24k credits are 20 to 30 units).
const GROW_WEEKS: u32 = 18;

/// A Ground Up game of `campaign` right after the story start (the start spawn of `Spawn_Starting_Forces_Web_Way.lua`), as a plain sim
/// without any AI, so nothing but the Pirates' growth changes the Pirate fleets.
fn started(c: &Ctx, campaign: &str, human: &str) -> (Sim, SimSnapshot) {
    let mut s = scripted(c, campaign, human);
    s.step(1.0);
    let snap = s.with_sim(|s| s.snapshot());
    (restored(c, campaign, snap.clone()), snap)
}

fn restored(c: &Ctx, campaign: &str, snap: SimSnapshot) -> Sim {
    let camp = c.data.campaign(campaign).unwrap();
    let mut sim = Sim::new(&c.data, camp, SimConfig { victory_enabled: false, ground_up: true, seed: 5, auto_resolve_battles: false, ..SimConfig::default() }).unwrap();
    sim.restore(snap).unwrap();
    sim
}

/// Pirate units per planet (type -> count).
fn pirate_units(s: &Sim) -> BTreeMap<PlanetId, BTreeMap<UnitTypeId, u32>> {
    let pirates = s.world().passive_faction().unwrap();
    let mut m: BTreeMap<PlanetId, BTreeMap<UnitTypeId, u32>> = BTreeMap::new();
    for f in s.galaxy().fleets.values().filter(|f| f.owner == pirates) {
        if let FleetLocation::AtPlanet(p) = f.location {
            for (&u, &n) in &f.units {
                *m.entry(p).or_default().entry(u).or_default() += n;
            }
        }
    }
    m
}

fn count(m: &BTreeMap<PlanetId, BTreeMap<UnitTypeId, u32>>) -> u32 {
    m.values().flat_map(|x| x.values()).sum()
}

fn value(s: &Sim, units: &BTreeMap<UnitTypeId, u32>) -> f32 {
    units.iter().map(|(&u, &n)| s.world().units[u as usize].cost * n as f32).sum()
}

/// 0 squadrons, 1 corvettes, 2 frigates, 3 capitals (and super capitals), 4 anything else.
fn class(s: &Sim, u: UnitTypeId) -> usize {
    match s.world().units[u as usize].category {
        ShipCategory::Fighter | ShipCategory::Bomber => 0,
        ShipCategory::Corvette => 1,
        ShipCategory::Frigate => 2,
        ShipCategory::Capital | ShipCategory::Super => 3,
        _ => 4,
    }
}

struct Grown {
    start: BTreeMap<PlanetId, BTreeMap<UnitTypeId, u32>>,
    end: BTreeMap<PlanetId, BTreeMap<UnitTypeId, u32>>,
    /// Units added per class over all planets.
    added: [u32; 5],
    /// Most units one planet got in one week.
    max_per_week: u32,
}

/// Runs `GROW_WEEKS` weeks with the human at `tech`.
fn grow(sim: &mut Sim, tech: i32) -> Grown {
    let w = sim.world().clone();
    let human = w.victory.active_player.expect("human");
    sim.galaxy_mut().factions[human as usize].tech = tech;
    let start = pirate_units(sim);
    let mut last = start.clone();
    let mut max_per_week = 0;
    for _ in 0..GROW_WEEKS {
        sim.step(w.config.week_seconds);
        let now = pirate_units(sim);
        for (p, us) in &now {
            let before: u32 = last.get(p).map_or(0, |m| m.values().sum());
            max_per_week = max_per_week.max(us.values().sum::<u32>().saturating_sub(before));
        }
        last = now;
    }
    let mut added = [0u32; 5];
    for (p, us) in &last {
        for (&u, &n) in us {
            let b = start.get(p).and_then(|m| m.get(&u)).copied().unwrap_or(0);
            added[class(sim, u)] += n.saturating_sub(b);
        }
    }
    Grown { start, end: last, added, max_per_week }
}

/// Half the cost of the cheapest capital ship the Pirates field: the most a light frigate may cost.
fn light_frigate_cap(s: &Sim, units: &BTreeMap<PlanetId, BTreeMap<UnitTypeId, u32>>) -> f32 {
    units.values().flat_map(|m| m.keys()).filter(|&&u| class(s, u) == 3).map(|&u| s.world().units[u as usize].cost).fold(f32::MAX, f32::min) * 0.5
}

#[test]
fn pirate_fleets_grow_with_the_humans_tech_by_squadrons_and_corvettes_and_never_by_capitals() {
    let Some(c) = ctx() else { return };
    for (campaign, human) in [("GC_Full_Web_Way_Rebel", "Rebel"), ("GC_Full_Web_Way_Imperial", "Empire"), ("GC_Full_Web_Way_Underworld", "Underworld")] {
        let mut frigate_share = Vec::new();
        for tech in 1..=5 {
            let (mut sim, _) = started(&c, campaign, human);
            let g = grow(&mut sim, tech);
            let add = galaxy_sim::PASSIVE_GROWTH_CREDITS[(tech - 1) as usize];
            let light = light_frigate_cap(&sim, &g.start);
            let total: u32 = g.added.iter().sum();
            let shares: Vec<f32> = g.added.iter().map(|&n| n as f32 / total.max(1) as f32).collect();
            println!(
                "{campaign} tech {tech}: +{add} credits per planet, {total} units added (squadrons / corvettes / frigates / capitals / other {:?}, shares {:.2?}), {} Pirate units, most per planet and week {}, light frigates up to {light}",
                g.added,
                shares,
                count(&g.end),
                g.max_per_week
            );
            assert_eq!(g.added[3], 0, "{campaign} tech {tech}: the growth never adds a capital ship");
            assert_eq!(g.added[4], 0, "{campaign} tech {tech}: only squadrons, corvettes and frigates");
            assert!(g.max_per_week <= galaxy_sim::PASSIVE_GROWTH_UNITS_PER_WEEK, "{campaign} tech {tech}: {} units in one week", g.max_per_week);
            for (p, us) in &g.end {
                let start = g.start.get(p).map_or(0.0, |m| value(&sim, m));
                let now = value(&sim, us);
                let name = &sim.world().planets[*p as usize].name;
                assert!(now >= start + add, "{campaign} tech {tech}: {name} grew to {now} of {}", start + add);
                assert!(now < start + add + light, "{campaign} tech {tech}: {name} overshoots: {now} for {}", start + add);
                for (&u, &n) in us {
                    let b = g.start.get(p).and_then(|m| m.get(&u)).copied().unwrap_or(0);
                    if n > b && class(&sim, u) == 2 {
                        assert!(sim.world().units[u as usize].cost <= light, "{campaign}: {} is no light frigate", sim.world().units[u as usize].name);
                    }
                }
            }
            if tech == 1 {
                assert_eq!(total, 0, "tech 1 adds nothing to the start fleets");
                continue;
            }
            let want = galaxy_sim::passive_growth_shares(tech);
            for k in 0..3 {
                assert!((shares[k] - want[k]).abs() < 0.06, "{campaign} tech {tech}: share {k} is {:.2}, the rule says {:.2}", shares[k], want[k]);
            }
            frigate_share.push(shares[2]);
        }
        assert!(frigate_share[0] > *frigate_share.last().unwrap(), "{campaign}: the frigate share falls with the tech level: {frigate_share:?}");
    }
}

#[test]
fn pirate_unit_counts_per_tech_level_and_the_cato_neimoidia_stronghold() {
    let Some(c) = ctx() else { return };
    let mut counts = Vec::new();
    for tech in 1..=5 {
        let (mut sim, _) = started(&c, "GC_Full_Web_Way_Rebel", "Rebel");
        let g = grow(&mut sim, tech);
        counts.push(count(&g.end));
        if tech == 5 {
            let cato = sim.world().planet_id("Cato_Neimoidia_WW").expect("Cato Neimoidia");
            let (a, b) = (value(&sim, &g.start[&cato]), value(&sim, &g.end[&cato]));
            let capitals = |m: &BTreeMap<UnitTypeId, u32>| m.iter().filter(|(u, _)| class(&sim, **u) == 3).map(|(_, n)| *n).sum::<u32>();
            println!("Cato Neimoidia: {a} -> {b} credits at tech 5, capitals {} -> {}", capitals(&g.start[&cato]), capitals(&g.end[&cato]));
            // A campaign stronghold grows by the same credits as every planet, not by more of its own capitals.
            assert!(b >= a + 24000.0 && b < a + 24000.0 + 7500.0, "Cato Neimoidia {a} -> {b}");
            assert_eq!(capitals(&g.end[&cato]), capitals(&g.start[&cato]));
        }
    }
    println!("Pirate units galaxy-wide by tech level 1..5 after {GROW_WEEKS} weeks: {counts:?}");
    // Pinned (measured with this rule; the old 2 x tech garrison gave 705 / 724 / 782 / 909 / 1089 in the same set-up).
    assert_eq!(counts, [717, 1464, 2057, 2535, 3267]);
}

#[test]
fn growth_continues_from_a_loaded_game_without_a_jump() {
    let Some(c) = ctx() else { return };
    let (mut sim, _) = started(&c, "GC_Full_Web_Way_Rebel", "Rebel");
    let human = sim.world().victory.active_player.unwrap();
    sim.galaxy_mut().factions[human as usize].tech = 4;
    let week = sim.world().config.week_seconds;
    for _ in 0..3 {
        sim.step(week);
    }
    let mut snap = sim.snapshot();
    // A save from before the rule carries no start values: the planets keep what they have and grow from there by the weekly step.
    for old in [false, true] {
        if old {
            snap.passive_start.clear();
        }
        let mut loaded = restored(&c, "GC_Full_Web_Way_Rebel", snap.clone());
        let at_load = pirate_units(&loaded);
        assert_eq!(at_load, pirate_units(&sim), "nothing changes at load");
        loaded.step(week);
        let after = pirate_units(&loaded);
        for (p, us) in &after {
            let before: u32 = at_load.get(p).map_or(0, |m| m.values().sum());
            assert!(us.values().sum::<u32>() <= before + galaxy_sim::PASSIVE_GROWTH_UNITS_PER_WEEK, "old save {old}: {} jumped", loaded.world().planets[*p as usize].name);
        }
        let grown = count(&after) - count(&at_load);
        println!("old save {old}: {grown} Pirate units added in the first week after the load");
        assert!(grown > 0, "the growth goes on");
    }
}

/// The costliest fleet each major AI held when it reached the tech level (Ground Up all-AI runs of `GC_Full_Web_Way_Rebel`, seeds 1 to 3,
/// `pirate_census` example): the typical main fleet of that stage.
const MAIN_FLEETS: [(&str, &str); 5] = [
    ("Rebel", "CR-90_Corvette=2,Marauder_Missile_Cruiser=2,Quasar_Carrier_R=1,Assault_Frigate_Mark_II=1,Z-95_Squadron_R=3,T-Wing_Squadron=2,Hullcracker_Squadron_R=1,Cutlass_Squadron=2"),
    ("Empire", "Mara_Jade_Z-95_Headhunter=1,Screed_Demolisher=1,Devastator_Star_Destroyer_MP=1,Executrix_Star_Destroyer_MP=1,Gladiator_Star_Destroyer=2,Ton_Falk_Carrier=1,Gozanti_Squadron_E=5,Arc-170_Squadron_E=3,Skipray_Squadron_E=1,Tartan_Patrol_Cruiser=3"),
    ("Empire", "Mara_Jade_Z-95_Headhunter=1,Screed_Demolisher=1,Devastator_Star_Destroyer_MP=1,Executrix_Star_Destroyer_MP=1,Vigil_Corvette=1,Gladiator_Star_Destroyer=2,Vindicator_Cruiser=1,Ton_Falk_Carrier=1,Gozanti_Squadron_E=5,Arc-170_Squadron_E=5,Skipray_Squadron_E=2,Tartan_Patrol_Cruiser=3"),
    ("Empire", "Arquitens_E=1,Screed_Demolisher=1,Devastator_Star_Destroyer_MP=1,Executrix_Star_Destroyer_MP=1,Vigil_Corvette=3,Gladiator_Star_Destroyer=2,Vindicator_Cruiser=1,Ton_Falk_Carrier=2,Gozanti_Squadron_E=6,Arc-170_Squadron_E=3,Skipray_Squadron_E=5,Tartan_Patrol_Cruiser=8"),
    ("Empire", "Screed_Demolisher=1,Devastator_Star_Destroyer_MP=1,Executrix_Star_Destroyer_MP=1,Vigil_Corvette=1,Gladiator_Star_Destroyer=3,Vindicator_Cruiser=1,Ton_Falk_Carrier=1"),
];

/// Result of a main fleet's attack on a Pirate planet.
struct Attack {
    planet: String,
    pirate_value: f32,
    /// Auto-resolve wins of 10 seeds and the mean share of the attacker's units lost.
    auto_wins: u32,
    auto_lost: f32,
    /// Tactical battle (one seed): attacker won, share of the attacker's credits lost.
    tactical: Option<(bool, f32)>,
}

/// The main fleet of `tech` attacks the Pirate planet at quantile `q` by fleet value (not Lehon, with a map) after the growth.
fn attack(c: &Ctx, tech: i32, q: f32, tactical: bool) -> Attack {
    let (mut sim, _) = started(c, "GC_Full_Web_Way_Rebel", "Rebel");
    grow(&mut sim, tech);
    let w = sim.world().clone();
    let pirates = w.passive_faction().unwrap();
    let (fname, list) = MAIN_FLEETS[(tech - 1) as usize];
    let attacker = w.faction_id(fname).unwrap();
    sim.galaxy_mut().factions[attacker as usize].tech = tech;
    let mut planets: Vec<(f32, PlanetId)> = pirate_units(&sim)
        .iter()
        .filter(|(p, _)| w.planets[**p as usize].space_map.is_some() && !w.planets[**p as usize].name.to_ascii_lowercase().contains("lehon") && sim.galaxy().planets[**p as usize].owner == Some(pirates))
        .map(|(p, us)| (value(&sim, us), *p))
        .collect();
    planets.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let (pirate_value, planet) = planets[((planets.len() - 1) as f32 * q) as usize];
    let from = w.routes.iter().find_map(|r| (r.a == planet).then_some(r.b).or((r.b == planet).then_some(r.a))).unwrap_or(planet);
    let units: BTreeMap<UnitTypeId, u32> = list.split(',').map(|e| e.split_once('=').unwrap()).map(|(n, k)| (w.unit_id(n).unwrap_or_else(|| panic!("{n}")), k.parse().unwrap())).collect();
    let g = sim.galaxy_mut();
    let id = g.fleets.keys().max().copied().unwrap_or(0) + 1;
    g.fleets.insert(id, Fleet { id, owner: attacker, units, location: FleetLocation::AtPlanet(planet), previous_planet: from, slot: 0 });
    let battle = sim.step(1.0).into_iter().find_map(|e| if let Event::BattlePending(b) = e { Some(b) } else { None }).expect("battle");
    let (mut auto_wins, mut lost) = (0, 0.0);
    for seed in 1..=10 {
        let p = sim.preview_auto_resolve(battle.id, seed).unwrap();
        auto_wins += u32::from(p.winner == Some(attacker));
        let a = p.sides.iter().find(|x| x.0 == attacker).unwrap();
        lost += 1.0 - a.2 as f32 / a.1.max(1) as f32;
    }
    let tactical = tactical.then(|| {
        let req = sim.tactical_setup(battle.id).unwrap();
        let names: Vec<&str> = req.sides.iter().flat_map(|x| x.groups.iter().chain(x.reserve.iter()).map(|g| g.name.as_str())).collect();
        let catalog = Arc::new(battle_sim::Catalog::build(&c.data, Some(&*c.fs), &names));
        let arena = req.map.as_ref().and_then(|m| battle_sim::Arena::load(&c.data, &*c.fs, m)).map(Arc::new);
        let r = galaxy_sim::run_headless(&req, &catalog, arena).unwrap();
        let ai = req.sides.iter().position(|x| x.faction == attacker).unwrap();
        let (mut c0, mut c1) = (0.0, 0.0);
        for (gi, g) in req.sides[ai].groups.iter().enumerate().filter(|(_, g)| g.kind == galaxy_sim::GroupKind::Unit) {
            let cost = w.units[g.type_id as usize].cost;
            c0 += cost * g.count as f32;
            c1 += cost * r.survived[ai][gi] as f32;
        }
        (r.winner == Some(ai), 1.0 - c1 / c0)
    });
    Attack { planet: w.planets[planet as usize].name.clone(), pirate_value, auto_wins, auto_lost: lost / 10.0, tactical }
}

#[test]
fn a_main_fleet_takes_a_typical_pirate_planet_with_noticeable_losses_at_every_tech_level() {
    let Some(c) = ctx() else { return };
    for tech in 2..=5 {
        for q in [0.5, 0.9] {
            let a = attack(&c, tech, q, q == 0.5);
            println!(
                "tech {tech}, planet at quantile {q} ({}, Pirate fleet {:.0} credits): auto-resolve {}/10 wins, {:.0}% units lost; tactical (won, credits lost) {:?}",
                a.planet,
                a.pirate_value,
                a.auto_wins,
                a.auto_lost * 100.0,
                a.tactical
            );
            assert_eq!(a.auto_wins, 10, "tech {tech}: the main fleet wins at the planet at quantile {q} in auto-resolve");
            // Measured with this rule (old rule: 40 / 44 / 16 / 11% at the median, 68 / 71 / 54 / 16% at quantile 0.9); the band leaves room for
            // battle tuning elsewhere, not for a different outcome.
            let auto = [[0.67, 0.71], [0.66, 0.70], [0.36, 0.57], [0.13, 0.62]][(tech - 2) as usize][usize::from(q > 0.5)];
            assert!((a.auto_lost - auto).abs() <= 0.1, "tech {tech}, quantile {q}: auto-resolve losses {:.2}, measured {auto}", a.auto_lost);
            if let Some((won, lost)) = a.tactical {
                // Measured: 11 / 8 / 9 / 1% of the attacker's credits (old rule: 3 / 4 / 3 / 1%). The tech 5 fleet is mostly heroes.
                assert!(won, "tech {tech}: the main fleet wins the tactical battle at the median planet");
                assert!(lost <= 0.25, "tech {tech}: moderate losses at the median planet, lost {:.0}%", lost * 100.0);
                if tech < 5 {
                    assert!(lost >= 0.05, "tech {tech}: noticeable losses at the median planet, lost {:.0}%", lost * 100.0);
                }
            }
        }
    }
}

/// Pirate planets the major and the minor factions take in 60 all-AI weeks of Ground Up `GC_Full_Web_Way_Rebel` (seed `seed`), and the Pirate units
/// left. No battle may come back with the same line-up (`BattleRepeats`).
fn all_ai_run(c: &Ctx, seed: u64) -> (u32, u32, u32) {
    let campaign = c.data.campaign("GC_Full_Web_Way_Rebel").unwrap().clone();
    let mut sim = ScriptedSim::new(
        c.fs.clone(),
        c.data.clone(),
        &campaign,
        SimConfig { victory_enabled: false, ground_up: true, seed, ..SimConfig::default() },
        ScriptedConfig { remake_ai: true, human: Some("none".into()), game_scoring: false, strategy: true, difficulty: Some(galaxy_script::DifficultyTier::Normal), ..ScriptedConfig::default() },
    )
    .unwrap();
    let (pirates, majors) = sim.with_sim(|s| (s.world().passive_faction().unwrap(), s.world().main_factions.clone()));
    let mut watch = galaxy_sim::BattleRepeats::for_world(&sim.with_sim(|s| s.world().clone()));
    let (mut weeks, mut by_majors, mut by_others) = (0, 0, 0);
    while weeks < 60 {
        let events = sim.step(1.0).sim_events;
        let repeated = watch.check(sim.with_sim(|s| s.galaxy().time), &events);
        assert!(repeated.is_empty(), "battles created again with the same line-up: {repeated:?}");
        for e in events {
            match e {
                Event::WeekEnded { week } => weeks = week,
                Event::PlanetCaptured { new_owner, old_owner: Some(o), .. } if o == pirates => {
                    if majors.contains(&new_owner) {
                        by_majors += 1;
                    } else {
                        by_others += 1;
                    }
                }
                _ => {}
            }
        }
    }
    let units: u32 = sim.with_sim(|s| s.galaxy().fleets.values().filter(|f| f.owner == pirates).map(|f| f.unit_count()).sum());
    (by_majors, by_others, units)
}

#[test]
fn in_sixty_all_ai_weeks_the_major_factions_still_take_pirate_planets() {
    let Some(c) = ctx() else { return };
    let mut total = 0;
    for seed in 1..=3 {
        let (majors, minors, units) = all_ai_run(&c, seed);
        println!("seed {seed}: 60 all-AI weeks, Pirate planets taken by the major factions {majors}, by the minors {minors}; {units} Pirate units left");
        assert!(majors >= 4, "seed {seed}: the major factions still take Pirate planets: {majors}");
        total += majors;
    }
    // Measured before the rule: 12 / 13 / 13 (38); with it: 10 / 6 / 7 (23). At least half of the old rate.
    assert!(total >= 19, "the major factions took {total} Pirate planets over 3 seeds");
}
