//! Battle setup, commands and the fixed-step simulation systems.
//!
//! One [`Battle::step`] runs, in order: reinforcements, spatial snapshot, target scans (parallel),
//! movement (parallel), weapons (parallel, producing fire requests), projectile flight and
//! collision (parallel, producing hits), hit application (sequential, in projectile order), ship
//! upkeep (shield regeneration, abilities, squadron launches) and victory checks. Parallel stages
//! only read an immutable snapshot and draw randomness from per-entity generators, so results do
//! not depend on thread scheduling.

use std::f32::consts::PI;
use std::sync::Arc;

use rayon::prelude::*;

use crate::battle::*;
use crate::defs::*;
use crate::math::{Vec3, segment_point_distance, wrap_angle};
use crate::rng::Rng;

/// Immutable per-step view of a ship used by the parallel stages.
#[derive(Clone, Copy)]
struct Snap {
    pos: Vec3,
    vel: Vec3,
    yaw: f32,
    radius: f32,
    side: u8,
    class: TargetClass,
    def: ShipId,
    alive: bool,
    golan: bool,
    /// Bit i set = hardpoint i (of the first 64) is alive.
    hp_alive: u64,
}

/// Uniform grid of ship ids for projectile collision.
struct Grid {
    n: i32,
    cell: f32,
    min: f32,
    cells: Vec<Vec<u32>>,
}

impl Grid {
    fn new(half: f32, mut cells: Vec<Vec<u32>>) -> Self {
        let cell = 500.0;
        let n = ((2.0 * half * 1.3) / cell).ceil() as i32 + 1;
        cells.resize((n * n) as usize, Vec::new());
        Self { n, cell, min: -half * 1.3, cells }
    }

    fn coord(&self, v: f32) -> i32 {
        (((v - self.min) / self.cell).floor() as i32).clamp(0, self.n - 1)
    }

    fn rebuild(&mut self, snaps: &[Snap]) {
        for c in &mut self.cells {
            c.clear();
        }
        for (i, s) in snaps.iter().enumerate() {
            if !s.alive {
                continue;
            }
            let (x0, x1) = (self.coord(s.pos.x - s.radius), self.coord(s.pos.x + s.radius));
            let (y0, y1) = (self.coord(s.pos.y - s.radius), self.coord(s.pos.y + s.radius));
            for y in y0..=y1 {
                for x in x0..=x1 {
                    self.cells[(y * self.n + x) as usize].push(i as u32);
                }
            }
        }
    }

    fn cell_at(&self, p: Vec3) -> &[u32] {
        &self.cells[(self.coord(p.y) * self.n + self.coord(p.x)) as usize]
    }
}

struct FireReq {
    ship: u32,
    hp: u16,
    proj: ProjId,
    from: Vec3,
    vel: Vec3,
    target: u32,
    aim_hp: u16,
}

/// Per-step environment for projectile flight.
struct FlyEnv<'a> {
    obstacles: &'a [crate::arena::Obstacle],
    /// Active missile shields: (centre, radius, side).
    shields: &'a [(Vec3, f32, u8)],
}

enum Out {
    Intercepted(Vec3),
    Fly,
    Gone,
    Hit(u32, Vec3),
}

fn target_priority(attacker: TargetClass, target: TargetClass) -> u8 {
    use TargetClass::*;
    match attacker {
        Fighter => match target {
            Fighter | Bomber => 0,
            Transport | Corvette => 1,
            _ => 2,
        },
        Bomber => match target {
            Capital | Super | Frigate | Structure => 0,
            Transport | Corvette => 1,
            _ => 2,
        },
        _ => match target {
            Super | Capital | Frigate | Structure => 0,
            Transport | Corvette => 1,
            Bomber => 2,
            _ => 3,
        },
    }
}

impl Battle {
    /// Creates a battle from a setup: places fleets, launches initial squadrons.
    pub fn new(setup: BattleSetup) -> Battle {
        let BattleSetup { catalog, sides, mut config } = setup;
        let n = sides.len().max(1);
        if let Some(arena) = &config.arena {
            config.half_size = arena.half_size().max(1000.0);
        }
        let h = config.half_size;
        // Start areas per side: map markers when there is an arena, else a simple layout.
        let mut side_places: Vec<Vec<(Vec3, f32)>> = Vec::new();
        for i in 0..sides.len() {
            let from_arena = config.arena.as_ref().and_then(|a| {
                let list = match i {
                    0 => &a.attacker_entries,
                    1 => &a.defender_positions,
                    _ => return None,
                };
                (!list.is_empty() && sides.len() == 2).then(|| list.clone())
            });
            side_places.push(from_arena.unwrap_or_else(|| {
                if sides.len() == 2 {
                    if i == 0 { vec![(Vec3::new(-0.65 * h, 0.0, 0.0), 0.0)] } else { vec![(Vec3::new(0.3 * h, 0.0, 0.0), PI)] }
                } else {
                    let a = PI + i as f32 * 2.0 * PI / n as f32;
                    vec![(Vec3::new(a.cos(), a.sin(), 0.0) * (0.6 * h), wrap_angle(a + PI))]
                }
            }));
        }
        let spawn_pos = side_places.iter().map(|p| p[0].0).collect();
        let mut b = Battle {
            grid_cells: Vec::new(),
            catalog,
            pop_used: vec![0; sides.len()],
            retreat_ordered: vec![false; sides.len()],
            sides,
            ships: Vec::new(),
            projectiles: Vec::new(),
            squads: Vec::new(),
            groups: Vec::new(),
            pending: Vec::new(),
            events: Vec::new(),
            tick: 0,
            time: 0.0,
            over: false,
            winner: None,
            spawn_pos,
            side_places,
            config,
        };
        for si in 0..b.sides.len() {
            b.place_side(si as u8);
        }
        b.launch_initial();
        b.check_over();
        b
    }

    fn place_side(&mut self, side: u8) {
        let cat = self.catalog.clone();
        let setup = self.sides[side as usize].clone();
        let places = self.side_places[side as usize].clone();
        let (origin, yaw) = places[0];
        let has_structures = !setup.structures.is_empty();
        let enemy_pos = self.spawn_pos[(side as usize + 1) % self.sides.len()];
        let mobile_order = if !setup.ai {
            Order::Idle
        } else if has_structures {
            Order::Guard(origin)
        } else {
            Order::AttackMove(enemy_pos)
        };

        // Stations first: the base on the station position, the rest on matching markers or in a ring.
        let station = self.config.arena.as_ref().and_then(|a| a.station).filter(|_| has_structures);
        let (station_pos, station_yaw) = station.unwrap_or((origin, yaw));
        let n_struct = setup.structures.len();
        for (i, &sid) in setup.structures.iter().enumerate() {
            let marker = self.config.arena.as_ref().and_then(|a| a.marker_for_structure(&cat.ship(sid).name)).map(|m| (m.pos, m.yaw));
            let (pos, syaw) = if i == 0 {
                (station_pos, station_yaw)
            } else if let Some(m) = marker {
                m
            } else {
                let a = (i - 1) as f32 * 2.0 * PI / (n_struct.max(2) - 1) as f32;
                (station_pos + Vec3::new(a.cos(), a.sin(), 0.0) * 900.0, station_yaw)
            };
            let id = self.spawn_ship(sid, side, pos, syaw, NONE, NONE, Vec3::ZERO);
            let g = self.groups.len() as u32;
            self.groups.push(Group { side, tag: NO_TAG, ship: sid, members: vec![id] });
            self.ships[id as usize].group = g;
            self.ships[id as usize].order = Order::Idle;
        }

        // Fleet instances, spread round-robin over the start areas.
        let mut instances: Vec<(ShipId, u32)> = Vec::new();
        for u in &setup.units {
            for _ in 0..u.count {
                instances.push((u.ship, u.tag));
            }
        }
        let n_places = places.len();
        for (e, &(o, y)) in places.iter().enumerate() {
            let mine: Vec<(ShipId, u32)> = instances.iter().enumerate().filter(|(i, _)| i % n_places == e).map(|(_, v)| *v).collect();
            let order = match mobile_order {
                Order::Guard(_) => Order::Guard(if has_structures { station_pos } else { o }),
                other => other,
            };
            self.place_fleet(side, mine, o, y, order, has_structures);
        }
    }

