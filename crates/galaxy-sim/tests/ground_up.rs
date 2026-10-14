//! "From the Ground Up" on the largest Remake map (real data, skipped without the game).

use std::collections::BTreeMap;

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn load() -> Option<GameData> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    Some(load_galaxy(&GameFs::for_mod(&install, Some("1770851727")).ok()?).0)
}

fn sim(data: &GameData, ground_up: bool) -> Sim {
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    Sim::new(data, campaign, SimConfig { victory_enabled: false, ground_up, ..SimConfig::default() }).expect("sim")
}

fn counts(sim: &Sim) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for p in &sim.galaxy().planets {
        let name = p.owner.map_or("none".to_owned(), |f| sim.world().factions[f as usize].name.clone());
        *m.entry(name).or_default() += 1;
    }
    m
}

#[test]
fn owner_counts_follow_the_ground_up_rules() {
    let Some(data) = load() else { return };
    let normal = counts(&sim(&data, false));
    let s = sim(&data, true);
    let c = counts(&s);
    println!("normal {normal:?}\nground up {c:?}");
    assert_eq!(c["Rebel"], 1, "the player starts with one planet");
    assert_eq!(c["Empire"], 3);
    assert_eq!(c["Underworld"], 3);
    for minor in ["Mandalorians", "Hapan", "CIS", "CSA", "Republic", "Warlords", "Cartels", "Sith"] {
        assert_eq!(c.get(minor), normal.get(minor), "{minor} keeps its planets");
    }
    assert!(!c.contains_key("none"), "no planet is left unowned");
    let total: usize = normal.values().sum();
    assert_eq!(c.values().sum::<usize>(), total);
    assert_eq!(c["Pirates"], total - c.iter().filter(|(k, _)| k.as_str() != "Pirates").map(|(_, n)| n).sum::<usize>());
    // Start planets: star base level 1, and the Rebels keep their home.
    let w = s.world();
    let rebel = w.faction_id("Rebel").unwrap();
    for p in s.galaxy().owned_planets(rebel) {
        assert_eq!(w.starbase_level(s.galaxy(), p), 1);
    }
    // Deterministic.
    assert_eq!(s.galaxy().state_hash(), sim(&data, true).galaxy().state_hash());
}

#[test]
fn every_passive_planet_has_a_star_base_so_a_fleet_arrival_starts_a_battle() {
    let Some(data) = load() else { return };
    let mut s = sim(&data, true);
    let w = s.world().clone();
    let (rebel, pirates) = (w.faction_id("Rebel").unwrap(), w.faction_id("Pirates").unwrap());
    let bare: Vec<_> = s.galaxy().owned_planets(pirates).filter(|&p| s.galaxy().planets[p as usize].starbase.is_none()).collect();
    assert!(bare.is_empty(), "passive planets without a star base: {}", bare.len());
    // A Rebel fleet sent to the nearest passive planet meets resistance instead of taking it by presence.
    let (fleet, from) = s.galaxy().fleets.iter().find(|(_, f)| f.owner == rebel && f.planet().is_some()).map(|(&id, f)| (id, f.planet().unwrap())).expect("a rebel fleet");
    let at = w.planets[from as usize].position;
    let dist = |p: PlanetId| (0..3).map(|i| (w.planets[p as usize].position[i] - at[i]).powi(2)).sum::<f32>();
    let target = s.galaxy().owned_planets(pirates).min_by(|&a, &b| dist(a).total_cmp(&dist(b))).expect("a passive planet");
    s.apply(Command::MoveFleet { faction: rebel, fleet, to: target }).expect("move");
    for _ in 0..600 {
        s.step(1.0);
        if s.galaxy().planets[target as usize].owner != Some(pirates) {
            break;
        }
    }
    assert_eq!(s.galaxy().planets[target as usize].owner, Some(pirates), "the passive planet is not taken without a battle");
}

