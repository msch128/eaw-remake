//! Several structures can wait in the structure queue at once: up to the free slots of their kind (real data, skipped without the game).

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

#[test]
fn ground_structures_queue_up_to_the_free_slots_and_chain_on_queued_prerequisites() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let p = w.planet_id("Coruscant_WW").unwrap();
    sim.set_planet_owner(p, None);
    sim.set_planet_owner(p, Some(empire));
    sim.add_credits(empire, 1.0e9);
    sim.set_tech_level(empire, 9);
    let mut sb = w.first_starbase[empire as usize];
    for _ in 1..3 {
        sb = sb.and_then(|b| w.starbases[b as usize].next);
    }
    sim.set_starbase(p, sb);
    let slots = w.planets[p as usize].ground_slots;
    assert_eq!(slots, 6, "Coruscant has 6 ground slots");
    // Order whatever is orderable until nothing is: a structure whose requirement is queued before it counts.
    let mut queued = Vec::new();
    loop {
        let next = (0..w.structures.len() as u32).find(|&s| !w.structures[s as usize].orbital && w.check_build_structure(sim.galaxy(), empire, p, s).is_ok());
        let Some(s) = next else { break };
        sim.apply(Command::BuildStructure { faction: empire, planet: p, structure: s }).unwrap();
        queued.push(s);
        // A unique structure cannot be queued twice.
        if w.structures[s as usize].max_per_planet == 1 {
            assert!(sim.apply(Command::BuildStructure { faction: empire, planet: p, structure: s }).is_err(), "{} is limited to one per planet", w.structures[s as usize].name);
        }
    }
    let names: Vec<&str> = queued.iter().map(|&s| w.structures[s as usize].name.as_str()).collect();
    println!("queued {} ground structures: {names:?}", queued.len());
    assert!(queued.len() >= 2, "several ground structures queue at once: {names:?}");
    assert!(queued.len() as i32 <= slots);
    let in_queue = |sim: &Sim| sim.galaxy().planets[p as usize].queues[Lane::Structures as usize].len();
    assert_eq!(in_queue(&sim), queued.len());
    // Cancelling the first one drops everything that needed it.
    let before = in_queue(&sim);
    sim.apply(Command::CancelProduction { faction: empire, planet: p, lane: Lane::Structures, index: 0 }).unwrap();
    assert!(in_queue(&sim) < before - 1, "the structures that needed the cancelled one leave the queue too ({before} -> {})", in_queue(&sim));
}

#[test]
fn space_structures_are_limited_by_their_slots() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let p = w.planet_id("Endor_WW").unwrap();
    sim.add_credits(empire, 1.0e9);
    sim.set_tech_level(empire, 9);
    let mut queued = 0;
    loop {
        let next = (0..w.structures.len() as u32).find(|&s| w.structures[s as usize].orbital && w.check_build_structure(sim.galaxy(), empire, p, s).is_ok());
        let Some(s) = next else { break };
        sim.apply(Command::BuildStructure { faction: empire, planet: p, structure: s }).unwrap();
        queued += 1;
    }
    let cap = sim.world().planet_capacity(sim.galaxy(), p);
    println!("space: queued {queued}, slots {} (free {})", cap.space.total, cap.space.free());
    assert!(queued >= 2 && queued <= cap.space.total, "queued {queued} of {} slots", cap.space.total);
    assert!(cap.space.free() >= 0);
}

#[test]
fn every_orderable_structure_of_a_start_planet_queues_up_to_its_slots() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    sim.add_credits(empire, 1.0e9);
    sim.set_tech_level(empire, 9);
    let mut best = (0, 0, 0);
    for p in sim.galaxy().owned_planets(empire).collect::<Vec<_>>() {
        for orbital in [false, true] {
            let orderable: Vec<u32> = (0..w.structures.len() as u32).filter(|&s| w.structures[s as usize].orbital == orbital && w.check_build_structure(sim.galaxy(), empire, p, s).is_ok()).collect();
            let free = {
                let c = sim.world().planet_capacity(sim.galaxy(), p);
                if orbital { c.space.free() } else { c.ground.free() }
            };
            let mut queued = 0;
            for &s in &orderable {
                if sim.apply(Command::BuildStructure { faction: empire, planet: p, structure: s }).is_ok() {
                    queued += 1;
                }
            }
            println!("planet {} orbital {orbital}: orderable {}, free slots {free}, queued {queued}", w.planets[p as usize].name, orderable.len(), );
            assert!(queued <= free.max(0) as usize);
            if queued > best.0 {
                best = (queued, orderable.len(), free as usize);
            }
        }
    }
    println!("best: queued {} of {} orderable, {} free slots", best.0, best.1, best.2);
    assert!(best.0 >= 2, "some planet queues several structures at once");
}

