//! Orbital structures that have no weapons themselves but are replaced by an armed one in the battle (Remake 4.0 data):
//! Golan I to III (`Place_Other_Type_At_Every_Specific_Marker_Position` = `<Faction>_Golan_N_Orbital`) and the Uplink Command
//! (`Tactical_Additional_Structure_Type` = `<Faction>_Control_Station`). They must take part in tactical battles and in the
//! statistical resolver with the hull, shield and weapons of the armed form. Real-data test, skips without the game and mod.

use std::sync::Arc;

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use battle_sim::{Arena, Catalog};
use galaxy_sim::*;

fn load() -> Option<(GameData, GameFs)> {
    let install = GameInstall::detect()?;
    if !install.mod_data("1770851727").is_dir() {
        eprintln!("skipping: Remake workshop mod not found");
        return None;
    }
    let fs = GameFs::for_mod(&install, Some("1770851727")).ok()?;
    let data = load_galaxy(&fs).0;
    Some((data, fs))
}

#[test]
fn golan_platforms_and_uplink_command_fight_with_the_numbers_of_their_battle_form() {
    let Some((data, _)) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let sim = Sim::new(&data, campaign, SimConfig::default()).expect("sim");
    let w = sim.world();
    for prefix in ["E", "R", "U"] {
        for n in 1..=3 {
            let (name, form) = (format!("{prefix}_Golan_{n}"), format!("{prefix}_Golan_{n}_Orbital"));
            let (s, f) = (w.structure_id(&name).unwrap(), w.structure_id(&form).unwrap());
            let st = &w.structures[s as usize];
            assert_eq!(st.battle_form, Some(f), "{name} stands in battle as {form}");
            let (mine, theirs) = (st.combat.as_ref().expect("combat numbers"), w.structures[f as usize].combat.as_ref().expect("form has weapons"));
            assert_eq!((mine.hull, mine.shield, mine.weapons.len()), (theirs.hull, theirs.shield, theirs.weapons.len()), "{name}");
        }
        let (uplink, station) = (w.structure_id(&format!("{prefix}_Uplink_Command")).unwrap(), w.structure_id(&format!("{prefix}_Control_Station")).unwrap());
        assert_eq!(w.structures[uplink as usize].battle_form, Some(station), "{prefix} Uplink Command stands in battle as its control station");
    }
    // Golan I of the Remake: 90000 hull, 30000 shield (Golan_1.xml / Hardpoints_Empire_Golan_1.xml).
    let golan1 = w.structure_id("E_Golan_1").unwrap();
    assert_eq!(w.structures[golan1 as usize].combat.as_ref().map(|c| (c.hull, c.shield)), Some((90000.0, 30000.0)));
}

#[test]
fn a_golan_platform_spawns_in_the_space_battle_over_its_planet() {
    let Some((data, fs)) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Imperial").expect("campaign");
    let mut sim = Sim::new(&data, campaign, SimConfig { auto_resolve_battles: false, victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let target = (0..w.planets.len() as u32)
        .find(|&p| w.real[p as usize] && w.planets[p as usize].space_map.is_some() && sim.galaxy().planets[p as usize].owner == Some(empire))
        .expect("an Empire planet with a map");
    let neighbour = w.routes.iter().find_map(|r| (r.a == target).then_some(r.b).or((r.b == target).then_some(r.a))).expect("a route");
    let (golan, uplink) = (w.structure_id("E_Golan_2").unwrap(), w.structure_id("E_Uplink_Command").unwrap());
    let mc80 = w.unit_id("MC-80_Star_Cruiser").unwrap();
    {
        let g = sim.galaxy_mut();
        g.planets[target as usize].structures = vec![golan, uplink];
        let id = g.fleets.keys().max().copied().unwrap_or(0) + 1;
        g.fleets.insert(id, Fleet { id, owner: rebel, units: [(mc80, 2)].into(), location: FleetLocation::AtPlanet(target), previous_planet: neighbour, slot: 0 });
    }
    let battle = sim.step(1.0).into_iter().find_map(|e| if let Event::BattlePending(b) = e { Some(b) } else { None }).expect("pending battle");
    let req = sim.tactical_setup(battle.id).expect("request");
    let names: Vec<&str> = req.sides.iter().flat_map(|s| s.groups.iter().map(|g| g.name.as_str())).collect();
    assert!(names.contains(&"E_Golan_2_Orbital"), "the Golan platform takes part: {names:?}");
    assert!(names.contains(&"E_Control_Station"), "the Uplink Command takes part as its control station: {names:?}");
    let catalog = Arc::new(Catalog::build(&data, Some(&fs), &names));
    let arena = req.map.as_ref().and_then(|m| Arena::load(&data, &fs, m)).map(Arc::new);
    let mut seen: Vec<(String, f32, f32)> = Vec::new();
    let mut first = true;
    run_headless_observed(&req, &catalog, arena, &mut |b| {
        if std::mem::take(&mut first) {
            seen = b.ships.iter().filter(|s| s.alive).map(|s| (catalog.ship(s.def).name.clone(), s.hull, s.shield)).collect();
        }
    })
    .expect("result");
    let golan_ship = seen.iter().find(|s| s.0 == "E_Golan_2_Orbital").expect("Golan II spawned");
    assert_eq!((golan_ship.1, golan_ship.2), (135000.0, 50000.0), "Golan II hull and shield are the data values");
    assert!(seen.iter().any(|s| s.0 == "E_Control_Station" && s.1 == 18000.0), "the control station spawned with its data hull");
}

#[test]
fn auto_resolve_counts_a_golan_platform() {
    let Some((data, _)) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Imperial").expect("campaign");
    let sim = Sim::new(&data, campaign, SimConfig::default()).expect("sim");
    let w = sim.world();
    let golan = w.structure_id("E_Golan_2").unwrap();
    let with = w.strength([(galaxy_sim::combat::Source::Structure(golan), 1)]);
    assert!(with > 10000.0, "a Golan II is a real defender for the statistical resolver: {with}");
}
