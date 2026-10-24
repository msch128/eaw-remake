//! Fleet slots: every faction has at most `FLEET_SLOTS` fleets at a planet, each with a stable slot index. Arriving fleets merge into the targeted
//! slot, else take a free slot, else join slot 1. Battles deploy and reinforce in slot order. Real data; skips without the game.

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
    /// Two neighbouring planets of the Empire without any fleet; `home` is where the slots are filled, `away` where a fleet starts.
    home: PlanetId,
    away: PlanetId,
    isd: UnitTypeId,
}

/// The slot rules bind the human (Rebel in this campaign); the tests play the Empire, so they switch the rule on for the AI factions too.
fn setup(data: &GameData) -> Setup {
    setup_with(data, true)
}

fn setup_with(data: &GameData, cap_ai: bool) -> Setup {
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let config = SimConfig { auto_resolve_battles: false, victory_enabled: false, fleet_slot_cap_ai: cap_ai, ..SimConfig::default() };
    let mut sim = Sim::new(data, campaign, config).expect("sim");
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    sim.galaxy_mut().fleets.clear();
    sim.galaxy_mut().battles.clear();
    sim.galaxy_mut().ai_paused = true;
    let (home, away) = w
        .routes
        .iter()
        .find(|r| w.real[r.a as usize] && w.real[r.b as usize])
        .map(|r| (r.a, r.b))
        .expect("two neighbouring planets");
    sim.set_planet_owner(home, Some(empire));
    sim.set_planet_owner(away, Some(empire));
    let isd = w.unit_id("Star_Destroyer").unwrap();
    Setup { sim, empire, home, away, isd }
}

fn new_fleet(r: Result<CommandOk, CommandError>) -> FleetId {
    match r {
        Ok(CommandOk::NewFleet(n)) => n,
        other => panic!("{other:?}"),
    }
}

fn slot(sim: &Sim, f: FleetId) -> u8 {
    sim.galaxy().fleets[&f].slot
}

fn count(sim: &Sim, f: FleetId) -> u32 {
    sim.galaxy().fleets[&f].unit_count()
}

/// Three fleets of 2 ISDs each in slots 1, 2, 3 at `home`.
fn three_fleets(s: &mut Setup) -> [FleetId; 3] {
    let a = s.sim.spawn_units(s.empire, s.home, s.isd, 6);
    let b = new_fleet(s.sim.apply(Command::SplitFleet { faction: s.empire, fleet: a, units: vec![(s.isd, 2)] }));
    let c = new_fleet(s.sim.apply(Command::SplitFleet { faction: s.empire, fleet: a, units: vec![(s.isd, 2)] }));
    [a, b, c]
}

/// Runs until the fleet has stopped moving.
fn arrive(sim: &mut Sim, f: FleetId) {
    for _ in 0..5000 {
        sim.step(1.0);
        if sim.galaxy().fleets.get(&f).is_none_or(|x| x.planet().is_some()) {
            return;
        }
    }
    panic!("fleet {f} never arrived");
}

#[test]
fn new_fleets_take_the_lowest_free_slot_and_a_fourth_is_refused() {
    let Some(data) = load() else { return };
    let mut s = setup(&data);
    let [a, b, c] = three_fleets(&mut s);
    assert_eq!((slot(&s.sim, a), slot(&s.sim, b), slot(&s.sim, c)), (1, 2, 3));
    let r = s.sim.apply(Command::SplitFleet { faction: s.empire, fleet: a, units: vec![(s.isd, 1)] });
    assert_eq!(r, Err(CommandError::NoSlot), "all three slots are taken");
    let r = s.sim.apply(Command::TransferUnits { faction: s.empire, from: a, to: None, units: vec![(s.isd, 1)], hyperspace: false });
    assert_eq!(r, Err(CommandError::NoSlot));
    assert_eq!(count(&s.sim, a), 2, "a refused command changes nothing");

    // Freeing slot 2 lets the next split take exactly that one, even though slot 3 is also held.
    s.sim.apply(Command::MergeFleets { faction: s.empire, into: a, from: b }).unwrap();
    assert_eq!(count(&s.sim, a), 4);
    let d = new_fleet(s.sim.apply(Command::SplitFleet { faction: s.empire, fleet: a, units: vec![(s.isd, 1)] }));
    assert_eq!(slot(&s.sim, d), 2);
    assert_eq!(slot(&s.sim, c), 3, "slots are stable");
}