#[test]
fn the_passive_faction_never_acts_but_the_other_ais_expand() {
    let Some(data) = load() else { return };
    let mut s = sim(&data, true);
    let w = s.world().clone();
    let (rebel, empire, pirates) = (w.faction_id("Rebel").unwrap(), w.faction_id("Empire").unwrap(), w.faction_id("Pirates").unwrap());
    assert_eq!(w.passive_faction(), Some(pirates));
    s.set_ai_factions((0..w.factions.len() as u32).filter(|&f| f != rebel));
    let pirate_fleets: Vec<_> = s.galaxy().fleets.iter().filter(|(_, f)| f.owner == pirates).map(|(&id, f)| (id, f.location.clone())).collect();
    let empire_before = s.galaxy().factions[empire as usize].stats.credits_spent;
    let pirates_before = s.galaxy().factions[pirates as usize].stats.credits_spent;
    for _ in 0..900 {
        s.step(1.0);
    }
    let g = s.galaxy();
    assert!(g.factions[empire as usize].stats.credits_spent > empire_before, "the Empire AI builds");
    assert_eq!(g.factions[pirates as usize].stats.credits_spent, pirates_before, "the passive faction spends nothing");
    assert!(g.planets.iter().all(|p| p.owner != Some(pirates) || p.queues.iter().all(|q| q.is_empty())), "no passive production queue");
    for (id, loc) in pirate_fleets {
        // Another faction may destroy a passive fleet, but it never moves.
        if let Some(f) = g.fleets.get(&id) {
            assert_eq!(f.location, loc, "passive fleets stay put");
        }
    }
    // Its commands are refused.
    let planet = g.owned_planets(pirates).next().unwrap();
    let unit = w.buildable_units[pirates as usize].first().copied().unwrap_or(0);
    assert_eq!(s.apply(Command::BuildUnit { faction: pirates, planet, unit }), Err(CommandError::NotAvailable));
}

/// The pirates' star base level (1-based, 0 = none) and units on a planet.
fn pirate_state(sim: &Sim, p: usize) -> (usize, u32) {
    let (w, g) = (sim.world(), sim.galaxy());
    let pirates = w.faction_id("Pirates").unwrap();
    let level = g.planets[p].starbase.map_or(0, |s| w.starbases[s as usize].level.max(0) as usize);
    let units = g.fleets.values().filter(|f| f.owner == pirates && f.location == FleetLocation::AtPlanet(p as u32)).map(|f| f.unit_count()).sum();
    (level, units)
}

#[test]
fn the_pirates_rearm_with_the_players_tech_in_every_game_and_never_expand() {
    let Some(data) = load() else { return };
    for ground_up in [false, true] {
        let mut s = sim(&data, ground_up);
        let w = s.world().clone();
        let pirates = w.faction_id("Pirates").unwrap();
        let human = w.faction_id("Rebel").unwrap();
        assert_eq!(w.passive_faction(), Some(pirates), "the pirates are passive in every game");
        let planets: Vec<usize> = (0..w.planets.len()).filter(|&p| w.real[p] && s.galaxy().planets[p].owner == Some(pirates) && !s.galaxy().planets[p].destroyed).collect();
        assert!(!planets.is_empty());
        // Orders of the passive faction are refused.
        assert!(s.apply(Command::UpgradeTech { faction: pirates, planet: planets[0] as u32 }).is_err());
        let tech0 = s.galaxy().factions[human as usize].tech;
        let before: Vec<(usize, u32)> = planets.iter().map(|&p| pirate_state(&s, p)).collect();
        assert!(before.iter().all(|b| b.0 >= 1), "a base on every pirate planet from the start");
        // The player reaches tech 4: after two weeks the bases follow and the garrisons grow by one per week up to 2 x tech.
        s.galaxy_mut().factions[human as usize].tech = 4;
        for _ in 0..2 {
            s.step(w.config.week_seconds);
        }
        let held: Vec<usize> = planets.iter().copied().filter(|&p| s.galaxy().planets[p].owner == Some(pirates)).collect();
        assert_eq!(held.len(), planets.len(), "without enemies the pirates neither lose nor gain planets (ground_up {ground_up})");
        // The pirate chain's level 3 is the Starforge, which the data restricts to Lehon (`Required_Planets` of Starforge.xml); every other pirate planet tops out at level 2.
        let want = 2;
        let mut raised = 0;
        let mut grew = 0;
        for &p in &held {
            let (lvl, units) = pirate_state(&s, p);
            let idx = planets.iter().position(|&q| q == p).unwrap();
            assert!(lvl >= before[idx].0, "a base is never lowered");
            let cap = (w.planets[p].max_space_base as usize).clamp(1, 2);
            assert!(lvl >= cap, "base reaches planet cap for {}: lvl {lvl} vs cap {cap}", w.planets[p].name);
            if w.planets[p].name.to_ascii_lowercase().contains("lehon") {
                assert!(lvl >= 3.min(w.planets[p].max_space_base as usize), "the Starforge planet follows tech 4 up to level 3");
            }
            raised += usize::from(lvl >= cap.min(want));
            grew += usize::from(units > before[idx].1);
        }
        println!("ground_up {ground_up}: tech {tech0} -> 4, {} pirate planets, {raised} at base level >= {want}, {grew} with a bigger garrison", held.len());
        assert!(raised * 10 >= held.len() * 9, "bases follow the player's tech: {raised} of {} at their planet's target level (at most {want})", held.len());
        assert!(grew > 0, "garrisons grow");
        for &p in &held {
            let is_lehon = w.planets[p].name.to_ascii_lowercase().contains("lehon");
            if !is_lehon && let Some(sb) = s.galaxy().planets[p].starbase {
                let name = &w.starbases[sb as usize].name;
                assert!(!name.eq_ignore_ascii_case("Pirate_Starbase_Lv3") && !name.to_ascii_lowercase().contains("starforge"),
                    "Starforge must never appear on non-Lehon planet {}", w.planets[p].name);
            }
        }
    }
}

