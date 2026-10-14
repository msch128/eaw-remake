//! A simple AI player: expands its economy, builds the best affordable ships and sends merged
//! fleets against the nearest planet it is clearly stronger than.
//!
//! [`plan`] only reads the world and state, so the plans of several factions can be computed in
//! parallel; the simulation applies the resulting commands in faction order.

use crate::combat::Source;
use crate::rng::Rng;
use crate::state::*;
use crate::world::*;

/// Credits the AI keeps in reserve.
const RESERVE: f64 = 1500.0;
/// Attack only if own strength exceeds the defense estimate by this factor.
const ATTACK_RATIO: f32 = 1.5;

/// Computes the commands `faction` wants to issue right now.
pub fn plan(world: &World, galaxy: &Galaxy, faction: FactionId, rng: &mut Rng) -> Vec<Command> {
    let mut cmds = Vec::new();
    if galaxy.factions[faction as usize].eliminated {
        return cmds;
    }
    plan_economy(world, galaxy, faction, rng, &mut cmds);
    plan_military(world, galaxy, faction, rng, &mut cmds);
    cmds
}

fn plan_economy(world: &World, galaxy: &Galaxy, faction: FactionId, rng: &mut Rng, cmds: &mut Vec<Command>) {
    let fs = &galaxy.factions[faction as usize];
    let mut budget = fs.credits - RESERVE;
    let owned: Vec<PlanetId> = galaxy.owned_planets(faction).collect();

    // Tech: buy the next level at the best developed planet.
    if let Some(&p) = owned.iter().max_by_key(|&&p| (world.starbase_level(galaxy, p), std::cmp::Reverse(p)))
        && let Ok(idx) = world.check_upgrade_tech(galaxy, faction, p)
    {
        let cost = world.factions[faction as usize].tech_upgrades[idx].cost as f64;
        if cost <= budget {
            budget -= cost;
            cmds.push(Command::UpgradeTech { faction, planet: p });
        }
    }

    // Shuffle the visiting order a little so poorer planets are not always starved.
    let mut order = owned.clone();
    for i in (1..order.len()).rev() {
        order.swap(i, rng.below(i as u64 + 1) as usize);
    }
    for &p in &order {
        let ps = &galaxy.planets[p as usize];
        let value = world.planets[p as usize].credit_value;

        // Star base first, then upgrades on valuable planets.
        if ps.queues[Lane::Base as usize].is_empty()
            && let Ok(next) = world.check_upgrade_starbase(galaxy, faction, p)
        {
            let sb = &world.starbases[next as usize];
            let rich = budget > 60.0 * sb.cost as f64;
            let wanted = ps.starbase.is_none()
                || sb.level <= 2
                || (value >= 100.0 && sb.level <= 4)
                || world.factions[faction as usize].home == Some(p)
                || rich;
            if wanted && (sb.cost as f64) <= budget {
                budget -= sb.cost as f64;
                cmds.push(Command::UpgradeStarbase { faction, planet: p });
            }
        }
        if ps.starbase.is_none() {
            continue;
        }

        // Structures: prefer ones that unlock buildable units, then income.
        let has_unlocker = ps.structures.iter().any(|&s| world.structure_unlocks[s as usize] > 0);
        if ps.queues[Lane::Structures as usize].is_empty() && (!has_unlocker || rng.unit() < 0.35) {
            let mut best: Option<(f32, StructureTypeId)> = None;
            for &s in &world.buildable_structures[faction as usize] {
                let st = &world.structures[s as usize];
                if (st.cost as f64) > budget || world.check_build_structure(galaxy, faction, p, s).is_err() {
                    continue;
                }
                let unlocks = world.structure_unlocks[s as usize] as f32;
                // Without a shipyard-like structure keep the orbital slots free for one.
                if !has_unlocker && unlocks == 0.0 && st.orbital {
                    continue;
                }
                let roi = (st.income_abs + value * st.income_pct) / st.cost.max(1.0) * 1000.0;
                let score = unlocks.min(8.0) * 2.0 + roi + rng.unit();
                if score > 1.0 && best.is_none_or(|(b, _)| score > b) {
                    best = Some((score, s));
                }
            }
            if let Some((_, s)) = best {
                budget -= world.structures[s as usize].cost as f64;
                cmds.push(Command::BuildStructure { faction, planet: p, structure: s });
            }
        }

        // Units.
        // A rich faction keeps several ships in the queue.
        let allowed = 1 + (budget.max(0.0) / 40_000.0) as usize;
        if ps.queues[Lane::Units as usize].len() < allowed.min(world.config.max_queue_len) && budget > 0.0 {
            let mut best: Option<(f32, UnitTypeId)> = None;
            for &u in &world.buildable_units[faction as usize] {
                let ut = &world.units[u as usize];
                if !matches!(ut.role, UnitRole::Ship) || ut.cost <= 0.0 || (ut.cost as f64) > budget {
                    continue;
                }
                if world.check_build_unit(galaxy, faction, p, u).is_err() {
                    continue;
                }
                let eff = ut.combat.durability() * ut.combat.raw_dps();
                let score = eff.sqrt() / ut.cost * (0.8 + 0.4 * rng.unit());
                if best.is_none_or(|(b, _)| score > b) {
                    best = Some((score, u));
                }
            }
            if let Some((_, u)) = best {
                budget -= world.units[u as usize].cost as f64;
                cmds.push(Command::BuildUnit { faction, planet: p, unit: u });
            }
        }
    }
}

