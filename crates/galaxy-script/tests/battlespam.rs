//! A save of a real campaign (week 142, human Empire, everybody else AI) in which planets were stuck: a hostile fleet alone in orbit, no star
//! base, a watchtower on the ground, a new battle every tick and no capture. The save is not part of the repository: the test runs when
//! `EAW_REMAKE_BATTLESPAM_SAVE` names a copy of it and is skipped otherwise. The same situations are built from the game data in
//! `galaxy-sim/tests/arrival.rs`.

use std::collections::BTreeMap;
use std::sync::Arc;

use alamo_data::{GameData, load_galaxy};
use alamo_fs::{FileSource, GameFs, GameInstall};
use galaxy_script::{SaveFile, ScriptedSim};
use galaxy_sim::{BattleRepeats, Event, FactionId, Galaxy, PlanetId, Sim, World};

struct Loaded {
    data: Arc<GameData>,
    file: SaveFile,
    sim: ScriptedSim,
}

fn load() -> Option<Loaded> {
    let path = std::env::var_os("EAW_REMAKE_BATTLESPAM_SAVE")?;
    let install = GameInstall::detect()?;
    let fs: Arc<dyn FileSource> = Arc::new(GameFs::for_mod(&install, Some("1770851727")).ok()?);
    let data = Arc::new(load_galaxy(&*fs).0);
    let file = SaveFile::read(&path).expect("the save reads");
    let sim = ScriptedSim::from_save(file.clone(), fs, data.clone(), |_| {}).expect("the save loads");
    Some(Loaded { data, file, sim })
}

/// Planets a single hostile faction holds the orbit of while the owner has no star base: the capture rule applies to them. With the
/// faction in orbit.
fn occupied(w: &World, g: &Galaxy) -> BTreeMap<PlanetId, FactionId> {
    let mut out = BTreeMap::new();
    for p in 0..g.planets.len() as PlanetId {
        let ps = &g.planets[p as usize];
        let mut in_orbit: Vec<FactionId> = g.fleets_at(p).filter(|f| w.is_combat_fleet(f)).map(|f| f.owner).collect();
        in_orbit.sort_unstable();
        in_orbit.dedup();
        if let (&[x], Some(o), None) = (in_orbit.as_slice(), ps.owner, ps.starbase)
            && w.real[p as usize]
            && g.hostile(w, x, o)
        {
            out.insert(p, x);
        }
    }
    out
}

/// The rules alone (no scripts, no AI, nobody moves): every occupied planet of the save falls to the fleet in its orbit after
/// `capture_delay_seconds`, without a single battle. Before, each of them produced one battle per tick and kept its owner.
#[test]
fn the_stuck_planets_of_the_save_fall_by_the_capture_rule() {
    let Some(l) = load() else { return };
    let campaign = l.data.campaign(&l.file.header.campaign).expect("campaign");
    let mut sim = Sim::new(&l.data, campaign, l.file.header.sim_config.clone()).expect("sim");
    sim.restore(l.sim.with_sim(|s| s.snapshot())).expect("restore");
    let w = sim.world().clone();
    assert!(!w.config.auto_resolve_battles, "the save is of the game, which settles battles itself");
    let stuck = occupied(&w, sim.galaxy());
    let name = |f: FactionId| w.factions[f as usize].name.clone();
    let owners = |g: &Galaxy| stuck.keys().map(|&p| g.planets[p as usize].owner).collect::<Vec<_>>();
    let before = owners(sim.galaxy());
    let towers = stuck.keys().filter(|&&p| sim.galaxy().planets[p as usize].structures.iter().any(|&s| w.structures[s as usize].name.ends_with("_Watchtower"))).count();
    assert_eq!((stuck.len(), towers), (9, 9), "nine planets of the save are held by a lone hostile fleet, each with a watchtower");

    let mut watch = BattleRepeats::for_world(&w);
    let (mut battles, mut captured) = (0, BTreeMap::new());
    let start = sim.galaxy().time;
    for _ in 0..60 {
        let events = sim.step(1.0);
        let now = sim.galaxy().time;
        assert!(watch.check(now, &events).is_empty());
        for e in &events {
            match e {
                Event::BattlePending(b) if stuck.contains_key(&b.planet) => battles += 1,
                Event::PlanetCaptured { planet, new_owner, .. } if stuck.contains_key(planet) => {
                    captured.insert(*planet, (*new_owner, now - start));
                }
                _ => {}
            }
        }
        for id in sim.pending_battles().iter().map(|b| b.id).collect::<Vec<_>>() {
            sim.resolve_battle(id);
        }
    }
    for ((&p, &x), old) in stuck.iter().zip(&before) {
        println!("{}: {} -> {} ({:?})", w.planets[p as usize].name, old.map(name).unwrap_or_default(), sim.galaxy().planets[p as usize].owner.map(name).unwrap_or_default(), captured.get(&p).map(|c| c.1));
        assert_eq!(captured.get(&p), Some(&(x, f64::from(w.config.capture_delay_seconds))), "{} falls to the fleet in its orbit", w.planets[p as usize].name);
        assert_eq!(sim.galaxy().planets[p as usize].owner, Some(x));
    }
    assert_eq!(battles, 0, "nothing in orbit to fight at any of them");
}

