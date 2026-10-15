//! Ground-to-space special weapons (Hypervelocity Gun, Ion Cannon) built on a planet fire at the enemy ships of the
//! space battle over it, with the damage and recharge of the XML. Real-data test, skips without the game and mod.

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

/// Battle over a Rebel planet: the owner holds `structure` (and a defending MC-80 so the fight lasts), two ISDs attack.
/// Returns what the observer saw: (weapon ever fired, attackers' total hull at the start and the end of the observations).
fn fight(data: &GameData, fs: &GameFs, structure: &str, with_starbase: bool) -> (bool, f32, f32, f32) {
    let campaign = data.campaign("GC_Full_Web_Way_Rebel").expect("campaign");
    let mut sim = Sim::new(data, campaign, SimConfig { auto_resolve_battles: false, victory_enabled: false, ..SimConfig::default() }).expect("sim");
    let w = sim.world().clone();
    let (empire, rebel) = (w.faction_id("Empire").unwrap(), w.faction_id("Rebel").unwrap());
    let sid = w.structures.iter().position(|s| s.name.eq_ignore_ascii_case(structure)).expect("structure type") as u32;
    let target = (0..w.planets.len() as u32)
        .find(|&p| w.real[p as usize] && w.planets[p as usize].space_map.is_some() && sim.galaxy().planets[p as usize].owner == Some(rebel))
        .expect("a rebel planet with a map");
    let neighbour = w.routes.iter().find_map(|r| (r.a == target).then_some(r.b).or((r.b == target).then_some(r.a))).expect("a route");
    let (isd, mc80) = (w.unit_id("Star_Destroyer").unwrap(), w.unit_id("MC-80_Star_Cruiser").unwrap());
    {
        let g = sim.galaxy_mut();
        if !with_starbase {
            g.planets[target as usize].starbase = None;
        }
        g.planets[target as usize].structures = vec![sid];
        let mut add = |owner, unit, n, from| {
            let id = g.fleets.keys().max().copied().unwrap_or(0) + 1;
            g.fleets.insert(id, Fleet { id, owner, units: [(unit, n)].into(), location: FleetLocation::AtPlanet(target), previous_planet: from, slot: 0 });
        };
        add(empire, isd, 2, neighbour);
        add(rebel, mc80, 1, target);
    }
    let battle = sim.step(1.0).into_iter().find_map(|e| if let Event::BattlePending(b) = e { Some(b) } else { None }).expect("pending battle");
    let req = sim.tactical_setup(battle.id).expect("request");
    let rs = req.sides.iter().position(|s| s.faction == rebel).unwrap();
    let names: Vec<&str> = req.sides.iter().flat_map(|s| s.groups.iter().map(|g| g.name.as_str())).collect();
    assert!(names.iter().any(|n| n.eq_ignore_ascii_case(structure)), "the structure takes part in the battle: {names:?}");
    let catalog = Arc::new(Catalog::build(data, Some(fs), &names));
    let arena = req.map.as_ref().and_then(|m| Arena::load(data, fs, m)).map(Arc::new);
    let (mut fired, mut start, mut last) = (false, -1.0f32, 0.0f32);
    let mut max_cd = 0.0f32;
    run_headless_observed(&req, &catalog, arena, &mut |b| {
        let enemy_hull: f32 = b.ships.iter().filter(|s| s.alive && s.side != rs as u8 && !catalog.ship(s.def).is_static).map(|s| s.hull).sum();
        if start < 0.0 {
            start = enemy_hull;
        }
        last = enemy_hull;
        for s in b.ships.iter().filter(|s| s.side == rs as u8 && catalog.ship(s.def).special_weapon.is_some()) {
            max_cd = max_cd.max(s.special_cd);
            if s.special_pulses > 0 || s.special_cd > 16.0 {
                fired = true;
            }
        }
    })
    .expect("result");
    (fired, start, last, max_cd)
}

#[test]
fn special_weapon_structures_fire_in_the_space_battle() {
    let Some((data, fs)) = load() else { return };
    for (name, with_starbase) in [("Hypervelocity_Gun", true), ("Seismic_Cannon", true), ("Nuke_Launcher", true), ("Ground_Ion_Cannon", true), ("Hypervelocity_Gun", false), ("Seismic_Cannon", false), ("Nuke_Launcher", false), ("Ground_Ion_Cannon", false)] {
        let (fired, start, last, max_cd) = fight(&data, &fs, name, with_starbase);
        eprintln!("{name} starbase {with_starbase}: fired {fired}, attacker hull {start:.0} -> {last:.0}, max cooldown {max_cd:.1}");
        assert!(fired, "{name} (starbase {with_starbase}) never fired");
        if name != "Ground_Ion_Cannon" {
            assert!(max_cd > 70.0, "{name} recharges for 80 s (Projectile_Fire_Recharge_Seconds), saw at most {max_cd}");
        }
    }
}