fn fleet_strength(world: &World, f: &Fleet) -> f32 {
    world.strength(f.units.iter().map(|(&u, &c)| (Source::Unit(u), c)))
}

/// Estimated defensive strength of a planet from the point of view of an attacker.
fn defense_strength(world: &World, galaxy: &Galaxy, planet: PlanetId, attacker: FactionId) -> f32 {
    let ps = &galaxy.planets[planet as usize];
    let mut parts: Vec<(Source, u32)> = Vec::new();
    if let Some(o) = ps.owner
        && galaxy.hostile(world, attacker, o)
    {
        if let Some(sb) = ps.starbase {
            parts.push((Source::Starbase(sb), 1));
            parts.extend(world.starbases[sb as usize].garrison.iter().map(|&(u, c)| (Source::Garrison(u), c)));
        }
        parts.extend(ps.structures.iter().filter(|&&s| { let st = &world.structures[s as usize]; st.combat.is_some() || st.space_reach }).map(|&s| (Source::Structure(s), 1)));
    }
    for f in galaxy.fleets_at(planet).filter(|f| galaxy.hostile(world, attacker, f.owner)) {
        parts.extend(f.units.iter().map(|(&u, &c)| (Source::Unit(u), c)));
    }
    world.strength(parts)
}

fn plan_military(world: &World, galaxy: &Galaxy, faction: FactionId, rng: &mut Rng, cmds: &mut Vec<Command>) {
    // Group idle fleets per planet.
    let mut idle: Vec<(PlanetId, Vec<&Fleet>)> = Vec::new();
    for f in galaxy.fleets.values().filter(|f| f.owner == faction) {
        if let FleetLocation::AtPlanet(p) = f.location {
            if galaxy.battles.iter().any(|b| b.planet == p) {
                continue;
            }
            match idle.iter_mut().find(|(q, _)| *q == p) {
                Some((_, v)) => v.push(f),
                None => idle.push((p, vec![f])),
            }
        }
    }
    idle.sort_by_key(|(p, _)| *p);
    for (p, fleets) in idle {
        let main = fleets[0];
        for other in &fleets[1..] {
            cmds.push(Command::MergeFleets { faction, into: main.id, from: other.id });
        }
        let mut total = main.clone();
        for other in &fleets[1..] {
            for (&u, &c) in &other.units {
                *total.units.entry(u).or_default() += c;
            }
        }
        let strength = fleet_strength(world, &total);
        if strength <= 0.0 {
            continue;
        }
        let (dist, _) = world.shortest_paths(p);
        let mut best: Option<(f32, PlanetId)> = None;
        for t in 0..world.planets.len() as PlanetId {
            if !world.real[t as usize] || t == p || dist[t as usize] == u64::MAX {
                continue;
            }
            let owner = galaxy.planets[t as usize].owner;
            if owner == Some(faction) || owner.is_some_and(|o| !galaxy.hostile(world, faction, o)) {
                continue;
            }
            let hops_cost = dist[t as usize] as f32 / 100.0;
            if hops_cost > 900.0 {
                continue;
            }
            let defense = defense_strength(world, galaxy, t, faction);
            if strength <= defense * ATTACK_RATIO || (defense > 0.0 && total.unit_count() < 3) {
                continue;
            }
            let value = world.planets[t as usize].credit_value + 50.0;
            let score = value / (1.0 + hops_cost / 60.0) * (0.9 + 0.2 * rng.unit());
            if best.is_none_or(|(b, _)| score > b) {
                best = Some((score, t));
            }
        }
        if let Some((_, t)) = best {
            cmds.push(Command::MoveFleetAround { faction, fleet: main.id, to: t });
        }
    }
}