/// The game as it runs (scripts and AI, battles settled after every tick): no battle comes back unchanged, and the occupied planets do not
/// keep their owners.
#[test]
fn the_save_plays_on_without_repeated_battles() {
    let Some(mut l) = load() else { return };
    let w = l.sim.with_sim(|s| s.world().clone());
    let stuck = l.sim.with_sim(|s| occupied(&w, s.galaxy()));
    let before: Vec<Option<FactionId>> = l.sim.with_sim(|s| stuck.keys().map(|&p| s.galaxy().planets[p as usize].owner).collect());
    let mut watch = BattleRepeats::for_world(&w);
    let mut per_planet: BTreeMap<PlanetId, u32> = BTreeMap::new();
    let mut repeated = Vec::new();
    for _ in 0..60 {
        let events = l.sim.step(1.0).sim_events;
        let now = l.sim.with_sim(|s| s.galaxy().time);
        repeated.extend(watch.check(now, &events));
        for e in &events {
            if let Event::BattlePending(b) = e {
                *per_planet.entry(b.planet).or_default() += 1;
            }
        }
        for id in l.sim.with_sim(|s| s.pending_battles().iter().map(|b| b.id).collect::<Vec<_>>()) {
            l.sim.resolve_battle(id);
        }
    }
    let name = |f: Option<FactionId>| f.map(|f| w.factions[f as usize].name.clone()).unwrap_or_default();
    let after: Vec<Option<FactionId>> = l.sim.with_sim(|s| stuck.keys().map(|&p| s.galaxy().planets[p as usize].owner).collect());
    for ((&p, old), new) in stuck.keys().zip(&before).zip(&after) {
        println!("{}: {} -> {}, {} battles in 60 s", w.planets[p as usize].name, name(*old), name(*new), per_planet.get(&p).copied().unwrap_or(0));
    }
    println!("battles in 60 s: {} at {} planets, most at one planet {}", per_planet.values().sum::<u32>(), per_planet.len(), per_planet.values().max().copied().unwrap_or(0));
    assert!(repeated.is_empty(), "battles created again with the same line-up: {repeated:?}");
    // Six battles a minute at one planet would be one per `battle_repeat_seconds`: a fight that goes on. It was 60 (one per tick) at nine planets.
    assert!(per_planet.values().all(|&n| n <= 6), "{per_planet:?}");
    assert_eq!(before.iter().zip(&after).filter(|(a, b)| a != b).count(), 9, "every occupied planet changed hands");
}