#[test]
fn passive_defense_scales_to_attacker_tech() {
    let Some(data) = load() else { return };
    let mut s = sim(&data, true);
    let w = s.world().clone();
    let pirates = w.passive_faction().expect("pirates");
    let human = w.victory.active_player.expect("human");
    let empire = w.faction_id("Empire").expect("empire");

    // Human player reaches tech 5, so global pirate rearm builds bases and recruits garrisons.
    s.galaxy_mut().factions[human as usize].tech = 5;
    for _ in 0..4 {
        s.step(w.config.week_seconds);
    }

    let planet = (0..w.planets.len() as u32)
        .find(|&p| w.real[p as usize] && s.galaxy().planets[p as usize].owner == Some(pirates))
        .expect("pirate planet");

    // Empire attacker is at Tech 1.
    s.galaxy_mut().factions[empire as usize].tech = 1;
    let battle_low_tech = galaxy_sim::PendingBattle {
        id: 100,
        planet,
        fleets: vec![],
        factions: vec![empire, pirates],
        defender: Some(pirates),
    };
    let sides_low = s.battle_sides(&battle_low_tech);
    let pirate_side_low = sides_low.iter().find(|s| s.faction == pirates).unwrap();

    // Human attacker is at Tech 5.
    let battle_high_tech = galaxy_sim::PendingBattle {
        id: 101,
        planet,
        fleets: vec![],
        factions: vec![human, pirates],
        defender: Some(pirates),
    };
    let sides_high = s.battle_sides(&battle_high_tech);
    let pirate_side_high = sides_high.iter().find(|s| s.faction == pirates).unwrap();

    let low_units: u32 = pirate_side_low.members.iter().map(|m| m.count).sum();
    let high_units: u32 = pirate_side_high.members.iter().map(|m| m.count).sum();
    assert!(low_units < high_units, "Low tech attacker faces smaller pirate forces: {low_units} vs {high_units}");
}