    /// Lays out fleet units in rows by size class around `origin`, facing `yaw`.
    fn place_fleet(&mut self, side: u8, instances: Vec<(ShipId, u32)>, origin: Vec3, yaw: f32, order: Order, has_structures: bool) {
        let cat = self.catalog.clone();
        let row_of = |c: TargetClass| match c {
            TargetClass::Super | TargetClass::Capital => 0,
            TargetClass::Frigate => 1,
            TargetClass::Corvette | TargetClass::Transport => 2,
            _ => 3,
        };
        let class_of = |id: ShipId| {
            let d = cat.ship(id);
            if d.is_squadron { d.members.first().map_or(TargetClass::Fighter, |m| cat.ship(m.0).class) } else { d.class }
        };
        let mut rows: [Vec<(ShipId, u32)>; 4] = Default::default();
        for inst in instances {
            rows[row_of(class_of(inst.0))].push(inst);
        }
        let toward_enemy = if has_structures { 1200.0 } else { 0.0 };
        for (ri, row) in rows.iter().enumerate() {
            if row.is_empty() {
                continue;
            }
            let spacing: Vec<f32> = row
                .iter()
                .map(|&(s, _)| {
                    let d = cat.ship(s);
                    let r = if d.is_squadron { 90.0 } else { d.radius };
                    r * 2.2 + 60.0
                })
                .collect();
            let total: f32 = spacing.iter().sum();
            let mut lateral = -total * 0.5;
            let depth = toward_enemy + ri as f32 * 700.0;
            for (k, &(sid, tag)) in row.iter().enumerate() {
                lateral += spacing[k] * 0.5;
                let local = Vec3::new(depth, lateral, 0.0);
                lateral += spacing[k] * 0.5;
                let pos = origin + local.rotate_yaw(yaw);
                let group = self.groups.len() as u32;
                self.groups.push(Group { side, tag, ship: sid, members: Vec::new() });
                let def = cat.ship(sid);
                if def.is_squadron {
                    let squad = self.squads.len() as u32;
                    self.squads.push(Vec::new());
                    let mut idx = 0usize;
                    for &(m, c) in &def.members {
                        for _ in 0..c {
                            let off = def.offsets.get(idx).copied().unwrap_or(Vec3::new(-20.0 * idx as f32, 25.0 * (idx % 2) as f32, 0.0));
                            idx += 1;
                            let id = self.spawn_ship(m, side, pos + off.rotate_yaw(yaw), yaw, group, squad, off);
                            self.squads[squad as usize].push(id);
                            self.groups[group as usize].members.push(id);
                            self.ships[id as usize].order = order;
                        }
                    }
                    self.pop_used[side as usize] += def.population;
                } else {
                    let id = self.spawn_ship(sid, side, pos, yaw, group, NONE, Vec3::ZERO);
                    self.groups[group as usize].members.push(id);
                    self.ships[id as usize].order = if def.is_static { Order::Idle } else { order };
                    self.pop_used[side as usize] += def.population;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_ship(&mut self, def_id: ShipId, side: u8, pos: Vec3, yaw: f32, group: u32, squad: u32, formation: Vec3) -> u32 {
        let def = self.catalog.ship(def_id);
        let id = self.ships.len() as u32;
        let mut rng = Rng::for_entity(self.config.seed, 0, id, 0x5111);
        let hps = def
            .hardpoints
            .iter()
            .map(|h| HpState {
                health: h.health,
                alive: true,
                cooldown: rng.range(0.0, h.recharge_max),
                pulses_left: 0,
                pulse_timer: 0.0,
                target: NONE,
            })
            .collect();
        let ai_initial_speed = if def.is_static { 0.0 } else { def.max_speed * 0.0 };
        self.ships.push(Ship {
            id,
            def: def_id,
            side,
            pos,
            yaw,
            speed: ai_initial_speed,
            hull: def.hull.max(1.0),
            shield: def.shield,
            alive: true,
            retreated: false,
            hps,
            order: Order::Idle,
            stance: Stance::Aggressive,
            target: NONE,
            target_hp: NO_HP,
            direct_cd: rng.range(0.0, 1.0),
            direct_pulses: 0,
            direct_timer: 0.0,
            group,
            squad,
            formation,
            carrier: NONE,
            slots: Vec::new(),
            reserve: def.reserve.clone(),
            abilities: vec![AbilityState { active: 0.0, cooldown: 0.0, target: NONE }; def.abilities.len()],
            tractor_cut: 0.0,
            stun: 0.0,
            stun_speed_cut: 0.0,
            stun_rate_cut: 0.0,
            shield_gen_ok: true,
            engine_factor: 1.0,
            phase: 0.0,
            phase_turn: 0.0,
            candidates: Vec::new(),
            scan_in: rng.range(0.0, self.config.retarget_interval),
        });
        id
    }

    /// Launches the squadrons carried by ships and stations at the start of the battle.
    fn launch_initial(&mut self) {
        let carriers: Vec<u32> = self.ships.iter().filter(|s| !self.catalog.ship(s.def).carried.is_empty()).map(|s| s.id).collect();
        for c in carriers {
            let carried = self.catalog.ship(self.ships[c as usize].def).carried.clone();
            for (sq_def, count) in carried {
                for _ in 0..count {
                    let members = self.launch_squadron(c, sq_def);
                    self.ships[c as usize].slots.push(SquadSlot { def: sq_def, members, respawn_in: -1.0 });
                }
            }
        }
    }

    /// Spawns the fighters of squadron definition `sq_def` next to `carrier`.
    fn launch_squadron(&mut self, carrier: u32, sq_def: ShipId) -> Vec<u32> {
        let cat = self.catalog.clone();
        let c = &self.ships[carrier as usize];
        let (side, cpos, cyaw, cdef) = (c.side, c.pos, c.yaw, cat.ship(c.def));
        let bay = cdef.hardpoints.iter().find(|h| h.kind == HpKind::FighterBay).map_or(Vec3::ZERO, |h| h.offset);
        let spread = cdef.radius * 0.8;
        let mut rng = Rng::for_entity(self.config.seed, self.tick, carrier, 0x1A0C + self.ships.len() as u64);
        let base = cpos + (bay + rng.in_ball() * spread).rotate_yaw(cyaw);
        let sq = cat.ship(sq_def);
        let squad = self.squads.len() as u32;
        self.squads.push(Vec::new());
        let order = self.ships[carrier as usize].order;
        let ai_order = if matches!(order, Order::Idle) { Order::Idle } else { order };
        let mut ids = Vec::new();
        let mut idx = 0usize;
        for &(m, n) in &sq.members {
            for _ in 0..n {
                let off = sq.offsets.get(idx).copied().unwrap_or(Vec3::new(-20.0 * idx as f32, 25.0 * (idx % 2) as f32, 0.0));
                idx += 1;
                let id = self.spawn_ship(m, side, base + off.rotate_yaw(cyaw), cyaw, NONE, squad, off);
                let s = &mut self.ships[id as usize];
                s.carrier = carrier;
                s.speed = cat.ship(m).max_speed * 0.5;
                s.order = ai_order;
                self.squads[squad as usize].push(id);
                ids.push(id);
            }
        }
        if self.config.record_events {
            self.events.push(Event::SquadronLaunched { carrier, ships: ids.clone() });
        }
        ids
    }

    /// Validates and executes a command for `side`.
    pub fn command(&mut self, side: u8, cmd: Command) -> Result<(), CommandError> {
        if self.over {
            return Err(CommandError::BattleOver);
        }
        let attack = matches!(cmd, Command::AttackMove { .. });
        let own = |b: &Battle, ids: &[u32]| ids.iter().all(|&i| b.ships.get(i as usize).is_some_and(|s| s.side == side && s.alive && !s.retreated));
        match cmd {
            Command::Move { ships, to } | Command::AttackMove { ships, to } => {
                if !own(self, &ships) {
                    return Err(CommandError::InvalidShip);
                }
                for i in ships {
                    let s = &mut self.ships[i as usize];
                    s.order = if attack { Order::AttackMove(to) } else { Order::Move(to) };
                    s.target_hp = NO_HP;
                }
            }
            Command::Attack { ships, target } => {
                if !own(self, &ships) || !self.ships.get(target as usize).is_some_and(|t| t.alive && t.side != side) {
                    return Err(CommandError::InvalidShip);
                }
                for i in ships {
                    let s = &mut self.ships[i as usize];
                    s.order = Order::Attack(target);
                    s.target = target;
                    s.target_hp = NO_HP;
                }
            }
            Command::AttackHardpoint { ships, target, hardpoint } => {
                if !own(self, &ships) || !self.ships.get(target as usize).is_some_and(|t| t.alive && t.side != side) {
                    return Err(CommandError::InvalidShip);
                }
                let tdef = self.catalog.ship(self.ships[target as usize].def);
                if tdef.hardpoints.get(hardpoint as usize).is_none_or(|h| !h.targetable) {
                    return Err(CommandError::InvalidHardpoint);
                }
                for i in ships {
                    let s = &mut self.ships[i as usize];
                    s.order = Order::Attack(target);
                    s.target = target;
                    s.target_hp = hardpoint;
                }
            }
            Command::Stop { ships } => {
                if !own(self, &ships) {
                    return Err(CommandError::InvalidShip);
                }
                for i in ships {
                    let s = &mut self.ships[i as usize];
                    s.order = Order::Idle;
                    s.target_hp = NO_HP;
                }
            }
            Command::UseAbility { ship, ability } => {
                if !own(self, &[ship]) {
                    return Err(CommandError::InvalidShip);
                }
                if !self.activate_ability(ship, ability as usize) {
                    return Err(CommandError::AbilityUnavailable);
                }
            }
            Command::SetStance { ships, stance } => {
                if !own(self, &ships) {
                    return Err(CommandError::InvalidShip);
                }
                for i in ships {
                    self.ships[i as usize].stance = stance;
                }
            }
            Command::Retreat => {
                if self.time < self.config.retreat_countdown {
                    return Err(CommandError::RetreatNotAllowed);
                }
                let interdicted = self.ships.iter().any(|s| {
                    let d = self.catalog.ship(s.def);
                    s.alive
                        && !s.retreated
                        && s.side != side
                        && (d.interdictor || d.abilities.iter().zip(&s.abilities).any(|(a, st)| a.interdict && st.active > 0.0))
                });
                if interdicted {
                    return Err(CommandError::RetreatBlocked);
                }
                self.retreat_ordered[side as usize] = true;
                for s in self.ships.iter_mut().filter(|s| s.side == side && s.alive && !self.catalog.ship(s.def).is_static) {
                    s.order = Order::Retreat;
                }
            }
            Command::Reinforce { ship, count } => {
                let pop = self.catalog.ship(ship).population * count as i32;
                if self.pop_used[side as usize] + pop > self.config.unit_cap {
                    return Err(CommandError::UnitCap);
                }
                self.pop_used[side as usize] += pop;
                self.pending.push(Reinforcement { at: self.time + self.config.reinforce_delay, side, ship, count });
            }
        }
        Ok(())
    }

    /// Best enemy for a tractor beam: the current target if eligible, else the nearest eligible ship in range.
    fn tractor_target(&self, ship: u32, t: &TractorDef) -> Option<u32> {
        let s = &self.ships[ship as usize];
        let def = self.catalog.ship(s.def);
        // The beam needs a living emitter (ships without a tractor hardpoint can always use it).
        let emitters: Vec<usize> = def.hardpoints.iter().enumerate().filter(|(_, h)| h.kind == HpKind::TractorBeam).map(|(i, _)| i).collect();
        if !emitters.is_empty() && !emitters.iter().any(|&i| s.hps[i].alive) {
            return None;
        }
        let eligible = |o: &Ship| {
            let od = self.catalog.ship(o.def);
            o.alive
                && !o.retreated
                && o.side != s.side
                && !od.is_static
                && (t.classes == 0 || t.classes & od.class.bit() != 0)
                && o.pos.dist(s.pos) <= t.range
        };
        if s.target != NONE && eligible(&self.ships[s.target as usize]) {
            return Some(s.target);
        }
        self.ships.iter().filter(|o| eligible(o)).min_by(|a, b| a.pos.dist(s.pos).total_cmp(&b.pos.dist(s.pos)).then(a.id.cmp(&b.id))).map(|o| o.id)
    }

    fn activate_ability(&mut self, ship: u32, ability: usize) -> bool {
        let def = self.catalog.ship(self.ships[ship as usize].def);
        let Some(a) = def.abilities.get(ability) else { return false };
        if a.toggle && self.ships[ship as usize].abilities[ability].active > 0.0 {
            // Toggle abilities are switched off by using them again.
            self.ships[ship as usize].abilities[ability].active = 0.0;
            return true;
        }
        if self.ships[ship as usize].abilities[ability].cooldown > 0.0 || self.ships[ship as usize].abilities[ability].active > 0.0 {
            return false;
        }
        let mut target = NONE;
        if let Some(t) = &a.tractor {
            match self.tractor_target(ship, t) {
                Some(x) => target = x,
                None => return false,
            }
        }
        let st = &mut self.ships[ship as usize].abilities[ability];
        st.target = target;
        st.active = a.duration;
        st.cooldown = if a.toggle { 0.0 } else { a.recharge + a.duration };
        if self.config.record_events {
            self.events.push(Event::AbilityActivated { ship, ability: ability as u16 });
        }
        true
    }

    /// Advances the battle by one fixed time step and returns the events it produced.
    pub fn step(&mut self) -> &[Event] {
        self.events.clear();
        if self.over {
            return &self.events;
        }
        let dt = self.config.dt;
        self.tick += 1;
        self.time += dt;
        self.spawn_reinforcements();

        let cat = self.catalog.clone();
        let cfg = self.config.clone();
        let par = cfg.parallel;

        // Snapshot and grid.
        let snaps: Vec<Snap> = self
            .ships
            .iter()
            .map(|s| {
                let d = cat.ship(s.def);
                let radius = if d.class as u8 >= TargetClass::Frigate as u8 { d.radius * 0.75 } else { d.radius };
                Snap {
                    pos: s.pos,
                    vel: Vec3::from_yaw(s.yaw) * s.speed,
                    yaw: s.yaw,
                    radius,
                    side: s.side,
                    class: d.class,
                    def: s.def,
                    alive: s.alive && !s.retreated,
                    golan: d.golan,
                    hp_alive: s.hps.iter().take(64).enumerate().fold(0u64, |m, (i, h)| m | (u64::from(h.alive) << i)),
                }
            })
            .collect();
        let mut grid = Grid::new(cfg.half_size, std::mem::take(&mut self.grid_cells));
        grid.rebuild(&snaps);

        // Target scans, movement.
        let tick = self.tick;
        let half = cfg.half_size;
        let spawn = self.spawn_pos.clone();
        let arena = cfg.arena.clone();
        let obstacles: &[crate::arena::Obstacle] = arena.as_ref().map_or(&[], |a| a.obstacles.as_slice());
        let squad_leaders: Vec<u32> = self
            .squads
            .iter()
            .map(|m| m.iter().copied().find(|&i| snaps[i as usize].alive).unwrap_or(NONE))
            .collect();
        {
            let work = |s: &mut Ship| {
                if !s.alive || s.retreated {
                    return;
                }
                scan(s, &snaps, &cat, &cfg);
                let mut rng = Rng::for_entity(cfg.seed, tick, s.id, 0x30FE);
                mov(s, &snaps, &cat, &cfg, &spawn, &squad_leaders, obstacles, &mut rng);
            };
            if par && self.ships.len() > 16 {
                self.ships.par_iter_mut().for_each(work);
            } else {
                self.ships.iter_mut().for_each(work);
            }
        }

        // Weapons.
        let reqs: Vec<Vec<FireReq>> = {
            let work = |s: &mut Ship| {
                let mut out = Vec::new();
                if s.alive && !s.retreated {
                    let mut rng = Rng::for_entity(cfg.seed, tick, s.id, 0xF1E3);
                    weapons(s, &snaps, &cat, &cfg, &mut rng, &mut out);
                }
                out
            };
            if par && self.ships.len() > 16 {
                self.ships.par_iter_mut().map(work).collect()
            } else {
                self.ships.iter_mut().map(work).collect()
            }
        };
        for r in reqs.into_iter().flatten() {
            if cfg.record_events {
                self.events.push(Event::ProjectileFired {
                    ship: r.ship,
                    hardpoint: (r.hp != NO_HP).then_some(r.hp),
                    projectile: r.proj,
                    from: r.from,
                    velocity: r.vel,
                });
            }
            let pd = cat.projectiles.get(r.proj as usize);
            self.projectiles.push(Projectile {
                pos: r.from,
                vel: r.vel,
                def: r.proj,
                side: self.ships[r.ship as usize].side,
                owner: r.ship,
                target: r.target,
                aim_hp: r.aim_hp,
                range_left: pd.map_or(2000.0, |p| p.max_distance),
            });
        }

        // Projectile flight and collision.
        let shields: Vec<(Vec3, f32, u8)> = self
            .ships
            .iter()
            .filter(|s| s.alive && !s.retreated)
            .flat_map(|s| {
                cat.ship(s.def)
                    .abilities
                    .iter()
                    .zip(&s.abilities)
                    .filter(|(a, st)| a.missile_radius > 0.0 && st.active > 0.0)
                    .map(|(a, _)| (s.pos, a.missile_radius, s.side))
                    .collect::<Vec<_>>()
            })
            .collect();
        let env = FlyEnv { obstacles, shields: &shields };
        let outs: Vec<Out> = {
            let work = |p: &mut Projectile| fly(p, &snaps, &grid, &cat, &env, dt);
            if par && self.projectiles.len() > 512 {
                self.projectiles.par_iter_mut().map(work).collect()
            } else {
                self.projectiles.iter_mut().map(work).collect()
            }
        };
        let mut keep = Vec::with_capacity(self.projectiles.len());
        let mut hits = Vec::new();
        for (i, o) in outs.into_iter().enumerate() {
            match o {
                Out::Fly => keep.push(self.projectiles[i]),
                Out::Gone => {}
                Out::Intercepted(pos) => {
                    if cfg.record_events {
                        self.events.push(Event::ProjectileIntercepted { projectile: self.projectiles[i].def, pos });
                    }
                }
                Out::Hit(ship, pos) => hits.push((self.projectiles[i], ship, pos)),
            }
        }
        self.projectiles = keep;
        for (p, ship, pos) in hits {
            self.apply_hit(&p, ship, pos, &snaps);
        }

        self.grid_cells = grid.cells;
        self.upkeep(dt, half);
        self.check_over();
        if self.over && cfg.record_events {
            self.events.push(Event::BattleOver { winner: self.winner });
        }
        &self.events
    }

    fn spawn_reinforcements(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let due: Vec<Reinforcement> = self.pending.iter().copied().filter(|r| r.at <= self.time).collect();
        self.pending.retain(|r| r.at > self.time);
        for r in due {
            let cat = self.catalog.clone();
            let def = cat.ship(r.ship);
            let origin = self.spawn_pos[r.side as usize];
            let yaw = (Vec3::ZERO - origin).y.atan2((Vec3::ZERO - origin).x);
            let mut ids = Vec::new();
            for k in 0..r.count {
                let mut rng = Rng::for_entity(self.config.seed, self.tick, k, 0x7E1F + r.ship as u64);
                let pos = origin + rng.in_ball() * 800.0;
                if def.is_squadron {
                    let squad = self.squads.len() as u32;
                    self.squads.push(Vec::new());
                    for (idx, &(m, c)) in def.members.iter().enumerate() {
                        for _ in 0..c {
                            let id = self.spawn_ship(m, r.side, pos + Vec3::new(-20.0 * idx as f32, 25.0 * (idx % 2) as f32, 0.0), yaw, NONE, squad, Vec3::ZERO);
                            self.squads[squad as usize].push(id);
                            ids.push(id);
                        }
                    }
                } else {
                    ids.push(self.spawn_ship(r.ship, r.side, pos, yaw, NONE, NONE, Vec3::ZERO));
                }
            }
            for &i in &ids {
                let s = &mut self.ships[i as usize];
                s.order = if self.sides[r.side as usize].ai { Order::AttackMove(self.spawn_pos[(r.side as usize + 1) % self.sides.len()]) } else { Order::Idle };
            }
            if self.config.record_events {
                self.events.push(Event::ReinforcementsArrived { side: r.side, ships: ids });
            }
        }
    }

    /// Applies a projectile hit (and its area damage) to ships.
    fn apply_hit(&mut self, p: &Projectile, ship: u32, pos: Vec3, snaps: &[Snap]) {
        let cat = self.catalog.clone();
        let pd = &cat.projectiles[p.def as usize];
        self.damage_ship(ship, p.owner, pd, p.def, pd.damage, pos, p.aim_hp);
        if pd.blast_range > 0.0 && pd.blast_damage > 0.0 {
            let mut near: Vec<(f32, u32)> = snaps
                .iter()
                .enumerate()
                .filter(|(i, s)| s.alive && s.side != p.side && *i as u32 != ship && self.ships[*i].alive)
                .map(|(i, s)| (s.pos.dist(pos) - s.radius, i as u32))
                .filter(|&(d, _)| d <= pd.blast_range)
                .collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            for &(d, i) in near.iter().take(pd.blast_victims as usize) {
                let falloff = 1.0 - (d.max(0.0) / pd.blast_range) * 0.5;
                self.damage_ship(i, p.owner, pd, p.def, pd.blast_damage * falloff, pos, NO_HP);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn damage_ship(&mut self, id: u32, attacker: u32, pd: &ProjDef, proj: ProjId, raw: f32, pos: Vec3, aim_hp: u16) {
        let cat = self.catalog.clone();
        let record = self.config.record_events;
        let s = &mut self.ships[id as usize];
        if !s.alive {
            return;
        }
        let def = cat.ship(s.def);
        let take: f32 = def
            .abilities
            .iter()
            .zip(&s.abilities)
            .filter(|(_, st)| st.active > 0.0)
            .map(|(a, _)| a.take_damage_mult)
            .product();
        let dmg = raw * take;
        let mut hull_part = 0.0;
        let mut shield_hit = false;
        if s.shield > 0.0 && pd.shield_damage {
            let sd = dmg * cat.damage.multiplier(pd.damage_type, def.shield_armor);
            if sd <= s.shield {
                s.shield -= sd;
                shield_hit = true;
                if record {
                    self.events.push(Event::Hit { ship: id, attacker, projectile: proj, pos, kind: HitKind::Shield, damage: sd });
                }
            } else {
                let overflow = (sd - s.shield) / sd;
                s.shield = 0.0;
                hull_part = dmg * overflow;
            }
        } else if s.shield <= 0.0 || !pd.shield_damage {
            hull_part = dmg;
        }
        if !shield_hit && pd.hull_damage && hull_part > 0.0 && (s.shield <= 0.0 || !pd.shield_damage) {
            let h = hull_part * cat.damage.multiplier(pd.damage_type, def.armor);
            s.hull -= h;
            // Hardpoint: the aimed one, else the destroyable hardpoint nearest to the impact.
            let local = (pos - s.pos).rotate_yaw(-s.yaw);
            let mut best: Option<(f32, usize)> = None;
            for (i, hd) in def.hardpoints.iter().enumerate() {
                if !s.hps[i].alive || !hd.destroyable {
                    continue;
                }
                let d = if aim_hp == i as u16 { -1.0 } else { hd.offset.dist(local) };
                if best.is_none_or(|(b, _)| d < b) {
                    best = Some((d, i));
                }
            }
            let mut kind = HitKind::Hull;
            if let Some((d, i)) = best
                && (d < def.radius * 0.5 || aim_hp == i as u16)
            {
                kind = HitKind::Hardpoint(i as u16);
                let hp = &mut s.hps[i];
                hp.health -= h;
                if hp.health <= 0.0 {
                    hp.alive = false;
                    hp.pulses_left = 0;
                    let hk = def.hardpoints[i].kind;
                    match hk {
                        HpKind::ShieldGenerator => {
                            s.shield_gen_ok = def
                                .hardpoints
                                .iter()
                                .enumerate()
                                .any(|(j, d)| d.kind == HpKind::ShieldGenerator && s.hps[j].alive);
                            if !s.shield_gen_ok {
                                s.shield = 0.0;
                            }
                        }
                        HpKind::Engine => {
                            let (alive, total) = def.hardpoints.iter().enumerate().filter(|(_, d)| d.kind == HpKind::Engine).fold(
                                (0, 0),
                                |(a, t), (j, _)| (a + usize::from(s.hps[j].alive), t + 1),
                            );
                            s.engine_factor = if total == 0 { 1.0 } else { (alive as f32 / total as f32).max(0.25) };
                        }
                        _ => {}
                    }
                    if record {
                        self.events.push(Event::HardpointDestroyed { ship: id, hardpoint: i as u16, kind: hk });
                    }
                }
            }
            if record {
                self.events.push(Event::Hit { ship: id, attacker, projectile: proj, pos, kind, damage: h });
            }
            if pd.ion_duration > 0.0 {
                s.stun = s.stun.max(pd.ion_duration);
                s.stun_speed_cut = s.stun_speed_cut.max(pd.ion_speed_cut);
                s.stun_rate_cut = s.stun_rate_cut.max(pd.ion_rate_cut);
            }
            if s.hull <= 0.0 {
                s.alive = false;
                if record {
                    self.events.push(Event::ShipDestroyed { ship: id, pos: s.pos });
                }
            }
        } else if !shield_hit && s.shield <= 0.0 && pd.ion_duration > 0.0 {
            s.stun = s.stun.max(pd.ion_duration);
            s.stun_speed_cut = s.stun_speed_cut.max(pd.ion_speed_cut);
            s.stun_rate_cut = s.stun_rate_cut.max(pd.ion_rate_cut);
        }
    }

    fn upkeep(&mut self, dt: f32, half: f32) {
        let cat = self.catalog.clone();
        let record = self.config.record_events;
        let tick = self.tick;
        // Tractor beams: slow their targets for as long as the beam is active and in range.
        for s in &mut self.ships {
            s.tractor_cut = 0.0;
        }
        let mut cuts: Vec<(u32, f32)> = Vec::new();
        for s in self.ships.iter().filter(|s| s.alive && !s.retreated) {
            for (a, st) in cat.ship(s.def).abilities.iter().zip(&s.abilities) {
                if let Some(t) = &a.tractor
                    && st.active > 0.0
                    && st.target != NONE
                {
                    let tg = &self.ships[st.target as usize];
                    if tg.alive && !tg.retreated && tg.pos.dist(s.pos) <= t.range * 1.1 {
                        cuts.push((st.target, t.slow));
                    }
                }
            }
        }
        for (id, c) in cuts {
            let s = &mut self.ships[id as usize];
            s.tractor_cut = s.tractor_cut.max(c);
        }
        // Per ship: regen, timers, ability autofire, edge retreat.
        for i in 0..self.ships.len() {
            let (alive, retreated) = (self.ships[i].alive, self.ships[i].retreated);
            if !alive || retreated {
                continue;
            }
            let def = cat.ship(self.ships[i].def);
            let mut auto_use = Vec::new();
            {
                let s = &mut self.ships[i];
                for st in &mut s.abilities {
                    st.active = (st.active - dt).max(0.0);
                    st.cooldown = (st.cooldown - dt).max(0.0);
                }
                s.stun = (s.stun - dt).max(0.0);
                if s.stun == 0.0 {
                    s.stun_speed_cut = 0.0;
                    s.stun_rate_cut = 0.0;
                }
                if s.shield_gen_ok && def.shield > 0.0 && s.shield < def.shield {
                    let mult: f32 = def.abilities.iter().zip(&s.abilities).filter(|(_, st)| st.active > 0.0).map(|(a, _)| a.shield_regen_mult).product();
                    s.shield = (s.shield + def.shield_regen / cat.shield_interval * mult * dt).min(def.shield);
                }
                if s.pos.x.abs().max(s.pos.y.abs()) > half * 1.15 && matches!(s.order, Order::Retreat) {
                    s.retreated = true;
                }
            }
            if self.ships[i].retreated {
                if record {
                    self.events.push(Event::ShipRetreated { ship: i as u32 });
                }
                continue;
            }
            // AI sides use weapon abilities when they have something to shoot at.
            let s = &self.ships[i];
            if self.sides[s.side as usize].ai && s.target != NONE && s.stance != Stance::HoldFire && (tick + i as u64).is_multiple_of(15) {
                let t = &self.ships[s.target as usize];
                let dist = t.pos.dist(s.pos);
                for (k, a) in def.abilities.iter().enumerate() {
                    let boosts_weapons = a.salvo || a.weapon_delay_mult < 0.99;
                    if boosts_weapons && s.abilities[k].cooldown <= 0.0 && s.abilities[k].active <= 0.0 && dist < def.max_weapon_range {
                        auto_use.push(k);
                        break;
                    }
                }
            }
            if self.sides[self.ships[i].side as usize].ai && (tick + i as u64).is_multiple_of(15) {
                let s = &self.ships[i];
                for (k, a) in def.abilities.iter().enumerate() {
                    let ready = s.abilities[k].cooldown <= 0.0 && s.abilities[k].active <= 0.0;
                    if a.toggle && a.missile_radius > 0.0 && s.abilities[k].active > 0.0 {
                        // Switch the shield off again when no enemy missiles are near.
                        let near = self
                            .projectiles
                            .iter()
                            .any(|p| p.side != s.side && matches!(cat.projectiles[p.def as usize].category.as_str(), "missile" | "torpedo" | "rocket") && p.pos.dist(s.pos) < a.missile_radius * 3.0);
                        if !near {
                            auto_use.push(k);
                        }
                        continue;
                    }
                    if !ready {
                        continue;
                    }
                    if let Some(t) = &a.tractor
                        && s.stance != Stance::HoldFire
                        && self.tractor_target(i as u32, t).is_some()
                    {
                        auto_use.push(k);
                    } else if a.missile_radius > 0.0 {
                        let incoming = self
                            .projectiles
                            .iter()
                            .filter(|p| p.side != s.side && matches!(cat.projectiles[p.def as usize].category.as_str(), "missile" | "torpedo" | "rocket"))
                            .filter(|p| p.pos.dist(s.pos) < a.missile_radius * 2.0)
                            .count();
                        if incoming >= 3 {
                            auto_use.push(k);
                        }
                    }
                }
            }
            for k in auto_use {
                self.activate_ability(i as u32, k);
            }
        }
        self.run_carriers(dt);
    }

    /// Squadron replacement from hangars and stations.
    fn run_carriers(&mut self, dt: f32) {
        let cat = self.catalog.clone();
        for c in 0..self.ships.len() {
            if !self.ships[c].alive || self.ships[c].slots.is_empty() {
                continue;
            }
            let def = cat.ship(self.ships[c].def);
            let bay_ok = {
                let s = &self.ships[c];
                let bays: Vec<usize> = def.hardpoints.iter().enumerate().filter(|(_, h)| h.kind == HpKind::FighterBay).map(|(i, _)| i).collect();
                bays.is_empty() || bays.iter().any(|&i| s.hps[i].alive)
            };
            for k in 0..self.ships[c].slots.len() {
                let dead = self.ships[c].slots[k].members.iter().all(|&m| !self.ships[m as usize].alive);
                if !dead {
                    self.ships[c].slots[k].respawn_in = -1.0;
                    continue;
                }
                if self.ships[c].slots[k].respawn_in < 0.0 {
                    self.ships[c].slots[k].respawn_in = def.spawn_delay.max(0.5);
                }
                self.ships[c].slots[k].respawn_in -= dt;
                if self.ships[c].slots[k].respawn_in > 0.0 || !bay_ok {
                    continue;
                }
                let sq = self.ships[c].slots[k].def;
                let Some(r) = self.ships[c].reserve.iter_mut().find(|(d, n)| *d == sq && *n > 0) else {
                    self.ships[c].slots[k].respawn_in = 1.0e9;
                    continue;
                };
                r.1 -= 1;
                let members = self.launch_squadron(c as u32, sq);
                self.ships[c].slots[k].members = members;
                self.ships[c].slots[k].respawn_in = -1.0;
            }
        }
    }

    fn check_over(&mut self) {
        if self.over {
            return;
        }
        let n = self.sides.len();
        // Fighters and bombers alone do not keep a side in the fight (Victory_Relevant ships do),
        // unless the side never had anything else.
        let mut had_big = vec![false; n];
        let mut active = vec![false; n];
        for s in &self.ships {
            let big = !matches!(self.catalog.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber);
            if big && s.group != NONE {
                had_big[s.side as usize] = true;
            }
            if s.alive && !s.retreated && big {
                active[s.side as usize] = true;
            }
        }
        for s in &self.ships {
            if s.alive && !s.retreated && !had_big[s.side as usize] {
                active[s.side as usize] = true;
            }
        }
        for r in &self.pending {
            active[r.side as usize] = true;
        }
        let left: Vec<usize> = (0..n).filter(|&i| active[i]).collect();
        if left.len() <= 1 {
            self.over = true;
            self.winner = left.first().map(|&i| i as u8);
            // The defeated side's remaining fighters are lost with it.
            if let Some(w) = self.winner {
                for s in self.ships.iter_mut().filter(|s| s.side != w && !s.retreated) {
                    s.alive = false;
                }
            }
        } else if self.time >= self.config.max_time {
            self.over = true;
            self.winner = None;
        }
    }
    /// Runs steps until the battle ends or `max_steps` is reached; returns the number of steps.
    pub fn run_to_end(&mut self, max_steps: u64) -> u64 {
        let mut n = 0;
        let was = self.config.record_events;
        self.config.record_events = false;
        while !self.over && n < max_steps {
            self.step();
            n += 1;
        }
        self.config.record_events = was;
        n
    }
}

/// Convenience: build and run a battle headless.
pub fn simulate(setup: BattleSetup, max_steps: u64) -> (BattleResult, u64) {
    let mut b = Battle::new(setup);
    let steps = b.run_to_end(max_steps);
    (b.result(), steps)
}

fn ability_product(def: &ShipDef, s: &Ship, f: impl Fn(&AbilityDef) -> f32) -> f32 {
    def.abilities.iter().zip(&s.abilities).filter(|(_, st)| st.active > 0.0).map(|(a, _)| f(a)).product()
}

/// Refreshes the sorted target candidate list and the auto target.
fn scan(s: &mut Ship, snaps: &[Snap], cat: &Catalog, cfg: &BattleConfig) {
    s.scan_in -= cfg.dt;
    let explicit = if let Order::Attack(t) = s.order { Some(t) } else { None };
    let target_ok = s.target != NONE && snaps[s.target as usize].alive;
    if s.scan_in > 0.0 && (target_ok || s.target == NONE) {
        return;
    }
    s.scan_in = cfg.retarget_interval;
    let def = cat.ship(s.def);
    let range = match (s.stance, s.order) {
        (Stance::Aggressive, Order::AttackMove(_) | Order::Idle) => f32::MAX,
        _ => (def.max_weapon_range.max(def.targeting_range) * 1.15).max(400.0),
    };
    let me = snaps[s.id as usize];
    // Priority table: AI attack-move orders use the `<set>_Attack_Move` variant.
    let set = if matches!(s.order, Order::AttackMove(_)) { def.priority_attack_move } else { def.priority }
        .and_then(|i| cat.priority_sets.get(i as usize));
    let mut found: Vec<(f32, f32, u32)> = Vec::with_capacity(32);
    for (i, t) in snaps.iter().enumerate() {
        if !t.alive || t.side == s.side {
            continue;
        }
        let d = t.pos.dist(me.pos) - t.radius;
        if d > range {
            continue;
        }
        let weight = match set {
            Some(p) => {
                if p.excluded & t.class.bit() != 0 && explicit != Some(i as u32) {
                    continue;
                }
                p.weight(t.class, t.golan).min(1000.0)
            }
            None => f32::from(target_priority(me.class, t.class)),
        };
        found.push((weight, d, i as u32));
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
    found.truncate(24);
    s.candidates.clear();
    if let Some(t) = explicit
        && snaps[t as usize].alive
    {
        s.candidates.push(t);
    }
    s.candidates.extend(found.iter().map(|f| f.2).filter(|&i| Some(i) != explicit));
    s.target = if s.stance == Stance::HoldFire && explicit.is_none() { NONE } else { s.candidates.first().copied().unwrap_or(NONE) };
    if explicit.is_none_or(|t| !snaps[t as usize].alive) && s.target_hp != NO_HP && s.target == NONE {
        s.target_hp = NO_HP;
    }
}

fn turn_towards(s: &mut Ship, desired: f32, rate: f32, dt: f32) -> f32 {
    let diff = wrap_angle(desired - s.yaw);
    s.yaw = wrap_angle(s.yaw + diff.clamp(-rate * dt, rate * dt));
    diff.abs()
}

/// Turns the ship so that the largest share of its weapons bears on `tpos` (broadside, bow or stern,
/// whichever the hardpoint arcs favour).
fn align_to_target(s: &mut Ship, def: &ShipDef, tpos: Vec3, dt: f32) {
    let to = tpos - s.pos;
    let bearing = to.y.atan2(to.x);
    if def.bearing_dps.iter().sum::<f32>() <= 0.0 {
        turn_towards(s, bearing, def.turn_rate, dt);
        return;
    }
    let rel_now = wrap_angle(bearing - s.yaw);
    let mut best = (-1.0f32, 0.0f32);
    for (k, &dps) in def.bearing_dps.iter().enumerate() {
        let ang = wrap_angle(k as f32 * 2.0 * PI / 24.0);
        let score = dps * (1.0 - 0.15 * wrap_angle(ang - rel_now).abs() / PI);
        if score > best.0 {
            best = (score, ang);
        }
    }
    turn_towards(s, wrap_angle(bearing - best.1), def.turn_rate, dt);
}

/// Movement controller.
#[allow(clippy::too_many_arguments)]
fn mov(s: &mut Ship, snaps: &[Snap], cat: &Catalog, cfg: &BattleConfig, spawn: &[Vec3], leaders: &[u32], obstacles: &[crate::arena::Obstacle], rng: &mut Rng) {
    let def = cat.ship(s.def);
    if def.is_static {
        return;
    }
    let dt = cfg.dt;
    let speed_mult = ability_product(def, s, |a| a.speed_mult) * (1.0 - s.stun_speed_cut) * (1.0 - s.tractor_cut) * s.engine_factor;
    let pinned = def.abilities.iter().zip(&s.abilities).any(|(a, st)| a.disable_movement && st.active > 0.0);
    let max_speed = if pinned { 0.0 } else { def.max_speed * speed_mult };
    let target = (s.target != NONE && snaps[s.target as usize].alive).then(|| snaps[s.target as usize]);
    let fighter = matches!(def.class, TargetClass::Fighter | TargetClass::Bomber);
    let aggressive = s.stance == Stance::Aggressive;

    // (point, stop radius, face-target-when-arrived)
    enum Goal {
        Free(f32),
        Point(Vec3, f32),
        Hold,
    }
    let mut face: Option<Vec3> = None;
    let goal = match s.order {
        Order::Retreat => {
            let away = Vec3::new(s.pos.x, s.pos.y, 0.0).normalized();
            let away = if away.len_sq() < 0.5 { (spawn[s.side as usize]).normalized() } else { away };
            Goal::Point(s.pos + away * cfg.half_size, 0.0)
        }
        Order::Move(p) => Goal::Point(p, def.radius.max(50.0)),
        Order::Attack(_) | Order::AttackMove(_) | Order::Idle if target.is_some() && (aggressive || matches!(s.order, Order::Attack(_))) => {
            let t = target.expect("checked");
            let d = t.pos.dist(s.pos);
            if fighter {
                if s.phase > 0.0 {
                    Goal::Free(s.yaw + s.phase_turn * dt)
                } else if d < 70.0 + t.radius * 0.5 {
                    s.phase = rng.range(1.0, 2.5);
                    s.phase_turn = rng.range(-0.8, 0.8);
                    Goal::Free(s.yaw)
                } else {
                    Goal::Point(t.pos, 0.0)
                }
            } else {
                let stand = if def.preferred_range > 0.0 { def.preferred_range + t.radius * 0.5 } else { 0.0 };
                if d > stand * 1.05 {
                    Goal::Point(t.pos, stand)
                } else {
                    face = Some(t.pos);
                    Goal::Hold
                }
            }
        }
        Order::AttackMove(p) => Goal::Point(p, def.radius.max(100.0)),
        Order::Guard(p) => {
            if let Some(t) = target
                && t.pos.dist(s.pos) < def.max_weapon_range * 1.1
            {
                face = Some(t.pos);
            }
            Goal::Point(p, 200.0)
        }
        Order::Idle | Order::Attack(_) => {
            // Fighters stay with their squadron leader or carrier.
            if fighter {
                let leader = if s.squad != NONE { leaders[s.squad as usize] } else { NONE };
                if leader != NONE && leader != s.id && snaps[leader as usize].alive {
                    let l = snaps[leader as usize];
                    Goal::Point(l.pos + s.formation.rotate_yaw(l.yaw), 20.0)
                } else if s.carrier != NONE && snaps[s.carrier as usize].alive {
                    Goal::Point(snaps[s.carrier as usize].pos, 700.0)
                } else {
                    Goal::Hold
                }
            } else {
                Goal::Hold
            }
        }
    };

    match goal {
        Goal::Free(yaw) => {
            let rate = def.turn_rate;
            turn_towards(s, yaw, rate, dt);
            let want = max_speed;
            s.speed += (want - s.speed).clamp(-def.accel * dt * 2.0, def.accel * dt);
            if s.phase > 0.0 {
                s.phase -= dt;
            }
        }
        Goal::Point(p, stop) => {
            let to = p - s.pos;
            let dist = to.len();
            let desired = to.y.atan2(to.x);
            let err = turn_towards(s, desired, def.turn_rate, dt);
            let braking = s.speed * s.speed / (2.0 * def.accel.max(1.0));
            let mut want = max_speed;
            if err > 1.2 {
                want *= 0.35;
            } else if err > 0.5 {
                want *= 0.7;
            }
            if dist <= stop + braking * 0.8 && !fighter {
                want = 0.0;
            }
            s.speed += (want - s.speed).clamp(-def.accel * dt * 1.5, def.accel * dt);
            if want == 0.0
                && dist <= stop
                && let Some(f) = face
            {
                align_to_target(s, def, f, dt);
            }
        }
        Goal::Hold => {
            s.speed += (0.0 - s.speed).clamp(-def.accel * dt * 1.5, def.accel * dt);
            if let Some(f) = face {
                align_to_target(s, def, f, dt);
            }
        }
    }
    s.speed = s.speed.clamp(0.0, max_speed.max(0.0));
    let fwd = Vec3::from_yaw(s.yaw);
    s.pos += fwd * (s.speed * dt);
    // Asteroids: push the ship out of any rock it overlaps and scrub some speed.
    for o in obstacles {
        let (dx, dy) = (s.pos.x - o.pos.x, s.pos.y - o.pos.y);
        let min = o.radius + def.radius * 0.5;
        if dx.abs() < min && dy.abs() < min {
            let d = (dx * dx + dy * dy).sqrt().max(1e-3);
            if d < min {
                s.pos.x = o.pos.x + dx / d * min;
                s.pos.y = o.pos.y + dy / d * min;
                s.speed *= 0.9;
            }
        }
    }
}

/// Checks whether a hardpoint at `muzzle` could shoot at target `t`.
#[allow(clippy::too_many_arguments)]
fn can_fire(hp: &HpDef, ship_yaw: f32, muzzle: Vec3, t: &Snap, proj: &ProjDef) -> bool {
    let to = t.pos - muzzle;
    let dist = to.len();
    let reach = (dist - t.radius * 0.6).max(0.0);
    if reach > hp.range || reach < hp.min_range {
        return false;
    }
    if hp.allowed != 0 && hp.allowed & t.class.bit() == 0 {
        return false;
    }
    if matches!(t.class, TargetClass::Fighter | TargetClass::Bomber) && (proj.damage >= 150.0 || !proj.hull_damage) {
        return false;
    }
    let (lim_yaw, lim_pitch) = (hp.arc_yaw, hp.arc_pitch);
    if lim_yaw >= PI - 0.01 && lim_pitch >= std::f32::consts::FRAC_PI_2 - 0.01 {
        return true;
    }
    // Direction in the hardpoint's mount frame (forward, left, normal).
    let d = to.rotate_yaw(-ship_yaw).normalized();
    let slack = (t.radius / dist.max(1.0)).atan();
    let yaw_off = d.dot(hp.left).atan2(d.dot(hp.forward)).abs();
    let pitch_off = d.dot(hp.normal).clamp(-1.0, 1.0).asin().abs();
    yaw_off <= lim_yaw + slack && pitch_off <= lim_pitch + slack
}
/// Picks the hardpoint of `target` to shoot at according to a priority set (`NO_HP` = none).
fn auto_hardpoint(set: &PrioritySet, tdef: &ShipDef, target: &Snap, muzzle: Vec3) -> u16 {
    if tdef.hardpoints.is_empty() || matches!(target.class, TargetClass::Fighter | TargetClass::Bomber) {
        return NO_HP;
    }
    for kind in &set.hardpoints {
        let mut best: Option<(f32, usize)> = None;
        for (i, h) in tdef.hardpoints.iter().enumerate().take(64) {
            if h.kind != *kind || !h.targetable || !h.destroyable || target.hp_alive & (1u64 << i) == 0 {
                continue;
            }
            let d = (target.pos + h.offset.rotate_yaw(target.yaw)).dist(muzzle);
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, i));
            }
        }
        if let Some((_, i)) = best {
            return i as u16;
        }
    }
    NO_HP
}

fn lead(muzzle: Vec3, t: &Snap, speed: f32) -> Vec3 {
    let d = t.pos.dist(muzzle);
    let tt = d / speed.max(1.0);
    t.pos + t.vel * tt
}

fn weapons(s: &mut Ship, snaps: &[Snap], cat: &Catalog, cfg: &BattleConfig, rng: &mut Rng, out: &mut Vec<FireReq>) {
    let dt = cfg.dt;
    let def = cat.ship(s.def);
    let explicit_attack = matches!(s.order, Order::Attack(_));
    let may_start = s.stance != Stance::HoldFire || explicit_attack;
    let rate_cut = if s.stun > 0.0 { 1.0 / (1.0 - s.stun_rate_cut).max(0.1) } else { 1.0 };
    let mut delay_mult = ability_product(def, s, |a| a.weapon_delay_mult) * rate_cut;
    // Hardpoint priorities of the ship's targeting set (AI attack-move sets name shield generators,
    // engines, hangars ... in order).
    let hp_set = if matches!(s.order, Order::AttackMove(_)) { def.priority_attack_move } else { def.priority }
        .and_then(|i| cat.priority_sets.get(i as usize))
        .filter(|p| !p.hardpoints.is_empty());
    let salvo_active = def.abilities.iter().zip(&s.abilities).any(|(a, st)| a.salvo && st.active > 0.0);
    let power_active = def.abilities.iter().zip(&s.abilities).any(|(a, st)| st.active > 0.0 && a.kind.eq_ignore_ascii_case("POWER_TO_WEAPONS"));
    if delay_mult <= 0.0 {
        delay_mult = 1.0;
    }
    let cos = s.yaw.cos();
    let sin = s.yaw.sin();
    let to_world = |off: Vec3| Vec3::new(off.x * cos - off.y * sin, off.x * sin + off.y * cos, off.z) + s.pos;

    for (i, hp) in def.hardpoints.iter().enumerate() {
        if hp.kind != HpKind::Weapon {
            continue;
        }
        let Some(pid) = hp.projectile else { continue };
        let pd = &cat.projectiles[pid as usize];
        let (alive, pulses_left, pulse_timer, cooldown, tgt) = {
            let st = &s.hps[i];
            (st.alive, st.pulses_left, st.pulse_timer, st.cooldown, st.target)
        };
        if !alive {
            continue;
        }
        let muzzle = to_world(hp.offset);
        if pulses_left > 0 {
            let timer = pulse_timer - dt;
            if timer > 0.0 {
                s.hps[i].pulse_timer = timer;
                continue;
            }
            let valid = tgt != NONE && snaps[tgt as usize].alive;
            let st = &mut s.hps[i];
            if !valid {
                st.pulses_left = 0;
            } else {
                let t = &snaps[tgt as usize];
                if can_fire(hp, s.yaw, muzzle, t, pd) {
                    let tdef = cat.ship(t.def);
                    let aim_hp = if s.target_hp != NO_HP && tgt == s.target {
                        s.target_hp
                    } else if let Some(set) = hp_set {
                        auto_hardpoint(set, tdef, t, muzzle)
                    } else {
                        NO_HP
                    };
                    let mut aim = lead(muzzle, t, pd.speed);
                    if aim_hp != NO_HP
                        && let Some(h) = tdef.hardpoints.get(aim_hp as usize)
                    {
                        aim += h.offset.rotate_yaw(t.yaw);
                    } else {
                        aim += rng.in_ball() * (t.radius * 0.45);
                    }
                    aim += rng.in_ball() * hp.inaccuracy[t.class as usize];
                    let dir = (aim - muzzle).normalized();
                    out.push(FireReq { ship: s.id, hp: i as u16, proj: pid, from: muzzle, vel: dir * pd.speed, target: tgt, aim_hp });
                }
                st.pulses_left -= 1;
                st.pulse_timer = hp.pulse_delay;
            }
            if st.pulses_left == 0 {
                let mut m = delay_mult;
                if salvo_active {
                    m *= hp.full_salvo_mult;
                }
                if power_active {
                    m *= hp.power_weapons_mult;
                }
                st.cooldown = rng.range(hp.recharge_min, hp.recharge_max) * m;
            }
            continue;
        }
        let cd = cooldown - dt;
        if cd > 0.0 || !may_start {
            s.hps[i].cooldown = cd.max(0.0);
            continue;
        }
        s.hps[i].cooldown = 0.0;
        // Look for a target among the candidates.
        let mut chosen = NONE;
        for &c in &s.candidates {
            let t = &snaps[c as usize];
            if t.alive && can_fire(hp, s.yaw, muzzle, t, pd) {
                chosen = c;
                break;
            }
        }
        if chosen != NONE {
            let st = &mut s.hps[i];
            st.target = chosen;
            st.pulses_left = hp.pulses;
            st.pulse_timer = 0.0;
        } else {
            s.hps[i].cooldown = 0.15 + rng.unit() * 0.1;
        }
    }

    // Hull weapon (fighters).
    if let Some(d) = &def.direct {
        let pd = &cat.projectiles[d.projectile as usize];
        if s.direct_pulses > 0 {
            s.direct_timer -= dt;
            if s.direct_timer <= 0.0 {
                let tgt = s.target;
                if tgt != NONE && snaps[tgt as usize].alive {
                    let t = &snaps[tgt as usize];
                    let muzzle = s.pos;
                    let to = t.pos - muzzle;
                    let ang = wrap_angle(to.y.atan2(to.x) - s.yaw).abs();
                    if to.len() - t.radius * 0.6 <= d.range && ang <= d.half_cone + (t.radius / to.len().max(1.0)).atan() && (pd.hull_damage || pd.shield_damage) {
                        let mut aim = lead(muzzle, t, pd.speed);
                        aim += rng.in_ball() * (t.radius * 0.35 + 6.0);
                        let dir = (aim - muzzle).normalized();
                        out.push(FireReq { ship: s.id, hp: NO_HP, proj: d.projectile, from: muzzle, vel: dir * pd.speed, target: tgt, aim_hp: NO_HP });
                    }
                }
                s.direct_pulses -= 1;
                s.direct_timer = d.pulse_delay;
                if s.direct_pulses == 0 {
                    s.direct_cd = d.recharge * delay_mult;
                }
            }
        } else {
            s.direct_cd -= dt;
            if s.direct_cd <= 0.0 && may_start {
                let tgt = s.target;
                if tgt != NONE && snaps[tgt as usize].alive {
                    let t = &snaps[tgt as usize];
                    let to = t.pos - s.pos;
                    let ang = wrap_angle(to.y.atan2(to.x) - s.yaw).abs();
                    let ok = to.len() - t.radius * 0.6 <= d.range && ang <= d.half_cone + (t.radius / to.len().max(1.0)).atan();
                    if ok && !(matches!(t.class, TargetClass::Fighter | TargetClass::Bomber) && pd.damage >= 150.0) {
                        s.direct_pulses = d.pulses;
                        s.direct_timer = 0.0;
                    } else {
                        s.direct_cd = 0.1;
                    }
                } else {
                    s.direct_cd = 0.2;
                }
            }
        }
    }
}

/// Moves one projectile and tests it against enemy ships.
fn fly(p: &mut Projectile, snaps: &[Snap], grid: &Grid, cat: &Catalog, env: &FlyEnv, dt: f32) -> Out {
    let pd = &cat.projectiles[p.def as usize];
    // Missile shields destroy missiles and torpedoes that enter their radius.
    if !env.shields.is_empty() && matches!(pd.category.as_str(), "missile" | "torpedo" | "rocket") {
        for &(pos, r, side) in env.shields {
            if side != p.side && pos.dist(p.pos) <= r {
                return Out::Intercepted(p.pos);
            }
        }
    }
    if pd.turn_rate > 0.0 && p.target != NONE {
        let t = &snaps[p.target as usize];
        if t.alive {
            let want = (t.pos - p.pos).normalized();
            let cur = p.vel.normalized();
            let cosang = cur.dot(want).clamp(-1.0, 1.0);
            let ang = cosang.acos();
            let max = pd.turn_rate * dt;
            if ang > 1e-4 {
                let k = (max / ang).min(1.0);
                let dir = (cur + (want - cur) * k).normalized();
                p.vel = dir * pd.speed;
            }
        }
    }
    let old = p.pos;
    let new = old + p.vel * dt;
    let mut best: Option<(f32, u32)> = None;
    let mut check = |cell: &[u32]| {
        for &id in cell {
            let s = &snaps[id as usize];
            if !s.alive || s.side == p.side {
                continue;
            }
            let (d, t) = segment_point_distance(old, new, s.pos);
            if d <= s.radius && best.is_none_or(|(bt, _)| t < bt) {
                best = Some((t, id));
            }
        }
    };
    check(grid.cell_at(old));
    let (c0, c1) = (grid.cell_at(old).as_ptr(), grid.cell_at(new).as_ptr());
    if c0 != c1 {
        check(grid.cell_at(new));
    }
    // Asteroids absorb projectiles.
    let mut rock_t = f32::MAX;
    for o in env.obstacles {
        if (o.pos.x - old.x).abs() > o.radius + 100.0 || (o.pos.y - old.y).abs() > o.radius + 100.0 {
            continue;
        }
        let (d, t) = segment_point_distance(old, new, o.pos);
        if d <= o.radius {
            rock_t = rock_t.min(t);
        }
    }
    if let Some((t, id)) = best
        && t <= rock_t
    {
        return Out::Hit(id, old + (new - old) * t);
    }
    if rock_t < f32::MAX {
        return Out::Gone;
    }
    p.pos = new;
    p.range_left -= p.vel.len() * dt;
    if p.range_left <= 0.0 { Out::Gone } else { Out::Fly }
}

/// Helper to build an `Arc<Catalog>` setup quickly.
pub fn setup(catalog: Arc<Catalog>, sides: Vec<SideSetup>, config: BattleConfig) -> BattleSetup {
    BattleSetup { catalog, sides, config }
}
