//! The simulation driver: initial state, commands, time stepping, movement, battles, capture.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::sync::Arc;

use alamo_data::{Campaign, GameData};

use crate::combat::{self, Member, SideInput, Source};
use crate::state::*;
use crate::world::*;

/// A running match: immutable [`World`] plus mutable [`Galaxy`] state.
pub struct Sim {
    pub(crate) world: Arc<World>,
    pub(crate) galaxy: Galaxy,
    pub(crate) ai: Vec<FactionId>,
    pub(crate) tactical: Option<Arc<battle_sim::Catalog>>,
    pub(crate) arenas: std::collections::HashMap<String, Arc<battle_sim::Arena>>,
    pub(crate) tactical_requests: BTreeMap<u32, crate::tactical::TacticalRequest>,
    /// Events produced by commands, delivered by the next [`Sim::step`].
    pub(crate) queued_events: Vec<Event>,
}

impl Sim {
    /// Builds the world for `campaign` and sets up the starting situation: owners, star bases,
    /// structures, fleets, credits and tech levels.
    pub fn new(data: &GameData, campaign: &Campaign, config: SimConfig) -> Result<Sim, SimError> {
        let seed = config.seed;
        let world = Arc::new(World::build(data, campaign, config)?);
        let mut galaxy = Galaxy::new(&world, seed);

        // Owners from the starting structures.
        for (planet, faction) in data.campaign_planet_owners(campaign) {
            if let (Some(p), Some(f)) = (world.planet_id(&planet), world.faction_id(&faction)) {
                galaxy.planets[p as usize].owner = Some(f);
            }
        }
        // Starting forces.
        for sf in &campaign.starting_forces {
            let (Some(p), Some(f)) = (world.planet_id(&sf.planet), world.faction_id(&sf.faction)) else { continue };
            let ps = &mut galaxy.planets[p as usize];
            if let Some(&sb) = world.starbase_index.get(&sf.object.to_ascii_lowercase()) {
                if ps.owner == Some(f) {
                    let better = ps.starbase.is_none_or(|cur| world.starbases[cur as usize].level < world.starbases[sb as usize].level);
                    if better {
                        ps.starbase = Some(sb);
                    }
                }
            } else if let Some(&st) = world.structure_index.get(&sf.object.to_ascii_lowercase()) {
                if ps.owner == Some(f) {
                    ps.structures.push(st);
                }
            } else if let Some(&u) = world.unit_index.get(&sf.object.to_ascii_lowercase()) {
                add_units(&mut galaxy, f, p, u, 1);
            }
        }
        let mut sim = Sim { world, galaxy, ai: Vec::new(), tactical: None, arenas: std::collections::HashMap::new(), tactical_requests: BTreeMap::new(), queued_events: Vec::new() };
        if sim.world.config.ground_up {
            let human = sim.world.victory.active_player;
            sim.apply_ground_up(human);
        }
        sim.rearm_passive(false);
        sim.world.clone().refresh_economy(&mut sim.galaxy);
        Ok(sim)
    }

    /// Immutable world data (shareable between threads).
    pub fn world(&self) -> &Arc<World> {
        &self.world
    }

    /// Current state.
    pub fn galaxy(&self) -> &Galaxy {
        &self.galaxy
    }

    /// Mutable state access for tests and tools.
    pub fn galaxy_mut(&mut self) -> &mut Galaxy {
        &mut self.galaxy
    }

    /// Makes the built-in AI play these factions.
    pub fn set_ai_factions(&mut self, factions: impl IntoIterator<Item = FactionId>) {
        self.ai = factions.into_iter().collect();
    }

    /// Battles that wait for resolution (only non-empty when `auto_resolve_battles` is off).
    pub fn pending_battles(&self) -> &[PendingBattle] {
        &self.galaxy.battles
    }