#[test]
fn dropping_units_on_a_slot_merges_into_its_fleet_or_fills_the_empty_box() {
    let Some(data) = load() else { return };
    let mut s = setup(&data);
    let [a, b, c] = three_fleets(&mut s);
    s.sim.apply(Command::MergeFleets { faction: s.empire, into: a, from: b }).unwrap();
    // Slot 3 is occupied: the unit joins its fleet.
    s.sim.apply(Command::TransferUnitsToSlot { faction: s.empire, from: a, slot: 3, units: vec![(s.isd, 1)], hyperspace: false }).unwrap();
    assert_eq!((count(&s.sim, a), count(&s.sim, c)), (3, 3));
    // Slot 2 is empty: a new fleet appears exactly there.
    let r = s.sim.apply(Command::TransferUnitsToSlot { faction: s.empire, from: a, slot: 2, units: vec![(s.isd, 1)], hyperspace: false });
    let n = new_fleet(r);
    assert_eq!((slot(&s.sim, n), count(&s.sim, n), count(&s.sim, a)), (2, 1, 2));
    // Its own slot and slots outside 1..=3 are nonsense.
    for bad in [1u8, 0, 4] {
        let r = s.sim.apply(Command::TransferUnitsToSlot { faction: s.empire, from: a, slot: bad, units: vec![(s.isd, 1)], hyperspace: false });
        assert_eq!(r, Err(CommandError::Invalid), "slot {bad}");
    }
}

#[test]
fn an_arriving_fleet_merges_into_the_named_slot_else_a_free_one_else_slot_one() {
    let Some(data) = load() else { return };
    let mut s = setup(&data);
    let [a, b, c] = three_fleets(&mut s);
    let mover = s.sim.spawn_units(s.empire, s.away, s.isd, 1);

    // Named slot 3: merges into the fleet there.
    s.sim.apply(Command::MoveFleetToSlot { faction: s.empire, fleet: mover, to: s.home, slot: 3 }).unwrap();
    assert!(s.sim.galaxy().dest_slots.contains_key(&mover));
    arrive(&mut s.sim, mover);
    assert!(!s.sim.galaxy().fleets.contains_key(&mover), "merged away");
    assert_eq!((count(&s.sim, a), count(&s.sim, b), count(&s.sim, c)), (2, 2, 3));
    assert!(s.sim.galaxy().dest_slots.is_empty());

    // No slot named, all taken: slot 1.
    let mover = s.sim.spawn_units(s.empire, s.away, s.isd, 1);
    s.sim.apply(Command::MoveFleet { faction: s.empire, fleet: mover, to: s.home }).unwrap();
    arrive(&mut s.sim, mover);
    assert_eq!((count(&s.sim, a), count(&s.sim, b), count(&s.sim, c)), (3, 2, 3));
    assert_eq!(s.sim.galaxy().fleets_at(s.home).count(), 3);

    // A free slot: the fleet keeps its identity and takes it.
    s.sim.apply(Command::MergeFleets { faction: s.empire, into: a, from: b }).unwrap();
    let mover = s.sim.spawn_units(s.empire, s.away, s.isd, 1);
    s.sim.apply(Command::MoveFleet { faction: s.empire, fleet: mover, to: s.home }).unwrap();
    arrive(&mut s.sim, mover);
    assert_eq!((slot(&s.sim, mover), count(&s.sim, mover)), (2, 1));

    // A named empty slot on arrival is taken as named.
    s.sim.apply(Command::MergeFleets { faction: s.empire, into: a, from: mover }).unwrap();
    let mover = s.sim.spawn_units(s.empire, s.away, s.isd, 1);
    s.sim.apply(Command::MoveFleetToSlot { faction: s.empire, fleet: mover, to: s.home, slot: 2 }).unwrap();
    arrive(&mut s.sim, mover);
    assert_eq!(slot(&s.sim, mover), 2);

    // Within the planet a fleet dropped on another slot merges there at once.
    s.sim.apply(Command::MoveFleetToSlot { faction: s.empire, fleet: mover, to: s.home, slot: 3 }).unwrap();
    assert!(!s.sim.galaxy().fleets.contains_key(&mover));
    assert_eq!(count(&s.sim, c), 4);
}

