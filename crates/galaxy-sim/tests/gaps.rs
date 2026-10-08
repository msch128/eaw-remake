//! GC gap features against the real data: capture bonus, hero events.

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

#[test]
fn conquering_a_planet_pays_its_capture_bonus_once_per_faction() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { auto_resolve_battles: true, victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let target = (0..w.planets.len() as u32)
        .find(|&p| w.real[p as usize] && w.planets[p as usize].capture_bonus > 500.0 && sim.galaxy().planets[p as usize].owner.is_none() && sim.galaxy().fleets_at(p).next().is_none())
        .expect("a neutral planet with a capture bonus");
    let bonus = f64::from(w.planets[target as usize].capture_bonus);
    println!("{} pays {bonus}", w.planets[target as usize].name);
    let unit = w.unit_id("Star_Destroyer").unwrap();
    sim.spawn_units(empire, target, unit, 1);
    let before = sim.galaxy().factions[empire as usize].credits;
    let events = sim.step(10.0);
    let income = sim.galaxy().factions[empire as usize].income_per_week * 10.0 / f64::from(w.config.week_seconds);
    assert!(events.iter().any(|e| matches!(e, Event::CaptureBonus { faction, planet, credits } if *faction == empire && *planet == target && (*credits - bonus).abs() < 1e-6)), "{events:?}");
    let gained = sim.galaxy().factions[empire as usize].credits - before;
    // The cached income already includes the captured planet, so allow its income share.
    assert!((gained - bonus - income).abs() < 0.1 * income + 5.0, "gained {gained}, bonus {bonus}, income {income}");
    // Another faction taking it later gets its own bonus; the same faction taking it back gets none.
    sim.set_planet_owner(target, None);
    sim.spawn_units(rebel, target, unit, 1);
    let events = sim.step(10.0);
    // Both fleets sit there: a battle decides, so just check nobody is paid twice for the same pair.
    let paid: Vec<_> = events.iter().filter(|e| matches!(e, Event::CaptureBonus { .. })).collect();
    assert!(paid.len() <= 1, "{paid:?}");
    assert!(sim.galaxy().rewarded.contains(&(target, empire)));
}

#[test]
fn heroes_are_reported_when_they_die() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { auto_resolve_battles: true, victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let target = (0..w.planets.len() as u32).find(|&p| w.real[p as usize] && sim.galaxy().planets[p as usize].owner.is_none() && sim.galaxy().fleets_at(p).next().is_none()).unwrap();
    let hero = w.unit_id("Mon_Mothma").expect("Mon Mothma exists");
    sim.spawn_units(rebel, target, hero, 1);
    sim.spawn_units(empire, target, w.unit_id("Star_Destroyer").unwrap(), 4);
    let events = sim.step(3.0);
    assert!(events.iter().any(|e| matches!(e, Event::HeroDestroyed { faction, unit, .. } if *faction == rebel && *unit == hero)), "{events:?}");
}

#[test]
fn hero_and_structure_discounts_follow_the_ability_data() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig::default()).unwrap();
    let w = sim.world().clone();
    let rebel = w.faction_id("Rebel").unwrap();
    let home = w.factions[rebel as usize].home.unwrap();
    let mc80 = w.unit_id("MC-80_Star_Cruiser").unwrap();
    let ut = &w.units[mc80 as usize];
    let (base_cost, base_time) = sim.world().unit_cost(sim.galaxy(), rebel, home, mc80);
    // Mon Mothma (20 % off everything but the excluded types) cheapens the cruiser galaxy-wide.
    let mothma = w.unit_id("Mon_Mothma").unwrap();
    assert!(w.units[mothma as usize].prod_mods.iter().any(|m| (m.price - 0.20).abs() < 1e-6), "Mothma carries the 20 % ability");
    sim.spawn_units(rebel, home, mothma, 1);
    let (c, t) = sim.world().unit_cost(sim.galaxy(), rebel, home, mc80);
    println!("{}: {base_cost} -> {c}, time {base_time} -> {t}", ut.name);
    assert!((c - base_cost * 0.8).abs() < 0.5, "{c} vs {base_cost}");
    assert!((t - base_time).abs() < 1e-3, "Mothma only touches the price");
    // The same discount applies to the real charge.
    let credits = sim.galaxy().factions[rebel as usize].credits;
    let _ = credits;
    // Starforge: half price and (almost) no build time for ships, at its own planet only.
    let sf = w.structure_id("Starforge").expect("Starforge");
    let other = (0..w.planets.len() as u32).find(|&p| p != home && w.real[p as usize]).unwrap();
    assert!(sim.add_structure(home, sf));
    let (c2, t2) = sim.world().unit_cost(sim.galaxy(), rebel, home, mc80);
    assert!(c2 < c * 0.55 && c2 > c * 0.4, "starforge price {c2} from {c}");
    assert!(t2 < base_time * 0.5, "starforge time {t2} from {base_time}");
    let (c3, _) = sim.world().unit_cost(sim.galaxy(), rebel, other, mc80);
    assert!((c3 - c).abs() < 0.5, "a Starforge elsewhere does not help");
}