#[test]
fn selling_a_structure_refunds_half_plus_one_and_frees_the_slot() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let p = w.planet_id("Coruscant_WW").unwrap();
    sim.set_planet_owner(p, None);
    sim.set_planet_owner(p, Some(empire));
    let palace = w.structure_id("Imperial_Palace_WW_Urban").unwrap();
    assert!(sim.add_structure(p, palace));
    let free_before = sim.world().planet_capacity(sim.galaxy(), p).ground.free();
    let credits = sim.galaxy().factions[empire as usize].credits;
    let r = sim.apply(Command::SellStructure { faction: empire, planet: p, structure: palace }).unwrap();
    let cost = f64::from(w.structures[palace as usize].cost);
    assert_eq!(r, CommandOk::Refunded(cost / 2.0 + 1.0));
    assert_eq!(sim.galaxy().factions[empire as usize].credits, credits + cost / 2.0 + 1.0);
    assert_eq!(sim.world().planet_capacity(sim.galaxy(), p).ground.free(), free_before + 1, "the slot is free again");
    // Nothing left to sell, not the owner's, unknown planet.
    assert_eq!(sim.apply(Command::SellStructure { faction: empire, planet: p, structure: palace }), Err(CommandError::Invalid));
    let rebel = w.faction_id("Rebel").unwrap();
    assert_eq!(sim.apply(Command::SellStructure { faction: rebel, planet: p, structure: palace }), Err(CommandError::NotOwner));
}

#[test]
fn orbital_and_ground_structures_build_in_parallel() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let p = w.planet_id("Coruscant_WW").unwrap();
    sim.set_planet_owner(p, None);
    sim.set_planet_owner(p, Some(empire));
    sim.add_credits(empire, 1.0e9);
    sim.set_tech_level(empire, 9);
    let mut sb = w.first_starbase[empire as usize];
    for _ in 1..3 {
        sb = sb.and_then(|b| w.starbases[b as usize].next);
    }
    sim.set_starbase(p, sb);

    // Queue 1 ground structure and 1 orbital structure
    let ground_s = (0..w.structures.len() as u32).find(|&s| !w.structures[s as usize].orbital && w.check_build_structure(sim.galaxy(), empire, p, s).is_ok()).expect("orderable ground");
    let orbital_s = (0..w.structures.len() as u32).find(|&s| w.structures[s as usize].orbital && w.check_build_structure(sim.galaxy(), empire, p, s).is_ok()).expect("orderable orbital");

    sim.apply(Command::BuildStructure { faction: empire, planet: p, structure: ground_s }).unwrap();
    sim.apply(Command::BuildStructure { faction: empire, planet: p, structure: orbital_s }).unwrap();

    let q = &sim.galaxy().planets[p as usize].queues[Lane::Structures as usize];
    assert_eq!(q.len(), 2);
    let g_rem_before = q[0].remaining;
    let o_rem_before = q[1].remaining;

    // Step 5 seconds
    sim.step(5.0);

    let q = &sim.galaxy().planets[p as usize].queues[Lane::Structures as usize];
    assert_eq!(q.len(), 2, "both structures still building");
    assert!((q[0].remaining - (g_rem_before - 5.0)).abs() < 0.1, "ground progressed by 5s");
    assert!((q[1].remaining - (o_rem_before - 5.0)).abs() < 0.1, "orbital progressed by 5s in parallel");
}

#[test]
fn sell_value_is_half_the_cost_plus_one_and_zero_for_unknown_types() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let w = sim.world();
    // Remake `Delete_Building`: Gas_Refinery_Default costs 7500, its sell text says 3750 and the script pays 3751.
    let refinery = w.structure_id("E_Gas_Refinery").expect("E_Gas_Refinery");
    assert_eq!(w.structures[refinery as usize].cost, 7500.0);
    assert_eq!(w.sell_value(refinery), 3751.0);
    assert_eq!(w.sell_value(u32::MAX), 0.0);
}