    /// Validates and executes a command.
    pub fn apply(&mut self, cmd: Command) -> Result<CommandOk, CommandError> {
        let world = self.world.clone();
        let g = &mut self.galaxy;
        // The passive faction of a "From the Ground Up" game never orders anything.
        if world.passive_faction().is_some_and(|p| cmd.faction() == Some(p)) {
            return Err(CommandError::NotAvailable);
        }
        let around = matches!(cmd, Command::MoveFleetAround { .. });
        match cmd {
            Command::BuildUnit { faction, planet, unit } => {
                world.check_build_unit(g, faction, planet, unit)?;
                let (cost, secs) = world.unit_cost(g, faction, planet, unit);
                g.factions[faction as usize].credits -= f64::from(cost);
                g.factions[faction as usize].stats.credits_spent += f64::from(cost);
                let t = secs * world.config.build_time_scale * g.factions[faction as usize].difficulty.build_time;
                g.planets[planet as usize].queues[Lane::Units as usize].push_back(ProductionItem {
                    kind: ProductionKind::Unit(unit),
                    remaining: t,
                    total: t,
                    cost,
                });
                Ok(CommandOk::Done)
            }
            Command::BuildStructure { faction, planet, structure } => {
                world.check_build_structure(g, faction, planet, structure)?;
                let st = &world.structures[structure as usize];
                g.factions[faction as usize].credits -= st.cost as f64;
                g.factions[faction as usize].stats.credits_spent += f64::from(st.cost);
                let t = st.build_time * world.config.build_time_scale * g.factions[faction as usize].difficulty.build_time;
                g.planets[planet as usize].queues[Lane::Structures as usize].push_back(ProductionItem {
                    kind: ProductionKind::Structure(structure),
                    remaining: t,
                    total: t,
                    cost: st.cost,
                });
                Ok(CommandOk::Done)
            }
            Command::UpgradeStarbase { faction, planet } => {
                let next = world.check_upgrade_starbase(g, faction, planet)?;
                let sb = &world.starbases[next as usize];
                g.factions[faction as usize].credits -= sb.cost as f64;
                g.factions[faction as usize].stats.credits_spent += f64::from(sb.cost);
                let t = sb.build_time * world.config.build_time_scale * g.factions[faction as usize].difficulty.build_time;
                g.planets[planet as usize].queues[Lane::Base as usize].push_back(ProductionItem {
                    kind: ProductionKind::Starbase(next),
                    remaining: t,
                    total: t,
                    cost: sb.cost,
                });
                Ok(CommandOk::Done)
            }
            Command::UpgradeTech { faction, planet } => {
                let idx = world.check_upgrade_tech(g, faction, planet)?;
                let up = &world.factions[faction as usize].tech_upgrades[idx];
                g.factions[faction as usize].credits -= up.cost as f64;
                g.factions[faction as usize].stats.credits_spent += f64::from(up.cost);
                let t = up.time * world.config.build_time_scale * g.factions[faction as usize].difficulty.build_time;
                g.planets[planet as usize].queues[Lane::Base as usize].push_back(ProductionItem {
                    kind: ProductionKind::Tech(idx),
                    remaining: t,
                    total: t,
                    cost: up.cost,
                });
                Ok(CommandOk::Done)
            }
            Command::DebugGrant { faction, planet, unit, count } => {
                if !world.config.debug_commands {
                    return Err(CommandError::NotAvailable);
                }
                if g.factions.get(faction as usize).is_none_or(|f| f.eliminated) || planet as usize >= world.planets.len() || unit as usize >= world.units.len() {
                    return Err(CommandError::Invalid);
                }
                let id = self.spawn_units(faction, planet, unit, count.clamp(1, 50));
                Ok(CommandOk::NewFleet(id))
            }
            Command::Cheat { faction, cheat } => crate::cheats::apply(&world, g, faction, cheat),
            Command::SellStructure { faction, planet, structure } => {
                let ps = g.planets.get_mut(planet as usize).ok_or(CommandError::Invalid)?;
                if ps.owner != Some(faction) {
                    return Err(CommandError::NotOwner);
                }
                let st = world.structures.get(structure as usize).ok_or(CommandError::Invalid)?;
                let at = ps.structures.iter().position(|&s| s == structure).ok_or(CommandError::Invalid)?;
                ps.structures.remove(at);
                let refund = f64::from(st.cost) / 3.0;
                g.factions[faction as usize].credits += refund;
                Ok(CommandOk::Refunded(refund))
            }
            Command::DestroyPlanet { faction, planet } => {
                let fleet = g
                    .fleets
                    .values()
                    .find(|f| f.owner == faction && f.location == FleetLocation::AtPlanet(planet) && f.units.iter().any(|(&u, &n)| n > 0 && world.units[u as usize].planet_destroyer))
                    .map(|f| f.id)
                    .ok_or(CommandError::NotAvailable)?;
                world.can_destroy_planet(g, faction, fleet, planet).map_err(CommandError::from)?;
                let ps = &g.planets[planet as usize];
                g.factions[faction as usize].superweapon_ready_at = g.time + f64::from(world.config.superweapon_cooldown_seconds);
                let owner = ps.owner;
                // Everyone else's ships in orbit die with the planet.
                let doomed: Vec<FleetId> = g.fleets.iter().filter(|(_, f)| f.owner != faction && f.location == FleetLocation::AtPlanet(planet)).map(|(&id, _)| id).collect();
                let mut ships_lost = 0;
                for id in doomed {
                    if let Some(f) = g.fleets.remove(&id) {
                        ships_lost += f.units.values().sum::<u32>();
                    }
                }
                g.battles.retain(|b| b.planet != planet);
                g.sync_ships();
                let mut events = Vec::new();
                self.apply_owner_change(planet, None, false, &mut events);
                let ps = &mut self.galaxy.planets[planet as usize];
                ps.destroyed = true;
                ps.base_health = None;
                events.push(Event::PlanetDestroyed { planet, by: faction, owner, ships_lost });
                self.queued_events.extend(events);
                Ok(CommandOk::Done)
            }
            Command::MoveFleet { faction, fleet, to } | Command::MoveFleetAround { faction, fleet, to } => {
                let f = g.fleets.get(&fleet).filter(|f| f.owner == faction).ok_or(CommandError::Invalid)?;
                let FleetLocation::AtPlanet(from) = f.location else { return Err(CommandError::WrongState) };
                if to as usize >= world.planets.len() {
                    return Err(CommandError::Invalid);
                }
                if from == to {
                    return Ok(CommandOk::Done);
                }
                if g.battles.iter().any(|b| b.fleets.contains(&fleet)) {
                    return Err(CommandError::WrongState);
                }
                // `MoveFleetAround` goes around systems that would stop the fleet, when there is a way around; `MoveFleet` takes the fastest route.
                let path = {
                    let gal: &Galaxy = g;
                    world.find_path_avoiding(from, to, |p| around && p != to && world.blockades(gal, p, faction))
                }
                .ok_or(CommandError::NoPath)?;
                g.escaping.remove(&fleet);
                start_move(&world, g, fleet, path);
                Ok(CommandOk::Done)
            }
            Command::MergeFleets { faction, into, from } => {
                if into == from {
                    return Err(CommandError::Invalid);
                }
                let a = g.fleets.get(&into).filter(|f| f.owner == faction).ok_or(CommandError::Invalid)?;
                let b = g.fleets.get(&from).filter(|f| f.owner == faction).ok_or(CommandError::Invalid)?;
                match (&a.location, &b.location) {
                    (FleetLocation::AtPlanet(x), FleetLocation::AtPlanet(y)) if x == y => {}
                    _ => return Err(CommandError::WrongState),
                }
                g.sync_ships();
                let b = g.fleets.remove(&from).expect("checked");
                let moved = g.ships.remove(&from).unwrap_or_default();
                if let Some(h) = g.hyperspace.remove(&from) {
                    g.hyperspace.entry(into).or_insert(h);
                }
                g.ships.entry(into).or_default().extend(moved);
                let a = g.fleets.get_mut(&into).expect("checked");
                for (u, c) in b.units {
                    *a.units.entry(u).or_default() += c;
                }
                Ok(CommandOk::Done)
            }
            Command::SplitFleet { faction, fleet, units } => {
                g.sync_ships();
                let f = g.fleets.get_mut(&fleet).filter(|f| f.owner == faction).ok_or(CommandError::Invalid)?;
                let FleetLocation::AtPlanet(p) = f.location else { return Err(CommandError::WrongState) };
                let mut taken = BTreeMap::new();
                for (u, c) in units {
                    if c == 0 {
                        continue;
                    }
                    if f.units.get(&u).copied().unwrap_or(0) < c + taken.get(&u).copied().unwrap_or(0) {
                        return Err(CommandError::Invalid);
                    }
                    *taken.entry(u).or_insert(0u32) += c;
                }
                if taken.is_empty() || taken.values().sum::<u32>() == f.unit_count() {
                    return Err(CommandError::Invalid);
                }
                let prev = f.previous_planet;
                for (u, c) in &taken {
                    let e = f.units.get_mut(u).expect("checked");
                    *e -= c;
                    if *e == 0 {
                        f.units.remove(u);
                    }
                }
                let id = g.next_fleet_id;
                g.next_fleet_id += 1;
                let mut moved_ids = Vec::new();
                for (u, c) in &taken {
                    for _ in 0..*c {
                        // The source fleet already lost the units; its id list still holds them.
                        if let Some(list) = g.ships.get_mut(&fleet)
                            && let Some(pos) = list.iter().rposition(|(_, t)| t == u)
                        {
                            moved_ids.push(list.remove(pos));
                        }
                    }
                }
                if g.hyperspace.get(&fleet).is_some_and(|h| moved_ids.iter().any(|(s, _)| *s == h.0)) {
                    g.hyperspace.remove(&fleet);
                }
                g.ships.insert(id, moved_ids);
                g.fleets.insert(id, Fleet { id, owner: faction, units: taken, location: FleetLocation::AtPlanet(p), previous_planet: prev });
                Ok(CommandOk::NewFleet(id))
            }
            Command::TransferUnits { faction, from, to, units, hyperspace } => {
                g.sync_ships();
                let src = g.fleets.get(&from).filter(|f| f.owner == faction).ok_or(CommandError::Invalid)?;
                let FleetLocation::AtPlanet(p) = src.location else { return Err(CommandError::WrongState) };
                if to == Some(from) {
                    return Err(CommandError::Invalid);
                }
                if let Some(t) = to {
                    let dst = g.fleets.get(&t).filter(|f| f.owner == faction).ok_or(CommandError::Invalid)?;
                    if dst.location != FleetLocation::AtPlanet(p) {
                        return Err(CommandError::WrongState);
                    }
                }
                let mut taken: BTreeMap<UnitTypeId, u32> = BTreeMap::new();
                for (u, c) in units {
                    if c > 0 {
                        *taken.entry(u).or_insert(0) += c;
                    }
                }
                if hyperspace && taken.values().sum::<u32>() != 1 {
                    return Err(CommandError::Invalid);
                }
                if taken.is_empty() || taken.iter().any(|(u, c)| src.units.get(u).copied().unwrap_or(0) < *c) {
                    return Err(CommandError::Invalid);
                }
                // Ships of the hyperspace slot stay behind unless nothing else of that type is left.
                let slot = g.hyperspace.get(&from).map(|h| h.0);
                let mut moved = Vec::new();
                for (u, c) in &taken {
                    let list = g.ships.get_mut(&from).expect("synced");
                    for _ in 0..*c {
                        let pos = list.iter().rposition(|(s, t)| t == u && Some(*s) != slot).or_else(|| list.iter().rposition(|(_, t)| t == u));
                        moved.push(list.remove(pos.expect("counts checked")));
                    }
                }
                if slot.is_some_and(|s| moved.iter().any(|(m, _)| *m == s)) {
                    g.hyperspace.remove(&from);
                }
                let src = g.fleets.get_mut(&from).expect("checked");
                for (u, c) in &taken {
                    let e = src.units.get_mut(u).expect("checked");
                    *e -= c;
                    if *e == 0 {
                        src.units.remove(u);
                    }
                }
                let prev = src.previous_planet;
                let emptied = src.units.is_empty();
                let target = match to {
                    Some(t) => t,
                    None => {
                        let id = g.next_fleet_id;
                        g.next_fleet_id += 1;
                        g.fleets.insert(id, Fleet { id, owner: faction, units: BTreeMap::new(), location: FleetLocation::AtPlanet(p), previous_planet: prev });
                        id
                    }
                };
                let dst = g.fleets.get_mut(&target).expect("exists");
                for (u, c) in &taken {
                    *dst.units.entry(*u).or_default() += c;
                }
                if hyperspace {
                    g.hyperspace.insert(target, moved[0]);
                }
                g.ships.entry(target).or_default().extend(moved);
                if emptied {
                    g.fleets.remove(&from);
                    g.ships.remove(&from);
                    g.hyperspace.remove(&from);
                }
                Ok(if to.is_none() { CommandOk::NewFleet(target) } else { CommandOk::Done })
            }
            Command::SetHyperspace { faction, fleet, unit } => {
                g.sync_ships();
                g.fleets.get(&fleet).filter(|f| f.owner == faction).ok_or(CommandError::Invalid)?;
                match unit {
                    Some(u) => {
                        let list = g.ships.get(&fleet).map(|l| l.as_slice()).unwrap_or_default();
                        if g.hyperspace.get(&fleet).is_some_and(|h| h.1 == u) {
                            return Ok(CommandOk::Done);
                        }
                        let pick = list.iter().rev().find(|(_, t)| *t == u).copied().ok_or(CommandError::Invalid)?;
                        g.hyperspace.insert(fleet, pick);
                    }
                    None => {
                        g.hyperspace.remove(&fleet);
                    }
                }
                Ok(CommandOk::Done)
            }
            Command::CancelProduction { faction, planet, lane, index } => {
                let ps = g.planets.get_mut(planet as usize).ok_or(CommandError::Invalid)?;
                if ps.owner != Some(faction) {
                    return Err(CommandError::NotOwner);
                }
                let q = &mut ps.queues[lane as usize];
                let item = q.remove(index as usize).ok_or(CommandError::Invalid)?;
                // Cost is paid when queued; the unspent share of the build time comes back (estimate x `cancel_refund`: the real game charges while it builds).
                let unspent = if item.total > 0.0 { (item.remaining / item.total).clamp(0.0, 1.0) } else { 1.0 };
                let refund = f64::from(item.cost) * f64::from(unspent) * f64::from(world.config.cancel_refund);
                g.factions[faction as usize].credits += refund;
                g.factions[faction as usize].stats.credits_spent -= refund;
                // Queued structures that needed the cancelled one (and nothing else provides it) cannot start any more: they go back too.
                if lane == Lane::Structures {
                    let mut refunded = refund;
                    loop {
                        let ps = &mut g.planets[planet as usize];
                        let orphan = (0..ps.queues[Lane::Structures as usize].len()).find(|&i| {
                            let ProductionKind::Structure(s) = ps.queues[Lane::Structures as usize][i].kind else { return false };
                            let mut earlier = ps.clone();
                            earlier.queues[Lane::Structures as usize].truncate(i);
                            world.missing_structures_queued(&earlier, &world.structures[s as usize].req_structures).is_some()
                        });
                        let Some(i) = orphan else { break };
                        let it = ps.queues[Lane::Structures as usize].remove(i).expect("index found above");
                        let back = f64::from(it.cost) * f64::from(world.config.cancel_refund);
                        g.factions[faction as usize].credits += back;
                        g.factions[faction as usize].stats.credits_spent -= back;
                        refunded += back;
                    }
                    return Ok(CommandOk::Refunded(refunded));
                }
                Ok(CommandOk::Refunded(refund))
            }
            Command::Retreat { faction, fleet } => {
                let f = g.fleets.get_mut(&fleet).filter(|f| f.owner == faction).ok_or(CommandError::Invalid)?;
                match f.location.clone() {
                    FleetLocation::Moving { from, to, route, progress, .. } => {
                        // Turn around on the lane.
                        let len = world.routes[route as usize].length;
                        f.location = FleetLocation::Moving { from: to, to: from, route, progress: (len - progress).max(0.0), rest: Vec::new() };
                        Ok(CommandOk::Done)
                    }
                    FleetLocation::AtPlanet(p) => {
                        let back = f.previous_planet;
                        if back == p {
                            return Err(CommandError::WrongState);
                        }
                        g.battles.iter_mut().for_each(|b| b.fleets.retain(|&x| x != fleet));
                        let path = world.find_path(p, back).ok_or(CommandError::NoPath)?;
                        start_move(&world, g, fleet, path);
                        Ok(CommandOk::Done)
                    }
                }
            }
        }
    }