#[test]
fn merging_keeps_the_pathfinder_of_the_receiving_fleet() {
    let Some(data) = load() else { return };
    let mut s = setup(&data);
    let w = s.sim.world().clone();
    let vsd = w.unit_id("Victory_Destroyer").unwrap();
    let a = s.sim.spawn_units(s.empire, s.home, s.isd, 2);
    let b = s.sim.spawn_units(s.empire, s.away, vsd, 1);
    s.sim.apply(Command::SetHyperspace { faction: s.empire, fleet: b, unit: Some(vsd) }).unwrap();

    // The receiving fleet has no pathfinder: it takes the incoming one.
    s.sim.apply(Command::MoveFleetToSlot { faction: s.empire, fleet: b, to: s.home, slot: 1 }).unwrap();
    arrive(&mut s.sim, b);
    assert_eq!(s.sim.galaxy().hyperspace_unit(a), Some(vsd));

    // A receiving fleet with a pathfinder keeps it.
    let c = s.sim.spawn_units(s.empire, s.away, s.isd, 1);
    s.sim.apply(Command::SetHyperspace { faction: s.empire, fleet: c, unit: Some(s.isd) }).unwrap();
    s.sim.apply(Command::MoveFleetToSlot { faction: s.empire, fleet: c, to: s.home, slot: 1 }).unwrap();
    arrive(&mut s.sim, c);
    assert_eq!(s.sim.galaxy().hyperspace_unit(a), Some(vsd));
    assert_eq!(count(&s.sim, a), 4);
}

#[test]
fn hosts_and_old_saves_with_unassigned_or_too_many_fleets_settle_into_three_slots() {
    let Some(data) = load() else { return };
    let mut s = setup(&data);
    let (empire, isd, home) = (s.empire, s.isd, s.home);
    let g = s.sim.galaxy_mut();
    for id in 1..=5u32 {
        g.fleets.insert(id, Fleet { id, owner: empire, units: [(isd, 1)].into(), location: FleetLocation::AtPlanet(home), previous_planet: home, slot: 0 });
    }
    s.sim.step(1.0);
    let at: Vec<&Fleet> = s.sim.galaxy().fleets_at(home).collect();
    assert_eq!(at.len(), 3);
    let mut slots: Vec<u8> = at.iter().map(|f| f.slot).collect();
    slots.sort_unstable();
    assert_eq!(slots, vec![1, 2, 3]);
    assert_eq!(at.iter().map(|f| f.unit_count()).sum::<u32>(), 5, "nothing is lost");
    assert_eq!(at.iter().find(|f| f.slot == 1).unwrap().unit_count(), 3, "the surplus joined slot 1");

    // A fleet saved before slots existed deserializes with slot 0.
    let mut json = serde_json::to_value(at[0]).unwrap();
    json.as_object_mut().unwrap().remove("slot");
    let old: Fleet = serde_json::from_value(json).unwrap();
    assert_eq!(old.slot, 0);
}

