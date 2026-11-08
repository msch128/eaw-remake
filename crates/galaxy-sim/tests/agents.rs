//! Non-combat agents (smuggler, underboss, field agent) on the Remake data: recruitment, ground base level, siphon, spy report, no battles.

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn load() -> Option<GameData> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    Some(load_galaxy(&fs).0)
}

fn rebel_sim(data: &GameData) -> Sim {
    Sim::new(data, data.campaign("GC_Full_Web_Way_Rebel").unwrap(), SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap()
}

fn fleet(sim: &mut Sim, owner: FactionId, units: &[(UnitTypeId, u32)], location: FleetLocation) -> FleetId {
    let g = sim.galaxy_mut();
    let id = g.fleets.keys().max().copied().unwrap_or(0) + 1;
    g.fleets.insert(id, Fleet { id, owner, units: units.iter().copied().collect(), location, previous_planet: 0, slot: 0 });
    g.sync_ships();
    id
}

/// An Empire planet with a star base (it would fight any hostile warship).
fn empire_base(sim: &Sim, empire: FactionId) -> PlanetId {
    let g = sim.galaxy();
    g.owned_planets(empire).find(|&p| g.planets[p as usize].starbase.is_some() && sim.world().planet_income(g, p) > 100.0).expect("an Empire planet with a star base")
}

#[test]
fn agents_come_from_the_non_combat_hero_teams_with_the_data_numbers() {
    let Some(data) = load() else { return };
    let sim = rebel_sim(&data);
    let w = sim.world().clone();
    let agents: Vec<&str> = w.units.iter().filter(|u| u.role == UnitRole::Agent).map(|u| u.name.as_str()).collect();
    for name in ["Rebel_Smuggler_Team", "Underboss_Team", "Rebel_Spy_Team"] {
        assert!(agents.contains(&name), "{name} in {agents:?}");
    }
    assert!(!agents.contains(&"Boba_Fett_Team"), "combat hero teams are no agents");
    // Units/Ground/Non-Combat_Heroes.xml: 500 credits, 60 s, tech 0, population 1, ground base level 1, star base level 0;
    // Units/Ground/Transports.xml: Alliance_Shuttle and Shuttle_Tyderium inherit Hyperspace_Speed 0.75 from Transport_Base.
    for name in ["Rebel_Smuggler_Team", "Underboss_Team", "Rebel_Spy_Team"] {
        let u = &w.units[w.unit_id(name).unwrap() as usize];
        assert_eq!((u.cost, u.build_time, u.tech, u.population, u.req_ground_base, u.req_starbase), (500.0, 60.0, 0, 1, 1, 0), "{name}");
        assert_eq!(u.hyperspace_speed, 0.75, "{name}");
        assert!(u.galactic_stealth && !u.hero && u.combat.durability() == 0.0, "{name}");
    }
    let smuggler = &w.units[w.unit_id("Rebel_Smuggler_Team").unwrap() as usize];
    assert_eq!(smuggler.siphon, Some(SiphonAbility { min: 0.20, max: 0.30, cycles: 5 }));
    assert_eq!(smuggler.affiliation, ["rebel"]);
    assert_eq!(w.units[w.unit_id("Underboss_Team").unwrap() as usize].affiliation, ["underworld"]);
    let spy = w.units[w.unit_id("Rebel_Spy_Team").unwrap() as usize].spy.expect("spy ability");
    assert!(spy.despawn && spy.duration == 60.0);
    assert!(spy.see.base_levels && spy.see.credit_income && spy.see.current_production && spy.see.fleet_contents && !spy.see.minor_stealth);
    // Gameconstants.xml `Fiscal_Cycle_Time_In_Secs` and the GroundBases.xml levels.
    assert_eq!((w.fiscal_cycle_seconds, w.ground_base_max), (45.0, 5));
}

#[test]
fn a_smuggler_is_recruited_at_a_rebel_planet_and_joins_a_fleet() {
    let Some(data) = load() else { return };
    let mut sim = rebel_sim(&data);
    let w = sim.world().clone();
    let rebel = w.faction_id("Rebel").unwrap();
    let yavin = w.planet_id("Yavin_WW").unwrap();
    let smuggler = w.unit_id("Rebel_Smuggler_Team").unwrap();
    sim.galaxy_mut().factions[rebel as usize].credits = 10_000.0;
    assert!(w.buildable_units[rebel as usize].contains(&smuggler));
    assert_eq!(w.check_build_unit(sim.galaxy(), rebel, yavin, smuggler), Ok(()));
    sim.apply(Command::BuildUnit { faction: rebel, planet: yavin, unit: smuggler }).unwrap();
    assert_eq!(sim.galaxy().factions[rebel as usize].credits, 9_500.0);
    sim.step(59.0);
    assert!(!sim.galaxy().fleets.values().any(|f| f.units.contains_key(&smuggler)));
    sim.step(2.0);
    let f = sim.galaxy().fleets.values().find(|f| f.units.contains_key(&smuggler)).expect("the smuggler is in a fleet");
    assert_eq!((f.owner, f.location.clone()), (rebel, FleetLocation::AtPlanet(yavin)));
    // Population 1 counts.
    let used = sim.galaxy().factions[rebel as usize].population_used;
    sim.galaxy_mut().fleets.values_mut().for_each(|f| {
        f.units.remove(&smuggler);
    });
    sim.step(1.0);
    assert_eq!(sim.galaxy().factions[rebel as usize].population_used, used - 1);
}

#[test]
fn agents_need_ground_base_level_1_and_their_structure() {
    let Some(data) = load() else { return };
    let mut sim = rebel_sim(&data);
    let w = sim.world().clone();
    let rebel = w.faction_id("Rebel").unwrap();
    let yavin = w.planet_id("Yavin_WW").unwrap();
    let smuggler = w.unit_id("Rebel_Smuggler_Team").unwrap();
    let spy = w.unit_id("Rebel_Spy_Team").unwrap();
    sim.galaxy_mut().factions[rebel as usize].credits = 10_000.0;
    // GroundBases.xml: the ground base level follows the number of ground structures; orbital ones do not count.
    let ground = sim.galaxy().planets[yavin as usize].structures.iter().filter(|&&s| !w.structures[s as usize].orbital).count() as i32;
    assert_eq!(w.ground_base_level(sim.galaxy(), yavin), ground.min(5));
    assert!(ground >= 1);
    // The field agent needs the Spy_Center, which only the `Neutral` affiliation may build in the Remake (`<!--Rebel-->,Neutral`).
    let center = w.structure_id("Spy_Center").unwrap();
    assert!(!w.buildable_structures[rebel as usize].contains(&center));
    assert_eq!(w.check_build_unit(sim.galaxy(), rebel, yavin, spy), Err(CommandError::RequirementsNotMet("Spy_Center".into())));
    sim.galaxy_mut().planets[yavin as usize].structures.push(center);
    assert_eq!(w.check_build_unit(sim.galaxy(), rebel, yavin, spy), Ok(()));
    // Without any ground structure the planet has ground base level 0: refused for the level and for the outpost.
    sim.galaxy_mut().planets[yavin as usize].structures.retain(|&s| w.structures[s as usize].orbital);
    assert_eq!(w.ground_base_level(sim.galaxy(), yavin), 0);
    assert_eq!(w.check_build_unit(sim.galaxy(), rebel, yavin, smuggler), Err(CommandError::RequirementsNotMet("ground base level 1".into())));
    let unmet = w.unmet_for_unit(sim.galaxy(), rebel, yavin, smuggler);
    assert!(unmet.contains(&Unmet::GroundBaseLevel(1)) && unmet.iter().any(|u| matches!(u, Unmet::Structures(_))), "{unmet:?}");
}

#[test]
fn an_agent_fleet_starts_no_battle_captures_nothing_and_blocks_nobody() {
    let Some(data) = load() else { return };
    let mut sim = rebel_sim(&data);
    let w = sim.world().clone();
    let (rebel, empire) = (w.faction_id("Rebel").unwrap(), w.faction_id("Empire").unwrap());
    let target = empire_base(&sim, empire);
    let smuggler = w.unit_id("Rebel_Smuggler_Team").unwrap();
    sim.galaxy_mut().planets[target as usize].starbase = None;
    let agent = fleet(&mut sim, rebel, &[(smuggler, 1)], FleetLocation::AtPlanet(target));
    let events = sim.step(20.0);
    assert!(!events.iter().any(|e| matches!(e, Event::BattlePending(b) if b.planet == target)), "{events:?}");
    assert_eq!(sim.galaxy().planets[target as usize].owner, Some(empire), "no capture by agents (capture delay {} s)", w.config.capture_delay_seconds);
    assert!(sim.galaxy().fleets.contains_key(&agent));
    // An Empire fleet passing its own planet is not stopped by the hostile agent there.
    assert!(!w.blockades(sim.galaxy(), target, empire));
    // With a warship the fleet fights, the agent does not: it is no member of the battle and survives it.
    let xwing = w.unit_id("X-Wing_Squadron").or_else(|| w.unit_id("X-wing_Squadron")).expect("an X-wing squadron");
    sim.galaxy_mut().fleets.get_mut(&agent).unwrap().units.insert(xwing, 1);
    let isd = w.unit_id("Star_Destroyer").unwrap();
    fleet(&mut sim, empire, &[(isd, 3)], FleetLocation::AtPlanet(target));
    let events = sim.step(1.0);
    assert!(events.iter().any(|e| matches!(e, Event::BattleResolved { planet, .. } if *planet == target)), "{events:?}");
    let left: u32 = sim.galaxy().fleets.values().filter(|f| f.owner == rebel).map(|f| f.units.get(&smuggler).copied().unwrap_or(0)).sum();
    assert_eq!(left, 1, "the smuggler is not lost with the squadron");
}

#[test]
fn a_smuggler_siphons_20_to_30_percent_of_the_planet_income_for_5_fiscal_cycles() {
    let Some(data) = load() else { return };
    let mut sim = rebel_sim(&data);
    let w = sim.world().clone();
    let (rebel, empire) = (w.faction_id("Rebel").unwrap(), w.faction_id("Empire").unwrap());
    let target = empire_base(&sim, empire);
    let smuggler = w.unit_id("Rebel_Smuggler_Team").unwrap();
    let yavin = w.planet_id("Yavin_WW").unwrap();
    // Not at an own planet, not in hyperspace.
    let home = fleet(&mut sim, rebel, &[(smuggler, 1)], FleetLocation::AtPlanet(yavin));
    assert_eq!(sim.apply(Command::ActivateAgent { faction: rebel, fleet: home }), Err(CommandError::RequirementsNotMet("an enemy planet".into())));
    assert_eq!(sim.apply(Command::ActivateAgent { faction: empire, fleet: home }), Err(CommandError::Invalid));
    let agent = fleet(&mut sim, rebel, &[(smuggler, 1)], FleetLocation::AtPlanet(target));
    sim.step(1.0);
    let (empire_before, rebel_before) = (sim.galaxy().factions[empire as usize].income_per_week, sim.galaxy().factions[rebel as usize].income_per_week);
    let income = f64::from(w.planet_income(sim.galaxy(), target));
    assert_eq!(sim.apply(Command::ActivateAgent { faction: rebel, fleet: agent }), Ok(CommandOk::Done));
    assert!(!sim.galaxy().fleets.contains_key(&agent), "Causes_Despawn: the smuggler is used up");
    let s = sim.galaxy().planets[target as usize].siphon.expect("a siphon runs");
    assert!((0.20..=0.30).contains(&s.share) && s.cycles_left == 5 && s.by == rebel, "{s:?}");
    let taken = income * f64::from(s.share);
    let fs = &sim.galaxy().factions;
    assert!((empire_before - fs[empire as usize].income_per_week - taken * f64::from(w.factions[empire as usize].credits_factor)).abs() < 1e-3);
    assert!((fs[rebel as usize].income_per_week - rebel_before - taken * f64::from(w.factions[rebel as usize].credits_factor)).abs() < 1e-3);
    // One siphon per planet.
    let second = fleet(&mut sim, rebel, &[(smuggler, 1)], FleetLocation::AtPlanet(target));
    assert_eq!(sim.apply(Command::ActivateAgent { faction: rebel, fleet: second }), Err(CommandError::RequirementsNotMet("no other siphon at the planet".into())));
    sim.galaxy_mut().fleets.remove(&second);
    // A new share each cycle, all within the data range; over after 5 x 45 s.
    let mut shares = vec![s.share];
    for _ in 0..4 {
        sim.step(45.0);
        let s = sim.galaxy().planets[target as usize].siphon.expect("still running");
        assert!((0.20..=0.30).contains(&s.share));
        shares.push(s.share);
    }
    assert_eq!(sim.galaxy().planets[target as usize].siphon.unwrap().cycles_left, 1);
    sim.step(45.0);
    assert!(sim.galaxy().planets[target as usize].siphon.is_none(), "5 cycles of 45 s are over");
    assert!(shares.windows(2).any(|w| w[0] != w[1]), "the share is drawn again each cycle: {shares:?}");
}

#[test]
fn a_siphon_ends_when_the_planet_is_no_longer_an_enemys() {
    let Some(data) = load() else { return };
    let mut sim = rebel_sim(&data);
    let w = sim.world().clone();
    let (rebel, empire) = (w.faction_id("Rebel").unwrap(), w.faction_id("Empire").unwrap());
    let target = empire_base(&sim, empire);
    let smuggler = w.unit_id("Rebel_Smuggler_Team").unwrap();
    let agent = fleet(&mut sim, rebel, &[(smuggler, 1)], FleetLocation::AtPlanet(target));
    sim.apply(Command::ActivateAgent { faction: rebel, fleet: agent }).unwrap();
    sim.galaxy_mut().planets[target as usize].owner = Some(rebel);
    sim.step(1.0);
    assert!(sim.galaxy().planets[target as usize].siphon.is_none());
}

#[test]
fn the_siphon_share_is_deterministic_and_survives_save_and_load() {
    let Some(data) = load() else { return };
    let run = |data: &GameData| {
        let mut sim = rebel_sim(data);
        let w = sim.world().clone();
        let (rebel, empire) = (w.faction_id("Rebel").unwrap(), w.faction_id("Empire").unwrap());
        let target = empire_base(&sim, empire);
        let agent = fleet(&mut sim, rebel, &[(w.unit_id("Rebel_Smuggler_Team").unwrap(), 1)], FleetLocation::AtPlanet(target));
        sim.apply(Command::ActivateAgent { faction: rebel, fleet: agent }).unwrap();
        sim.step(50.0);
        (sim, target)
    };
    let (mut a, target) = run(&data);
    let (b, _) = run(&data);
    assert_eq!(a.galaxy().planets[target as usize].siphon, b.galaxy().planets[target as usize].siphon);
    assert_eq!(a.galaxy().state_hash(), b.galaxy().state_hash());

    let json = serde_json::to_string(&a.snapshot().galaxy).unwrap();
    let galaxy: Galaxy = serde_json::from_str(&json).unwrap();
    let mut c = rebel_sim(&data);
    let mut snap = c.snapshot();
    snap.galaxy = galaxy;
    c.restore(snap).unwrap();
    assert_eq!(c.galaxy().planets[target as usize].siphon, a.galaxy().planets[target as usize].siphon);
    a.step(100.0);
    c.step(100.0);
    assert_eq!(c.galaxy().planets[target as usize].siphon, a.galaxy().planets[target as usize].siphon);
    assert_eq!(c.galaxy().state_hash(), a.galaxy().state_hash());
    // An old save without the new fields still loads.
    let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
    v.as_object_mut().unwrap().remove("spy_reports");
    for p in v["planets"].as_array_mut().unwrap() {
        p.as_object_mut().unwrap().remove("siphon");
    }
    let old: Galaxy = serde_json::from_value(v).unwrap();
    assert!(old.spy_reports.is_empty() && old.planets.iter().all(|p| p.siphon.is_none()));
}

#[test]
fn a_field_agent_reports_on_the_hostile_planet_it_reaches_for_60_seconds() {
    let Some(data) = load() else { return };
    let mut sim = rebel_sim(&data);
    let w = sim.world().clone();
    let (rebel, empire) = (w.faction_id("Rebel").unwrap(), w.faction_id("Empire").unwrap());
    let spy = w.unit_id("Rebel_Spy_Team").unwrap();
    // A far Empire planet, hidden to the Rebels.
    let map = sim.galaxy().sight_map(&w, rebel);
    let target = sim.galaxy().owned_planets(empire).find(|&p| map[p as usize] == Sight::Hidden).expect("a hidden Empire planet");
    let next = w.adjacency[target as usize][0].0;
    let route = w.route_between(next, target).unwrap();
    let agent = fleet(&mut sim, rebel, &[(spy, 1)], FleetLocation::Moving { from: next, to: target, route, progress: w.routes[route as usize].length - 0.01, rest: Vec::new() });
    let events = sim.step(1.0);
    assert!(events.iter().any(|e| matches!(e, Event::AgentUsed { faction, planet, unit } if *faction == rebel && *planet == target && *unit == spy)), "{events:?}");
    assert!(!sim.galaxy().fleets.contains_key(&agent), "Causes_Despawn: the agent is used up");
    let r = sim.galaxy().spy_reports.iter().find(|r| r.planet == target).copied().expect("a report");
    assert_eq!(r.faction, rebel);
    assert!((r.until - sim.galaxy().time - 60.0).abs() <= 1.0, "Duration_In_Secs 60: {r:?}");
    assert_eq!(sim.galaxy().planet_sight(&w, rebel, target), Sight::Detailed);
    sim.step(60.0);
    assert!(sim.galaxy().spy_reports.is_empty());
    assert_eq!(sim.galaxy().planet_sight(&w, rebel, target), Sight::Hidden);
}

#[test]
fn a_faction_with_only_agents_left_is_eliminated() {
    let Some(data) = load() else { return };
    let mut sim = rebel_sim(&data);
    let w = sim.world().clone();
    let rebel = w.faction_id("Rebel").unwrap();
    let smuggler = w.unit_id("Rebel_Smuggler_Team").unwrap();
    let yavin = w.planet_id("Yavin_WW").unwrap();
    let g = sim.galaxy_mut();
    g.fleets.retain(|_, f| f.owner != rebel);
    for p in g.planets.iter_mut().filter(|p| p.owner == Some(rebel)) {
        p.owner = None;
    }
    fleet(&mut sim, rebel, &[(smuggler, 1)], FleetLocation::AtPlanet(yavin));
    let events = sim.step(1.0);
    assert!(events.iter().any(|e| matches!(e, Event::FactionEliminated { faction } if *faction == rebel)));
    assert!(!sim.galaxy().fleets.values().any(|f| f.owner == rebel));
}

#[test]
fn the_underboss_needs_a_cantina_or_criminal_palace() {
    let Some(data) = load() else { return };
    let sim = Sim::new(&data, data.campaign("GC_Full_Web_Way_Underworld").unwrap(), SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let underworld = w.faction_id("Underworld").unwrap();
    let boss = w.unit_id("Underboss_Team").unwrap();
    let at = |name: &str| w.check_build_unit(sim.galaxy(), underworld, w.planet_id(name).unwrap(), boss);
    // The start structures of the campaign: Socorro and Tatooine have a cantina, Teth has none.
    assert_eq!(at("Socorro_WW"), Ok(()));
    assert_eq!(at("Tatooine_WW"), Ok(()));
    assert!(matches!(at("Teth_WW"), Err(CommandError::RequirementsNotMet(r)) if r.contains("Cantina_Temperate")));
    // The Rebels cannot recruit it at all.
    let rebel = w.faction_id("Rebel").unwrap();
    assert!(!w.buildable_units[rebel as usize].contains(&boss));
}

fn dist(w: &World, a: PlanetId, b: PlanetId) -> f32 {
    let (p, q) = (w.planets[a as usize].position, w.planets[b as usize].position);
    ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt()
}

/// A real planet owned by `owner` with another real `owner` planet in `(0, range]` and a third beyond `range` (but within twice of it).
fn spy_layout(sim: &Sim, owner: FactionId, range: f32) -> Option<(PlanetId, PlanetId, PlanetId)> {
    let w = sim.world();
    let mine: Vec<PlanetId> = sim.galaxy().owned_planets(owner).filter(|&p| w.real[p as usize]).collect();
    mine.iter().find_map(|&a| {
        let near = mine.iter().copied().find(|&b| b != a && dist(w, a, b) <= range)?;
        let far = mine.iter().copied().find(|&c| dist(w, a, c) > range && dist(w, a, c) < 2.0 * range)?;
        Some((a, near, far))
    })
}

#[test]
fn a_spy_unit_reveals_what_its_flags_name_within_its_galactic_influence_range() {
    let Some(data) = load() else { return };
    let mut sim = rebel_sim(&data);
    let w = sim.world().clone();
    let (rebel, empire) = (w.faction_id("Rebel").unwrap(), w.faction_id("Empire").unwrap());
    sim.galaxy_mut().fleets.clear();
    // MTC_Spy_R (Galactic_Influence_Range 180, Hyperspace_Fleet_Reveal_Range 180, every See_* flag).
    let mtc = w.unit_id("MTC_Spy_R").unwrap();
    let spy = w.units[mtc as usize].spy.unwrap();
    assert_eq!((spy.range, spy.fleet_reveal_range), (180.0, 180.0));
    let (at, near, far) = spy_layout(&sim, empire, 180.0).expect("Empire planets 180 apart");
    let before = sim.galaxy().intel_map(&w, rebel);
    assert!(!before[near as usize].any() && sim.galaxy().planet_sight(&w, rebel, near) != Sight::Detailed);
    fleet(&mut sim, rebel, &[(mtc, 1)], FleetLocation::AtPlanet(at));
    let intel = sim.galaxy().intel_map(&w, rebel);
    assert_eq!(intel[near as usize], spy.see, "within 180: {} map units", dist(&w, at, near));
    assert!(intel[near as usize].credit_income && intel[near as usize].base_levels && intel[near as usize].current_production);
    assert!(!intel[far as usize].any(), "beyond 180: {} map units", dist(&w, at, far));
    assert_eq!(sim.galaxy().planet_sight(&w, rebel, near), Sight::Detailed);
    // An Empire fleet at the near planet shows its contents.
    let isd = w.unit_id("Star_Destroyer").unwrap();
    let enemy = fleet(&mut sim, empire, &[(isd, 1)], FleetLocation::AtPlanet(near));
    assert_eq!(sim.galaxy().fleet_sight(&w, rebel, enemy), Sight::Detailed);
    // Own planets are known in full, without spies.
    let yavin = w.planet_id("Yavin_WW").unwrap();
    assert_eq!(intel[yavin as usize], SpyFlags::ALL);
}

#[test]
fn dodonna_sees_the_number_of_fleets_but_not_their_contents_or_stealthy_heroes() {
    let Some(data) = load() else { return };
    let mut sim = rebel_sim(&data);
    let w = sim.world().clone();
    let (rebel, empire) = (w.faction_id("Rebel").unwrap(), w.faction_id("Empire").unwrap());
    sim.galaxy_mut().fleets.clear();
    // Dodonna_Ship_Reveal: See_Num_Fleets Yes, See_Fleet_Contents No, no stealth flags; Galactic_Influence_Range 50.
    let dodonna = w.unit_id("Dodonna_Quasar").unwrap();
    let spy = w.units[dodonna as usize].spy.unwrap();
    assert!(spy.see.num_fleets && !spy.see.fleet_contents && !spy.see.major_stealth && spy.range == 50.0);
    let Some((at, near, _)) = spy_layout(&sim, empire, 50.0) else {
        eprintln!("no Empire planets 50 map units apart; range check skipped");
        return;
    };
    fleet(&mut sim, rebel, &[(dodonna, 1)], FleetLocation::AtPlanet(at));
    assert_eq!(sim.galaxy().planet_sight(&w, rebel, near), Sight::Seen);
    let isd = w.unit_id("Star_Destroyer").unwrap();
    let plain = fleet(&mut sim, empire, &[(isd, 1)], FleetLocation::AtPlanet(near));
    assert_eq!(sim.galaxy().fleet_sight(&w, rebel, plain), Sight::Seen);
    let stealthy = (0..w.units.len() as u32).find(|&u| w.units[u as usize].galactic_stealth && w.units[u as usize].role == UnitRole::Ship).unwrap();
    let hidden = fleet(&mut sim, empire, &[(stealthy, 1)], FleetLocation::AtPlanet(near));
    assert_eq!(sim.galaxy().fleet_sight(&w, rebel, hidden), Sight::Hidden);
}

#[test]
fn a_spy_center_spies_350_around_and_reveals_hyperspace_fleets_and_agents() {
    let Some(data) = load() else { return };
    let mut sim = rebel_sim(&data);
    let w = sim.world().clone();
    let (rebel, empire) = (w.faction_id("Rebel").unwrap(), w.faction_id("Empire").unwrap());
    sim.galaxy_mut().fleets.clear();
    // Buildings/Ground/GC/Rebel/Spy_Center.xml: Galactic_Influence_Range 350, Hyperspace_Fleet_Reveal_Range 350, every See_* flag.
    let center = w.structure_id("Spy_Center").unwrap();
    let spy = w.structures[center as usize].spy.unwrap();
    assert_eq!((spy.range, spy.fleet_reveal_range), (350.0, 350.0));
    assert!(spy.see.minor_stealth && spy.see.major_stealth && spy.see.current_production);
    // The center on a Rebel planet next to an Empire planet.
    let (host, target) = (0..w.planets.len() as u32)
        .filter(|&p| w.real[p as usize] && sim.galaxy().planets[p as usize].owner == Some(empire))
        .find_map(|t| w.adjacency[t as usize].iter().map(|&(n, _)| n).find(|&n| sim.galaxy().planets[n as usize].owner == Some(rebel) && dist(&w, n, t) <= 350.0).map(|h| (h, t)))
        .expect("a Rebel planet within 350 of an Empire planet");
    sim.galaxy_mut().planets[host as usize].structures.push(center);
    let intel = sim.galaxy().intel_map(&w, rebel);
    assert_eq!(intel[target as usize], spy.see, "{} map units", dist(&w, host, target));
    // A stealthy minor hero (an Underworld agent) at the target is found: See_Minor_Stealth_Heroes.
    let underworld = w.faction_id("Underworld").unwrap();
    let boss = w.unit_id("Underboss_Team").unwrap();
    let agent = fleet(&mut sim, underworld, &[(boss, 1)], FleetLocation::AtPlanet(target));
    assert_eq!(sim.galaxy().fleet_sight(&w, rebel, agent), Sight::Detailed);
    // A fleet in hyperspace just off the center's planet is revealed with its contents.
    let route = w.adjacency[host as usize][0].1;
    let r = &w.routes[route as usize];
    let (from, to) = if r.a == host { (r.a, r.b) } else { (r.b, r.a) };
    let isd = w.unit_id("Star_Destroyer").unwrap();
    let moving = fleet(&mut sim, empire, &[(isd, 1)], FleetLocation::Moving { from, to, route, progress: 1.0, rest: Vec::new() });
    assert_eq!(sim.galaxy().fleet_sight(&w, rebel, moving), Sight::Detailed);
    // Without the center the lane rule alone applies: the Rebel planet is one end, so only "Seen".
    sim.galaxy_mut().planets[host as usize].structures.retain(|&s| s != center);
    assert_eq!(sim.galaxy().fleet_sight(&w, rebel, moving), Sight::Seen);
    assert_eq!(sim.galaxy().fleet_sight(&w, rebel, agent), Sight::Hidden);
}

#[test]
fn a_field_agent_report_carries_its_flags_and_is_saved() {
    let Some(data) = load() else { return };
    let mut sim = rebel_sim(&data);
    let w = sim.world().clone();
    let (rebel, empire) = (w.faction_id("Rebel").unwrap(), w.faction_id("Empire").unwrap());
    let spy = w.unit_id("Rebel_Spy_Team").unwrap();
    let target = empire_base(&sim, empire);
    fleet(&mut sim, rebel, &[(spy, 1)], FleetLocation::AtPlanet(target));
    sim.step(1.0);
    let see = w.units[spy as usize].spy.unwrap().see;
    let intel = sim.galaxy().intel_map(&w, rebel)[target as usize];
    assert_eq!(intel.union(see), intel);
    assert!(intel.current_production && intel.credit_income && intel.base_levels);
    // The Empire's own view of its planet is unchanged.
    assert_eq!(sim.galaxy().intel_map(&w, empire)[target as usize], SpyFlags::ALL);
    let json = serde_json::to_string(&sim.snapshot().galaxy).unwrap();
    let mut other = rebel_sim(&data);
    let mut snap = other.snapshot();
    snap.galaxy = serde_json::from_str(&json).unwrap();
    other.restore(snap).unwrap();
    assert_eq!(other.galaxy().spy_reports, sim.galaxy().spy_reports);
    assert_eq!(other.galaxy().intel_map(&w, rebel), sim.galaxy().intel_map(&w, rebel));
    other.step(60.0);
    assert!(other.galaxy().spy_reports.is_empty());
}
