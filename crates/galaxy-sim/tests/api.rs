//! Tests of the script-facing API (spawning, buildings, ownership, tech, alliances, ship ids).

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

fn new_sim(data: &GameData) -> Sim {
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    Sim::new(data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim")
}

#[test]
fn spawn_ids_merge_split_and_owner_change() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data);
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let home = w.planet_id("Coruscant_WW").unwrap();
    let isd = w.unit_id("Star_Destroyer").unwrap();
    let pop_before = sim.galaxy().factions[empire as usize].population_used;

    let fleet = sim.spawn_units(empire, home, isd, 3);
    assert_eq!(sim.galaxy().factions[empire as usize].population_used, pop_before + 3 * w.units[isd as usize].population, "economy refreshed");
    let ids: Vec<ShipId> = sim.ships_of(fleet).into_iter().filter(|(_, u)| *u == isd).map(|(s, _)| s).collect();
    assert!(ids.len() >= 3);
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "ids are unique");

    // Ids survive a split and a merge.
    let new_fleet = match sim.apply(Command::SplitFleet { faction: empire, fleet, units: vec![(isd, 2)] }) {
        Ok(CommandOk::NewFleet(f)) => f,
        other => panic!("{other:?}"),
    };
    let moved: Vec<ShipId> = sim.ships_of(new_fleet).into_iter().map(|(s, _)| s).collect();
    assert_eq!(moved.len(), 2);
    assert!(moved.iter().all(|s| ids.contains(s)));
    assert_eq!(sim.locate_ship(moved[0]).map(|x| x.0), Some(new_fleet));
    sim.apply(Command::MergeFleets { faction: empire, into: fleet, from: new_fleet }).unwrap();
    assert_eq!(sim.locate_ship(moved[0]).map(|x| x.0), Some(fleet));

    // Despawn and change owner keep ids consistent.
    assert!(sim.despawn_ship(moved[1]));
    assert!(sim.locate_ship(moved[1]).is_none());
    assert!(sim.change_ship_owner(moved[0], rebel));
    let (f, u) = sim.locate_ship(moved[0]).expect("ship keeps its id");
    assert_eq!(u, isd);
    assert_eq!(sim.galaxy().fleets[&f].owner, rebel);
}

#[test]
fn buildings_ownership_economy_and_tech() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data);
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let target = w.planet_id("Yavin_WW").unwrap();

    // set_planet_owner clears buildings, emits PlanetCaptured and refreshes the economy.
    let income_before = sim.galaxy().factions[rebel as usize].income_per_week;
    let events = sim.set_planet_owner(target, Some(empire));
    assert!(events.iter().any(|e| matches!(e, Event::PlanetCaptured { planet, new_owner, old_owner } if *planet == target && *new_owner == empire && *old_owner == Some(rebel))));
    assert!(sim.galaxy().planets[target as usize].starbase.is_none());
    assert!(sim.galaxy().factions[rebel as usize].income_per_week < income_before);
    assert!(sim.set_planet_owner(target, Some(empire)).is_empty(), "no change, no event");

    // Slots: structures stop being accepted once the orbital slots are full.
    let yard = w.structure_id("E_Shipyard_1").unwrap();
    assert!(sim.add_structure(target, yard));
    assert!(!sim.add_structure(target, yard), "max one shipyard per planet");
    assert!(sim.remove_structure(target, yard));
    assert!(!sim.remove_structure(target, yard));
    let base = w.starbase_index["empire_star_base_2"];
    sim.set_starbase(target, Some(base));
    assert_eq!(sim.galaxy().planets[target as usize].starbase, Some(base));

    // Credits and tech.
    let before = sim.galaxy().factions[empire as usize].credits;
    sim.add_credits(empire, 1234.0);
    assert_eq!(sim.galaxy().factions[empire as usize].credits, before + 1234.0);
    sim.add_credits(empire, -1.0e12);
    assert_eq!(sim.galaxy().factions[empire as usize].credits, 0.0);
    let tech = sim.galaxy().factions[empire as usize].tech;
    let events = sim.set_tech_level(empire, tech + 2);
    let advanced: Vec<i32> = events.iter().filter_map(|e| if let Event::TechAdvanced { level, .. } = e { Some(*level) } else { None }).collect();
    assert_eq!(advanced, [tech + 1, tech + 2]);
    assert!(events.iter().any(|e| matches!(e, Event::ConstructionComplete { kind: ProductionKind::Tech(_), .. })));
    sim.set_unlocked(empire, "Some_Locked_Unit", true);
    assert!(sim.galaxy().factions[empire as usize].unlocked.contains("some_locked_unit"));
}

#[test]
fn alliances_neutral_forces_and_queries() {
    let Some(data) = load() else { return };
    let mut sim = new_sim(&data);
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    assert!(!sim.are_allied(empire, rebel));
    sim.set_alliance(rebel, empire, true);
    assert!(sim.are_allied(empire, rebel));
    assert!(!sim.galaxy().hostile(&w, empire, rebel));
    // Allied fleets sharing a planet do not fight.
    let spot = w.planet_id("Yavin_WW").unwrap();
    let isd = w.unit_id("Star_Destroyer").unwrap();
    sim.spawn_units(empire, spot, isd, 1);
    sim.spawn_units(rebel, spot, isd, 1);
    let events = sim.step(5.0);
    assert!(!events.iter().any(|e| matches!(e, Event::BattlePending(_) | Event::BattleResolved { .. })));
    sim.set_alliance(rebel, empire, false);
    let events = sim.step(5.0);
    assert!(events.iter().any(|e| matches!(e, Event::BattleResolved { planet, .. } if *planet == spot)));

    // Neutral (pirate) defenders fight arriving fleets and report losses.
    let other = (0..w.planets.len() as u32)
        .find(|&p| w.real[p as usize] && sim.galaxy().planets[p as usize].owner.is_none() && sim.galaxy().fleets_at(p).next().is_none())
        .expect("a free neutral planet");
    let pirate = sim.neutral_faction().expect("pirates");
    let f = sim.spawn_neutral_units(other, w.unit_id("CR-90_Corvette").unwrap(), 2).expect("fleet");
    assert_eq!(sim.galaxy().fleets[&f].owner, pirate);
    sim.spawn_units(empire, other, isd, 2);
    let events = sim.step(3.0);
    assert!(events.iter().any(|e| matches!(e, Event::BattleWon { faction, .. } if *faction == empire)));
    assert!(events.iter().any(|e| matches!(e, Event::BattleLost { faction, .. } if *faction == pirate)));
    assert!(events.iter().any(|e| matches!(e, Event::UnitsDestroyed { faction, count, ships, .. } if *faction == pirate && *count as usize == ships.len())));

    // Perception and object kinds.
    let coruscant = w.planet_id("Coruscant_WW").unwrap();
    assert!(sim.planet_credit_value(coruscant) > 500.0);
    let _ = sim.planet_has_ground_access(coruscant);
    assert_eq!(sim.object_kind("Star_Destroyer"), ObjectKind::SpaceUnit);
    assert_eq!(sim.object_kind("Empire_Star_Base_2"), ObjectKind::Starbase);
    assert_eq!(sim.object_kind("E_Shipyard_1"), ObjectKind::Structure);
    assert_eq!(sim.object_kind("Trooper_Walker_Regiment"), ObjectKind::Ground);
    assert_eq!(sim.object_kind("does_not_exist"), ObjectKind::Other);
}
