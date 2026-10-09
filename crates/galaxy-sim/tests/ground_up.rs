//! "From the Ground Up" on the largest Remake map (real data, skipped without the game).

use std::collections::BTreeMap;

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

fn sim(data: &GameData, ground_up: bool) -> Sim {
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    Sim::new(data, campaign, SimConfig { victory_enabled: false, ground_up, ..SimConfig::default() }).expect("sim")
}

fn counts(sim: &Sim) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for p in &sim.galaxy().planets {
        let name = p.owner.map_or("none".to_owned(), |f| sim.world().factions[f as usize].name.clone());
        *m.entry(name).or_default() += 1;
    }
    m
}

#[test]
fn owner_counts_follow_the_ground_up_rules() {
    let Some(data) = load() else { return };
    let normal = counts(&sim(&data, false));
    let s = sim(&data, true);
    let c = counts(&s);
    println!("normal {normal:?}\nground up {c:?}");
    assert_eq!(c["Rebel"], 1, "the player starts with one planet");
    assert_eq!(c["Empire"], 3);
    assert_eq!(c["Underworld"], 3);
    for minor in ["Mandalorians", "Hapan", "CIS", "CSA", "Republic", "Warlords", "Cartels", "Sith"] {
        assert_eq!(c.get(minor), normal.get(minor), "{minor} keeps its planets");
    }
    assert!(!c.contains_key("none"), "no planet is left unowned");
    let total: usize = normal.values().sum();
    assert_eq!(c.values().sum::<usize>(), total);
    assert_eq!(c["Pirates"], total - c.iter().filter(|(k, _)| k.as_str() != "Pirates").map(|(_, n)| n).sum::<usize>());
    // Start planets: star base level 1, and the Rebels keep their home.
    let w = s.world();
    let rebel = w.faction_id("Rebel").unwrap();
    for p in s.galaxy().owned_planets(rebel) {
        assert_eq!(w.starbase_level(s.galaxy(), p), 1);
    }
    // Deterministic.
    assert_eq!(s.galaxy().state_hash(), sim(&data, true).galaxy().state_hash());
}

#[test]
fn the_passive_faction_never_acts_but_the_other_ais_expand() {
    let Some(data) = load() else { return };
    let mut s = sim(&data, true);
    let w = s.world().clone();
    let (rebel, empire, pirates) = (w.faction_id("Rebel").unwrap(), w.faction_id("Empire").unwrap(), w.faction_id("Pirates").unwrap());
    assert_eq!(w.passive_faction(), Some(pirates));
    s.set_ai_factions((0..w.factions.len() as u32).filter(|&f| f != rebel));
    let pirate_fleets: Vec<_> = s.galaxy().fleets.iter().filter(|(_, f)| f.owner == pirates).map(|(&id, f)| (id, f.location.clone())).collect();
    let empire_before = s.galaxy().factions[empire as usize].stats.credits_spent;
    let pirates_before = s.galaxy().factions[pirates as usize].stats.credits_spent;
    for _ in 0..900 {
        s.step(1.0);
    }
    let g = s.galaxy();
    assert!(g.factions[empire as usize].stats.credits_spent > empire_before, "the Empire AI builds");
    assert_eq!(g.factions[pirates as usize].stats.credits_spent, pirates_before, "the passive faction spends nothing");
    assert!(g.planets.iter().all(|p| p.owner != Some(pirates) || p.queues.iter().all(|q| q.is_empty())), "no passive production queue");
    for (id, loc) in pirate_fleets {
        // Another faction may destroy a passive fleet, but it never moves.
        if let Some(f) = g.fleets.get(&id) {
            assert_eq!(f.location, loc, "passive fleets stay put");
        }
    }
    // Its commands are refused.
    let planet = g.owned_planets(pirates).next().unwrap();
    let unit = w.buildable_units[pirates as usize].first().copied().unwrap_or(0);
    assert_eq!(s.apply(Command::BuildUnit { faction: pirates, planet, unit }), Err(CommandError::NotAvailable));
}