#[test]
fn pirates_never_raid_or_expand_over_many_weeks() {
    let Some(data) = load() else { return };
    let mut s = sim(&data, true);
    let w = s.world().clone();
    let pirates = w.faction_id("Pirates").unwrap();
    let worlds_before = s.galaxy().owned_planets(pirates).count();
    for _ in 0..12 {
        for ev in s.step(w.config.week_seconds) {
            assert!(!format!("{ev:?}").contains("Raid"), "pirates never launch raids: {ev:?}");
        }
    }
    let g = s.galaxy();
    assert!(g.owned_planets(pirates).count() <= worlds_before, "pirates never expand");
    for (id, f) in g.fleets.iter().filter(|(_, f)| f.owner == pirates) {
        assert!(matches!(f.location, FleetLocation::AtPlanet(_)), "pirate fleet {id} never moves");
    }
}

#[test]
fn ground_up_supports_faction_override_for_republic() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let sim = Sim::new(
        &data,
        campaign,
        SimConfig {
            victory_enabled: false,
            ground_up: true,
            faction_override: Some("Republic".into()),
            ..SimConfig::default()
        },
    )
    .expect("sim with Republic override");
    let rep = sim.world().faction_id("Republic").expect("Republic faction");
    assert_eq!(sim.world().victory.active_player, Some(rep), "Republic is the active human player");
    assert_eq!(sim.galaxy().owned_planets(rep).count(), 1, "Republic starts with 1 planet in From the Ground Up");
}

// ---------------------------------------------------------------------------------------------------------------------
// Main factions from the campaign data, start conditions per faction, build rules, long AI runs, base and Clone Wars stacks.
// ---------------------------------------------------------------------------------------------------------------------

/// The base Remake (`base`) or the Clone Wars stack (`fft`: 3229239424 + 2794270450 + 1770851727), skipped when a folder is missing.
fn load_stack(stack: &str) -> Option<GameData> {
    let install = GameInstall::detect()?;
    let needed: &[&str] = if stack == "fft" { &["3229239424", "2794270450", "1770851727"] } else { &["1770851727"] };
    if let Some(missing) = needed.iter().find(|id| !install.mod_data(id).is_dir()) {
        eprintln!("skipping: workshop mod {missing} not found");
        return None;
    }
    let alias = if stack == "fft" { "fft" } else { "1770851727" };
    Some(load_galaxy(&GameFs::for_mod(&install, Some(alias)).ok()?).0)
}

fn ground_up_sim(data: &GameData, campaign: &str, ground_up: bool) -> Sim {
    let c = data.campaign(campaign).unwrap_or_else(|| panic!("campaign {campaign}"));
    Sim::new(data, c, SimConfig { victory_enabled: false, ground_up, ..SimConfig::default() }).expect("sim")
}

fn names(w: &World, list: &[FactionId]) -> Vec<String> {
    let mut v: Vec<String> = list.iter().map(|&f| w.factions[f as usize].name.clone()).collect();
    v.sort();
    v
}

/// Planets owned by a faction.
fn owned(sim: &Sim, f: FactionId) -> Vec<PlanetId> {
    sim.galaxy().owned_planets(f).collect()
}

#[test]
fn main_factions_come_from_the_campaign_data() {
    if let Some(data) = load_stack("base") {
        for (camp, human) in [("GC_Full_Web_Way_Rebel", "Rebel"), ("GC_Full_Web_Way_Imperial", "Empire"), ("GC_Full_Web_Way_Underworld", "Underworld")] {
            let s = ground_up_sim(&data, camp, true);
            let w = s.world();
            assert_eq!(names(w, &w.main_factions), ["Empire", "Rebel", "Underworld"], "{camp}");
            assert_eq!(w.victory.active_player, w.faction_id(human), "{camp}");
            assert!(!w.main_factions.contains(&w.passive_faction().unwrap()), "the passive faction is no main faction");
        }
    }
    if let Some(data) = load_stack("fft") {
        // In the Clone Wars campaigns the played side is a great power; the other of Republic / CIS is a minor (`GC_MinorAi`) and keeps its planets.
        for (camp, human, minor) in [("GC_Full_Web_Way_Republic", "Republic", "CIS"), ("GC_Full_Web_Way_CIS", "CIS", "Republic")] {
            let s = ground_up_sim(&data, camp, true);
            let w = s.world();
            let main = names(w, &w.main_factions);
            println!("{camp}: main {main:?}");
            assert!(main.iter().any(|n| n == human), "{camp}: {human} is a main faction");
            assert!(!main.iter().any(|n| n == minor), "{camp}: {minor} stays a minor");
            assert!(main.len() >= 3, "{camp}: {main:?}");
        }
    }
}