    /// Advances the simulation by `dt` seconds (in internal ticks) and returns what happened.
    pub fn step(&mut self, dt: f32) -> Vec<Event> {
        let mut events = std::mem::take(&mut self.queued_events);
        self.galaxy.tick_accum += dt;
        let tick = self.world.config.tick_seconds;
        while self.galaxy.tick_accum >= tick && self.galaxy.winner.is_none() {
            self.galaxy.tick_accum -= tick;
            self.tick(tick, &mut events);
        }
        self.galaxy.sync_ships();
        // The cached income and population are current at the end of every step (structures and ships finished since the last refresh count).
        self.world.clone().refresh_economy(&mut self.galaxy);
        events
    }

    fn tick(&mut self, dt: f32, events: &mut Vec<Event>) {
        let world = self.world.clone();
        let w = &*world;
        self.galaxy.time += dt as f64;

        // Economy.
        w.refresh_economy(&mut self.galaxy);
        self.repair(dt);
        for fs in self.galaxy.factions.iter_mut() {
            if fs.eliminated {
                continue;
            }
            let cap = w.config.credit_cap_per_planet * fs.planets_owned as f64;
            let income = if fs.income_per_week > 0.0 { fs.income_per_week * f64::from(fs.difficulty.credit) } else { fs.income_per_week };
            let before = fs.credits;
            fs.credits += income * dt as f64 / w.config.week_seconds as f64;
            if fs.planets_owned > 0 && fs.credits > cap {
                fs.credits = cap;
            }
            fs.credits = fs.credits.max(0.0);
            fs.stats.credits_earned += (fs.credits - before).max(0.0);
        }

        self.gamble_income();
        self.produce(dt, events);
        self.move_fleets(dt, events);
        self.detect_battles(events);
        self.capture(dt, events);
        self.check_elimination(events);

        // Weeks.
        while self.galaxy.time >= (self.galaxy.week + 1) as f64 * w.config.week_seconds as f64 {
            self.galaxy.week += 1;
            events.push(Event::WeekEnded { week: self.galaxy.week });
            self.rearm_passive(true);
            self.check_victory(events);
        }

        // AI.
        if !self.ai.is_empty() && self.galaxy.winner.is_none() && !self.galaxy.ai_paused {
            self.galaxy.ai_timer += dt;
            if self.galaxy.ai_timer >= w.config.ai_interval_seconds {
                self.galaxy.ai_timer = 0.0;
                self.run_ai();
            }
        }
    }