/// Battle start (S2): groups and reserve follow the slots; a pathfinder in any fleet holds back every other unit of the side (design rule).
#[test]
fn battles_deploy_in_slot_order_and_a_pathfinder_holds_back_every_other_unit() {
    let Some(data) = load() else { return };
    let w = setup(&data).sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let (isd, vsd) = (w.unit_id("Star_Destroyer").unwrap(), w.unit_id("Victory_Destroyer").unwrap());
    let nebulon = w.unit_id("Nebulon_B_Frigate").unwrap();
    // Slot 1 holds the Victory, slot 2 two Star Destroyers; `pathfinder` marks the fleet of that slot. Returns (deployed, reserve) as (name, count) lists.
    let run = |pathfinder: Option<u8>| -> (Vec<(String, u32)>, Vec<(String, u32)>) {
        let mut s = setup(&data);
        let target = (0..w.planets.len() as u32)
            .find(|&p| w.real[p as usize] && w.planets[p as usize].space_map.is_some() && s.sim.galaxy().planets[p as usize].owner.is_none() && s.sim.galaxy().fleets_at(p).next().is_none())
            .expect("empty planet with a map");
        // Insert the slot-2 fleet first so that fleet id order and slot order differ.
        let g = s.sim.galaxy_mut();
        let (f2, f1, fr) = (10, 11, 12);
        g.fleets.insert(f2, Fleet { id: f2, owner: empire, units: [(isd, 2)].into(), location: FleetLocation::AtPlanet(target), previous_planet: target, slot: 2 });
        g.fleets.insert(f1, Fleet { id: f1, owner: empire, units: [(vsd, 1)].into(), location: FleetLocation::AtPlanet(target), previous_planet: target, slot: 1 });
        g.fleets.insert(fr, Fleet { id: fr, owner: rebel, units: [(nebulon, 1)].into(), location: FleetLocation::AtPlanet(target), previous_planet: target, slot: 1 });
        match pathfinder {
            Some(1) => s.sim.apply(Command::SetHyperspace { faction: empire, fleet: f1, unit: Some(vsd) }).unwrap(),
            Some(_) => s.sim.apply(Command::SetHyperspace { faction: empire, fleet: f2, unit: Some(isd) }).unwrap(),
            None => CommandOk::Done,
        };
        let battle = s.sim.step(1.0).into_iter().find_map(|e| if let Event::BattlePending(b) = e { Some(b) } else { None }).expect("pending battle");
        let req = s.sim.tactical_setup(battle.id).unwrap();
        let side = req.sides.iter().find(|x| x.faction == empire).unwrap();
        let list = |v: &[TacticalGroup]| v.iter().filter(|g| g.kind == GroupKind::Unit).map(|g| (g.name.to_ascii_lowercase(), g.count)).collect::<Vec<_>>();
        (list(&side.groups), list(&side.reserve))
    };
    let (vic, star) = ("victory_destroyer".to_string(), "star_destroyer".to_string());
    let (d, r) = run(None);
    assert_eq!((d, r), (vec![(vic.clone(), 1), (star.clone(), 2)], vec![]), "no pathfinder: everyone deploys, slot 1 first");
    // Slot 1 has a pathfinder: only the Victory jumps in, the Star Destroyers of slot 2 wait as reserve.
    let (d, r) = run(Some(1));
    assert_eq!((d, r), (vec![(vic.clone(), 1)], vec![(star.clone(), 2)]));
    // Slot 2 has a pathfinder: one Star Destroyer jumps in; the Victory (slot 1) and the other Star Destroyer wait, in slot order.
    let (d, r) = run(Some(2));
    assert_eq!((d, r), (vec![(star.clone(), 1)], vec![(vic, 1), (star, 1)]));
}

/// The cap binds the AI by default; `fleet_slot_cap_ai: false` lifts it for the AI factions, the human always keeps it.
#[test]
fn ai_factions_are_not_capped_unless_configured_but_the_human_is() {
    let Some(data) = load() else { return };
    assert!(SimConfig::default().fleet_slot_cap_ai, "the AI is capped by default");
    let mut s = setup_with(&data, false);
    let a = s.sim.spawn_units(s.empire, s.home, s.isd, 8);
    for _ in 0..4 {
        s.sim.apply(Command::SplitFleet { faction: s.empire, fleet: a, units: vec![(s.isd, 1)] }).expect("the AI may keep more than three fleets");
    }
    s.sim.step(1.0);
    assert_eq!(s.sim.galaxy().fleets_at(s.home).count(), 5);

    let rebel = s.sim.world().faction_id("Rebel").unwrap();
    assert!(s.sim.world().slot_limited(rebel) && !s.sim.world().slot_limited(s.empire));
    s.sim.set_planet_owner(s.home, Some(rebel));
    let r = s.sim.spawn_units(rebel, s.home, s.isd, 8);
    let mut fleets = 1;
    for _ in 0..4 {
        fleets += s.sim.apply(Command::SplitFleet { faction: rebel, fleet: r, units: vec![(s.isd, 1)] }).is_ok() as u32;
    }
    assert_eq!(fleets, 3, "the human gets three fleets, no more");
}

#[test]
fn new_ships_join_slot_one_and_never_a_fleet_in_slot_two_or_three() {
    let Some(data) = load() else { return };
    let mut s = setup(&data);
    // The only fleet at home sits in slot 3.
    let kept = s.sim.spawn_units(s.empire, s.away, s.isd, 1);
    s.sim.apply(Command::MoveFleetToSlot { faction: s.empire, fleet: kept, to: s.home, slot: 3 }).unwrap();
    arrive(&mut s.sim, kept);
    assert_eq!(slot(&s.sim, kept), 3);
    // A finished ship starts a new fleet in slot 1; the next one joins it.
    let built = s.sim.spawn_units(s.empire, s.home, s.isd, 1);
    assert_ne!(built, kept, "the slot 3 fleet does not grow");
    assert_eq!((slot(&s.sim, built), count(&s.sim, built), count(&s.sim, kept)), (1, 1, 1));
    assert_eq!(s.sim.spawn_units(s.empire, s.home, s.isd, 1), built);
    assert_eq!((count(&s.sim, built), count(&s.sim, kept)), (2, 1));
}