#[test]
fn start_conditions_per_main_faction() {
    for (stack, campaigns) in [
        ("base", vec!["GC_Full_Web_Way_Rebel", "GC_Full_Web_Way_Imperial", "GC_Full_Web_Way_Underworld"]),
        ("fft", vec!["GC_Full_Web_Way_Republic", "GC_Full_Web_Way_CIS"]),
    ] {
        let Some(data) = load_stack(stack) else { continue };
        for camp in campaigns {
            let normal = ground_up_sim(&data, camp, false);
            let s = ground_up_sim(&data, camp, true);
            let (w, g) = (s.world().clone(), s.galaxy());
            let human = w.victory.active_player.expect("human");
            let pirates = w.passive_faction().expect("pirates");
            for &f in &w.main_factions {
                let name = &w.factions[f as usize].name;
                let planets = owned(&s, f);
                let want = if f == human { 1 } else { 3 };
                assert_eq!(planets.len(), want.min(owned(&normal, f).len()), "{camp}: {name} planets");
                // Credits and tech are the campaign's (`Starting_Credits`, `Starting_Tech_Level`): Ground Up reduces planets, not money.
                assert_eq!(g.factions[f as usize].credits, normal.galaxy().factions[f as usize].credits, "{camp}: {name} start credits");
                assert_eq!(g.factions[f as usize].tech, normal.galaxy().factions[f as usize].tech, "{camp}: {name} start tech");
                assert!(g.factions[f as usize].credits > 0.0, "{camp}: {name} has start credits");
                // The home planet is kept, with a level 1 star base.
                if let Some(home) = w.factions[f as usize].home.filter(|h| owned(&normal, f).contains(h)) {
                    assert!(planets.contains(&home), "{camp}: {name} keeps its home {}", w.planets[home as usize].name);
                }
                for &p in &planets {
                    assert_eq!(w.starbase_level(g, p), 1, "{camp}: {name} star base level on {}", w.planets[p as usize].name);
                }
                // Fleets stay only on kept planets.
                let ships: u32 = g.fleets.values().filter(|fl| fl.owner == f).map(|fl| fl.unit_count()).sum();
                assert!(g.fleets.values().filter(|fl| fl.owner == f).all(|fl| fl.planet().is_some_and(|p| planets.contains(&p))), "{camp}: {name} fleets only at kept planets");
                println!("{camp}: {name}: {} planets, {ships} ships, {:.0} credits, tech {}", planets.len(), g.factions[f as usize].credits, g.factions[f as usize].tech);
            }
            // Minors keep their planets.
            for p in 0..w.planets.len() {
                let (a, b) = (normal.galaxy().planets[p].owner, g.planets[p].owner);
                if let Some(o) = a.filter(|o| !w.main_factions.contains(o) && *o != pirates) {
                    assert_eq!(b, Some(o), "{camp}: minor planet {} keeps its owner", w.planets[p].name);
                }
            }
            // The player can act at once: something is queueable at the start planet with the start credits.
            let home = owned(&s, human)[0];
            let ship = w.buildable_units[human as usize].iter().any(|&u| w.check_build_unit(g, human, home, u).is_ok());
            let building = w.buildable_structures[human as usize].iter().any(|&st| w.check_build_structure(g, human, home, st).is_ok());
            let upgrade = w.check_upgrade_starbase(g, human, home).is_ok() || w.check_upgrade_tech(g, human, home).is_ok();
            assert!(ship || building || upgrade, "{camp}: nothing to do at the start planet {}", w.planets[home as usize].name);
        }
    }
}