    fn run_ai(&mut self) {
        let passive = self.world.passive_faction();
        let ai: Vec<FactionId> = self.ai.iter().copied().filter(|&f| Some(f) != passive).collect();
        let world = self.world.clone();
        let mut rngs: Vec<_> = ai.iter().map(|&f| self.galaxy.rng.fork(f as u64 + 1)).collect();
        let galaxy = &self.galaxy;
        let plans: Vec<Vec<Command>> = std::thread::scope(|s| {
            let handles: Vec<_> = ai
                .iter()
                .zip(rngs.iter_mut())
                .map(|(&f, rng)| {
                    let w = &*world;
                    s.spawn(move || crate::ai::plan(w, galaxy, f, rng))
                })
                .collect();
            handles.into_iter().map(|h| h.join().expect("ai thread")).collect()
        });
        for cmds in plans {
            for c in cmds {
                let _ = self.apply(c);
            }
        }
    }

    fn produce(&mut self, dt: f32, events: &mut Vec<Event>) {
        let world = self.world.clone();
        let w = &*world;
        for pi in 0..self.galaxy.planets.len() {
            let Some(owner) = self.galaxy.planets[pi].owner else { continue };
            for lane in 0..3 {
                let done = {
                    let q = &mut self.galaxy.planets[pi].queues[lane];
                    match q.front_mut() {
                        Some(item) => {
                            item.remaining -= dt;
                            item.remaining <= 0.0
                        }
                        None => false,
                    }
                };
                if !done {
                    continue;
                }
                let item = self.galaxy.planets[pi].queues[lane].pop_front().expect("front exists");
                let planet = pi as PlanetId;
                match item.kind.clone() {
                    ProductionKind::Unit(u) => {
                        let fs = &mut self.galaxy.factions[owner as usize];
                        *fs.built_lifetime.entry(u).or_default() += 1;
                        match &w.units[u as usize].role {
                            UnitRole::Ship => {
                                add_units(&mut self.galaxy, owner, planet, u, 1);
                            }
                            UnitRole::Research(target) => {
                                fs.unlocked.insert(target.to_ascii_lowercase());
                            }
                        }
                    }
                    ProductionKind::Structure(s) => self.galaxy.planets[pi].structures.push(s),
                    ProductionKind::Starbase(sb) => self.galaxy.planets[pi].starbase = Some(sb),
                    ProductionKind::Tech(_) => {
                        self.galaxy.factions[owner as usize].tech += 1;
                        events.push(Event::TechAdvanced { faction: owner, level: self.galaxy.factions[owner as usize].tech });
                    }
                }
                let st = &mut self.galaxy.factions[owner as usize].stats;
                match item.kind {
                    ProductionKind::Unit(_) => st.ships_built += 1,
                    ProductionKind::Structure(_) | ProductionKind::Starbase(_) => st.structures_built += 1,
                    ProductionKind::Tech(_) => {}
                }
                events.push(Event::ConstructionComplete { faction: owner, planet, kind: item.kind });
            }
        }
    }

    fn move_fleets(&mut self, dt: f32, events: &mut Vec<Event>) {
        let world = self.world.clone();
        let w = &*world;
        let ids: Vec<FleetId> = self
            .galaxy
            .fleets
            .iter()
            .filter(|(_, f)| matches!(f.location, FleetLocation::Moving { .. }))
            .map(|(&id, _)| id)
            .collect();
        for id in ids {
            let speed = w.fleet_speed(&self.galaxy.fleets[&id]);
            loop {
                let f = self.galaxy.fleets.get_mut(&id).expect("fleet exists");
                let FleetLocation::Moving { from, to, route, progress, rest } = f.location.clone() else { break };
                let r = &w.routes[route as usize];
                let new_progress = progress + speed * r.hs_factor * w.config.galactic_speed_scale * dt;
                if new_progress < r.length {
                    f.location = FleetLocation::Moving { from, to, route, progress: new_progress, rest };
                    break;
                }
                // Arrived at `to`.
                let leftover_time = if speed > 0.0 {
                    (new_progress - r.length) / (speed * r.hs_factor * w.config.galactic_speed_scale)
                } else {
                    0.0
                };
                let owner = f.owner;
                let blocked = !rest.is_empty() && !self.galaxy.escaping.contains(&id) && w.blockades(&self.galaxy, to, owner);
                if rest.is_empty() {
                    self.galaxy.escaping.remove(&id);
                }
                let f = self.galaxy.fleets.get_mut(&id).expect("fleet exists");
                if rest.is_empty() || blocked {
                    f.location = FleetLocation::AtPlanet(to);
                    f.previous_planet = from;
                    events.push(Event::FleetArrived { fleet: id, planet: to });
                    break;
                }
                // Continue along the path.
                let next = rest[0];
                let next_route = w.route_between(to, next).expect("path edges exist");
                f.location = FleetLocation::Moving {
                    from: to,
                    to: next,
                    route: next_route,
                    progress: leftover_time * speed * w.routes[next_route as usize].hs_factor * w.config.galactic_speed_scale,
                    rest: rest[1..].to_vec(),
                };
            }
        }
    }

    /// Finds planets where hostile parties meet and starts (or resolves) a battle there.
    /// Evaluates the gambling structures (`Planet_Income_Gambling_Ability`) whose time has come.
    fn gamble_income(&mut self) {
        let world = self.world.clone();
        let now = self.galaxy.time;
        for p in 0..self.galaxy.planets.len() {
            let Some(g) = self.galaxy.planets[p].structures.iter().find_map(|&s| world.structures[s as usize].gamble) else {
                if self.galaxy.planets[p].gamble != 0.0 {
                    self.galaxy.planets[p].gamble = 0.0;
                }
                continue;
            };
            if now >= self.galaxy.planets[p].gamble_at {
                let win = self.galaxy.rng.unit() < g.chance;
                let ps = &mut self.galaxy.planets[p];
                ps.gamble = if win { g.max } else { g.min };
                ps.gamble_at = now + f64::from(g.interval);
            }
        }
    }

    fn detect_battles(&mut self, events: &mut Vec<Event>) {
        let world = self.world.clone();
        let w = &*world;
        let mut by_planet: BTreeMap<PlanetId, Vec<FactionId>> = BTreeMap::new();
        for f in self.galaxy.fleets.values() {
            if let FleetLocation::AtPlanet(p) = f.location {
                let v = by_planet.entry(p).or_default();
                if !v.contains(&f.owner) {
                    v.push(f.owner);
                }
            }
        }
        for (p, mut factions) in by_planet {
            if self.galaxy.battles.iter().any(|b| b.planet == p) {
                continue;
            }
            let ps = &self.galaxy.planets[p as usize];
            let defender = ps.owner.filter(|_| ps.starbase.is_some() || ps.structures.iter().any(|&s| w.structures[s as usize].combat.is_some()));
            if let Some(d) = defender
                && !factions.contains(&d)
            {
                factions.push(d);
            }
            let hostile_pair = factions.iter().any(|&a| factions.iter().any(|&b| self.galaxy.hostile(w, a, b)));
            if !hostile_pair {
                continue;
            }
            factions.sort_unstable();
            let fleets = self
                .galaxy
                .fleets
                .values()
                .filter(|f| f.location == FleetLocation::AtPlanet(p))
                .map(|f| f.id)
                .collect();
            let id = self.galaxy.next_battle_id;
            self.galaxy.next_battle_id += 1;
            let battle = PendingBattle { id, planet: p, fleets, factions, defender };
            self.galaxy.battles.push(battle.clone());
            events.push(Event::BattlePending(battle));
            if w.config.auto_resolve_battles {
                events.extend(self.resolve_battle(id).unwrap_or_default());
            }
        }
    }


