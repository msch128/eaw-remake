//! A fleet that arrives at an enemy planet: enemy fleet -> battle; only a star base or armed structures -> battle too (you fight the station);
//! no space defenses at all -> no battle, the capture rule applies.

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

struct Setup {
    sim: Sim,
    rebel: FactionId,
    empire: FactionId,
    target: PlanetId,
    from: PlanetId,
    fleet: FleetId,
}

/// A Rebel fleet (the human) waits next to an Empire planet that has no fleet; `prepare` shapes the target.
fn setup(data: &GameData, prepare: impl FnOnce(&mut Sim, PlanetId, FactionId)) -> Setup {
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(data, campaign, SimConfig { auto_resolve_battles: false, victory_enabled: false, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let (rebel, empire) = (w.faction_id("Rebel").unwrap(), w.faction_id("Empire").unwrap());
    sim.galaxy_mut().fleets.clear();
    sim.galaxy_mut().battles.clear();
    // An Empire planet with a star base, next to a planet that is not Empire's.
    let (target, from) = (0..w.planets.len() as u32)
        .filter(|&p| w.real[p as usize] && sim.galaxy().planets[p as usize].owner == Some(empire) && sim.galaxy().planets[p as usize].starbase.is_some())
        .find_map(|p| w.routes.iter().find_map(|r| (r.a == p).then_some(r.b).or((r.b == p).then_some(r.a))).filter(|&n| w.real[n as usize]).map(|n| (p, n)))
        .expect("an Empire star base planet with a neighbour");
    prepare(&mut sim, target, empire);
    let fleet = sim.spawn_units(rebel, from, w.unit_id("MC-80_Star_Cruiser").unwrap(), 3);
    Setup { sim, rebel, empire, target, from, fleet }
}

/// Orders the fleet to the target and runs until it has arrived and a tick has passed; returns the pending battle if any.
fn arrive(s: &mut Setup) -> Option<PendingBattle> {
    s.sim.apply(Command::MoveFleet { faction: s.rebel, fleet: s.fleet, to: s.target }).expect("move");
    let mut battle = None;
    for _ in 0..4000 {
        for e in s.sim.step(1.0) {
            if let Event::BattlePending(b) = e {
                battle = Some(b);
            }
        }
        if battle.is_some() || s.sim.galaxy().fleets.get(&s.fleet).and_then(|f| f.planet()) == Some(s.target) && s.sim.galaxy().battles.is_empty() {
            // One more tick so that a battle that starts when the fleet has settled is seen.
            for e in s.sim.step(1.0) {
                if let Event::BattlePending(b) = e {
                    battle = Some(b);
                }
            }
            break;
        }
    }
    battle
}

#[test]
fn an_enemy_fleet_in_orbit_makes_a_battle() {
    let Some(data) = load() else { return };
    let mut s = setup(&data, |sim, target, empire| {
        let w = sim.world().clone();
        sim.spawn_units(empire, target, w.unit_id("Star_Destroyer").unwrap(), 2);
    });
    let b = arrive(&mut s).expect("a battle is pending");
    assert_eq!(b.planet, s.target);
    assert!(b.factions.contains(&s.rebel) && b.factions.contains(&s.empire));
    assert!(b.fleets.contains(&s.fleet));
}

#[test]
fn only_a_star_base_still_makes_a_battle() {
    let Some(data) = load() else { return };
    let mut s = setup(&data, |_, _, _| {});
    assert!(s.sim.galaxy().planets[s.target as usize].starbase.is_some());
    assert!(s.sim.galaxy().fleets_at(s.target).next().is_none(), "no fleet defends the planet");
    let b = arrive(&mut s).expect("you fight the station");
    assert_eq!(b.defender, Some(s.empire));
    assert!(b.factions.contains(&s.rebel));
    // The tactical set-up has the star base on the defending side and the human on the attacking side.
    let req = s.sim.tactical_setup(b.id).expect("request");
    assert!(req.starbase_level >= 1);
    assert!(req.sides.last().unwrap().groups.iter().any(|g| g.kind == GroupKind::Starbase));
    assert!(!req.sides.first().unwrap().defender && req.sides.first().unwrap().faction == s.rebel);
    // No capture while the station stands.
    s.sim.step(30.0);
    assert_eq!(s.sim.galaxy().planets[s.target as usize].owner, Some(s.empire));
}

#[test]
fn a_planet_without_space_defenses_is_taken_by_the_capture_rule() {
    let Some(data) = load() else { return };
    let mut s = setup(&data, |sim, target, _| {
        sim.set_starbase(target, None);
        let w = sim.world().clone();
        let armed: Vec<_> = sim.galaxy().planets[target as usize].structures.iter().copied().filter(|&st| w.structures[st as usize].combat.is_some()).collect();
        for st in armed {
            sim.remove_structure(target, st);
        }
    });
    let battle = arrive(&mut s);
    assert!(battle.is_none(), "nothing to fight: {battle:?}");
    assert!(s.sim.galaxy().battles.is_empty());
    let _ = s.from;
    let mut taken = false;
    for _ in 0..40 {
        taken |= s.sim.step(1.0).iter().any(|e| matches!(e, Event::PlanetCaptured { planet, new_owner, .. } if *planet == s.target && *new_owner == s.rebel));
    }
    assert!(taken, "the lone fleet captures the undefended planet");
    assert_eq!(s.sim.galaxy().planets[s.target as usize].owner, Some(s.rebel));
}
