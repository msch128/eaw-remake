//! Structure level swaps from `Scripts/GameObject/Upgrade_*.lua` on Remake 4.0 data. Prices and requirements come from the build item;
//! completing the item replaces the structure the way `Upgrade_Structure` / `Downgrade_Structure` do. Skips without the game and mod.

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

fn sim_of(data: &GameData) -> Sim {
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    Sim::new(data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).expect("sim")
}

fn prepare(sim: &mut Sim) -> (FactionId, PlanetId) {
    let w = sim.world().clone();
    let (f, p) = (w.faction_id("Empire").unwrap(), w.planet_id("Coruscant_WW").unwrap());
    sim.set_planet_owner(p, None);
    sim.set_planet_owner(p, Some(f));
    let mut sb = w.first_starbase[f as usize];
    for _ in 1..3 {
        sb = sb.and_then(|b| w.starbases[b as usize].next);
    }
    sim.set_starbase(p, sb);
    sim.add_credits(f, 1.0e9);
    sim.set_tech_level(f, 9);
    (f, p)
}

fn unit<'a>(w: &'a World, name: &str) -> (UnitTypeId, &'a UnitType) {
    let id = w.unit_id(name).unwrap_or_else(|| panic!("no build item {name}"));
    (id, &w.units[id as usize])
}

fn place(sim: &mut Sim, planet: PlanetId, name: &str) {
    let id = sim.world().structure_id(name).unwrap_or_else(|| panic!("no structure {name}"));
    assert!(sim.add_structure(planet, id), "{name} did not fit");
}

fn has(sim: &Sim, planet: PlanetId, name: &str) -> bool {
    let id = sim.world().structure_id(name).unwrap();
    sim.galaxy().planets[planet as usize].structures.contains(&id)
}

fn pay(sim: &mut Sim, faction: FactionId, planet: PlanetId, unit: UnitTypeId) -> f64 {
    let list = sim.world().units[unit as usize].cost;
    let quoted = sim.world().unit_cost(sim.galaxy(), faction, planet, unit).0;
    let before = sim.galaxy().factions[faction as usize].credits;
    sim.apply(Command::BuildUnit { faction, planet, unit }).unwrap_or_else(|e| panic!("queue {unit}: {e:?}"));
    let paid = before - sim.galaxy().factions[faction as usize].credits;
    eprintln!("paid {paid} list {list} quoted {quoted} for {}", sim.world().units[unit as usize].name);
    close(paid, f64::from(quoted));
    paid
}

/// Credits the completing tick added beyond this tick's income. The 500 (or other) item price was already paid at queue.
fn script_refund(sim: &mut Sim, faction: FactionId, planet: PlanetId) -> f64 {
    {
        // An orbital swap waits in the space queue, a ground one in the ground queue.
        let ps = &mut sim.galaxy_mut().planets[planet as usize];
        assert_eq!(ps.queues.iter().map(|q| q.len()).sum::<usize>(), 1, "one queued swap");
        ps.queues.iter_mut().flatten().next().expect("counted above").remaining = 0.01;
    }
    let (per_tick, before) = {
        let fs = &sim.galaxy().factions[faction as usize];
        let income = fs.income_per_week;
        let scaled = if income > 0.0 { income * f64::from(fs.difficulty.credit) } else { income };
        (scaled / f64::from(sim.world().fiscal_cycle_seconds), fs.credits)
    };
    sim.step(1.0);
    sim.galaxy().factions[faction as usize].credits - before - per_tick
}

fn close(got: f64, expect: f64) {
    assert!((got - expect).abs() < 1e-4, "got {got}, expected {expect}");
}

