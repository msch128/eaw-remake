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