    /// What the statistical auto-resolve would make of pending battle `id` with a given `seed`, without changing anything (calibration and UI estimates).
    /// Per side: faction, ships at the start and ships alive at the end (star base, structures and garrison squadrons are not counted), plus the winner.
    pub fn preview_auto_resolve(&self, id: u32, seed: u64) -> Option<AutoPreview> {
        self.preview_auto_resolve_with(id, seed, combat::Tuning::of(&self.world))
    }

    /// [`Sim::preview_auto_resolve`] with explicit tuning (calibration).
    pub fn preview_auto_resolve_with(&self, id: u32, seed: u64, tuning: combat::Tuning) -> Option<AutoPreview> {
        let battle = self.galaxy.battles.iter().find(|b| b.id == id)?;
        let sides = self.battle_sides(battle);
        let mut rng = crate::rng::Rng::new(seed);
        let mut result = combat::resolve_tuned(&self.world, &sides, &mut rng, &self.galaxy.allied, &self.galaxy.factions.iter().map(|f| f.difficulty).collect::<Vec<_>>(), tuning);
        self.apply_escape(&sides, &mut result);
        let ships = |ms: &[Member]| ms.iter().filter(|m| matches!(m.source, Source::Unit(_))).map(|m| m.count).sum::<u32>();
        Some(AutoPreview {
            sides: sides.iter().enumerate().map(|(i, s)| (s.faction, ships(&s.members), ships(&result.survivors[i]))).collect(),
            winner: result.winner.map(|i| sides[i].faction),
        })
    }

    /// Resolves a pending battle with the statistical auto-resolve and applies the result.
    /// Returns `None` if the battle id is unknown.
    pub fn resolve_battle_auto(&mut self, id: u32) -> Option<Vec<Event>> {
        self.resolve_inner(id, false)
    }

    /// Resolves a pending battle with the resolver chosen in [`SimConfig::resolver`].
    pub fn resolve_battle(&mut self, id: u32) -> Option<Vec<Event>> {
        let small_enough = self.galaxy.battles.iter().find(|b| b.id == id).is_none_or(|b| {
            let ships: u32 = self.battle_sides(b).iter().flat_map(|s| &s.members).filter(|m| !matches!(m.source, Source::Garrison(_))).map(|m| m.count).sum();
            ships <= self.world.config.tactical_resolver_max_ships
        });
        let tactical = self.world.config.resolver == BattleResolver::Tactical && self.tactical.is_some() && small_enough;
        self.resolve_inner(id, tactical)
    }

    /// Resolves a pending battle by simulating the tactical battle headless (needs a catalog from
    /// [`Sim::set_tactical_catalog`]; falls back to the statistical resolver otherwise).
    pub fn resolve_battle_tactical(&mut self, id: u32) -> Option<Vec<Event>> {
        let tactical = self.tactical.is_some();
        self.resolve_inner(id, tactical)
    }

    /// Provides the `battle-sim` catalog used by the tactical resolver. Build it with
    /// `battle_sim::Catalog::build(data, Some(fs), &sim.tactical_names())`.
    pub fn set_tactical_catalog(&mut self, catalog: Arc<battle_sim::Catalog>) {
        self.tactical = Some(catalog);
    }

    /// Provides tactical map arenas by map file name (see `battle_sim::Arena::load`); battles over a
    /// planet use the arena of its `Space_Tactical_Map`.
    pub fn set_tactical_arenas(&mut self, arenas: std::collections::HashMap<String, Arc<battle_sim::Arena>>) {
        self.arenas = arenas.into_iter().map(|(k, v)| (k.to_ascii_lowercase(), v)).collect();
    }