#[test]
fn the_space_side_of_the_queue_holds_five_items_and_a_sixth_is_refused() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let p = w.planet_id("Coruscant_WW").unwrap();
    sim.add_credits(empire, 1.0e9);
    sim.set_tech_level(empire, 9);
    let unit = *w.buildable_units[empire as usize].iter().find(|&&u| w.check_build_unit(sim.galaxy(), empire, p, u).is_ok()).expect("a buildable ship");
    assert_eq!(w.config.max_queue_len, 5, "five slots per side, like queue00..09 of the bar");
    for _ in 0..5 {
        sim.apply(Command::BuildUnit { faction: empire, planet: p, unit }).unwrap();
    }
    assert_eq!(sim.apply(Command::BuildUnit { faction: empire, planet: p, unit }), Err(CommandError::NoSlot), "the sixth ship has no queue slot");
    assert_eq!(sim.world().queued_on_side(&sim.galaxy().planets[p as usize], false), 5);
    assert_eq!(sim.world().queued_on_side(&sim.galaxy().planets[p as usize], true), 0, "the ground side is separate");
}

fn fleet_of(sim: &mut Sim, owner: FactionId, unit: UnitTypeId, n: u32, at: PlanetId) -> FleetId {
    let g = sim.galaxy_mut();
    let id = g.fleets.keys().max().copied().unwrap_or(0) + 1;
    g.fleets.insert(id, Fleet { id, owner, units: [(unit, n)].into(), location: FleetLocation::AtPlanet(at), previous_planet: at, slot: 0 });
    id
}

#[test]
fn selling_a_ship_pays_half_plus_one_and_the_last_one_removes_the_fleet() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let rebel = w.faction_id("Rebel").unwrap();
    let p = w.planet_id("Coruscant_WW").unwrap();
    let isd = w.unit_id("Star_Destroyer").unwrap();
    let f = fleet_of(&mut sim, empire, isd, 2, p);
    let cost = f64::from(w.units[isd as usize].cost);
    assert_eq!(w.unit_sell_value(isd), cost / 2.0 + 1.0);
    let credits = sim.galaxy().factions[empire as usize].credits;
    assert_eq!(sim.apply(Command::SellUnit { faction: empire, fleet: f, unit: isd }), Ok(CommandOk::Refunded(cost / 2.0 + 1.0)));
    assert_eq!(sim.galaxy().factions[empire as usize].credits, credits + cost / 2.0 + 1.0);
    assert_eq!(sim.galaxy().fleets[&f].units[&isd], 1);
    assert!(sim.apply(Command::SellUnit { faction: empire, fleet: f, unit: isd }).is_ok());
    assert!(!sim.galaxy().fleets.contains_key(&f), "the empty fleet is gone");
    assert_eq!(sim.apply(Command::SellUnit { faction: empire, fleet: f, unit: isd }), Err(CommandError::Invalid), "nothing left to sell");
    let other = fleet_of(&mut sim, empire, isd, 1, p);
    assert_eq!(sim.apply(Command::SellUnit { faction: rebel, fleet: other, unit: isd }), Err(CommandError::Invalid), "not the issuer's fleet");
}

#[test]
fn heroes_and_ships_that_are_not_in_the_fleet_cannot_be_sold() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    let p = w.planet_id("Coruscant_WW").unwrap();
    let hero = (0..w.units.len() as u32).find(|&u| w.units[u as usize].hero && matches!(w.units[u as usize].role, UnitRole::Ship)).expect("a hero ship");
    let f = fleet_of(&mut sim, empire, hero, 1, p);
    assert_eq!(sim.apply(Command::SellUnit { faction: empire, fleet: f, unit: hero }), Err(CommandError::NotAvailable));
    assert!(sim.galaxy().fleets.contains_key(&f), "the hero stays");
    let isd = w.unit_id("Star_Destroyer").unwrap();
    assert_eq!(sim.apply(Command::SellUnit { faction: empire, fleet: f, unit: isd }), Err(CommandError::Invalid), "this type is not in the fleet");
}
