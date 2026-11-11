//! Wording of the galaxy notices against the real data: display names, and the loss count of a played battle.

use alamo_data::load_galaxy;
use alamo_fs::{GameFs, GameInstall};
use galaxy_play::notify::{BattleLossCounts, describe_event, unit_returned_text};
use galaxy_play::PlayCatalog;
use galaxy_sim::{Event, Sim, SimConfig};

fn sim_and_names() -> Option<(Sim, PlayCatalog)> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let data = load_galaxy(&GameFs::for_mod(&install, Some("1770851727")).unwrap()).0;
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let names = PlayCatalog::from_data(sim.world(), &data);
    Some((sim, names))
}

#[test]
fn a_returning_hero_is_named_with_display_names() {
    let Some((sim, names)) = sim_and_names() else { return };
    let w = sim.world();
    let hero = w.units.iter().find(|u| u.hero && u.name.contains('_')).expect("a hero with an underscore name");
    let text = unit_returned_text(w, &names, &hero.name, "Coruscant_WW");
    assert!(!text.contains('_'), "no object id in {text}");
    assert!(text.ends_with(&format!("returned at {}.", w.planets[w.planet_id("Coruscant_WW").unwrap() as usize].display_name)), "{text}");
}

#[test]
fn the_battle_notice_counts_the_ships_the_result_screen_lists() {
    let Some((sim, names)) = sim_and_names() else { return };
    let me = sim.world().faction_id("Rebel");
    let p = sim.world().planet_id("Coruscant_WW").unwrap();
    let event = Event::BattleResolved { id: 7, planet: p, winner: me, losses: vec![(me.unwrap(), 0)] };
    let none = BattleLossCounts::default();
    let text = |shown: &BattleLossCounts| describe_event(sim.world(), &names, sim.galaxy(), me, &event, shown).expect("a notice").1;
    assert!(text(&none).contains("(0 ships lost)"), "without a played result the simulation's count stands");
    let mut shown = BattleLossCounts::default();
    shown.0.insert(7, 12);
    assert!(text(&shown).contains("(12 ships lost)"), "{}", text(&shown));
}