    /// Distinct `Space_Tactical_Map` names of the galaxy's planets.
    pub fn tactical_map_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.world.planets.iter().filter_map(|p| p.space_map.clone()).collect();
        v.sort_by_key(|s| s.to_ascii_lowercase());
        v.dedup_by_key(|s| s.to_ascii_lowercase());
        v
    }

    /// Names of every unit, star base and structure type, i.e. what the tactical catalog must contain.
    pub fn tactical_names(&self) -> Vec<String> {
        let w = &self.world;
        let mut v: Vec<String> = w.units.iter().filter(|u| matches!(u.role, UnitRole::Ship)).map(|u| u.name.clone()).collect();
        v.extend(w.starbases.iter().map(|s| s.name.clone()));
        v.extend(w.structures.iter().filter(|s| s.combat.is_some()).map(|s| s.name.clone()));
        v
    }

    /// The combatants of a pending battle, one side per faction.
    pub(crate) fn battle_sides(&self, battle: &PendingBattle) -> Vec<SideInput> {
        let w = &*self.world;
        let p = battle.planet;
        let ps = &self.galaxy.planets[p as usize];
        let mut sides: Vec<SideInput> = Vec::new();
        for &fac in &battle.factions {
            let mut members = Vec::new();
            for fid in &battle.fleets {
                let Some(f) = self.galaxy.fleets.get(fid) else { continue };
                if f.owner != fac || f.location != FleetLocation::AtPlanet(p) {
                    continue;
                }
                for (&u, &c) in &f.units {
                    members.push(Member { source: Source::Unit(u), fleet: Some(f.id), count: c });
                }
            }
            if battle.defender == Some(fac) {
                if let Some(sb) = ps.starbase {
                    members.push(Member { source: Source::Starbase(sb), fleet: None, count: 1 });
                    for &(u, c) in &w.starbases[sb as usize].garrison {
                        members.push(Member { source: Source::Garrison(u), fleet: None, count: c });
                    }
                }
                let mut counts: BTreeMap<StructureTypeId, u32> = BTreeMap::new();
                for &s in &ps.structures {
                    if w.structures[s as usize].combat.is_some() {
                        *counts.entry(s).or_default() += 1;
                    }
                }
                for (s, c) in counts {
                    members.push(Member { source: Source::Structure(s), fleet: None, count: c });
                }
            }
            sides.push(SideInput { faction: fac, members });
        }
        sides
    }

    /// Deterministic seed of a battle (also used for the statistical resolver).
    pub(crate) fn battle_rng(&self, id: u32) -> crate::rng::Rng {
        self.galaxy.rng.clone().fork(0xBA77_1E00 ^ id as u64)
    }

    fn resolve_inner(&mut self, id: u32, tactical: bool) -> Option<Vec<Event>> {
        let pos = self.galaxy.battles.iter().position(|b| b.id == id)?;
        let battle = self.galaxy.battles.remove(pos);
        self.tactical_requests.remove(&id);
        let world = self.world.clone();
        let w = &*world;
        let p = battle.planet;

        let sides = self.battle_sides(&battle);
        let mut rng = self.battle_rng(id);
        let seed = rng.next_u64();
        if tactical && let Some(catalog) = self.tactical.clone() {
            let arena = w.planets[p as usize].space_map.as_ref().and_then(|m| self.arenas.get(&m.to_ascii_lowercase())).cloned();
            self.galaxy.sync_ships();
            let request = crate::tactical::TacticalRequest::from_sides(w, &battle, &sides, seed, &self.galaxy);
            if let Some(result) = crate::tactical::run_headless(&request, &catalog, arena) {
                let (sides, combat) = request.combat_result(&result);
                let events = self.apply_combat(&battle, &sides, combat);
                self.apply_health(&request, &result);
                return Some(events);
            }
        }
        let mut result = combat::resolve_allied(w, &sides, &mut rng, &self.galaxy.allied, &self.galaxy.factions.iter().map(|f| f.difficulty).collect::<Vec<_>>());
        self.apply_escape(&sides, &mut result);
        Some(self.apply_combat(&battle, &sides, result))
    }

    /// The statistical resolver kills every ship of the losing side; the tactical battles let the loser's AI retreat with part of its fleet. Gives each
    /// losing side that may retreat (owns more than one planet, not Pirates) `SimConfig::combat_escape` of every fleet unit back as retreaters.
    pub(crate) fn apply_escape(&self, sides: &[SideInput], result: &mut combat::CombatResult) {
        let Some(win) = result.winner else { return };
        let share = self.world.config.combat_escape;
        for (si, side) in sides.iter().enumerate() {
            let may = si != win && self.galaxy.factions[side.faction as usize].planets_owned > 1 && !self.world.factions[side.faction as usize].name.eq_ignore_ascii_case("pirates");
            if !may || share <= 0.0 {
                continue;
            }
            for m in side.members.iter().filter(|m| matches!(m.source, Source::Unit(_))) {
                let kept = (m.count as f32 * share).round() as u32;
                if let (true, Some(fid)) = (kept > 0, m.fleet) {
                    let survivors = &mut result.survivors[si];
                    match survivors.iter_mut().find(|x| x.source == m.source && x.fleet == m.fleet) {
                        Some(x) => x.count = x.count.max(kept),
                        None => survivors.push(Member { count: kept, ..*m }),
                    }
                    if !result.retreated.contains(&fid) {
                        result.retreated.push(fid);
                    }
                }
            }
        }
    }

    /// Removes the units, star base and structures that did not survive from the galaxy (fleets are cut by type).
    /// Returns the `UnitsDestroyed` / `HeroDestroyed` events and the unit losses per faction.
    pub(crate) fn apply_losses(&mut self, p: PlanetId, sides: &[SideInput], survivors: &[Vec<Member>]) -> (Vec<Event>, BTreeMap<FactionId, u32>) {
        let world = self.world.clone();
        let w = &*world;
        let mut events = Vec::new();
        self.galaxy.sync_ships();
        let mut losses: BTreeMap<FactionId, u32> = BTreeMap::new();
        for (si, side) in sides.iter().enumerate() {
            let mut surviving: BTreeMap<(FleetId, UnitTypeId), u32> = BTreeMap::new();
            let (mut base_alive, mut struct_alive) = (false, BTreeMap::new());
            for m in &survivors[si] {
                match (m.source, m.fleet) {
                    (Source::Unit(u), Some(f)) => *surviving.entry((f, u)).or_default() += m.count,
                    (Source::Starbase(_), _) => base_alive = true,
                    (Source::Structure(s), _) => *struct_alive.entry(s).or_insert(0u32) += m.count,
                    _ => {}
                }
            }
            for m in &side.members {
                match (m.source, m.fleet) {
                    (Source::Unit(u), Some(fid)) => {
                        let left = surviving.get(&(fid, u)).copied().unwrap_or(0);
                        *losses.entry(side.faction).or_default() += m.count - left;
                        if m.count > left {
                            let ships = self.galaxy.take_ships(fid, u, m.count - left);
                            self.galaxy.factions[side.faction as usize].stats.ships_lost += m.count - left;
                            events.push(Event::UnitsDestroyed { faction: side.faction, planet: p, unit: u, count: m.count - left, ships: ships.into_iter().map(|x| x.0).collect() });
                            if w.units[u as usize].hero {
                                events.push(Event::HeroDestroyed { faction: side.faction, planet: p, unit: u });
                            }
                        }
                        if let Some(f) = self.galaxy.fleets.get_mut(&fid) {
                            if left == 0 {
                                f.units.remove(&u);
                            } else {
                                f.units.insert(u, left);
                            }
                        }
                    }
                    (Source::Starbase(_), _) if !base_alive => self.galaxy.planets[p as usize].starbase = None,
                    (Source::Structure(s), _) => {
                        let left = struct_alive.get(&s).copied().unwrap_or(0);
                        let mut to_remove = m.count - left.min(m.count);
                        self.galaxy.planets[p as usize].structures.retain(|&x| {
                            if x == s && to_remove > 0 {
                                to_remove -= 1;
                                false
                            } else {
                                true
                            }
                        });
                    }
                    _ => {}
                }
            }
        }
        self.galaxy.fleets.retain(|_, f| !f.units.is_empty());
        (events, losses)
    }

    /// Applies a combat result (statistical or tactical) to the galaxy: losses, destroyed star base and
    /// structures, retreats, events.
    pub(crate) fn apply_combat(&mut self, battle: &PendingBattle, sides: &[SideInput], result: combat::CombatResult) -> Vec<Event> {
        let world = self.world.clone();
        let w = &*world;
        let (id, p) = (battle.id, battle.planet);
        let ps = self.galaxy.planets[p as usize].clone();
        let (mut events, losses) = self.apply_losses(p, sides, &result.survivors);

        let winner = result.winner.map(|i| sides[i].faction);
        for (si, side) in sides.iter().enumerate() {
            if result.winner == Some(si) {
                self.galaxy.factions[side.faction as usize].stats.battles_won += 1;
                events.push(Event::BattleWon { faction: side.faction, planet: p });
            } else if result.winner.is_some() || result.survivors[si].is_empty() {
                self.galaxy.factions[side.faction as usize].stats.battles_lost += 1;
                events.push(Event::BattleLost { faction: side.faction, planet: p });
            }
        }
        // Draw: every attacker fleet falls back.
        let mut retreating: Vec<FleetId> = Vec::new();
        if winner.is_none() && result.survivors.iter().filter(|s| !s.is_empty()).count() > 1 {
            let owner = ps.owner;
            retreating.extend(
                self.galaxy
                    .fleets
                    .values()
                    .filter(|f| f.location == FleetLocation::AtPlanet(p) && Some(f.owner) != owner && f.previous_planet != p)
                    .map(|f| f.id),
            );
        }
        // Ships of a side that left the battle by retreating (and lost it) fall back too.
        retreating.extend(result.retreated.iter().copied().filter(|fid| winner.is_some_and(|wf| self.galaxy.fleets.get(fid).is_some_and(|f| f.owner != wf))));
        retreating.sort_unstable();
        retreating.dedup();
        for fid in retreating {
            let Some(f) = self.galaxy.fleets.get(&fid) else { continue };
            if f.location != FleetLocation::AtPlanet(p) {
                continue;
            }
            // Escape move: straight to the nearest friendly planet (other planets are passed without a battle and blockades do not stop it);
            // with no friendly planet left, to the nearest neighbour system.
            if let Some(path) = w.escape_path(&self.galaxy, p, f.owner) {
                self.galaxy.escaping.insert(fid);
                start_move(w, &mut self.galaxy, fid, path);
            }
        }
        events.push(Event::BattleResolved { id, planet: p, winner, losses: losses.into_iter().collect() });
        events
    }

    fn capture(&mut self, dt: f32, events: &mut Vec<Event>) {
        let world = self.world.clone();
        let w = &*world;
        let mut present: BTreeMap<PlanetId, Vec<FactionId>> = BTreeMap::new();
        for f in self.galaxy.fleets.values() {
            if let FleetLocation::AtPlanet(p) = f.location {
                let v = present.entry(p).or_default();
                if !v.contains(&f.owner) {
                    v.push(f.owner);
                }
            }
        }
        for p in 0..self.galaxy.planets.len() as PlanetId {
            let owner = self.galaxy.planets[p as usize].owner;
            let battle_here = self.galaxy.battles.iter().any(|b| b.planet == p) || self.galaxy.planets[p as usize].destroyed;
            let occupier = match present.get(&p) {
                Some(v) if !battle_here && v.len() == 1 && Some(v[0]) != owner && w.real[p as usize] => {
                    let x = v[0];
                    let defended = self.galaxy.planets[p as usize].starbase.is_some();
                    (!defended && owner.is_none_or(|o| self.galaxy.hostile(w, x, o))).then_some(x)
                }
                _ => None,
            };
            let ps = &mut self.galaxy.planets[p as usize];
            match (occupier, ps.capture) {
                (Some(x), Some((c, t))) if c == x => {
                    let t = t + dt;
                    if t >= w.config.capture_delay_seconds {
                        self.apply_owner_change(p, Some(x), false, events);
                        self.grant_capture_bonus(p, x, events);
                    } else {
                        ps.capture = Some((x, t));
                    }
                }
                (Some(x), _) => ps.capture = Some((x, dt)),
                (None, _) => ps.capture = None,
            }
        }
    }

    /// `Planet_Capture_Bonus_Reward` for a conqueror, once per planet and faction (estimate: the data has the amount only).
    fn grant_capture_bonus(&mut self, p: PlanetId, faction: FactionId, events: &mut Vec<Event>) {
        let bonus = f64::from(self.world.planets[p as usize].capture_bonus) * f64::from(self.world.config.capture_bonus_scale);
        if bonus > 0.0 && self.galaxy.rewarded.insert((p, faction)) {
            self.galaxy.factions[faction as usize].credits += bonus;
            self.galaxy.factions[faction as usize].stats.credits_earned += bonus;
            events.push(Event::CaptureBonus { faction, planet: p, credits: bonus });
        }
    }

    /// Changes a planet's owner with all capture side effects: the star base, queues and (unless
    /// `keep_buildings`) structures are lost, the capture timer is reset, `PlanetCaptured` is emitted for
    /// a new owner, and elimination / victory are re-checked.
    pub(crate) fn apply_owner_change(&mut self, p: PlanetId, new: Option<FactionId>, keep_buildings: bool, events: &mut Vec<Event>) {
        let ps = &mut self.galaxy.planets[p as usize];
        let old = ps.owner;
        ps.capture = None;
        if old == new {
            return;
        }
        ps.owner = new;
        if let Some(o) = old {
            self.galaxy.factions[o as usize].stats.planets_lost += 1;
        }
        if let Some(x) = new {
            self.galaxy.factions[x as usize].stats.planets_captured += 1;
        }
        let ps = &mut self.galaxy.planets[p as usize];
        ps.queues = Default::default();
        if !keep_buildings {
            ps.starbase = None;
            ps.structures.clear();
        }
        if let Some(x) = new {
            events.push(Event::PlanetCaptured { planet: p, old_owner: old, new_owner: x });
        }
        self.after_ownership_change(events);
    }

    fn after_ownership_change(&mut self, events: &mut Vec<Event>) {
        self.world.clone().refresh_economy(&mut self.galaxy);
        self.check_elimination(events);
        self.check_victory(events);
    }

    fn check_elimination(&mut self, events: &mut Vec<Event>) {
        for f in 0..self.galaxy.factions.len() as FactionId {
            if self.galaxy.factions[f as usize].eliminated {
                continue;
            }
            let has_planet = self.galaxy.planets.iter().any(|p| p.owner == Some(f));
            let has_fleet = self.galaxy.fleets.values().any(|fl| fl.owner == f);
            if !has_planet && !has_fleet {
                self.galaxy.factions[f as usize].eliminated = true;
                events.push(Event::FactionEliminated { faction: f });
            }
        }
    }

    fn check_victory(&mut self, events: &mut Vec<Event>) {
        if self.galaxy.winner.is_some() || !self.world.config.victory_enabled {
            return;
        }
        let w = self.world.clone();
        let real: Vec<usize> = (0..w.planets.len()).filter(|&i| w.real[i]).collect();
        let alive: Vec<FactionId> = (0..self.galaxy.factions.len() as FactionId).filter(|&f| !self.galaxy.factions[f as usize].eliminated).collect();
        let mut winner = None;
        for &f in &alive {
            let holds = |ids: Vec<usize>| ids.iter().all(|&i| self.galaxy.planets[i].owner == Some(f));
            let human = w.victory.active_player == Some(f);
            let won = if human {
                (!w.victory.human_named_planets.is_empty() && holds(w.victory.human_named_planets.iter().map(|&p| p as usize).collect()))
                    || (w.victory.human_all_planets && holds(real.clone()))
            } else {
                (w.victory.ai_all_planets && holds(real.clone()))
                    || (!w.victory.ai_named_planets.is_empty() && holds(w.victory.ai_named_planets.iter().map(|&p| p as usize).collect()))
            };
            if won {
                winner = Some(f);
                break;
            }
        }
        if winner.is_none() && alive.len() == 1 {
            winner = Some(alive[0]);
        }
        if let Some(f) = winner {
            self.galaxy.winner = Some(f);
            events.push(Event::Victory { faction: f });
        }
    }
}

