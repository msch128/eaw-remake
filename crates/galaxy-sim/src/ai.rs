//! A simple AI player: expands its economy, builds the best affordable ships and sends merged
//! fleets against the nearest planet it is clearly stronger than.
//!
//! [`plan`] only reads the world and state, so the plans of several factions can be computed in
//! parallel; the simulation applies the resulting commands in faction order.

use crate::combat::{Member, SideInput, Source};
use crate::rng::Rng;
use crate::state::*;
use crate::world::*;

/// Credits the AI keeps in reserve.
const RESERVE: f64 = 1500.0;
/// Attack only if own strength exceeds the defense estimate by this factor.
const ATTACK_RATIO: f32 = 1.5;
/// The AI saves at most this many weeks of income for a ship (estimate).
const SAVING_WEEKS: f64 = 4.0;

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

    // A faction that cannot build a single ship anywhere saves for the cheapest shipyard it could build next instead of spending its credits on cheaper
    // things: shipyards are expensive in the data, and an AI that never saves for one never fields a ship.
    let saving = shipyard_goal(world, galaxy, faction, &owned);

    // Tech: buy the next level at the best developed planet.
    if let Some(&p) = owned.iter().max_by_key(|&&p| (world.starbase_level(galaxy, p), std::cmp::Reverse(p)))
        && let Ok(idx) = world.check_upgrade_tech(galaxy, faction, p)
    {
        let cost = world.factions[faction as usize].tech_upgrades[idx].cost as f64;
        if cost <= budget && saving.is_none_or(|(goal, _)| budget - cost >= goal) {
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
            if wanted && (sb.cost as f64) <= budget && saving.is_none_or(|(goal, level)| sb.level <= level || budget - (sb.cost as f64) >= goal) {
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
                if saving.is_some_and(|(goal, _)| !needed_for_ships(world, galaxy, faction, s) && budget - (st.cost as f64) < goal) {
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
        let crowded = fs.population_used * 2 >= fs.population_cap;
        // A rich faction keeps several ships in the queue.
        let allowed = 1 + (budget.max(0.0) / 40_000.0) as usize;
        if ps.queues[Lane::Units as usize].len() < allowed.min(world.config.max_queue_len) && budget > 0.0 {
            let mut best: Option<(f32, UnitTypeId)> = None;
            for &u in &world.buildable_units[faction as usize] {
                let ut = &world.units[u as usize];
                if !matches!(ut.role, UnitRole::Ship) || ut.cost <= 0.0 || f64::from(ut.cost) > budget.max(0.0) + if crowded { SAVING_WEEKS * fs.income_per_week.max(0.0) } else { 0.0 } {
                    continue;
                }
                if !matches!(world.check_build_unit(galaxy, faction, p, u), Ok(()) | Err(CommandError::InsufficientCredits)) {
                    continue;
                }
                let eff = ut.combat.durability() * ut.combat.raw_dps();
                // Once the population cap is half used, strong ships come first: cheap squadrons are the best per credit but fill the cap and cannot take a defended planet; the AI saves for the best ship it can build.
                let score = if crowded { eff.sqrt() / ut.cost.sqrt() } else { eff.sqrt() / ut.cost } * (0.8 + 0.4 * rng.unit());
                if best.is_none_or(|(b, _)| score > b) {
                    best = Some((score, u));
                }
            }
            if let Some((_, u)) = best.filter(|&(_, u)| f64::from(world.units[u as usize].cost) <= budget) {
                budget -= world.units[u as usize].cost as f64;
                cmds.push(Command::BuildUnit { faction, planet: p, unit: u });
            }
        }
    }
}

fn fleet_strength(world: &World, f: &Fleet) -> f32 {
    world.tuned_strength(f.units.iter().map(|(&u, &c)| (Source::Unit(u), c)))
}

/// What defends a planet against `attacker`, as the battle would field it: the owner's star base with its garrison and armed structures, and the
/// hostile fleets in orbit. The passive faction's base and garrison scale to the attacker's tech level like in `Sim::battle_sides`.
///
/// `known` is [`Galaxy::ai_knowledge`] of the attacker. At a planet it does not know, only what the map shows is used (**estimate**, labelled: the owner's star base and its
/// garrison); fleets in orbit and armed structures stay hidden, so the AI can be surprised there.
fn defense_parts(world: &World, galaxy: &Galaxy, known: &[bool], planet: PlanetId, attacker: FactionId) -> Vec<(Source, u32)> {
    let ps = &galaxy.planets[planet as usize];
    let mut parts: Vec<(Source, u32)> = Vec::new();
    let passive = world.passive_faction();
    let mut units_left = u32::MAX;
    let mut starbase = ps.starbase;
    if ps.owner.is_some() && ps.owner == passive {
        let tech = galaxy.factions.get(attacker as usize).map_or(1, |s| s.tech);
        let (sb, max_units) = world.passive_defense_at(planet, ps.owner.unwrap_or(0), tech, ps.starbase);
        starbase = sb;
        units_left = max_units;
    }
    if let Some(o) = ps.owner
        && galaxy.hostile(world, attacker, o)
    {
        if let Some(sb) = starbase {
            parts.push((Source::Starbase(sb), 1));
            parts.extend(world.starbases[sb as usize].garrison.iter().map(|&(u, c)| (Source::Garrison(u), c)));
        }
        if known[planet as usize] {
            parts.extend(ps.structures.iter().filter(|&&s| { let st = &world.structures[s as usize]; st.combat.is_some() || st.space_reach }).map(|&s| (Source::Structure(s), 1)));
        }
    }
    for f in galaxy.fleets_at(planet).filter(|f| known[planet as usize] && galaxy.hostile(world, attacker, f.owner)) {
        for (&u, &c) in &f.units {
            let take = if Some(f.owner) == passive { c.min(units_left) } else { c };
            if Some(f.owner) == passive {
                units_left -= take;
            }
            if take > 0 {
                parts.push((Source::Unit(u), take));
            }
        }
    }
    parts
}

/// Estimated defensive strength of a planet from the point of view of an attacker.
fn defense_strength(world: &World, galaxy: &Galaxy, known: &[bool], planet: PlanetId, attacker: FactionId) -> f32 {
    world.tuned_strength(defense_parts(world, galaxy, known, planet, attacker))
}

/// Plays the battle on `fleet` against the planet's defence with the statistical resolver (fixed seed) and says whether the attacker wins. The
/// Lanchester estimate alone can be far off against star bases and swarms of small craft: a fleet that "outguns" a base 2 to 1 may still lose it all.
fn wins_battle(world: &World, galaxy: &Galaxy, known: &[bool], faction: FactionId, fleet: &Fleet, planet: PlanetId) -> bool {
    let defence = defense_parts(world, galaxy, known, planet, faction);
    if defence.is_empty() {
        return true;
    }
    let side = |faction: FactionId, parts: Vec<(Source, u32)>| SideInput { faction, members: parts.into_iter().map(|(source, count)| Member { source, fleet: None, count }).collect() };
    let owner = galaxy.planets[planet as usize].owner.unwrap_or(faction);
    let sides = [side(faction, fleet.units.iter().map(|(&u, &c)| (Source::Unit(u), c)).collect()), side(owner, defence)];
    let mut rng = Rng::new(0xA1_5EED ^ (u64::from(planet) << 16) ^ u64::from(faction));
    let result = crate::combat::resolve_allied(world, &sides, &mut rng, &galaxy.allied, &[]);
    result.winner == Some(0)
}

/// [`World::blockades`] as the AI knows it: at a planet it does not know it counts only the star base (visible on the map), not armed structures or fleets.
fn blockades_known(world: &World, galaxy: &Galaxy, known: &[bool], planet: PlanetId, owner: FactionId) -> bool {
    if known[planet as usize] {
        return world.blockades(galaxy, planet, owner);
    }
    let ps = &galaxy.planets[planet as usize];
    ps.starbase.is_some() && ps.owner.is_some_and(|o| galaxy.hostile(world, owner, o))
}

fn plan_military(world: &World, galaxy: &Galaxy, faction: FactionId, rng: &mut Rng, cmds: &mut Vec<Command>) {
    let known = galaxy.ai_knowledge(world, faction);
    let known = &known[..];
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
        let mut candidates: Vec<(f32, PlanetId)> = Vec::new();
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
            let defense = defense_strength(world, galaxy, known, t, faction);
            if strength <= defense * ATTACK_RATIO || (defense > 0.0 && total.unit_count() < 3) {
                continue;
            }
            let value = world.planets[t as usize].credit_value + 50.0;
            let score = value / (1.0 + hops_cost / 60.0) * (0.9 + 0.2 * rng.unit());
            candidates.push((score, t));
        }
        // Best score first; a target is only worth it when the fleet can also fight through the blockades on its way (it stops at the first hostile
        // star base or fleet; going the long way round is tried first, like `MoveFleetAround`).
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let best = candidates.into_iter().find(|&(_, t)| {
            let Some(path) = world.find_path_avoiding(p, t, |q| q != t && blockades_known(world, galaxy, known, q, faction)) else { return false };
            path.iter().filter(|&&q| q != p && q != t).all(|&q| !blockades_known(world, galaxy, known, q, faction) || wins_battle(world, galaxy, known, faction, &total, q)) && wins_battle(world, galaxy, known, faction, &total, t)
        });
        if let Some((_, t)) = best {
            cmds.push(Command::MoveFleetAround { faction, fleet: main.id, to: t });
        }
    }
}