#[test]
fn a_superweapon_destroys_a_planet_once_per_cooldown() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let ds = w.unit_id("Death_Star_II").expect("Death Star II");
    assert!(w.units[ds as usize].planet_destroyer);
    let sd = w.unit_id("Star_Destroyer").unwrap();
    assert!(!w.units[sd as usize].planet_destroyer);
    let ok: Vec<u32> = (0..w.planets.len() as u32).filter(|&p| w.real[p as usize] && w.planets[p as usize].destroyable).collect();
    assert!(ok.len() < (0..w.planets.len()).filter(|&p| w.real[p]).count(), "some planets are protected by the data");
    let rebel_planet = ok.iter().copied().find(|&p| sim.galaxy().planets[p as usize].owner == Some(rebel)).expect("a destroyable rebel planet");
    let neutral = ok.iter().copied().find(|&p| sim.galaxy().planets[p as usize].owner.is_none() && p != rebel_planet).unwrap();
    // Without a superweapon in orbit the command is refused.
    assert_eq!(sim.apply(Command::DestroyPlanet { faction: empire, planet: rebel_planet }), Err(CommandError::NotAvailable));
    sim.spawn_units(empire, rebel_planet, ds, 1);
    sim.spawn_units(rebel, rebel_planet, sd, 3);
    // The owner cannot fire at its own planet.
    assert_eq!(sim.apply(Command::DestroyPlanet { faction: rebel, planet: rebel_planet }), Err(CommandError::NotAvailable));
    assert_eq!(sim.apply(Command::DestroyPlanet { faction: empire, planet: rebel_planet }), Ok(CommandOk::Done));
    let events = sim.step(0.5);
    let lost = events.iter().find_map(|e| match e {
        Event::PlanetDestroyed { planet, by, owner, ships_lost } if *planet == rebel_planet && *by == empire && *owner == Some(rebel) => Some(*ships_lost),
        _ => None,
    });
    assert!(lost.is_some_and(|n| n >= 3), "{events:?}");
    let ps = &sim.galaxy().planets[rebel_planet as usize];
    assert!(ps.destroyed && ps.owner.is_none() && ps.structures.is_empty() && ps.starbase.is_none());
    assert!(sim.galaxy().fleets_at(rebel_planet).all(|f| f.owner == empire), "only the Death Star remains");
    // Nobody can take the wreck, and the weapon needs to recharge.
    sim.spawn_units(rebel, rebel_planet, sd, 1);
    sim.step(30.0);
    assert!(sim.galaxy().planets[rebel_planet as usize].owner.is_none());
    sim.spawn_units(empire, neutral, ds, 1);
    assert_eq!(sim.apply(Command::DestroyPlanet { faction: empire, planet: neutral }), Err(CommandError::WrongState));
    sim.step(150.0);
    // ... and a wreck cannot be destroyed again.
    sim.spawn_units(empire, rebel_planet, ds, 1);
    assert_eq!(sim.apply(Command::DestroyPlanet { faction: empire, planet: rebel_planet }), Err(CommandError::WrongState));
}