/// Adds `count` units of type `u` to the faction's idle fleet at `planet` (creating one if needed).
pub(crate) fn add_units(g: &mut Galaxy, owner: FactionId, planet: PlanetId, u: UnitTypeId, count: u32) -> FleetId {
    let existing = g
        .fleets
        .values_mut()
        .find(|f| f.owner == owner && f.location == FleetLocation::AtPlanet(planet));
    if let Some(f) = existing {
        *f.units.entry(u).or_default() += count;
        return f.id;
    }
    let id = g.next_fleet_id;
    g.next_fleet_id += 1;
    let mut units = BTreeMap::new();
    units.insert(u, count);
    g.fleets.insert(id, Fleet { id, owner, units, location: FleetLocation::AtPlanet(planet), previous_planet: planet });
    id
}

fn start_move(w: &World, g: &mut Galaxy, fleet: FleetId, path: Vec<PlanetId>) {
    // `path[0]` is the current planet.
    if path.len() < 2 {
        return;
    }
    let route = w.route_between(path[0], path[1]).expect("path edges exist");
    let f = g.fleets.get_mut(&fleet).expect("fleet exists");
    f.previous_planet = path[0];
    f.location = FleetLocation::Moving { from: path[0], to: path[1], route, progress: 0.0, rest: path[2..].to_vec() };
}

impl World {
    /// Route index of the lane joining `a` and `b`, preferring the faster one if there are several.
    pub fn route_between(&self, a: PlanetId, b: PlanetId) -> Option<u32> {
        self.adjacency[a as usize]
            .iter()
            .filter(|&&(to, _)| to == b)
            .map(|&(_, r)| r)
            .max_by(|&x, &y| {
                let (rx, ry) = (&self.routes[x as usize], &self.routes[y as usize]);
                (rx.hs_factor / rx.length).total_cmp(&(ry.hs_factor / ry.length)).then(y.cmp(&x))
            })
    }

    /// Hyperspace speed of a fleet (slowest ship).
    pub fn fleet_speed(&self, f: &Fleet) -> f32 {
        f.units
            .keys()
            .map(|&u| self.units[u as usize].hyperspace_speed)
            .filter(|&s| s > 0.0)
            .fold(f32::MAX, f32::min)
            .clamp(0.1, 1.5)
    }

