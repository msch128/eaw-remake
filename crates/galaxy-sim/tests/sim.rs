//! Tests against the real Remake data; skipped when the game is not installed.

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn load() -> Option<GameData> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    Some(load_galaxy(&fs).0)
}

fn new_sim(data: &GameData, config: SimConfig) -> Sim {
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    Sim::new(data, campaign, config).expect("sim")
}

fn run_ai(data: &GameData, seed: u64, weeks: u32) -> u64 {
    let mut sim = new_sim(data, SimConfig { seed, victory_enabled: false, ..SimConfig::default() });
    sim.set_ai_factions(0..sim.world().factions.len() as u32);
    sim.step(weeks as f32 * 60.0);
    sim.galaxy().state_hash()
}

#[test]
fn initial_state_matches_campaign() {
    let Some(data) = load() else { return };
    let sim = new_sim(&data, SimConfig::default());
    let (w, g) = (sim.world(), sim.galaxy());
    let rebel = w.faction_id("Rebel").unwrap();
    assert_eq!(g.factions[rebel as usize].credits, 15000.0);
    assert_eq!(g.planets[w.planet_id("Coruscant_WW").unwrap() as usize].owner, w.faction_id("Empire"));
    assert!(g.planets[w.planet_id("Yavin_WW").unwrap() as usize].starbase.is_some());
    assert!(g.fleets.values().any(|f| f.owner == rebel));
    assert!(g.factions.iter().all(|f| f.income_per_week.is_finite()));
    assert!(w.routes.len() > 150);
}

#[test]
fn simulation_is_deterministic() {
    let Some(data) = load() else { return };
    let a = run_ai(&data, 7, 12);
    let b = run_ai(&data, 7, 12);
    let c = run_ai(&data, 8, 12);
    assert_eq!(a, b, "same seed must give identical state");
    assert_ne!(a, c, "different seeds should diverge");
}

#[test]
fn build_unit_validates_and_completes() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data, SimConfig::default());
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let rebel = w.faction_id("Rebel").unwrap();
    let home = w.planet_id("Coruscant_WW").unwrap();

    // Wrong owner, unknown ids.
    let some_unit = w.buildable_units[empire as usize][0];
    assert_eq!(sim.apply(Command::BuildUnit { faction: rebel, planet: home, unit: some_unit }), Err(CommandError::NotOwner));
    assert_eq!(sim.apply(Command::BuildUnit { faction: empire, planet: 99_999, unit: some_unit }), Err(CommandError::Invalid));

    // Find something Coruscant can build right now.
    let unit = w.buildable_units[empire as usize]
        .iter()
        .copied()
        .find(|&u| w.units[u as usize].cost > 0.0 && w.check_build_unit(sim.galaxy(), empire, home, u).is_ok())
        .expect("a buildable unit");
    let cost = w.units[unit as usize].cost as f64;
    let before = sim.galaxy().factions[empire as usize].credits;
    sim.apply(Command::BuildUnit { faction: empire, planet: home, unit }).unwrap();
    assert!((sim.galaxy().factions[empire as usize].credits - (before - cost)).abs() < 1e-6);

    let mut done = false;
    for _ in 0..40 {
        let events = sim.step(10.0);
        done |= events
            .iter()
            .any(|e| matches!(e, Event::ConstructionComplete { faction, kind: ProductionKind::Unit(u), .. } if *faction == empire && *u == unit));
        if done {
            break;
        }
    }
    assert!(done, "unit finished building");
    // Not enough credits.
    sim.galaxy_mut().factions[empire as usize].credits = 0.0;
    assert_eq!(sim.apply(Command::BuildUnit { faction: empire, planet: home, unit }), Err(CommandError::InsufficientCredits));
}

#[test]
fn fleet_moves_and_captures_neutral_planet() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data, SimConfig { victory_enabled: false, ..SimConfig::default() });
    let w = sim.world().clone();
    let g = sim.galaxy().clone();
    // A fleet next to an undefended neutral planet.
    let (fleet, target) = g
        .fleets
        .values()
        .find_map(|f| {
            let p = f.planet()?;
            w.adjacency[p as usize]
                .iter()
                .map(|&(n, _)| n)
                .find(|&n| {
                    let ps = &g.planets[n as usize];
                    w.real[n as usize] && ps.owner.is_none() && g.fleets_at(n).next().is_none()
                })
                .map(|n| (f.clone(), n))
        })
        .expect("a fleet next to a neutral planet");
    sim.apply(Command::MoveFleet { faction: fleet.owner, fleet: fleet.id, to: target }).unwrap();
    assert_eq!(sim.apply(Command::MoveFleet { faction: fleet.owner, fleet: fleet.id, to: target }), Err(CommandError::WrongState));
    let mut arrived = false;
    let mut captured = false;
    for _ in 0..200 {
        for e in sim.step(5.0) {
            match e {
                Event::FleetArrived { fleet: f, planet } if f == fleet.id && planet == target => arrived = true,
                Event::PlanetCaptured { planet, new_owner, .. } if planet == target && new_owner == fleet.owner => captured = true,
                _ => {}
            }
        }
        if captured {
            break;
        }
    }
    assert!(arrived && captured);
    assert_eq!(sim.galaxy().planets[target as usize].owner, Some(fleet.owner));
}