#[test]
fn what_the_player_can_build_follows_the_data_rules_on_one_planet() {
    let Some(data) = load_stack("base") else { return };
    let s = ground_up_sim(&data, "GC_Full_Web_Way_Rebel", true);
    let (w, g) = (s.world().clone(), s.galaxy());
    let human = w.victory.active_player.unwrap();
    let home = owned(&s, human)[0];
    let tech = g.factions[human as usize].tech;
    let mut offered = 0;
    for &u in &w.buildable_units[human as usize] {
        let ut = &w.units[u as usize];
        let ok = w.check_build_unit(g, human, home, u).is_ok();
        if ok {
            offered += 1;
            assert!(ut.tech <= tech, "{} needs tech {} (have {tech})", ut.name, ut.tech);
            assert!(ut.req_starbase <= w.starbase_level(g, home), "{} needs star base {}", ut.name, ut.req_starbase);
            assert!(ut.req_planets.is_empty() || ut.req_planets.contains(&home), "{} is tied to other planets", ut.name);
            assert!(!ut.locked || g.factions[human as usize].unlocked.contains(&ut.name.to_ascii_lowercase()), "{} is locked", ut.name);
        }
        // Objects of tech >= 90 (story-only) are never offered, and nothing above the start tech is.
        if ut.tech >= 90 || ut.tech > tech {
            assert!(!ok, "{} (tech {}) must not be offered", ut.name, ut.tech);
        }
    }
    println!("{offered} ships queueable at the start planet {} (tech {tech}, base level {})", w.planets[home as usize].name, w.starbase_level(g, home));
}

/// Start of a main faction in an all-AI run.
struct Start {
    faction: FactionId,
    planets: usize,
    ships_built: u32,
    /// `income_per_week` at the start.
    income: f64,
}

/// A sim with every main faction under the built-in AI (the passive faction and the minors are not run) and the factions' start values.
fn start_ai_run(data: &GameData, campaign: &str) -> (Sim, Vec<Start>) {
    let mut s = ground_up_sim(data, campaign, true);
    let w = s.world().clone();
    s.set_ai_factions(w.main_factions.iter().copied());
    let g = s.galaxy();
    let before = w
        .main_factions
        .iter()
        .map(|&f| Start { faction: f, planets: owned(&s, f).len(), ships_built: g.factions[f as usize].stats.ships_built, income: g.factions[f as usize].income_per_week })
        .collect();
    (s, before)
}

/// Advances the sim by `weeks` more weeks, resolving every battle at once.
fn advance_weeks(s: &mut Sim, weeks: u32) {
    let end = weeks as f32 * s.world().config.week_seconds;
    let tick = s.world().config.tick_seconds.max(1.0);
    let mut t = 0.0;
    while t < end {
        s.step(tick);
        t += tick;
        for b in s.pending_battles().iter().map(|b| b.id).collect::<Vec<_>>() {
            let _ = s.resolve_battle(b);
        }
    }
}

/// Income per week below which a start does not carry an expansion (estimate for "too poor to outgrow the pirates' tech-scaled defence within 90 weeks":
/// the Underworld's three planets earn 185, every other start earns 1200 or more; a home with negative income cannot build at all).
const POOR_INCOME: f64 = 300.0;

fn assert_all_ai_build_and_expand(stack: &str, campaign: &str) {
    let Some(data) = load_stack(stack) else { return };
    let (mut s, before) = start_ai_run(&data, campaign);
    let report = |s: &Sim, label: &str| {
        let (w, g) = (s.world(), s.galaxy());
        for b in &before {
            let st = &g.factions[b.faction as usize].stats;
            println!(
                "{campaign} {label}: {} (start income {:.0}/week): planets {} -> {}, ships built {}, structures {}, tech {}, captured {}, lost {}",
                w.factions[b.faction as usize].name, b.income, b.planets, g.owned_planets(b.faction).count(), st.ships_built, st.structures_built, g.factions[b.faction as usize].tech, st.planets_captured, st.planets_lost
            );
        }
    };
    // Every main faction that starts with income builds ships, structures or tech within 30 weeks.
    advance_weeks(&mut s, 30);
    report(&s, "week 30");
    for b in before.iter().filter(|b| b.income > 0.0) {
        let (w, g) = (s.world(), s.galaxy());
        let st = &g.factions[b.faction as usize].stats;
        let name = &w.factions[b.faction as usize].name;
        assert!(st.ships_built > b.ships_built, "{campaign}: {name} built no ships in 30 weeks");
        assert!(st.structures_built > 0 || g.factions[b.faction as usize].tech > 1, "{campaign}: {name} neither built structures nor raised tech");
    }
    // Every faction that is not poor has expanded by week 90 (the pirates' defence scales with the attackers' tech, so it takes a while).
    advance_weeks(&mut s, 60);
    report(&s, "week 90");
    let (w, g) = (s.world(), s.galaxy());
    for b in before.iter().filter(|b| b.income >= POOR_INCOME) {
        let st = &g.factions[b.faction as usize].stats;
        let now = g.owned_planets(b.faction).count();
        assert!(now > b.planets || st.planets_captured > 0, "{campaign}: {} did not expand in 90 weeks ({} -> {now})", w.factions[b.faction as usize].name, b.planets);
    }
    let pirates = w.passive_faction().unwrap();
    assert_eq!(g.factions[pirates as usize].stats.credits_spent, 0.0, "the passive faction spends nothing");
    assert!(g.planets.iter().all(|p| p.owner != Some(pirates) || p.queues.iter().all(|q| q.is_empty())), "no passive production");
}

