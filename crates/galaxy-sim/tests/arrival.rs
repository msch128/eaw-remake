//! A fleet that arrives at an enemy planet: enemy fleet -> battle; only a star base or armed structures -> battle too (you fight the station);
//! no space defenses at all -> no battle, the capture rule applies. And no battle comes back unchanged: a ground building is no party in
//! orbit, a ground-to-space weapon gets one battle, an undecided fight is repeated at most once per `SimConfig::battle_repeat_seconds`.

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

/// Plays `seconds` the way the game does (one tick, then every pending battle is settled) and returns the battles that appeared at the
/// target with the time they were created.
fn play(sim: &mut Sim, target: PlanetId, seconds: u32) -> Vec<(f64, PendingBattle)> {
    let mut seen = Vec::new();
    for _ in 0..seconds {
        for e in sim.step(1.0) {
            if let Event::BattlePending(b) = e
                && b.planet == target
            {
                seen.push((sim.galaxy().time, b));
            }
        }
        for id in sim.pending_battles().iter().map(|b| b.id).collect::<Vec<_>>() {
            sim.resolve_battle_auto(id);
        }
    }
    seen
}

/// Watchtowers, fortresses and turret towers are ground buildings (`DUMMY_GROUND_STRUCTURE` with a `LandBehavior` and no `SpaceBehavior`):
/// they are no party in orbit. A tower used to make its owner a defender that no space battle could destroy, so the game created a new
/// battle every tick and the planet never fell.
#[test]
fn a_ground_tower_does_not_hold_the_orbit() {
    let Some(data) = load() else { return };
    for tower in ["Empire_Watchtower", "Empire_Fortress", "Rocket_Tower", "Flak_Tower"] {
        let mut s = setup(&data, |sim, target, _| {
            sim.set_starbase(target, None);
            let w = sim.world().clone();
            sim.galaxy_mut().planets[target as usize].structures = vec![w.structure_id(tower).expect(tower)];
        });
        let w = s.sim.world().clone();
        assert!(!w.blockades(s.sim.galaxy(), s.target, s.rebel), "{tower} stops no fleet");
        let battle = arrive(&mut s);
        assert!(battle.is_none(), "{tower} has nothing in space to fight: {battle:?}");
        let battles = play(&mut s.sim, s.target, 10);
        assert!(battles.is_empty(), "{tower}: {} battles in 10 s", battles.len());
        assert_eq!(s.sim.galaxy().planets[s.target as usize].owner, Some(s.rebel), "{tower}: the lone fleet takes the planet");
    }
}

/// A ground-to-space weapon is the planet's only defence: the arriving fleet gets its battle (the weapon fires at it), and once that battle
/// is decided the planet falls by the capture rule. The weapon cannot be hit, so the same battle used to come back every tick.
#[test]
fn a_ground_to_space_weapon_alone_gives_one_battle_and_then_the_planet_falls() {
    let Some(data) = load() else { return };
    for gun in ["Hypervelocity_Gun", "Ground_Ion_Cannon"] {
        let mut s = setup(&data, |sim, target, _| {
            sim.set_starbase(target, None);
            let w = sim.world().clone();
            sim.galaxy_mut().planets[target as usize].structures = vec![w.structure_id(gun).expect(gun)];
        });
        let b = arrive(&mut s).unwrap_or_else(|| panic!("{gun} fires at the fleet"));
        assert_eq!(b.defender, Some(s.empire));
        s.sim.resolve_battle_auto(b.id).expect("the battle is settled");
        assert!(s.sim.galaxy().fleets.contains_key(&s.fleet), "{gun}: three MC-80 outlast the weapon");
        let battles = play(&mut s.sim, s.target, 10);
        assert!(battles.is_empty(), "{gun}: {} more battles in 10 s with nothing changed", battles.len());
        assert_eq!(s.sim.galaxy().planets[s.target as usize].owner, Some(s.rebel), "{gun}: the planet falls");
    }
}