#[test]
fn the_data_prices_are_on_the_swap_items() {
    let Some(data) = load() else { return };
    eprintln!("upgrade scripts: {}", data.upgrade_scripts.len());
    let sim = sim_of(&data);
    let w = sim.world();
    let swaps: Vec<&UnitType> = w.units.iter().filter(|u| matches!(u.role, UnitRole::StructureSwap { .. })).collect();
    let downs = swaps.iter().filter(|u| matches!(u.role, UnitRole::StructureSwap { downgrade: true, .. })).count();
    eprintln!("structure swap items: {} downgrades: {}", swaps.len(), downs);
    assert!(swaps.len() >= 40, "parsed swaps {}", swaps.len());
    assert!(downs >= 8, "parsed downgrades {downs}");

    let (id, u) = unit(w, "UPGRADE_E_GOLAN_2_DUMMY");
    assert!(w.buildable_units[w.faction_id("Empire").unwrap() as usize].binary_search(&id).is_ok());
    assert_eq!((u.cost, u.build_time, u.tech, u.req_starbase), (48000.0, 150.0, 3, 2));
    let UnitRole::StructureSwap { from, to, downgrade, max_per_planet } = u.role else { panic!("{:?}", u.role) };
    assert!(!downgrade);
    assert_eq!(max_per_planet, Some(1));
    assert_eq!((w.structures[from as usize].name.as_str(), w.structures[to as usize].name.as_str()), ("E_Golan_1", "E_Golan_2"));

    let (_, up3) = unit(w, "UPGRADE_E_GOLAN_3_DUMMY");
    assert_eq!((up3.cost, up3.build_time, up3.tech, up3.req_starbase), (30000.0, 315.0, 4, 3));
    let (_, direct) = unit(w, "UPGRADE_E_GOLAN_3_DUMMY2");
    assert_eq!((direct.cost, direct.build_time, direct.tech, direct.req_starbase), (78000.0, 405.0, 4, 3));
    let (_, down) = unit(w, "DOWNGRADE_E_GOLAN_2_TO_1_DUMMY");
    assert_eq!((down.cost, down.build_time, down.req_starbase), (500.0, 10.0, 2));
    assert!(matches!(down.role, UnitRole::StructureSwap { downgrade: true, .. }));

    let (_, hub) = unit(w, "UPGRADE_TRADE_HUB_E");
    assert_eq!((hub.cost, hub.build_time, hub.tech), (11500.0, 60.0, 2));
    let (_, mine) = unit(w, "UPGRADE_MINING_FACILITY_E_TEMPERATE");
    assert_eq!((mine.cost, mine.build_time), (3200.0, 80.0));
    let (_, agri) = unit(w, "UPGRADE_AGRICULTURAL_COMMUNITY_E_TEMPERATE");
    assert_eq!((agri.cost, agri.build_time), (4000.0, 80.0));
    let (_, merc) = unit(w, "UPGRADE_E_MERCENARY_SETTLEMENT_DUMMY");
    assert_eq!((merc.cost, merc.build_time), (1.0, 5.0));
    let (_, citadel) = unit(w, "UPGRADE_E_MERCENARY_CITADEL_DUMMY");
    assert_eq!(citadel.cost, 1.0);

    assert_eq!(w.structures[w.structure_id("E_Golan_1").unwrap() as usize].cost, 52000.0);
    assert_eq!(w.structures[w.structure_id("E_Golan_2").unwrap() as usize].cost, 100000.0);
    assert_eq!(w.structures[w.structure_id("E_Golan_3").unwrap() as usize].cost, 130000.0);
    assert_eq!(w.structures[w.structure_id("Mining_Facility_Lv2_E_Temperate").unwrap() as usize].income_abs, 200.0);
    assert_eq!(w.structures[w.structure_id("Agricultural_Community_Lv2_E_Temperate").unwrap() as usize].income_abs, 200.0);
}

#[test]
fn golan_one_becomes_two_then_three_and_a_save_keeps_it() {
    let Some(data) = load() else { return };
    let mut sim = sim_of(&data);
    let w = sim.world().clone();
    let (empire, coruscant) = prepare(&mut sim);
    let (item, _) = unit(&w, "UPGRADE_E_GOLAN_2_DUMMY");
    assert!(w.check_build_unit(sim.galaxy(), empire, coruscant, item).is_err(), "no Golan I, so the upgrade is refused");

    place(&mut sim, coruscant, "E_GOLAN_1_SPAWNER");
    let _paid = pay(&mut sim, empire, coruscant, item);
    assert_eq!(w.units[item as usize].cost, 48000.0);
    assert!(matches!(w.check_build_unit(sim.galaxy(), empire, coruscant, item), Err(CommandError::BuildLimit)), "Build_Max_Instances_Per_Planet 1");

    close(script_refund(&mut sim, empire, coruscant), 0.0);
    assert!(has(&sim, coruscant, "E_Golan_2"), "spawner stand-in became Golan II");
    assert!(!has(&sim, coruscant, "E_GOLAN_1_SPAWNER"));
    assert!(w.check_build_unit(sim.galaxy(), empire, coruscant, item).is_err(), "Golan I is gone, so I to II is refused");

    let (next, _) = unit(&w, "UPGRADE_E_GOLAN_3_DUMMY");
    let _paid = pay(&mut sim, empire, coruscant, next);
    assert_eq!(w.units[next as usize].cost, 30000.0);
    close(script_refund(&mut sim, empire, coruscant), 0.0);
    assert!(has(&sim, coruscant, "E_Golan_3"));
    assert!(!has(&sim, coruscant, "E_Golan_2"));

    let snap = serde_json::to_string(&sim.snapshot()).unwrap();
    let snap: SimSnapshot = serde_json::from_str(&snap).unwrap();
    let mut loaded = sim_of(&data);
    loaded.restore(snap).expect("restore");
    assert!(has(&loaded, coruscant, "E_Golan_3"), "the save keeps Golan III");
}

