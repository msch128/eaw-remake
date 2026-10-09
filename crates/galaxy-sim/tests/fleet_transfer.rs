//! Fleet organizer commands of the planet view: moving units between fleets and the hyperspace slot of a fleet.

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

struct Setup {
    sim: Sim,
    empire: FactionId,
    planet: PlanetId,
    isd: UnitTypeId,
}

fn setup(data: &GameData) -> Setup {
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let mut sim = Sim::new(data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    // A quiet planet nobody sits at, so the fleets are the only ones there.
    let planet = (0..w.planets.len() as PlanetId).find(|&p| w.real[p as usize] && sim.galaxy().fleets_at(p).next().is_none()).unwrap();
    sim.set_planet_owner(planet, Some(empire));
    let isd = w.unit_id("Star_Destroyer").unwrap();
    Setup { sim, empire, planet, isd }
}

fn units(sim: &Sim, f: FleetId) -> Vec<(UnitTypeId, u32)> {
    sim.galaxy().fleets[&f].units.iter().map(|(u, c)| (*u, *c)).collect()
}

fn transfer(sim: &mut Sim, faction: FactionId, from: FleetId, to: Option<FleetId>, units: Vec<(UnitTypeId, u32)>, hyperspace: bool) -> Result<CommandOk, CommandError> {
    sim.apply(Command::TransferUnits { faction, from, to, units, hyperspace })
}

fn new_fleet(r: Result<CommandOk, CommandError>) -> FleetId {
    match r {
        Ok(CommandOk::NewFleet(n)) => n,
        other => panic!("{other:?}"),
    }
}

#[test]
fn units_move_between_fleets_and_into_a_new_fleet() {
    let Some(data) = load() else { return };
    let Setup { mut sim, empire, planet, isd, .. } = setup(&data);
    let a = sim.spawn_units(empire, planet, isd, 3);
    let b = new_fleet(sim.apply(Command::SplitFleet { faction: empire, fleet: a, units: vec![(isd, 1)] }));
    sim.apply(Command::TransferUnits { faction: empire, from: a, to: None, units: vec![(isd, 1)], hyperspace: false }).unwrap();
    let ids_before: Vec<ShipId> = sim.ships_of(a).into_iter().map(|(s, _)| s).collect();
    assert_eq!(ids_before.len(), 1);

    // The only ISD of `a` into b: `a` has no ships left and disappears.
    transfer(&mut sim, empire, a, Some(b), vec![(isd, 1)], false).unwrap();
    assert!(!sim.galaxy().fleets.contains_key(&a), "emptied source fleet is gone");
    assert_eq!(units(&sim, b).iter().find(|(u, _)| *u == isd), Some(&(isd, 2)));
    assert!(sim.ships_of(b).iter().any(|(s, _)| *s == ids_before[0]), "the ship keeps its id");

    // One of the two into an empty box: a new fleet at the same planet.
    let new = new_fleet(transfer(&mut sim, empire, b, None, vec![(isd, 1)], false));
    assert_eq!(units(&sim, new), vec![(isd, 1)]);
    assert_eq!(sim.galaxy().fleets[&new].location, FleetLocation::AtPlanet(planet));
}

#[test]
fn bad_transfers_are_refused() {
    let Some(data) = load() else { return };
    let Setup { mut sim, empire, planet, isd, .. } = setup(&data);
    let w = sim.world().clone();
    let a = sim.spawn_units(empire, planet, isd, 2);
    let rebel = w.faction_id("Rebel").unwrap();
    let other = sim.spawn_units(rebel, planet, isd, 1);
    assert!(matches!(transfer(&mut sim, empire, a, Some(a), vec![(isd, 1)], false), Err(CommandError::Invalid)), "onto itself");
    assert!(matches!(transfer(&mut sim, empire, a, None, vec![(isd, 3)], false), Err(CommandError::Invalid)), "more than there is");
    assert!(matches!(transfer(&mut sim, empire, a, Some(other), vec![(isd, 1)], false), Err(CommandError::Invalid)), "an enemy fleet");
    assert!(matches!(transfer(&mut sim, empire, a, None, vec![], false), Err(CommandError::Invalid)), "nothing");
    assert!(matches!(transfer(&mut sim, rebel, a, None, vec![(isd, 1)], false), Err(CommandError::Invalid)), "not the owner");
    assert!(matches!(transfer(&mut sim, empire, a, None, vec![(isd, 2)], true), Err(CommandError::Invalid)), "the slot takes one ship");
    assert_eq!(units(&sim, a), vec![(isd, 2)], "refusals change nothing");
}

#[test]
fn hyperspace_slot_follows_the_ship() {
    let Some(data) = load() else { return };
    let Setup { mut sim, empire, planet, isd, .. } = setup(&data);
    let a = sim.spawn_units(empire, planet, isd, 3);
    let ships: Vec<ShipId> = sim.ships_of(a).into_iter().map(|(s, _)| s).collect();
    sim.apply(Command::SetHyperspace { faction: empire, fleet: a, unit: Some(isd) }).unwrap();
    let slot = sim.galaxy().hyperspace_ship(a).expect("slot set");
    assert!(ships.contains(&slot));
    assert_eq!(sim.galaxy().hyperspace_unit(a), Some(isd));
    assert!(matches!(sim.apply(Command::SetHyperspace { faction: empire, fleet: a, unit: Some(0) }), Err(CommandError::Invalid)), "no ship of that type in the fleet");

    // Transferring the other ships leaves the slot ship behind.
    let b = new_fleet(transfer(&mut sim, empire, a, None, vec![(isd, 2)], false));
    assert_eq!(sim.galaxy().hyperspace_ship(a), Some(slot), "the slot ship was not taken");
    assert_eq!(sim.galaxy().hyperspace_ship(b), None);

    // A drop onto the hyperspace slot of another fleet: one ship, and it sits in that slot afterwards.
    let c = new_fleet(transfer(&mut sim, empire, b, None, vec![(isd, 1)], true));
    assert!(sim.galaxy().hyperspace_ship(c).is_some());
    assert_eq!(sim.galaxy().hyperspace_ship(b), None);

    // Merging keeps the slot of the absorbed fleet when the receiver has none.
    sim.apply(Command::MergeFleets { faction: empire, into: b, from: a }).unwrap();
    assert_eq!(sim.galaxy().hyperspace_ship(b), Some(slot));
    assert_eq!(sim.galaxy().hyperspace_ship(a), None);

    // Moving the slot ship itself (nothing else of its type is left) leaves the slot behind, an emptied source takes it along.
    let d = new_fleet(transfer(&mut sim, empire, b, None, vec![(isd, 2)], false));
    assert!(!sim.galaxy().fleets.contains_key(&b));
    assert_eq!(sim.galaxy().hyperspace_ship(b), None);
    assert_eq!(sim.galaxy().hyperspace_ship(d), None, "a plain move does not fill the slot");

    // The slot dies with its ship, and can be cleared.
    sim.apply(Command::SetHyperspace { faction: empire, fleet: d, unit: Some(isd) }).unwrap();
    let doomed = sim.galaxy().hyperspace_ship(d).unwrap();
    assert!(sim.despawn_ship(doomed));
    assert_eq!(sim.galaxy().hyperspace_ship(d), None);
    sim.apply(Command::SetHyperspace { faction: empire, fleet: d, unit: Some(isd) }).unwrap();
    sim.apply(Command::SetHyperspace { faction: empire, fleet: d, unit: None }).unwrap();
    assert_eq!(sim.galaxy().hyperspace_ship(d), None);
}
