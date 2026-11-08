//! `Rebel_Shield_Upgrade` is on the Rebel space build tab. Finishing it notifies `STORY_CONSTRUCT` and grants the reflector shields.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_play::{Availability, BuildMenu, BuildTarget, PlayCatalog};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::cheats::Cheat;
use galaxy_sim::{Command, CommandError, SimConfig, UnitRole};

fn open() -> Option<(ScriptedSim, Arc<alamo_data::GameData>)> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
    let (data, _) = load_galaxy(&*fs);
    let data = Arc::new(data);
    let campaign = data.campaign("GC_Full_Web_Way_Rebel")?.clone();
    let sim = ScriptedSim::new(fs, data.clone(), &campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }, ScriptedConfig { remake_ai: false, ..ScriptedConfig::default() }).ok()?;
    Some((sim, data))
}

#[test]
fn rebel_shield_upgrade_is_offered_at_tech_4_and_grants_the_shields() {
    let Some((mut sim, data)) = open() else { return };
    let w = sim.with_sim(|s| s.world().clone());
    let names = PlayCatalog::from_data(&w, &data);
    let (rebel, empire) = (w.faction_id("Rebel").unwrap(), w.faction_id("Empire").unwrap());
    let item = w.unit_id("Rebel_Shield_Upgrade").expect("the upgrade is in the catalog");
    let ut = &w.units[item as usize];
    assert_eq!(ut.role, UnitRole::Dummy);
    assert_eq!(ut.cost, 75_000.0);
    assert_eq!(ut.build_time, 600.0);
    assert_eq!(ut.tech, 4);
    assert_eq!(ut.req_starbase, 1);
    assert_eq!(ut.population, 0);
    assert_eq!(ut.limit_current, Some(1));
    assert_eq!(ut.limit_lifetime, Some(1));
    assert!(ut.affiliation.iter().any(|a| a == "rebel"));
    assert!(w.buildable_units[rebel as usize].binary_search(&item).is_ok());
    assert!(w.buildable_units[empire as usize].binary_search(&item).is_err(), "Empire does not build it");

    let planet = w.planet_id("Coruscant_WW").unwrap();
    sim.with_sim_mut(|s| {
        s.set_planet_owner(planet, None);
        s.set_planet_owner(planet, Some(rebel));
        s.set_starbase(planet, w.first_starbase[rebel as usize]);
        s.add_credits(rebel, 1.0e7);
        s.set_tech_level(rebel, 3);
    });
    let hidden = sim.with_sim(|s| BuildMenu::for_planet(&w, s.galaxy(), &names, rebel, planet).unwrap());
    assert!(hidden.options.iter().all(|o| o.target != BuildTarget::Unit(item)), "tech 3 hides it");

    sim.with_sim_mut(|s| {
        s.set_tech_level(rebel, 4);
    });
    let listed = sim.with_sim(|s| BuildMenu::for_planet(&w, s.galaxy(), &names, rebel, planet).unwrap());
    let entry = listed.options.iter().find(|o| o.target == BuildTarget::Unit(item)).expect("tech 4 offers it");
    assert_eq!(entry.cost, 75_000.0);
    assert!((entry.build_seconds - 600.0 * w.config.build_time_scale).abs() < 0.1);
    assert!(matches!(entry.availability, Availability::Requires(_)), "the research institute is still missing: {:?}", entry.availability);

    let institute = w.structure_id("R_Research_Institute").unwrap();
    sim.with_sim_mut(|s| {
        assert!(s.add_structure(planet, institute), "Coruscant takes the research institute");
    });
    let ready = sim.with_sim(|s| BuildMenu::for_planet(&w, s.galaxy(), &names, rebel, planet).unwrap());
    let entry = ready.options.iter().find(|o| o.target == BuildTarget::Unit(item)).unwrap();
    assert_eq!(entry.availability, Availability::Available);

    assert!(!sim.with_sim(|s| s.has_power(rebel, "CR-90_Corvette", "Reflector_Shield")));
    sim.with_sim_mut(|s| {
        s.apply(Command::BuildUnit { faction: rebel, planet, unit: item }).expect("queued");
        s.apply(Command::Cheat { faction: rebel, cheat: Cheat::FinishProduction { planet } }).unwrap();
    });
    sim.step(1.0);
    assert!(sim.with_sim(|s| s.has_power(rebel, "CR-90_Corvette", "Reflector_Shield")));
    assert!(sim.with_sim(|s| s.has_power(rebel, "DP-20_Corvette", "Reflector_Shield")));
    assert!(!sim.with_sim(|s| s.has_power(rebel, "X-Wing", "Reflector_Shield")));
    assert!(!sim.with_sim(|s| s.has_power(empire, "CR-90_Corvette", "Reflector_Shield")));
    assert_eq!(sim.with_sim(|s| s.galaxy().factions[rebel as usize].powers.len()), 25);
    assert_eq!(sim.host_stats().rewards_applied.get("NEW_POWER_FOR_ALL").copied(), Some(25));
    assert!(sim.with_sim(|s| s.galaxy().fleets.values().all(|f| f.units.get(&item).copied().unwrap_or(0) == 0)), "a dummy purchase spawns no ship");
    let again = sim.with_sim(|s| s.world().check_build_unit(s.galaxy(), rebel, planet, item));
    assert!(matches!(again, Err(CommandError::BuildLimit)), "lifetime limit 1: {again:?}");
}