#[test]
fn thirty_week_all_ai_run_every_main_faction_builds_and_expands() {
    for camp in ["GC_Full_Web_Way_Rebel", "GC_Full_Web_Way_Imperial", "GC_Full_Web_Way_Underworld"] {
        assert_all_ai_build_and_expand("base", camp);
    }
}

#[test]
fn thirty_week_all_ai_run_with_republic_and_cis_as_the_played_side() {
    for camp in ["GC_Full_Web_Way_Republic", "GC_Full_Web_Way_CIS"] {
        assert_all_ai_build_and_expand("fft", camp);
    }
}

#[test]
fn the_all_ai_run_is_deterministic() {
    let Some(data) = load_stack("base") else { return };
    let run = || {
        let (mut s, _) = start_ai_run(&data, "GC_Full_Web_Way_Rebel");
        advance_weeks(&mut s, 6);
        s
    };
    let (a, b) = (run(), run());
    assert_eq!(a.galaxy().state_hash(), b.galaxy().state_hash());
}

#[test]
fn passive_faction_choice_works_in_the_clone_wars_stack() {
    let Some(data) = load_stack("fft") else { return };
    for camp in ["GC_Full_Web_Way_Republic", "GC_Full_Web_Way_CIS", "GC_Full_Web_Way_Rebel"] {
        let mut s = ground_up_sim(&data, camp, true);
        let w = s.world().clone();
        let pirates = w.passive_faction().unwrap_or_else(|| panic!("{camp}: no Pirates faction"));
        assert_eq!(w.factions[pirates as usize].name, "Pirates");
        assert!(s.galaxy().owned_planets(pirates).count() > 0, "{camp}: the pirates hold the released planets");
        assert!(s.galaxy().planets.iter().all(|p| p.owner.is_some() || p.destroyed), "{camp}: no planet is unowned");
        assert!(!w.main_factions.contains(&pirates));
        let planet = s.galaxy().owned_planets(pirates).next().unwrap();
        let unit = w.buildable_units[pirates as usize].first().copied().unwrap_or(0);
        assert_eq!(s.apply(Command::BuildUnit { faction: pirates, planet, unit }), Err(CommandError::NotAvailable), "{camp}: passive faction refused");
    }
}

#[test]
fn the_sim_config_keeps_ground_up_in_saves() {
    // The save header stores the whole `SimConfig`; the flag must survive the round trip, and old headers without it load as a normal game.
    let cfg = SimConfig { ground_up: true, faction_override: Some("Republic".into()), ..SimConfig::default() };
    let back: SimConfig = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
    assert!(back.ground_up);
    assert_eq!(back.faction_override.as_deref(), Some("Republic"));
    let mut v = serde_json::to_value(SimConfig::default()).unwrap();
    v.as_object_mut().unwrap().remove("ground_up");
    assert!(!serde_json::from_value::<SimConfig>(v).unwrap().ground_up);
}