#[test]
fn hostile_fleets_trigger_battle_and_auto_resolve() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data, SimConfig { auto_resolve_battles: false, victory_enabled: false, ..SimConfig::default() });
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    // Put a strong and a weak fleet over an empty neutral planet.
    let target = (0..w.planets.len() as u32)
        .find(|&p| w.real[p as usize] && sim.galaxy().planets[p as usize].owner.is_none() && sim.galaxy().fleets_at(p).next().is_none())
        .expect("empty planet");
    let strong = w.unit_id("Star_Destroyer").unwrap();
    let weak = w.unit_id("X-Wing_Squadron").unwrap();
    for (owner, unit, n) in [(empire, strong, 3u32), (rebel, weak, 2u32)] {
        let g = sim.galaxy_mut();
        let id = g.fleets.keys().max().copied().unwrap_or(0) + 1;
        g.fleets.insert(id, Fleet { id, owner, units: [(unit, n)].into(), location: FleetLocation::AtPlanet(target), previous_planet: target });
    }
    let events = sim.step(1.0);
    let battle = events
        .iter()
        .find_map(|e| if let Event::BattlePending(b) = e { Some(b.clone()) } else { None })
        .expect("battle pending");
    assert_eq!(battle.planet, target);
    assert_eq!(sim.pending_battles().len(), 1);
    let resolved = sim.resolve_battle_auto(battle.id).expect("known battle");
    match resolved.last() {
        Some(Event::BattleResolved { winner, losses, .. }) => {
            assert_eq!(*winner, Some(empire), "three Star Destroyers beat two X-wing squadrons");
            assert!(losses.iter().any(|&(f, n)| f == rebel && n == 2));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(sim.pending_battles().is_empty());
    assert!(sim.galaxy().fleets_at(target).all(|f| f.owner != rebel));
}

#[test]
fn merge_and_split_fleets() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data, SimConfig::default());
    let g = sim.galaxy().clone();
    let f = g.fleets.values().find(|f| f.unit_count() >= 2).expect("a fleet with 2+ ships").clone();
    let (&u, &n) = f.units.iter().next().unwrap();
    let new_id = match sim.apply(Command::SplitFleet { faction: f.owner, fleet: f.id, units: vec![(u, 1)] }) {
        Ok(CommandOk::NewFleet(id)) => id,
        other => panic!("split failed: {other:?}"),
    };
    assert_eq!(sim.galaxy().fleets[&f.id].unit_count(), f.unit_count() - 1);
    sim.apply(Command::MergeFleets { faction: f.owner, into: f.id, from: new_id }).unwrap();
    assert_eq!(sim.galaxy().fleets[&f.id].unit_count(), f.unit_count());
    assert_eq!(sim.galaxy().fleets[&f.id].units[&u], n);
    assert!(sim.apply(Command::SplitFleet { faction: f.owner, fleet: f.id, units: vec![(u, 9999)] }).is_err());
}

#[test]
fn ai_match_progresses() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data, SimConfig { victory_enabled: false, ..SimConfig::default() });
    sim.set_ai_factions(0..sim.world().factions.len() as u32);
    let mut weeks = 0;
    let mut captures = 0;
    for e in sim.step(20.0 * 60.0) {
        match e {
            Event::WeekEnded { .. } => weeks += 1,
            Event::PlanetCaptured { .. } => captures += 1,
            _ => {}
        }
    }
    assert_eq!(weeks, 20);
    assert!(captures > 0, "the AI should capture planets");
    assert!(sim.galaxy().factions.iter().all(|f| f.credits >= 0.0));
}

#[test]
fn tactical_resolver_settles_a_battle() {
    let Some(data) = load() else { return };
    let install = GameInstall::detect().unwrap();
    let fs = GameFs::for_mod(&install, Some("1770851727")).unwrap();
    let mut sim = new_sim(&data, SimConfig { auto_resolve_battles: false, victory_enabled: false, resolver: BattleResolver::Tactical, ..SimConfig::default() });
    let w = sim.world().clone();
    // Only the units of this test are needed in the catalog.
    let names = ["Star_Destroyer", "X-Wing_Squadron", "TIE_Fighter_Squadron"];
    sim.set_tactical_catalog(std::sync::Arc::new(battle_sim::Catalog::build(&data, Some(&fs), &names)));
    assert!(sim.tactical_names().len() > 1000);

    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let target = (0..w.planets.len() as u32)
        .find(|&p| w.real[p as usize] && sim.galaxy().planets[p as usize].owner.is_none() && sim.galaxy().fleets_at(p).next().is_none())
        .expect("empty planet");
    let isd = w.unit_id("Star_Destroyer").unwrap();
    let xw = w.unit_id("X-Wing_Squadron").unwrap();
    for (owner, unit, n) in [(empire, isd, 2u32), (rebel, xw, 3u32)] {
        let g = sim.galaxy_mut();
        let id = g.fleets.keys().max().copied().unwrap_or(0) + 1;
        g.fleets.insert(id, Fleet { id, owner, units: [(unit, n)].into(), location: FleetLocation::AtPlanet(target), previous_planet: target });
    }
    let battle = sim.step(1.0).into_iter().find_map(|e| if let Event::BattlePending(b) = e { Some(b) } else { None }).expect("pending");
    let events = sim.resolve_battle(battle.id).expect("resolved");
    match events.last() {
        Some(Event::BattleResolved { winner, losses, .. }) => {
            assert_eq!(*winner, Some(empire));
            assert!(losses.iter().any(|&(f, n)| f == rebel && n == 3), "{losses:?}");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(sim.galaxy().fleets_at(target).all(|f| f.owner == empire));
}