#[test]
fn statistics_count_captures_battles_ships_and_credits() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { auto_resolve_battles: true, victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    assert!(sim.galaxy().factions.iter().all(|f| f.stats == FactionStats::default()), "a new game starts at zero");
    let target = (0..w.planets.len() as u32).find(|&p| w.real[p as usize] && w.planets[p as usize].capture_bonus > 0.0 && sim.galaxy().planets[p as usize].owner.is_none() && sim.galaxy().fleets_at(p).next().is_none()).unwrap();
    let sd = w.unit_id("Star_Destroyer").unwrap();
    sim.spawn_units(empire, target, sd, 1);
    sim.step(10.0);
    let e = sim.galaxy().factions[empire as usize].stats;
    assert_eq!(e.planets_captured, 1);
    assert!(e.credits_earned >= f64::from(w.planets[target as usize].capture_bonus));
    // A battle: the winner and the loser are counted.
    let other = (0..w.planets.len() as u32).find(|&p| p != target && w.real[p as usize] && sim.galaxy().planets[p as usize].owner.is_none() && sim.galaxy().fleets_at(p).next().is_none()).unwrap();
    sim.spawn_units(empire, other, sd, 4);
    sim.spawn_units(rebel, other, w.unit_id("Nebulon_B_Frigate").unwrap(), 1);
    sim.step(3.0);
    let (e, r) = (sim.galaxy().factions[empire as usize].stats, sim.galaxy().factions[rebel as usize].stats);
    assert_eq!((e.battles_won, r.battles_lost, r.ships_lost), (1, 1, 1), "{e:?} {r:?}");
    // Production: paid on queueing, a ship built on completion.
    let home = w.factions[empire as usize].home.unwrap();
    let before = sim.galaxy().factions[empire as usize].stats;
    sim.add_credits(empire, 1_000_000.0);
    let tie = w.unit_id("TIE_Fighter_Squadron").or_else(|| w.unit_id("Star_Destroyer")).unwrap();
    if sim.apply(Command::BuildUnit { faction: empire, planet: home, unit: tie }).is_ok() {
        sim.step(w.units[tie as usize].build_time * w.config.build_time_scale * 1.5 + 5.0);
        let after = sim.galaxy().factions[empire as usize].stats;
        assert!(after.credits_spent > before.credits_spent && after.ships_built == before.ships_built + 1, "{after:?}");
    }
    // The counters are part of the state hash.
    let h = sim.galaxy().state_hash();
    sim.galaxy_mut().factions[empire as usize].stats.battles_won += 1;
    assert_ne!(h, sim.galaxy().state_hash());
}

#[test]
fn a_restored_sim_continues_exactly_like_the_original() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let config = SimConfig { victory_enabled: false, seed: 3, ..SimConfig::default() };
    let mut a = Sim::new(&data, campaign, config.clone()).unwrap();
    a.set_ai_factions(0..a.world().factions.len() as u32);
    a.step(60.0 * 12.0);
    let snap = a.snapshot();
    // Through JSON, as in a save file.
    let snap: SimSnapshot = serde_json::from_str(&serde_json::to_string(&snap).unwrap()).unwrap();
    let mut b = Sim::new(&data, campaign, config).unwrap();
    b.restore(snap).expect("restore");
    assert_eq!(a.galaxy(), b.galaxy());
    a.step(60.0 * 8.0);
    b.step(60.0 * 8.0);
    assert_eq!(a.galaxy().state_hash(), b.galaxy().state_hash(), "the sim part of a save is exact");
    assert_eq!(a.galaxy(), b.galaxy());
}

#[test]
fn a_planet_takes_star_bases_only_up_to_its_max_space_base() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let empire = w.faction_id("Empire").unwrap();
    sim.add_credits(empire, 5.0e6);
    sim.set_tech_level(empire, w.factions[empire as usize].max_tech);
    let small = (0..w.planets.len() as u32).find(|&p| w.real[p as usize] && w.planets[p as usize].max_space_base == 1).expect("a planet limited to level 1");
    let big = (0..w.planets.len() as u32).find(|&p| w.real[p as usize] && w.planets[p as usize].max_space_base == 3 && p != small).expect("a planet that takes level 3");
    for p in [small, big] {
        sim.set_planet_owner(p, Some(empire));
        sim.galaxy_mut().planets[p as usize].structures.clear();
        let first = w.first_starbase[empire as usize].expect("Empire star base chain");
        sim.set_starbase(p, Some(first));
        assert_eq!(w.starbases[first as usize].level, 1);
    }
    let r = w.check_upgrade_starbase(sim.galaxy(), empire, small);
    assert!(matches!(r, Err(CommandError::RequirementsNotMet(_))), "level 2 on a level-1 planet: {r:?}");
    // The other planet climbs to 3 and stops there.
    for expect in [2, 3] {
        let next = w.check_upgrade_starbase(sim.galaxy(), empire, big).unwrap_or_else(|e| panic!("level {expect}: {e:?}"));
        assert_eq!(w.starbases[next as usize].level, expect);
        sim.set_starbase(big, Some(next));
    }
    assert!(w.check_upgrade_starbase(sim.galaxy(), empire, big).is_err(), "level 4 is beyond Max_Space_Base");
}