    /// True if a fleet of `owner` passing through `planet` has to stop there.
    pub fn blockades(&self, galaxy: &Galaxy, planet: PlanetId, owner: FactionId) -> bool {
        let ps = &galaxy.planets[planet as usize];
        if let Some(o) = ps.owner
            && galaxy.hostile(self, owner, o)
            && (ps.starbase.is_some() || ps.structures.iter().any(|&s| self.structures[s as usize].combat.is_some()))
        {
            return true;
        }
        galaxy.fleets.values().any(|f| f.location == FleetLocation::AtPlanet(planet) && galaxy.hostile(self, owner, f.owner))
    }

    /// Escape route of a fleet of `owner` that lost the battle at `from`: the fastest path to the nearest friendly planet (own or allied, not `from`);
    /// when none is left, to the nearest neighbour system (estimate for "the first route": the cheapest lane, ties by lane index). `None` without a lane.
    pub fn escape_path(&self, galaxy: &Galaxy, from: PlanetId, owner: FactionId) -> Option<Vec<PlanetId>> {
        let (dist, prev) = self.shortest_paths(from);
        let friendly = |p: usize| galaxy.planets[p].owner.is_some_and(|o| o == owner || !galaxy.hostile(self, owner, o));
        let best = (0..self.planets.len()).filter(|&p| p != from as usize && self.real[p] && dist[p] != u64::MAX && friendly(p)).min_by_key(|&p| (dist[p], p));
        if let Some(p) = best {
            return Self::walk_back(&prev, from, p as PlanetId);
        }
        let lane = |&&(to, r): &&(PlanetId, u32)| {
            let route = &self.routes[r as usize];
            (!self.real[to as usize], (route.length / route.hs_factor * 100.0) as u64, r)
        };
        let &(to, _) = self.adjacency[from as usize].iter().min_by_key(lane)?;
        Some(vec![from, to])
    }

    /// Fastest lane path from `from` to `to` (inclusive), by travel time.
    pub fn find_path(&self, from: PlanetId, to: PlanetId) -> Option<Vec<PlanetId>> {
        self.find_path_avoiding(from, to, |_| false)
    }

    /// Fastest path that does not pass through planets for which `avoid` is true (the destination itself is always allowed).
    /// Falls back to the plain fastest path when no such route exists, so a fleet still gets there (and fights its way through).
    pub fn find_path_avoiding(&self, from: PlanetId, to: PlanetId, avoid: impl Fn(PlanetId) -> bool) -> Option<Vec<PlanetId>> {
        let (dist, prev) = self.shortest_paths_avoiding(from, to, &avoid);
        if dist[to as usize] == u64::MAX {
            return if (0..self.planets.len() as PlanetId).any(&avoid) { self.find_path(from, to) } else { None };
        }
        Self::walk_back(&prev, from, to)
    }

    fn walk_back(prev: &[Option<PlanetId>], from: PlanetId, to: PlanetId) -> Option<Vec<PlanetId>> {
        let mut path = vec![to];
        let mut cur = to;
        while cur != from {
            cur = prev[cur as usize]?;
            path.push(cur);
        }
        path.reverse();
        Some(path)
    }

    /// Dijkstra from `src`: travel time in milliseconds-ish integer units and predecessor table.
    pub fn shortest_paths(&self, src: PlanetId) -> (Vec<u64>, Vec<Option<PlanetId>>) {
        self.shortest_paths_avoiding(src, src, &|_| false)
    }

    /// [`Self::shortest_paths`] where planets for which `avoid` is true are only reached, never passed through (except `src`).
    fn shortest_paths_avoiding(&self, src: PlanetId, _dst: PlanetId, avoid: &dyn Fn(PlanetId) -> bool) -> (Vec<u64>, Vec<Option<PlanetId>>) {
        let n = self.planets.len();
        let mut dist = vec![u64::MAX; n];
        let mut prev = vec![None; n];
        let mut heap = BinaryHeap::new();
        dist[src as usize] = 0;
        heap.push(Reverse((0u64, src)));
        while let Some(Reverse((d, u))) = heap.pop() {
            if d > dist[u as usize] {
                continue;
            }
            if u != src && avoid(u) {
                continue;
            }
            for &(v, r) in &self.adjacency[u as usize] {
                let route = &self.routes[r as usize];
                let cost = d + (route.length / route.hs_factor * 100.0) as u64 + 1;
                if cost < dist[v as usize] {
                    dist[v as usize] = cost;
                    prev[v as usize] = Some(u);
                    heap.push(Reverse((cost, v)));
                }
            }
        }
        (dist, prev)
    }
}

/// Result of [`Sim::preview_auto_resolve`].
#[derive(Debug, Clone, PartialEq)]
pub struct AutoPreview {
    /// `(faction, ships at the start, ships alive at the end)` per side.
    pub sides: Vec<(FactionId, u32, u32)>,
    /// Winning faction, `None` for a draw.
    pub winner: Option<FactionId>,
}

/// The complete mutable state of a [`Sim`]: everything that is not derived from the game data (the [`World`] is rebuilt from the data and the
/// campaign). Plain data, serialisable. Frozen tactical requests are not part of it (they are rebuilt from the state by `tactical_setup`), and
/// events that were queued but not yet reported by `step` are dropped.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SimSnapshot {
    /// Planets, factions (credits, tech, unlocks, statistics, cooldowns), fleets, build queues, damage, battles, RNG, time.
    pub galaxy: Galaxy,
    /// Factions run by galaxy-sim's built-in AI.
    pub ai: Vec<FactionId>,
}

impl Sim {
    /// Copies the complete mutable state.
    pub fn snapshot(&self) -> SimSnapshot {
        SimSnapshot { galaxy: self.galaxy.clone(), ai: self.ai.clone() }
    }

    /// Replaces the mutable state with a snapshot taken from a sim built from the same data and campaign. Refuses a snapshot whose dimensions
    /// (planets, factions) do not fit this world, or that refers to units, structures or star bases this world does not have.
    pub fn restore(&mut self, snap: SimSnapshot) -> Result<(), String> {
        let w = self.world.clone();
        let g = &snap.galaxy;
        if g.planets.len() != w.planets.len() || g.factions.len() != w.factions.len() {
            return Err(format!("the save has {} planets and {} factions, the game data {} and {}", g.planets.len(), g.factions.len(), w.planets.len(), w.factions.len()));
        }
        let (units, structs, bases) = (w.units.len() as u32, w.structures.len() as u32, w.starbases.len() as u32);
        let bad_kind = |k: &ProductionKind| match *k {
            ProductionKind::Unit(u) => u >= units,
            ProductionKind::Structure(s) => s >= structs,
            ProductionKind::Starbase(b) => b >= bases,
            ProductionKind::Tech(_) => false,
        };
        let bad = g.planets.iter().any(|p| p.structures.iter().any(|&s| s >= structs) || p.starbase.is_some_and(|b| b >= bases) || p.queues.iter().flatten().any(|i| bad_kind(&i.kind)))
            || g.fleets.values().any(|f| f.units.keys().any(|&u| u >= units) || f.owner as usize >= w.factions.len() || f.planet().is_some_and(|p| p as usize >= w.planets.len()));
        if bad {
            return Err("the save refers to units or structures the game data does not have".into());
        }
        self.galaxy = snap.galaxy;
        self.ai = snap.ai;
        self.tactical_requests.clear();
        self.queued_events.clear();
        self.galaxy.sync_ships();
        Ok(())
    }
}