/// When the faction cannot build any shipyard-built ship (a ship that lists `Required_Special_Structures`, i.e. more than a fighter squadron) at any owned planet now: the credits it needs to change that on the cheapest planet (the next shipyard, a structure
/// some ship lists in `Required_Special_Structures`, plus the star base upgrades up to its `Required_Star_Base_Level`) and the star base level that path needs.
/// `None` if it can build a ship, or no shipyard is within reach.
fn shipyard_goal(world: &World, galaxy: &Galaxy, faction: FactionId, owned: &[PlanetId]) -> Option<(f64, i32)> {
    let credit_only = |r: Result<(), CommandError>| matches!(r, Ok(()) | Err(CommandError::InsufficientCredits));
    let can_ship = owned.iter().any(|&p| {
        world.buildable_units[faction as usize]
            .iter()
            .any(|&u| matches!(world.units[u as usize].role, UnitRole::Ship) && !world.units[u as usize].req_structures.is_empty() && credit_only(world.check_build_unit(galaxy, faction, p, u)))
    });
    if can_ship {
        return None;
    }
    let tech = galaxy.factions[faction as usize].tech;
    let mut best: Option<(f64, i32)> = None;
    for &p in owned {
        let ps = &galaxy.planets[p as usize];
        for &s in &world.buildable_structures[faction as usize] {
            let st = &world.structures[s as usize];
            if !needed_for_ships(world, galaxy, faction, s)
                || st.tech > tech
                || (!st.req_planets.is_empty() && !st.req_planets.contains(&p))
                || !world.planet_takes_starbase(p, st.req_starbase)
                || world.missing_structures(ps, &st.req_structures).is_some()
            {
                continue;
            }
            // Star base upgrades on the way (a planet whose chain ends below the requirement is skipped).
            let (mut cost, mut level) = (f64::from(st.cost), world.starbase_level(galaxy, p));
            let mut cur = ps.starbase;
            while level < st.req_starbase {
                let next = match cur {
                    None => world.first_starbase[faction as usize],
                    Some(c) => world.starbases[c as usize].next,
                };
                let Some(n) = next else { break };
                cost += f64::from(world.starbases[n as usize].cost);
                level = world.starbases[n as usize].level;
                cur = Some(n);
            }
            if level >= st.req_starbase && best.is_none_or(|(c, _)| cost < c) {
                best = Some((cost, st.req_starbase));
            }
        }
    }
    best
}

/// True if a ship the faction can field at its tech lists `s` in its `Required_Special_Structures`.
fn needed_for_ships(world: &World, galaxy: &Galaxy, faction: FactionId, s: StructureTypeId) -> bool {
    let tech = galaxy.factions[faction as usize].tech;
    world.buildable_units[faction as usize].iter().any(|&u| {
        let ut = &world.units[u as usize];
        matches!(ut.role, UnitRole::Ship) && ut.tech <= tech && ut.req_structures.iter().any(|g| g.contains(&s))
    })
}

/// What the AI of `attacker` believes defends `planet` (fog of war applied, see [`Galaxy::ai_knowledge`]); the strength every attack decision uses.
pub fn believed_defense(world: &World, galaxy: &Galaxy, attacker: FactionId, planet: PlanetId) -> f32 {
    defense_strength(world, galaxy, &galaxy.ai_knowledge(world, attacker), planet, attacker)
}
