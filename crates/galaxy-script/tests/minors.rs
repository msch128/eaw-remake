//! Playing a minor faction of a campaign (the "Advanced" option): Hapan and CIS for 20 weeks.

use std::sync::Arc;

use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{ScriptedConfig, ScriptedSim};
use galaxy_sim::{Command, Event, SimConfig};

fn play_minor(name: &str) {
    let Some(install) = GameInstall::detect() else { return };
    if !install.mod_data("1770851727").is_dir() {
        return;
    }
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).unwrap());
    let data = Arc::new(load_galaxy(&*fs).0);
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap().clone();
    let mut sim = ScriptedSim::new(
        fs,
        data,
        &campaign,
        SimConfig { victory_enabled: true, player_faction: Some(name.into()), ..SimConfig::default() },
        ScriptedConfig { human: Some(name.into()), ..ScriptedConfig::default() },
    )
    .unwrap();
    let (w, me) = sim.with_sim(|s| (s.world().clone(), s.world().faction_id(name).expect("the minor is in the campaign")));
    assert_eq!(w.victory.active_player, Some(me), "victory rules follow the chosen faction");
    let (planets, ships, credits) = sim.with_sim(|s| {
        let g = s.galaxy();
        (g.owned_planets(me).count(), g.fleets.values().filter(|f| f.owner == me).map(|f| f.unit_count()).sum::<u32>(), g.factions[me as usize].credits)
    });
    println!("{name}: {planets} planets, {ships} ships, {credits:.0} credits at the start; {} tech upgrades defined, tech {}/{}", w.factions[me as usize].tech_upgrades.len(), sim.with_sim(|s| s.galaxy().factions[me as usize].tech), w.factions[me as usize].max_tech);
    assert!(planets >= 1 && credits > 0.0, "a sensible start from the campaign data");

    // What can be built where?
    let options: Vec<(u32, u32)> = sim.with_sim(|s| {
        let g = s.galaxy();
        g.owned_planets(me).flat_map(|p| (0..w.units.len() as u32).filter(move |_| true).map(move |u| (p, u))).filter(|&(p, u)| w.check_build_unit(g, me, p, u).is_ok()).collect()
    });
    println!("{name}: {} (planet, unit) build options", options.len());
    let mut built = false;
    if let Some(&(planet, unit)) = options.iter().min_by_key(|&&(_, u)| w.units[u as usize].build_time as i64) {
        let r = sim.apply(Command::BuildUnit { faction: me, planet, unit });
        println!("{name}: queued {} at {}: {r:?}", w.units[unit as usize].name, w.planets[planet as usize].name);
        built = r.is_ok();
    }
    let before_built = sim.with_sim(|s| s.galaxy().factions[me as usize].stats.ships_built);
    // A move order.
    let mover = sim.with_sim(|s| s.galaxy().fleets.values().find(|f| f.owner == me && f.planet().is_some()).map(|f| (f.id, f.planet().unwrap())));
    let mut moved = None;
    if let Some((fleet, at)) = mover {
        let to = w.routes.iter().find_map(|r| (r.a == at).then_some(r.b).or((r.b == at).then_some(r.a))).expect("a neighbour");
        moved = Some((fleet, to, sim.apply(Command::MoveFleet { faction: me, fleet, to }).is_ok()));
    }
    let errors_before = sim.diagnostics().script_errors;
    let mut week = 0;
    let mut eliminated = false;
    while week < 20 {
        for e in sim.step(1.0).sim_events {
            match e {
                Event::WeekEnded { week: wk } => week = wk,
                Event::FactionEliminated { faction } if faction == me => eliminated = true,
                _ => {}
            }
        }
    }
    assert!(sim.ai_character(name).is_none(), "the AI does not play the human faction");
    let techs: Vec<(String, i32)> = sim.with_sim(|s| ["Rebel", "Empire", "Underworld"].iter().map(|n| (n.to_string(), s.galaxy().factions[w.faction_id(n).unwrap() as usize].tech)).collect());
    println!("{name}: AI major tech levels after 20 weeks {techs:?}");
    assert!(techs.iter().any(|t| t.1 >= 2), "the AI majors research without a human major to follow: {techs:?}");
    let d = sim.diagnostics();
    assert_eq!(d.script_errors, errors_before, "{:?}", d.last_script_error);
    assert_eq!(d.script_errors, 0, "{:?}", d.last_script_error);
    if built {
        let after = sim.with_sim(|s| s.galaxy().factions[me as usize].stats.ships_built);
        assert!(after > before_built, "the queued ship was finished");
    }
    if let Some((fleet, to, ok)) = moved {
        assert!(ok, "the move order was accepted");
        if !eliminated {
            let at = sim.with_sim(|s| s.galaxy().fleets.get(&fleet).map(|f| f.planet()));
            println!("{name}: fleet {fleet} ordered to {} is now {at:?}", w.planets[to as usize].name);
        }
    }
    let (planets, ships) = sim.with_sim(|s| {
        let g = s.galaxy();
        (g.owned_planets(me).count(), g.fleets.values().filter(|f| f.owner == me).map(|f| f.unit_count()).sum::<u32>())
    });
    println!("{name} after 20 weeks: {planets} planets, {ships} ships, eliminated {eliminated}");
}

#[test]
fn a_human_hapan_plays_20_weeks() {
    play_minor("Hapan");
}

#[test]
fn a_human_cis_plays_20_weeks() {
    play_minor("CIS");
}