/// Two hostile fleets that cannot decide their battle (no combat rounds here) and have nowhere to fall back to stay in orbit together.
/// The battle is not created again every tick: at most once per `SimConfig::battle_repeat_seconds` while nothing changes, and at once
/// when something does.
#[test]
fn an_undecided_battle_is_not_refought_every_tick() {
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let mut sim = Sim::new(&data, campaign, SimConfig { auto_resolve_battles: false, victory_enabled: false, combat_max_rounds: 0, ..SimConfig::default() }).unwrap();
    let w = sim.world().clone();
    let (rebel, empire) = (w.faction_id("Rebel").unwrap(), w.faction_id("Empire").unwrap());
    sim.galaxy_mut().fleets.clear();
    sim.galaxy_mut().battles.clear();
    let target = (0..w.planets.len() as u32).find(|&p| w.real[p as usize] && sim.galaxy().planets[p as usize].owner == Some(empire)).unwrap();
    sim.set_starbase(target, None);
    sim.galaxy_mut().planets[target as usize].structures.clear();
    // Both fleets appear at the planet, so neither has a planet it came from.
    sim.spawn_units(empire, target, w.unit_id("Star_Destroyer").unwrap(), 2);
    let mc80 = w.unit_id("MC-80_Star_Cruiser").unwrap();
    sim.spawn_units(rebel, target, mc80, 2);
    let start = sim.galaxy().time;
    let battles = play(&mut sim, target, 30);
    let times: Vec<f64> = battles.iter().map(|(t, _)| t - start).collect();
    assert_eq!(w.config.battle_repeat_seconds, 10.0);
    assert_eq!(times, [1.0, 11.0, 21.0], "one battle per 10 s while nothing changes");
    // The rule as the soak runs check it: none of these battles is a repeat, and they are the same line-up every time.
    let mut watch = BattleRepeats::for_world(&w);
    assert!(battles.iter().all(|(t, b)| watch.see(*t, b) == Sighting::Fresh { after_repeats: 0 }));
    assert!(battles.iter().all(|(_, b)| b.line_up == battles[0].1.line_up && b.line_up != 0));
    assert_eq!(sim.galaxy().fleets_at(target).count(), 2, "nobody was lost and nobody left");
    // A reinforcement changes who is there: the battle is on again at once.
    sim.spawn_units(rebel, target, mc80, 1);
    let again = play(&mut sim, target, 1);
    assert_eq!(again.len(), 1, "a changed line-up is fought without waiting");
    assert_ne!(again[0].1.line_up, battles[0].1.line_up);
    assert_eq!(watch.see(again[0].0, &again[0].1), Sighting::Fresh { after_repeats: 0 });
}

/// A save of an earlier build has neither the rule's number in its header nor the new fields in its state: it loads with the default
/// window and judges the first battle at every planet anew.
#[test]
fn a_save_without_the_repeat_rule_loads_with_its_defaults() {
    let mut config = serde_json::to_value(SimConfig::default()).unwrap();
    assert!(config.as_object_mut().unwrap().remove("battle_repeat_seconds").is_some());
    assert_eq!(serde_json::from_value::<SimConfig>(config).unwrap().battle_repeat_seconds, 10.0);
    let battle: PendingBattle = serde_json::from_str(r#"{"id":7,"planet":3,"fleets":[5],"factions":[0,1],"defender":1}"#).unwrap();
    assert_eq!((battle.id, battle.line_up), (7, 0));
    let Some(data) = load() else { return };
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").unwrap();
    let sim = Sim::new(&data, campaign, SimConfig::default()).unwrap();
    let mut planet = serde_json::to_value(&sim.galaxy().planets[0]).unwrap();
    assert!(planet.as_object_mut().unwrap().remove("battle_settled").is_some());
    assert_eq!(serde_json::from_value::<PlanetState>(planet).unwrap().battle_settled, None);
}
