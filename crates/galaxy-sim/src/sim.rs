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
        let mut sim = Sim { world, galaxy, ai: Vec::new(), tactical: None, arenas: std::collections::HashMap::new() };
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
        match cmd {
            Command::BuildUnit { faction, planet, unit } => {
                world.check_build_unit(g, faction, planet, unit)?;
                let ut = &world.units[unit as usize];
                g.factions[faction as usize].credits -= ut.cost as f64;
                let t = ut.build_time * world.config.build_time_scale;
                g.planets[planet as usize].queues[Lane::Units as usize].push_back(ProductionItem {
                    kind: ProductionKind::Unit(unit),
                    remaining: t,
                    total: t,
                    cost: ut.cost,
                });
                Ok(CommandOk::Done)
            }
            Command::BuildStructure { faction, planet, structure } => {
                world.check_build_structure(g, faction, planet, structure)?;
                let st = &world.structures[structure as usize];
                g.factions[faction as usize].credits -= st.cost as f64;
                let t = st.build_time * world.config.build_time_scale;
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
                let t = sb.build_time * world.config.build_time_scale;
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
                let t = up.time * world.config.build_time_scale;
                g.planets[planet as usize].queues[Lane::Base as usize].push_back(ProductionItem {
                    kind: ProductionKind::Tech(idx),
                    remaining: t,
                    total: t,
                    cost: up.cost,
                });
                Ok(CommandOk::Done)
            }
            Command::MoveFleet { faction, fleet, to } => {
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
                let path = world.find_path(from, to).ok_or(CommandError::NoPath)?;
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
                g.ships.insert(id, moved_ids);
                g.fleets.insert(id, Fleet { id, owner: faction, units: taken, location: FleetLocation::AtPlanet(p), previous_planet: prev });
                Ok(CommandOk::NewFleet(id))
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
        let mut events = Vec::new();
        self.galaxy.tick_accum += dt;
        let tick = self.world.config.tick_seconds;
        while self.galaxy.tick_accum >= tick && self.galaxy.winner.is_none() {
            self.galaxy.tick_accum -= tick;
            self.tick(tick, &mut events);
        }
        self.galaxy.sync_ships();
        events
    }

    fn tick(&mut self, dt: f32, events: &mut Vec<Event>) {
        let world = self.world.clone();
        let w = &*world;
        self.galaxy.time += dt as f64;

        // Economy.
        w.refresh_economy(&mut self.galaxy);
        for fs in self.galaxy.factions.iter_mut() {
            if fs.eliminated {
                continue;
            }
            let cap = w.config.credit_cap_per_planet * fs.planets_owned as f64;
            fs.credits += fs.income_per_week * dt as f64 / w.config.week_seconds as f64;
            if fs.planets_owned > 0 && fs.credits > cap {
                fs.credits = cap;
            }
            fs.credits = fs.credits.max(0.0);
        }

        self.produce(dt, events);
        self.move_fleets(dt, events);
        self.detect_battles(events);
        self.capture(dt, events);
        self.check_elimination(events);

        // Weeks.
        while self.galaxy.time >= (self.galaxy.week + 1) as f64 * w.config.week_seconds as f64 {
            self.galaxy.week += 1;
            events.push(Event::WeekEnded { week: self.galaxy.week });
            self.check_victory(events);
        }

        // AI.
        if !self.ai.is_empty() && self.galaxy.winner.is_none() {
            self.galaxy.ai_timer += dt;
            if self.galaxy.ai_timer >= w.config.ai_interval_seconds {
                self.galaxy.ai_timer = 0.0;
                self.run_ai();
            }
        }
    }

    fn run_ai(&mut self) {
        let ai = self.ai.clone();
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
                let blocked = !rest.is_empty() && w.blockades(&self.galaxy, to, owner);
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

    /// Resolves a pending battle with the statistical auto-resolve and applies the result.
    /// Returns `None` if the battle id is unknown.
    pub fn resolve_battle_auto(&mut self, id: u32) -> Option<Vec<Event>> {
        self.resolve_inner(id, false)
    }

    /// Resolves a pending battle with the resolver chosen in [`SimConfig::resolver`].
    pub fn resolve_battle(&mut self, id: u32) -> Option<Vec<Event>> {
        let tactical = self.world.config.resolver == BattleResolver::Tactical && self.tactical.is_some();
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

    fn resolve_inner(&mut self, id: u32, tactical: bool) -> Option<Vec<Event>> {
        let pos = self.galaxy.battles.iter().position(|b| b.id == id)?;
        let battle = self.galaxy.battles.remove(pos);
        let world = self.world.clone();
        let w = &*world;
        let p = battle.planet;
        let mut events = Vec::new();

        // Sides, one per faction.
        let ps = self.galaxy.planets[p as usize].clone();
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
        let mut rng = self.galaxy.rng.fork(0xBA77_1E00 ^ id as u64);
        let seed = rng.next_u64();
        let arena = w.planets[p as usize].space_map.as_ref().and_then(|m| self.arenas.get(&m.to_ascii_lowercase())).cloned();
        let result = tactical
            .then(|| self.tactical.as_ref().and_then(|c| crate::tactical::run(w, c, &sides, seed, arena)))
            .flatten()
            .unwrap_or_else(|| combat::resolve_allied(w, &sides, &mut rng, &self.galaxy.allied));

        // Apply fleet losses.
        self.galaxy.sync_ships();
        let mut losses: BTreeMap<FactionId, u32> = BTreeMap::new();
        for (si, side) in sides.iter().enumerate() {
            let mut surviving: BTreeMap<(FleetId, UnitTypeId), u32> = BTreeMap::new();
            let (mut base_alive, mut struct_alive) = (false, BTreeMap::new());
            for m in &result.survivors[si] {
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
                            events.push(Event::UnitsDestroyed { faction: side.faction, planet: p, unit: u, count: m.count - left, ships: ships.into_iter().map(|x| x.0).collect() });
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

        let winner = result.winner.map(|i| sides[i].faction);
        for (si, side) in sides.iter().enumerate() {
            if result.winner == Some(si) {
                events.push(Event::BattleWon { faction: side.faction, planet: p });
            } else if result.survivors[si].is_empty() {
                events.push(Event::BattleLost { faction: side.faction, planet: p });
            }
        }
        // Draw: every attacker fleet falls back.
        if winner.is_none() && result.survivors.iter().filter(|s| !s.is_empty()).count() > 1 {
            let owner = ps.owner;
            let retreating: Vec<FleetId> = self
                .galaxy
                .fleets
                .values()
                .filter(|f| f.location == FleetLocation::AtPlanet(p) && Some(f.owner) != owner && f.previous_planet != p)
                .map(|f| f.id)
                .collect();
            for fid in retreating {
                let f = &self.galaxy.fleets[&fid];
                if let Some(path) = w.find_path(p, f.previous_planet) {
                    start_move(w, &mut self.galaxy, fid, path);
                }
            }
        }
        events.push(Event::BattleResolved { id, planet: p, winner, losses: losses.into_iter().collect() });
        Some(events)
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
            let battle_here = self.galaxy.battles.iter().any(|b| b.planet == p);
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
                    } else {
                        ps.capture = Some((x, t));
                    }
                }
                (Some(x), _) => ps.capture = Some((x, dt)),
                (None, _) => ps.capture = None,
            }
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

    /// Fastest lane path from `from` to `to` (inclusive), by travel time.
    pub fn find_path(&self, from: PlanetId, to: PlanetId) -> Option<Vec<PlanetId>> {
        let (dist, prev) = self.shortest_paths(from);
        if dist[to as usize] == u64::MAX {
            return None;
        }
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
