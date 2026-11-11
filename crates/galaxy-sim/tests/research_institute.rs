//! Remake tech levels come from the Research Institute: `E_Tech-2` ... `R_Tech-5` (`Build_Advances_Tech_Level`, variants of `Dummy_<F>`) need the
//! faction's `<F>_Research_Institute` on the planet, take 300 / 450 / 600 / 750 s, and the Clone Wars stack adds `CIS_Tech-*` / `Republic_Tech-*`.
//! Prices are the objects' own `Build_Cost_Credits` (1000 for level 1, then 20000 / 30000 / 40000 / 50000). Skipped when a mod folder is missing.

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{GameFs, GameInstall};
use galaxy_sim::*;

fn load(alias: &str, mods: &[&str]) -> Option<GameData> {
    let install = GameInstall::detect()?;
    for id in mods {
        if !install.mod_data(id).is_dir() {
            eprintln!("skipping: workshop mod {id} not found");
            return None;
        }
    }
    Some(load_galaxy(&GameFs::for_mod(&install, Some(alias)).ok()?).0)
}

/// The chain, the institute requirement and the purchase rule of every main faction of `campaign`.
fn check_chain(data: &GameData, campaign: &str, name: &str, prefix: &str) -> bool {
    let c = data.campaign(campaign).unwrap();
    let mut sim = Sim::new(data, c, SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let f = w.faction_id(name).unwrap();
    let info = &w.factions[f as usize];
    assert!(!info.tech_upgrades.is_empty(), "{name} has a tech chain");
    for (n, up) in info.tech_upgrades.iter().enumerate() {
        let level = info.start_tech + n as i32 + 1;
        assert_eq!(up.name, format!("{prefix}_Tech-{level}"), "{name}: level {level} is the mod's upgrade object");
        let seconds = match level { 1 => 50.0, 2 => 300.0, 3 => 450.0, 4 => 600.0, _ => 750.0 };
        assert_eq!(up.time, seconds, "{name} level {level} time");
        let price = match level { 1 => 1000.0, 2 => 20000.0, 3 => 30000.0, 4 => 40000.0, _ => 50000.0 };
        assert_eq!(up.cost, price, "{name} level {level} costs the object's Build_Cost_Credits");
        assert_eq!(up.req_structures.len(), 1, "{name} level {level} needs one structure group");
        let inst = &w.structures[up.req_structures[0][0] as usize].name;
        assert!(inst.to_ascii_lowercase().ends_with("research_institute"), "{name} level {level} needs {inst}");
    }
    // Bought only where the institute stands.
    let owned: Vec<PlanetId> = sim.galaxy().owned_planets(f).collect();
    let req = &info.tech_upgrades[(sim.galaxy().factions[f as usize].tech - info.start_tech).max(0) as usize].req_structures;
    let with: Vec<PlanetId> = owned.iter().copied().filter(|&p| w.missing_structures(&sim.galaxy().planets[p as usize], req).is_none()).collect();
    if with.is_empty() {
        return false;
    }
    let without = owned.iter().copied().find(|p| !with.contains(p));
    if let Some(p) = without {
        assert!(
            matches!(w.check_upgrade_tech(sim.galaxy(), f, p), Err(CommandError::RequirementsNotMet(_))),
            "{name}: no tech at a planet without the institute"
        );
    }
    sim.galaxy_mut().factions[f as usize].credits = 1.0e6;
    let p = with.iter().copied().find(|&p| w.starbase_level(sim.galaxy(), p) >= 1);
    if let Some(p) = p {
        assert!(w.check_upgrade_tech(sim.galaxy(), f, p).is_ok(), "{name}: tech can be bought at the institute planet");
    }
    true
}

#[test]
fn clone_wars_factions_research_too() {
    let Some(data) = load("fft", &["3229239424", "2794270450", "1770851727"]) else { return };
    for (faction, suffix) in [("CIS", "_cis"), ("Republic", "_republic")] {
        // Only some campaigns start with the institute (Shatterpoint); the chain itself is the same everywhere.
        let found = data.campaigns.iter().filter(|c| c.has_galaxy() && c.name.to_ascii_lowercase().ends_with(suffix)).any(|c| check_chain(&data, &c.name, faction, faction));
        assert!(found, "a {faction} campaign starts with the Research Institute");
    }
}

#[test]
fn remake_tech_levels_need_the_research_institute() {
    let Some(data) = load("1770851727", &["1770851727"]) else { return };
    // Each campaign gives the institute to its own player faction (Starting_Forces); the others reach tech through the story scripts.
    for (campaign, faction, prefix) in [("GC_Full_Web_Way_Rebel", "Rebel", "R"), ("GC_Full_Web_Way_Imperial", "Empire", "E"), ("GC_Full_Web_Way_Underworld", "Underworld", "U")] {
        assert!(check_chain(&data, campaign, faction, prefix), "{faction} starts with a Research Institute in {campaign}");
    }
}

#[test]
fn the_ai_buys_levels_without_the_institute() {
    let Some(data) = load("1770851727", &["1770851727"]) else { return };
    let mut sim = Sim::new(&data, data.campaign("GC_Full_Web_Way_Rebel").unwrap(), SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    assert_ne!(w.victory.active_player, Some(empire), "the Empire is the AI in the Rebel campaign");
    sim.galaxy_mut().factions[empire as usize].credits = 1.0e6;
    let planet = sim.galaxy().owned_planets(empire).find(|&p| w.starbase_level(sim.galaxy(), p) >= 1).expect("an Empire planet with a star base");
    assert!(w.check_upgrade_tech(sim.galaxy(), empire, planet).is_ok(), "the AI needs no Research Institute");
}

#[test]
fn reduce_technology_price_ability_at_maw_and_scarif() {
    let Some(data) = load("1770851727", &["1770851727"]) else { return };
    let c = data.campaign("GC_Full_Web_Way_Imperial").unwrap();
    let mut sim = Sim::new(&data, c, SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();

    let maw = w.planet_id("The_Maw_WW").expect("The Maw in campaign");
    let scarif = w.planet_id("Scarif_WW").expect("Scarif in campaign");

    // Verify abilities loaded from data on the planets
    let maw_mods: Vec<_> = w.planets[maw as usize].prod_mods.iter().filter(|m| m.tech).collect();
    assert_eq!(maw_mods.len(), 1, "The Maw has one tech discount mod");
    assert_eq!(maw_mods[0].price, 0.33, "The Maw tech price reduction is 0.33 in data");

    let scarif_mods: Vec<_> = w.planets[scarif as usize].prod_mods.iter().filter(|m| m.tech).collect();
    assert_eq!(scarif_mods.len(), 1, "Scarif has one tech discount mod");
    assert_eq!(scarif_mods[0].price, 0.25, "Scarif tech price reduction is 0.25 in data");

    let base_cost = w.factions[empire as usize].tech_upgrades[0].cost; // level 2 price of the data

    // Without Maw or Scarif: full base cost
    sim.galaxy_mut().planets[maw as usize].owner = None;
    sim.galaxy_mut().planets[scarif as usize].owner = None;
    assert_eq!(w.tech_cost(sim.galaxy(), empire, 0), base_cost);

    // Owning The Maw: 0.33 discount
    sim.galaxy_mut().planets[maw as usize].owner = Some(empire);
    let maw_cost = w.tech_cost(sim.galaxy(), empire, 0);
    assert!((maw_cost - base_cost * 0.67).abs() < 0.01, "Maw discount: expected {}, got {}", base_cost * 0.67, maw_cost);

    // Owning Scarif: 0.25 discount
    sim.galaxy_mut().planets[maw as usize].owner = None;
    sim.galaxy_mut().planets[scarif as usize].owner = Some(empire);
    let scarif_cost = w.tech_cost(sim.galaxy(), empire, 0);
    assert!((scarif_cost - base_cost * 0.75).abs() < 0.01, "Scarif discount: expected {}, got {}", base_cost * 0.75, scarif_cost);

    // Stacking: owning both Maw (0.33) and Scarif (0.25) in same stacking category (0) -> max discount (0.33)
    sim.galaxy_mut().planets[maw as usize].owner = Some(empire);
    let both_cost = w.tech_cost(sim.galaxy(), empire, 0);
    assert!((both_cost - base_cost * 0.67).abs() < 0.01, "Both discount: expected {}, got {}", base_cost * 0.67, both_cost);

    // Verify Command::UpgradeTech charges the discounted price
    sim.galaxy_mut().factions[empire as usize].credits = 100000.0;
    let p = sim.galaxy().owned_planets(empire).find(|&p| w.check_upgrade_tech(sim.galaxy(), empire, p).is_ok()).expect("valid upgrade planet");
    sim.apply(Command::UpgradeTech { faction: empire, planet: p }).unwrap();
    let credits_left = sim.galaxy().factions[empire as usize].credits;
    assert!((credits_left - (100000.0 - both_cost as f64)).abs() < 0.01, "charged discounted tech price: expected {}, got {}", 100000.0 - both_cost as f64, credits_left);
}

#[test]
fn built_in_ai_plans_tech_at_the_discounted_maw_price() {
    let Some(data) = load("1770851727", &["1770851727"]) else { return };
    let c = data.campaign("GC_Full_Web_Way_Imperial").unwrap();
    // Rebel is the human, so Empire does not need a Research Institute on the planet.
    let mut sim = Sim::new(&data, c, SimConfig { victory_enabled: false, faction_override: Some("Rebel".into()), ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    assert_ne!(w.victory.active_player, Some(empire));
    let maw = w.planet_id("The_Maw_WW").expect("The Maw");
    sim.galaxy_mut().planets[maw as usize].owner = Some(empire);
    for planet in &mut sim.galaxy_mut().planets {
        if planet.owner == Some(empire) {
            planet.queues[Lane::Space as usize].retain(|i| !matches!(i.kind, ProductionKind::Starbase(_) | ProductionKind::Tech(_)));
        }
    }

    let info = &w.factions[empire as usize];
    let idx = (sim.galaxy().factions[empire as usize].tech - info.start_tech).max(0) as usize;
    let full = f64::from(info.tech_upgrades[idx].cost);
    let discounted = f64::from(w.tech_cost(sim.galaxy(), empire, idx));
    assert!((discounted - full * 0.67).abs() < 0.01, "The Maw is 33% off: full {full}, discounted {discounted}");

    let planet = sim.galaxy().owned_planets(empire).max_by_key(|&p| (w.starbase_level(sim.galaxy(), p), std::cmp::Reverse(p))).expect("planet");
    if w.starbase_level(sim.galaxy(), planet) < info.tech_upgrades[idx].req_starbase {
        sim.galaxy_mut().planets[planet as usize].starbase = w.first_starbase[empire as usize];
    }
    assert!(w.check_upgrade_tech(sim.galaxy(), empire, planet).is_ok(), "the planet plan() picks can take the upgrade");

    // ai.rs keeps 1500 credits back. The budget sits strictly between the discounted price and the list price.
    let reserve = 1500.0;
    let budget = discounted + (full - discounted) / 2.0;
    sim.galaxy_mut().factions[empire as usize].credits = budget + reserve;
    let buy = |sim: &Sim| ai::plan(sim.world(), sim.galaxy(), empire, &mut rng::Rng::new(1)).into_iter().any(|c| matches!(c, Command::UpgradeTech { faction, .. } if faction == empire));
    assert!(buy(&sim), "budget {budget} is at least the Maw price {discounted} and under the list price {full}");

    sim.galaxy_mut().factions[empire as usize].credits = discounted + reserve - 1.0;
    assert!(!buy(&sim), "one credit under the Maw price does not buy");
}

#[test]
fn tech_level_clamped_to_campaign_max_tech() {
    let Some(data) = load("1770851727", &["1770851727"]) else { return };
    let c = data.campaign("GC_Full_Web_Way_Imperial").unwrap();
    let mut sim = Sim::new(&data, c, SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let rebel = w.faction_id("Rebel").unwrap();
    let max_tech = w.factions[rebel as usize].max_tech;
    assert_eq!(max_tech, 5);

    sim.galaxy_mut().factions[rebel as usize].tech = 4;
    sim.galaxy_mut().factions[rebel as usize].credits = 1.0e6;
    let planet = sim.galaxy().owned_planets(rebel).find(|&p| w.starbase_level(sim.galaxy(), p) >= 1).expect("rebel planet");
    sim.galaxy_mut().planets[planet as usize].queues[Lane::Space as usize].clear();
    sim.apply(Command::UpgradeTech { faction: rebel, planet }).expect("queue tech 5");

    sim.set_tech_level(rebel, max_tech);
    assert_eq!(sim.galaxy().factions[rebel as usize].tech, 5);

    sim.galaxy_mut().planets[planet as usize].queues[Lane::Space as usize][0].remaining = 0.01;
    sim.step(1.0);

    assert_eq!(
        sim.galaxy().factions[rebel as usize].tech,
        max_tech,
        "tech level must not exceed max_tech ({max_tech})"
    );
}


/// The space queue of the best developed planet is full: the built-in AI buys its tech level at the next planet that takes the order, where
/// it starts at once, and does not wait for a slot at the first one.
#[test]
fn built_in_ai_buys_tech_elsewhere_when_the_best_planet_has_a_full_space_queue() {
    let Some(data) = load("1770851727", &["1770851727"]) else { return };
    let c = data.campaign("GC_Full_Web_Way_Imperial").unwrap();
    let mut sim = Sim::new(&data, c, SimConfig { victory_enabled: false, faction_override: Some("Rebel".into()), ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    sim.add_credits(empire, 1.0e7);
    let takes_tech = |sim: &Sim| -> Vec<u32> { sim.galaxy().owned_planets(empire).filter(|&p| w.check_upgrade_tech(sim.galaxy(), empire, p).is_ok()).collect() };
    assert!(takes_tech(&sim).len() >= 2, "the Empire starts with several star base planets");
    let best = sim.galaxy().owned_planets(empire).max_by_key(|&p| (w.starbase_level(sim.galaxy(), p), std::cmp::Reverse(p))).expect("planet");
    assert!(takes_tech(&sim).contains(&best));
    let ship = w.buildable_units[empire as usize]
        .iter()
        .copied()
        .find(|&u| w.units[u as usize].role == UnitRole::Ship && w.units[u as usize].limit_current.is_none() && w.check_build_unit(sim.galaxy(), empire, best, u).is_ok())
        .expect("a ship the planet can build");
    for _ in 0..w.config.max_queue_len {
        sim.apply(Command::BuildUnit { faction: empire, planet: best, unit: ship }).expect("queued");
    }
    assert_eq!(w.check_upgrade_tech(sim.galaxy(), empire, best), Err(CommandError::QueueFull));
    let at: Vec<u32> = ai::plan(&w, sim.galaxy(), empire, &mut rng::Rng::new(1)).into_iter().filter_map(|c| if let Command::UpgradeTech { planet, .. } = c { Some(planet) } else { None }).collect();
    assert_eq!(at.len(), 1, "one tech order");
    assert!(at[0] != best && takes_tech(&sim).contains(&at[0]), "ordered at {} where the queue has room", w.planets[at[0] as usize].name);
}