#[test]
fn a_downgrade_refunds_half_the_cost_gap_and_a_missing_upgrade_refunds_eighty_percent() {
    let Some(data) = load() else { return };
    let mut sim = sim_of(&data);
    let w = sim.world().clone();
    let (empire, coruscant) = prepare(&mut sim);
    let from_cost = f64::from(w.structures[w.structure_id("E_Golan_1").unwrap() as usize].cost);
    let to_cost = f64::from(w.structures[w.structure_id("E_Golan_2").unwrap() as usize].cost);
    assert_eq!((from_cost, to_cost), (52000.0, 100000.0));

    place(&mut sim, coruscant, "E_GOLAN_1_SPAWNER");
    let (item, _) = unit(&w, "UPGRADE_E_GOLAN_2_DUMMY");
    let _ = pay(&mut sim, empire, coruscant, item);
    assert!(sim.remove_structure(coruscant, w.structure_id("E_GOLAN_1_SPAWNER").unwrap()));
    close(script_refund(&mut sim, empire, coruscant), (to_cost - from_cost) * 0.8);
    assert!(!has(&sim, coruscant, "E_Golan_2"), "nothing was there to replace");

    place(&mut sim, coruscant, "E_Golan_2");
    let (down, _) = unit(&w, "DOWNGRADE_E_GOLAN_2_TO_1_DUMMY");
    assert_eq!(w.units[down as usize].cost, 500.0);
    let _paid = pay(&mut sim, empire, coruscant, down);
    close(script_refund(&mut sim, empire, coruscant), (to_cost - from_cost) * 0.5 + 1.0);
    assert!(has(&sim, coruscant, "E_Golan_1"));
    assert!(!has(&sim, coruscant, "E_Golan_2"));
}

#[test]
fn trade_hub_mining_agriculture_and_the_mercenary_chain() {
    let Some(data) = load() else { return };
    let mut sim = sim_of(&data);
    let w = sim.world().clone();
    let (empire, coruscant) = prepare(&mut sim);

    place(&mut sim, coruscant, "E_Trade_Hub");
    let (hub, _) = unit(&w, "UPGRADE_TRADE_HUB_E");
    assert_eq!(w.units[hub as usize].cost, 11500.0);
    let _paid = pay(&mut sim, empire, coruscant, hub);
    close(script_refund(&mut sim, empire, coruscant), 0.0);
    assert!(has(&sim, coruscant, "E_Trade_Hub_Lv2"));
    assert!(!has(&sim, coruscant, "E_Trade_Hub"));

    place(&mut sim, coruscant, "Mining_Facility_E_Temperate");
    place(&mut sim, coruscant, "Mining_Company_Lv2_E_Temperate");
    let (mine, _) = unit(&w, "UPGRADE_MINING_FACILITY_E_TEMPERATE");
    assert!(w.check_build_unit(sim.galaxy(), empire, coruscant, mine).is_ok());
    assert_eq!(w.units[mine as usize].cost, 3200.0);
    let _paid = pay(&mut sim, empire, coruscant, mine);
    close(script_refund(&mut sim, empire, coruscant), 0.0);
    assert!(has(&sim, coruscant, "Mining_Facility_Lv2_E_Temperate"));
    assert!(!has(&sim, coruscant, "Mining_Facility_E_Temperate"));
    assert_eq!(w.structures[w.structure_id("Mining_Facility_Lv2_E_Temperate").unwrap() as usize].income_abs, 200.0);

    place(&mut sim, coruscant, "Agricultural_Community_E_Temperate");
    let (agri, _) = unit(&w, "UPGRADE_AGRICULTURAL_COMMUNITY_E_TEMPERATE");
    assert_eq!(w.units[agri as usize].cost, 4000.0);
    let _paid = pay(&mut sim, empire, coruscant, agri);
    close(script_refund(&mut sim, empire, coruscant), 0.0);
    assert!(has(&sim, coruscant, "Agricultural_Community_Lv2_E_Temperate"));
    assert_eq!(w.structures[w.structure_id("Agricultural_Community_Lv2_E_Temperate").unwrap() as usize].income_abs, 200.0);

    place(&mut sim, coruscant, "E_Mercenary_Outpost");
    let (settlement, _) = unit(&w, "UPGRADE_E_MERCENARY_SETTLEMENT_DUMMY");
    assert_eq!(w.units[settlement as usize].cost, 1.0);
    let _paid = pay(&mut sim, empire, coruscant, settlement);
    close(script_refund(&mut sim, empire, coruscant), 0.0);
    assert!(has(&sim, coruscant, "E_Mercenary_Settlement"));
    let (citadel, _) = unit(&w, "UPGRADE_E_MERCENARY_CITADEL_DUMMY");
    assert_eq!(w.units[citadel as usize].cost, 1.0);
    let _paid = pay(&mut sim, empire, coruscant, citadel);
    close(script_refund(&mut sim, empire, coruscant), 0.0);
    assert!(has(&sim, coruscant, "E_Mercenary_Citadel"));
    assert!(!has(&sim, coruscant, "E_Mercenary_Settlement"));
}
