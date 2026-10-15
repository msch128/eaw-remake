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

/// Hardpoints a snapshot tracks per ship. The biggest ships have 213 (Subjugator), 185 (Super Star Destroyers) and 158 (Executor);
/// hardpoints beyond this count are never aimed at.
const HP_MASK_BITS: usize = 256;

/// Which hardpoints of a ship are alive: bit `i` of a 256-bit set.
#[derive(Clone, Copy, Default)]
struct HpMask([u64; HP_MASK_BITS / 64]);

impl HpMask {
    fn from_alive(alive: impl Iterator<Item = bool>) -> Self {
        let mut m = Self::default();
        for (i, a) in alive.take(HP_MASK_BITS).enumerate() {
            m.0[i / 64] |= u64::from(a) << (i % 64);
        }
        m
    }

    /// Is hardpoint `i` alive? Hardpoints past the mask count as dead.
    fn test(&self, i: usize) -> bool {
        i < HP_MASK_BITS && self.0[i / 64] >> (i % 64) & 1 == 1
    }

    /// Is any hardpoint alive?
    fn any(&self) -> bool {
        self.0.iter().any(|w| *w != 0)
    }
}

/// Immutable per-step view of a ship used by the parallel stages.
#[derive(Clone, Copy)]
struct Snap {
    pos: Vec3,
    vel: Vec3,
    yaw: f32,
    radius: f32,
    /// Radius within which a projectile path hits the unit.
    hit_radius: f32,
    side: u8,
    class: TargetClass,
    def: ShipId,
    alive: bool,
    golan: bool,
    /// Bit i set = hardpoint i (of the first [`HP_MASK_BITS`]) is alive.
    hp_alive: HpMask,
    /// Hull and shield left as a share of the maximum (0..1).
    health: f32,
    /// Bit s set = side s sees this ship (fog of war).
    seen_by: u8,
    /// `SENSOR_JAMMING` active: homing missiles cannot hold a lock on it.
    jammed: bool,
    /// Ship that carries this one (`NONE` = none): carried ships dock with it and do not keep out of its hull.
    carrier: u32,
    /// Special ground-to-space weapon on the planet surface.
    special: bool,
    /// Targeted hardpoint index.
    target_hp: u16,
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
            if !s.alive || s.special {
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
    /// `(damage dealt, health, shield)` multipliers of a side (1.0 without an entry).
    /// Applies carried-over damage to a freshly spawned single ship.
    fn apply_start_health(&mut self, id: u32, side: u8, tag: u32, def: ShipId) {
        let n = self.start_health_used.entry((side, tag, def)).or_default();
        let k = *n;
        *n += 1;
        let entry = self.config.start_health.iter().find(|h| h.side == side && h.tag == tag && h.ship == def);
        if let Some((hull, shield)) = entry.and_then(|h| h.health.get(k)).copied() {
            let d = self.catalog.ship(def);
            let s = &mut self.ships[id as usize];
            s.hull = (hull.clamp(0.0, 1.0) * d.hull).max(1.0);
            s.shield = shield.clamp(0.0, 1.0) * d.shield;
        }
    }

    fn side_mult(&self, side: u8) -> (f32, f32, f32) {
        self.config.side_mults.get(side as usize).map_or((1.0, 1.0, 1.0), |m| (m.damage.max(0.01), m.health.max(0.01), m.shield.max(0.01)))
    }

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
            cmd_events: Vec::new(),
            retreat_ordered: vec![false; sides.len()],
            cheats: crate::cheats::Cheats::new(sides.len()),
            retreat_jump_at: vec![None; sides.len()],
            hunt_seen: [0; 4],
            ability_at: None,
            lured: Vec::new(),
            reveals: Vec::new(),
            zones: Vec::new(),
            lasers: Vec::new(),
            drains: Vec::new(),
            nav_grid: None,
            search: vec![(0, 0.0); sides.len()],
            sides,
            ships: Vec::new(),
            projectiles: Vec::new(),
            squads: Vec::new(),
            groups: Vec::new(),
            start_health_used: std::collections::HashMap::new(),
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
        b.relax_spawn();
        b.launch_initial();
        b.check_over();
        // Rocks and stations do not move: one distance field serves the whole battle.
        if b.config.navigation {
            let cat = b.catalog.clone();
            let blockers: Vec<crate::nav::Blocker> = b
                .config
                .arena
                .iter()
                .flat_map(|a| a.obstacles.iter().map(|o| crate::nav::Blocker { pos: o.pos, radius: o.radius }))
                .chain(b.ships.iter().filter(|s| cat.ship(s.def).is_static).map(|s| crate::nav::Blocker { pos: s.pos, radius: cat.ship(s.def).radius * 0.75 * 0.9 }))
                .collect();
            b.nav_grid = crate::nav::NavGrid::build(b.config.half_size, 250.0, &blockers);
        }
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
            let def = cat.ship(sid);
            let marker = self.config.arena.as_ref().and_then(|a| a.marker_for_structure(&def.name)).map(|m| (m.pos, m.yaw));
            let (pos, syaw) = if def.special_weapon.is_some() {
                let back_dir = Vec3::new(-station_yaw.cos(), -station_yaw.sin(), 0.0);
                (station_pos + back_dir * 1500.0 + Vec3::new(0.0, 0.0, -150.0), station_yaw)
            } else if i == 0 {
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
            self.apply_start_health(id, side, NO_TAG, sid);
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

    /// Start areas of a map can lie closer together than the ships placed on them: pushes overlapping big ships apart before the first
    /// tick (stations stay where they are), so no battle starts with hulls inside each other.
    fn relax_spawn(&mut self) {
        let cat = self.catalog.clone();
        let radius = |d: &ShipDef| if d.class as u8 >= TargetClass::Frigate as u8 { d.radius * 0.75 } else { d.radius };
        let hulls: Vec<usize> = (0..self.ships.len()).filter(|&i| !matches!(cat.ship(self.ships[i].def).class, TargetClass::Fighter | TargetClass::Bomber)).collect();
        for _ in 0..80 {
            let mut moved = false;
            for (a, &i) in hulls.iter().enumerate() {
                for &j in &hulls[a + 1..] {
                    let (di, dj) = (cat.ship(self.ships[i].def), cat.ship(self.ships[j].def));
                    if di.is_static && dj.is_static {
                        continue;
                    }
                    let min = (radius(di) + radius(dj)) * 0.9;
                    let (dx, dy) = (self.ships[i].pos.x - self.ships[j].pos.x, self.ships[i].pos.y - self.ships[j].pos.y);
                    let d = (dx * dx + dy * dy).sqrt();
                    if d >= min {
                        continue;
                    }
                    let (ux, uy) = if d > 1e-3 { (dx / d, dy / d) } else { (1.0, 0.0) };
                    let push = min - d;
                    let (wi, wj) = match (di.is_static, dj.is_static) {
                        (true, _) => (0.0, 1.0),
                        (_, true) => (1.0, 0.0),
                        _ => (0.5, 0.5),
                    };
                    self.ships[i].pos.x += ux * push * wi;
                    self.ships[i].pos.y += uy * push * wi;
                    self.ships[j].pos.x -= ux * push * wj;
                    self.ships[j].pos.y -= uy * push * wj;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
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
        // Rows are packed: a fleet of fighters only starts at the origin, not three rows ahead of it.
        for (ri, row) in rows.iter().filter(|r| !r.is_empty()).enumerate() {
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
                } else if def.class == TargetClass::Fighter || def.class == TargetClass::Bomber {
                    let squad = self.squads.len() as u32;
                    self.squads.push(Vec::new());
                    let id = self.spawn_ship(sid, side, pos, yaw, group, squad, Vec3::ZERO);
                    self.squads[squad as usize].push(id);
                    self.groups[group as usize].members.push(id);
                    self.ships[id as usize].order = order;
                } else {
                    let id = self.spawn_ship(sid, side, pos, yaw, group, NONE, Vec3::ZERO);
                    self.ships[id as usize].lane = local.y.clamp(-1500.0, 1500.0);
                    self.apply_start_health(id, side, tag, sid);
                    self.groups[group as usize].members.push(id);
                    self.ships[id as usize].order = if def.is_static { Order::Idle } else { order };
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
            special_cd: if let Some(sp) = &def.special_weapon { 15.0_f32.min(sp.recharge) } else { 0.0 },
            special_pulses: 0,
            special_timer: 0.0,
            special_target: NONE,
            hunt_cell: NO_CELL,
            hunt_since: 0.0,
            flyout_left: 0.0,
            group,
            squad,
            formation,
            carrier: NONE,
            slots: Vec::new(),
            reserve: self.spawned_for(def, side).1,
            abilities: def.abilities.iter().map(|a| AbilityState { active: 0.0, cooldown: a.super_laser.as_ref().map_or(0.0, |l| l.initial_cooldown), target: NONE, autocast: a.supports_autofire && a.autocast_default }).collect(),
            tractor_cut: 0.0,
            path: Vec::new(),
            path_goal: Vec3::ZERO,
            path_age: 1.0e9,
            since_hit: 1.0e3,
            shield_lock: 0.0,
            stun: 0.0,
            stun_speed_cut: 0.0,
            stun_rate_cut: 0.0,
            shield_gen_ok: true,
            engine_factor: 1.0,
            phase: 0.0,
            phase_turn: 0.0,
            lane: 0.0,
            candidates: Vec::new(),
            scan_in: rng.range(0.0, self.config.retarget_interval),
            seen_until: [0.0; 4],
            heal_in: [0.0; 2],
            bonus: Bonus::default(),
            mods: [(0.0, 0.0); ModKind::COUNT],
        });
        id
    }

    /// The squadrons a carrier launches at the start and holds in reserve, for the tech level of its side: the highest level the data has
    /// at or below it, else the lowest. Definitions without per-level entries use the same lists at every level.
    fn spawned_for(&self, def: &ShipDef, side: u8) -> (Squadrons, Squadrons) {
        if def.spawned_by_tech.is_empty() {
            return (def.carried.clone(), def.reserve.clone());
        }
        let tech = self.config.side_tech.get(side as usize).copied().unwrap_or(0);
        let pick = def.spawned_by_tech.iter().filter(|t| t.0 <= tech).max_by_key(|t| t.0).or_else(|| def.spawned_by_tech.iter().min_by_key(|t| t.0));
        pick.map_or((Vec::new(), Vec::new()), |t| (t.1.clone(), t.2.clone()))
    }

    /// Launches the squadrons carried by ships and stations. When staged_hangar_launch is true,
    /// the first squadron launches immediately and subsequent squadrons launch sequentially from the hangar bay.
    fn launch_initial(&mut self) {
        let carriers: Vec<u32> = self.ships.iter().filter(|s| !self.spawned_for(self.catalog.ship(s.def), s.side).0.is_empty()).map(|s| s.id).collect();
        for c in carriers {
            let def = self.catalog.ship(self.ships[c as usize].def);
            let carried = self.spawned_for(def, self.ships[c as usize].side).0;
            let delay = def.spawn_delay.max(0.5);
            let mut first = true;
            let mut launch_delay = delay;
            for (sq_def, count) in carried {
                for _ in 0..count {
                    if !self.config.staged_hangar_launch || first {
                        let members = self.launch_squadron(c, sq_def);
                        self.ships[c as usize].slots.push(SquadSlot { def: sq_def, members, respawn_in: -1.0 });
                        first = false;
                    } else {
                        self.ships[c as usize].slots.push(SquadSlot {
                            def: sq_def,
                            members: Vec::new(),
                            respawn_in: launch_delay,
                        });
                        launch_delay += delay;
                    }
                }
            }
        }
    }

    /// Spawns the fighters of squadron definition `sq_def` emerging from the hangar bay of `carrier`.
    fn launch_squadron(&mut self, carrier: u32, sq_def: ShipId) -> Vec<u32> {
        let cat = self.catalog.clone();
        let c = &self.ships[carrier as usize];
        let (side, cpos, cyaw, cdef) = (c.side, c.pos, c.yaw, cat.ship(c.def));
        // Squadrons leave through a hangar that still works (the first one that does; a carrier without hangar hardpoints launches from its centre).
        let bay_def = cdef
            .hardpoints
            .iter()
            .enumerate()
            .filter(|(_, h)| h.kind == HpKind::FighterBay)
            .find(|(i, _)| c.hps.get(*i).is_none_or(|s| s.alive))
            .or_else(|| cdef.hardpoints.iter().enumerate().find(|(_, h)| h.kind == HpKind::FighterBay))
            .map(|(_, h)| h);
        let bay = bay_def.map_or(Vec3::ZERO, |h| h.offset);
        let flyout = bay_def.map_or(0.0, |h| h.flyout);
        // The way out: away from the ship's centre through the bay (a bay on the centre line opens to the bow).
        let out_yaw = if bay.x.hypot(bay.y) > 20.0 { cyaw + bay.y.atan2(bay.x) } else { cyaw };
        let mut rng = Rng::for_entity(self.config.seed, self.tick, carrier, 0x1A0C + self.ships.len() as u64);
        // Emerge tightly from the hangar bay opening (not scattered across a kilometer ball around the ship)
        let base = cpos + (bay + rng.in_ball() * 25.0).rotate_yaw(cyaw);
        let sq = cat.ship(sq_def);
        let squad = self.squads.len() as u32;
        self.squads.push(Vec::new());
        let ai_order = if self.is_ai(side) {
            let enemy_pos = self.spawn_pos[(side as usize + 1) % self.sides.len()];
            Order::AttackMove(enemy_pos)
        } else if matches!(self.ships[carrier as usize].order, Order::Idle) {
            Order::Idle
        } else {
            self.ships[carrier as usize].order
        };
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
                if flyout > 0.0 {
                    s.yaw = out_yaw;
                    s.flyout_left = flyout;
                }
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

    /// Recomputes which enemy ships each side sees: within the `Space_FOW_Reveal_Range` of one of its ships (a stealthed ship only within
    /// `stealth_detection`) or inside a revealed area. A seen ship stays visible for `fog_linger` seconds.
    fn update_visibility(&mut self) {
        let cat = self.catalog.clone();
        let (time, linger, detect) = (self.time, self.config.fog_linger, self.config.stealth_detection);
        self.reveals.retain(|r| r.3 > time);
        let sides = self.sides.len().min(4);
        for side in 0..sides as u8 {
            let observers: Vec<(Vec3, f32)> = self.ships.iter().filter(|s| s.alive && !s.retreated && s.side == side).map(|s| (s.pos, if cat.ship(s.def).sensor_range > 0.0 { cat.ship(s.def).sensor_range } else { f32::MAX })).collect();
            let reveals: Vec<(Vec3, f32)> = self.reveals.iter().filter(|r| r.0 == side).map(|r| (r.1, r.2)).collect();
            for s in self.ships.iter_mut().filter(|s| s.alive && !s.retreated && s.side != side) {
                let d = cat.ship(s.def);
                let stealthed = d.abilities.iter().zip(&s.abilities).any(|(a, st)| a.stealth && st.active > 0.0);
                let r = d.radius;
                let seen = reveals.iter().any(|&(p, rad)| p.dist(s.pos) <= rad + r)
                    || observers.iter().any(|&(p, range)| {
                        let dist = p.dist(s.pos) - r;
                        dist <= if stealthed { range.min(detect) } else { range }
                    });
                if seen {
                    s.seen_until[side as usize] = time + linger;
                }
            }
        }
    }

    /// An AI side searches for and engages enemy forces. Visible enemy ships (prioritizing mobile units,
    /// then space stations/structures) are engaged by idle, guarding, and attack-moving ships.
    /// When nothing is in sight, attack-moving ships sweep the map along search points.
    fn ai_search(&mut self) {
        let cat = self.catalog.clone();
        let (time, half) = (self.time, self.config.half_size);
        for side in 0..self.sides.len().min(4) as u8 {
            if !self.is_ai(side) {
                continue;
            }
            let station_under_attack = self.ships.iter().any(|st| st.alive && !st.retreated && st.side == side && cat.ship(st.def).is_static && st.since_hit < 15.0);
            let visible_enemy = self.ships.iter()
                .filter(|s| s.alive && !s.retreated && s.side != side && (s.side >= 4 || !self.config.fog_of_war || time <= s.seen_until[side as usize]))
                .max_by_key(|s| {
                    let d = cat.ship(s.def);
                    let mobile_bonus = if d.is_static { 0 } else { 100 };
                    mobile_bonus + d.class as u8
                });
            if let Some(target_enemy) = visible_enemy {
                self.search[side as usize] = (0, time);
                let enemy_pos = target_enemy.pos;
                for s in self.ships.iter_mut().filter(|s| s.alive && !s.retreated && s.side == side && !cat.ship(s.def).is_static) {
                    let should_order = match s.order {
                        Order::AttackMove(_) => s.pos.dist(enemy_pos) > 1200.0,
                        Order::Idle => true,
                        Order::Guard(guard_pt) => {
                            station_under_attack || s.pos.dist(enemy_pos) < 5000.0 || guard_pt.dist(enemy_pos) < 5000.0 || s.since_hit < 15.0
                        }
                        _ => false,
                    };
                    if should_order {
                        let sdef = cat.ship(s.def);
                        let dest = if sdef.is_flanker() && s.pos.dist(enemy_pos) > 2500.0 {
                            let to = enemy_pos - s.pos;
                            let flank_sign = if s.id % 2 == 0 { 1.0 } else { -1.0 };
                            let perp = Vec3::new(-to.y, to.x, 0.0).normalized() * flank_sign;
                            enemy_pos + perp * (3200.0 + ((s.id % 3) as f32 * 600.0))
                        } else {
                            enemy_pos
                        };
                        s.order = Order::AttackMove(dest);
                    }
                }
                continue;
            }
            let (idx, since) = self.search[side as usize];
            let enemy = self.spawn_pos[(side as usize + 1) % self.sides.len()];
            let cur_point = search_point(idx, enemy, half);
            let reached = self.ships.iter().any(|s| s.alive && !s.retreated && s.side == side && !cat.ship(s.def).is_static && s.pos.dist(cur_point) < 2500.0);
            let timeout = if idx == 0 { 90.0 } else { 45.0 };
            if !reached && time - since < timeout {
                continue;
            }
            let next = idx + 1;
            let point = search_point(next, enemy, half);
            self.search[side as usize] = (next, time);
            for s in self.ships.iter_mut().filter(|s| s.alive && !s.retreated && s.side == side && !cat.ship(s.def).is_static && s.target == NONE && matches!(s.order, Order::AttackMove(_))) {
                s.order = Order::AttackMove(point);
            }
        }
    }

    /// True if `side` currently sees ship `id` (always true for its own ships and without fog of war).
    pub fn is_visible_to(&self, side: u8, id: u32) -> bool {
        let Some(s) = self.ships.get(id as usize) else { return false };
        !self.config.fog_of_war || s.side == side || side >= 4 || self.catalog.ship(s.def).is_static || self.time <= s.seen_until[side as usize]
    }

    /// Whether `side` may call in reinforcements at `pos` (see [`crate::placement_check`]): inside the battle area, outside the no-spawn zones of enemy
    /// structures and in an area the side sees. The UI evaluates the same rule on the snapshot to colour the reinforcement ghost.
    pub fn reinforcement_zone_check(&self, side: u8, pos: Vec3) -> Result<(), CommandError> {
        let units: Vec<crate::PlacementUnit> = self
            .ships
            .iter()
            .filter(|s| s.alive && !s.retreated)
            .map(|s| crate::PlacementUnit::new(self.catalog.ship(s.def), s.side, s.pos, self.config.no_spawn_radius))
            .collect();
        let reveals: Vec<(Vec3, f32)> = self.reveals.iter().filter(|r| r.0 == side).map(|r| (r.1, r.2)).collect();
        crate::placement_check(self.config.half_size - 300.0, self.config.fog_of_war, side, pos, &units, &reveals)
    }

    /// Population of everything of `side` that is alive (or on its way) now, counted per unit (a squadron once while any of its fighters lives).
    /// The unit cap applies to this number, so losses make room for reinforcements.
    pub fn pop_in_play(&self, side: u8) -> i32 {
        let mut pop = 0;
        for g in self.groups.iter().filter(|g| g.side == side) {
            let def = self.catalog.ship(g.ship);
            let per: u32 = if def.is_squadron { def.members.iter().map(|&(_, c)| c).sum::<u32>().max(1) } else { 1 };
            let alive = g.members.iter().filter(|&&m| self.ships[m as usize].alive && !self.ships[m as usize].retreated).count() as u32;
            pop += def.population * alive.div_ceil(per) as i32;
        }
        pop + self.pending.iter().filter(|r| r.side == side).map(|r| self.catalog.ship(r.ship).population * r.count as i32).sum::<i32>()
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
                for &i in &ships {
                    self.stop_hunt(i);
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
                for &i in &ships {
                    self.stop_hunt(i);
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
                if !self.ships[target as usize].hps.get(hardpoint as usize).is_some_and(|h| h.alive) {
                    return Err(CommandError::InvalidHardpoint);
                }
                for &i in &ships {
                    self.stop_hunt(i);
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
                for &i in &ships {
                    self.stop_hunt(i);
                }
                for i in ships {
                    let s = &mut self.ships[i as usize];
                    s.order = Order::Idle;
                    s.target_hp = NO_HP;
                }
            }
            Command::UseAbilityAt { ship, ability, pos } => {
                if !own(self, &[ship]) {
                    return Err(CommandError::InvalidShip);
                }
                // Super laser: the enemy ship nearest to the chosen point (within 600 units) is the target.
                let laser = self.catalog.ship(self.ships[ship as usize].def).abilities.get(ability as usize).and_then(|a| a.super_laser.clone());
                let mut shot = None;
                if let Some(l) = &laser {
                    let cat = self.catalog.clone();
                    shot = self
                        .ships
                        .iter()
                        .filter(|o| o.alive && !o.retreated && o.side != side && (l.classes == 0 || l.classes & cat.ship(o.def).class.bit() != 0) && o.pos.dist(pos) <= 600.0 + cat.ship(o.def).radius)
                        .min_by(|a, b| a.pos.dist(pos).total_cmp(&b.pos.dist(pos)).then(a.id.cmp(&b.id)))
                        .map(|o| o.id);
                    if shot.is_none() {
                        return Err(CommandError::AbilityUnavailable);
                    }
                }
                // Corrupt systems: the enemy ship nearest to the chosen point, within the caster's range.
                let corrupt = self.catalog.ship(self.ships[ship as usize].def).abilities.get(ability as usize).and_then(|a| a.corrupt.clone());
                let mut drained = None;
                if let Some(c) = &corrupt {
                    let cat = self.catalog.clone();
                    let from = self.ships[ship as usize].pos;
                    drained = self
                        .ships
                        .iter()
                        .filter(|o| o.alive && !o.retreated && o.side != side && o.pos.dist(from) <= c.max_range && o.pos.dist(pos) <= c.pick_radius + cat.ship(o.def).radius && cat.ship(o.def).shield > 0.0)
                        .min_by(|a, b| a.pos.dist(pos).total_cmp(&b.pos.dist(pos)).then(a.id.cmp(&b.id)))
                        .map(|o| o.id);
                    if drained.is_none() {
                        return Err(CommandError::AbilityUnavailable);
                    }
                }
                self.ability_at = Some(pos);
                let activated = self.activate_ability(ship, ability as usize);
                self.ability_at = None;
                if !activated {
                    return Err(CommandError::AbilityUnavailable);
                }
                if let (Some(c), Some(t)) = (&corrupt, drained) {
                    self.drains.push((t, c.drain, self.time + c.duration));
                }
                if let (Some(l), Some(t)) = (&laser, shot) {
                    self.lasers.push((ship, t, self.time + l.delay));
                    if self.config.record_events {
                        self.cmd_events.push(Event::SuperLaserCharging { ship, target: t, fire_in: l.delay });
                    }
                }
                let d = self.catalog.ship(self.ships[ship as usize].def);
                if let Some(a) = d.abilities.get(ability as usize)
                    && let Some(z) = &a.zone
                {
                    let from = self.ships[ship as usize].pos;
                    let to = pos - from;
                    let at = if to.len() > z.max_range { from + to.normalized() * z.max_range } else { pos };
                    if let Some(zone) = self.zones.last_mut() {
                        zone.1 = at;
                    }
                }
                if let Some(a) = d.abilities.get(ability as usize).filter(|a| a.reveal_radius > 0.0) {
                    self.reveals.retain(|r| r.3 > self.time);
                    self.reveals.push((side, pos, a.reveal_radius, self.time + a.duration));
                }
            }
            Command::SetAutocast { ship, ability, on } => {
                if !own(self, &[ship]) {
                    return Err(CommandError::InvalidShip);
                }
                let d = self.catalog.ship(self.ships[ship as usize].def);
                if !d.abilities.get(ability as usize).is_some_and(|a| a.supports_autofire) {
                    return Err(CommandError::AbilityUnavailable);
                }
                self.ships[ship as usize].abilities[ability as usize].autocast = on;
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
                if self.interdicted(side) {
                    return Err(CommandError::RetreatBlocked);
                }
                if self.retreat_jump_at[side as usize].is_some() {
                    return Ok(());
                }
                self.retreat_ordered[side as usize] = true;
                self.retreat_jump_at[side as usize] = Some(self.time + self.config.retreat_jump_seconds.max(0.0));
                if self.config.record_events {
                    self.cmd_events.push(Event::RetreatOrdered { side, jump_in: self.config.retreat_jump_seconds });
                }
                for s in self.ships.iter_mut().filter(|s| s.side == side && s.alive && !self.catalog.ship(s.def).is_static) {
                    s.order = Order::Retreat;
                }
            }
            Command::Cheat(cheat) => self.apply_cheat(side, cheat),
            Command::ReinforceAt { ship, count, pos, yaw_offset } => {
                self.reinforcement_zone_check(side, pos)?;
                let pop = self.catalog.ship(ship).population * count as i32;
                if self.pop_in_play(side) + pop > self.config.unit_cap && !self.cheats.has_no_pop_cap(side) {
                    return Err(CommandError::UnitCap);
                }
                if self.config.record_events {
                    self.cmd_events.push(Event::ReinforcementsCalled { side, ship, count });
                }
                self.pending.push(Reinforcement { at: self.time + self.config.reinforce_delay, side, ship, count, pos: Some(pos), yaw_offset: yaw_offset.clamp(-0.27, 0.27) });
            }
            Command::Reinforce { ship, count } => {
                let pop = self.catalog.ship(ship).population * count as i32;
                if self.pop_in_play(side) + pop > self.config.unit_cap && !self.cheats.has_no_pop_cap(side) {
                    if self.config.record_events {
                        self.cmd_events.push(Event::ReinforcementsRefused { side, ship, count });
                    }
                    return Err(CommandError::UnitCap);
                }
                if self.config.record_events {
                    self.cmd_events.push(Event::ReinforcementsCalled { side, ship, count });
                }
                self.pending.push(Reinforcement { at: self.time + self.config.reinforce_delay, side, ship, count, pos: None, yaw_offset: 0.0 });
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
        let is_hunt = a.hunt;
        let is_replenish = a.replenish;
        let weaken = a.weaken.clone();
        if is_replenish && self.wingmen(ship).1.is_empty() {
            // Nobody to bring back: the once-per-battle use is kept.
            return false;
        }
        if a.toggle && self.ships[ship as usize].abilities[ability].active > 0.0 {
            // Toggle abilities are switched off by using them again.
            self.ships[ship as usize].abilities[ability].active = 0.0;
            if is_hunt {
                self.stop_hunt(ship);
                if matches!(self.ships[ship as usize].order, Order::Attack(_) | Order::AttackMove(_)) {
                    self.ships[ship as usize].order = Order::Idle;
                }
            }
            return true;
        }
        if self.ships[ship as usize].abilities[ability].cooldown > 0.0 || self.ships[ship as usize].abilities[ability].active > 0.0 {
            return false;
        }
        let mut target = NONE;
        if a.reveal_radius > 0.0 {
            // Used without a place: the area around the ship.
            let (side, pos) = (self.ships[ship as usize].side, self.ships[ship as usize].pos);
            self.reveals.retain(|r| r.3 > self.time);
            self.reveals.push((side, pos, a.reveal_radius, self.time + a.duration));
        }
        if let Some(z) = &a.zone {
            // Used without a place: the area around the ship itself (a point-targeted use adds its own area in `UseAbilityAt`).
            let (side, pos) = (self.ships[ship as usize].side, self.ships[ship as usize].pos);
            self.zones.push((side, pos, z.radius, z.enemy_dps, self.time + a.duration));
        }
        if let Some(t) = &a.tractor {
            match self.tractor_target(ship, t) {
                Some(x) => target = x,
                None => return false,
            }
        }
        let no_cooldown = self.cheats.has_no_cooldowns(self.ships[ship as usize].side);
        let st = &mut self.ships[ship as usize].abilities[ability];
        st.target = target;
        st.active = a.duration;
        st.cooldown = if a.toggle || no_cooldown { 0.0 } else { a.recharge + a.duration };
        if a.salvo
            || a.kind.eq_ignore_ascii_case("FULL_SALVO")
            || a.kind.eq_ignore_ascii_case("POWER_TO_WEAPONS")
            || a.kind.eq_ignore_ascii_case("MAXIMUM_FIREPOWER")
            || a.kind.eq_ignore_ascii_case("CONCENTRATE_FIRE")
        {
            for hp_st in &mut self.ships[ship as usize].hps {
                hp_st.cooldown = 0.0;
                hp_st.pulse_timer = 0.0;
            }
        }
        if is_replenish {
            self.replenish_wingmen(ship);
        }
        if let Some(w) = weaken {
            self.weaken_area(ship, &w);
        }
        if self.config.record_events {
            self.events.push(Event::AbilityActivated { ship, ability: ability as u16 });
        }
        true
    }

    /// Advances the battle by one fixed time step and returns the events it produced.
    pub fn step(&mut self) -> &[Event] {
        self.events.clear();
        let carried = std::mem::take(&mut self.cmd_events);
        self.events.extend(carried);
        if self.over {
            return &self.events;
        }
        let dt = self.config.dt;
        self.tick += 1;
        self.time += dt;
        self.spawn_reinforcements();
        self.retreat_jumps();

        let cat = self.catalog.clone();
        let cfg = self.config.clone();
        let par = cfg.parallel;

        if cfg.fog_of_war && self.tick % 15 == 1 {
            self.update_visibility();
        }
        if self.tick.is_multiple_of(30) {
            self.ai_search();
        }
        // Snapshot and grid.
        let time_now = self.time;
        let fog = cfg.fog_of_war;
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
                    hit_radius: if d.background { 0.0 } else { radius.max(d.hardpoint_reach) },
                    side: s.side,
                    class: d.class,
                    def: s.def,
                    alive: s.alive && !s.retreated,
                    golan: d.golan,
                    hp_alive: HpMask::from_alive(s.hps.iter().map(|h| h.alive)),
                    carrier: s.carrier,
                    jammed: d.abilities.iter().zip(&s.abilities).any(|(a, st)| a.jam && st.active > 0.0),
                    health: ((s.hull.max(0.0) + s.shield.max(0.0)) / (d.hull.max(1.0) + d.shield)).clamp(0.0, 1.0),
                    // Stations and structures are known from the start (the map shows them); only mobile ships hide in the fog.
                    seen_by: if fog && !d.is_static {
                        (0..4u8).filter(|&k| k == s.side || time_now <= s.seen_until[k as usize]).fold(!0u8 << 4, |m, k| m | (1 << k))
                    } else {
                        0xFF
                    },
                    special: d.special_weapon.is_some() || d.background,
                    target_hp: s.target_hp,
                }
            })
            .collect();
        let mut grid = Grid::new(cfg.half_size, std::mem::take(&mut self.grid_cells));
        grid.rebuild(&snaps);

        // Target scans, movement.
        let tick = self.tick;
        let spawn = self.spawn_pos.clone();
        let arena = cfg.arena.clone();
        let obstacles: &[crate::arena::Obstacle] = arena.as_ref().map_or(&[], |a| a.obstacles.as_slice());
        // Mobile ships that keep each other apart (fighters and stations excluded).
        let big: Vec<u32> = self.ships.iter().filter(|s| s.alive && !s.retreated && !cat.ship(s.def).is_static && !matches!(cat.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber)).map(|s| s.id).collect();
        // Hulls that block big ships: every big ship and station (the last two lists are used for steering and for keeping fighters out of hulls).
        let hulls: Vec<u32> = if cfg.navigation { self.ships.iter().filter(|s| s.alive && !s.retreated && !matches!(cat.ship(s.def).class, TargetClass::Fighter | TargetClass::Bomber)).map(|s| s.id).collect() } else { big.clone() };
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
                mov(s, &snaps, &cat, &cfg, &spawn, &squad_leaders, obstacles, &big, &Nav { grid: self.nav_grid.as_ref(), hulls: &hulls }, &mut rng);
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
            // Homing missiles leave the launcher off the target line and curve in (`ProjDef::arc_offset`); the side is drawn per shot.
            let mut vel = r.vel;
            if let Some(pd) = pd
                && pd.arc_offset > 0.0
                && r.target != NONE
            {
                let mut arc_rng = Rng::for_entity(cfg.seed, tick, r.ship, 0xA4C0 ^ self.projectiles.len() as u64);
                let side = if arc_rng.next_u64() & 1 == 0 { 1.0 } else { -1.0 };
                vel = vel.rotate_yaw(side * pd.arc_offset * (0.6 + 0.4 * arc_rng.unit()));
            }
            self.projectiles.push(Projectile {
                pos: r.from,
                vel,
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
        self.upkeep(dt);
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
            let origin = r.pos.unwrap_or(self.spawn_pos[r.side as usize]);
            // Reinforcements jump in along the consistent hyperspace vector of their side.
            let yaw = self.side_places.get(r.side as usize).and_then(|p| p.first()).map_or(if r.side == 0 { 0.0 } else { std::f32::consts::PI }, |p| p.1);
            let yaw = yaw + r.yaw_offset;
            let mut ids = Vec::new();
            for k in 0..r.count {
                let mut rng = Rng::for_entity(self.config.seed, self.tick, k, 0x7E1F + r.ship as u64);
                let mut pos = origin + rng.in_ball() * 800.0;
                pos.z = origin.z;
                if r.pos.is_some() {
                    // A chosen point: arrive in a tight group on free space (nothing within `reinforce_clearance`), moving out in rings when it is taken.
                    let clear = self.config.reinforce_clearance;
                    let taken = |p: Vec3, ships: &[Ship]| ships.iter().any(|s| s.alive && !s.retreated && s.pos.dist(p) < clear + self.catalog.ship(s.def).radius);
                    let mut found = origin;
                    'ring: for ring in 0..12u32 {
                        let n = if ring == 0 { 1 } else { 6 * ring };
                        for j in 0..n {
                            let a = std::f32::consts::TAU * j as f32 / n as f32 + k as f32;
                            let p = origin + Vec3::new(a.cos(), a.sin(), 0.0) * (ring as f32 * clear * 1.5);
                            if !taken(p, &self.ships) {
                                found = p;
                                break 'ring;
                            }
                        }
                    }
                    pos = found;
                }
                // Reinforcements are results groups too (tag `REINFORCE_TAG`), so the galaxy learns which of them survived.
                let group = self.groups.len() as u32;
                self.groups.push(Group { side: r.side, tag: REINFORCE_TAG, ship: r.ship, members: Vec::new() });
                if def.is_squadron {
                    let squad = self.squads.len() as u32;
                    self.squads.push(Vec::new());
                    let mut s_idx = 0usize;
                    for &(m, c) in &def.members {
                        for _ in 0..c {
                            let off = def.offsets.get(s_idx).copied().unwrap_or(Vec3::new(-20.0 * s_idx as f32, 25.0 * (s_idx % 2) as f32, 0.0));
                            s_idx += 1;
                            let id = self.spawn_ship(m, r.side, pos + off.rotate_yaw(yaw), yaw, group, squad, off);
                            self.squads[squad as usize].push(id);
                            self.groups[group as usize].members.push(id);
                            ids.push(id);
                        }
                    }
                } else {
                    let id = self.spawn_ship(r.ship, r.side, pos, yaw, group, NONE, Vec3::ZERO);
                    self.groups[group as usize].members.push(id);
                    ids.push(id);
                }
            }
            let ai_side = self.is_ai(r.side);
            for &i in &ids {
                let s = &mut self.ships[i as usize];
                s.order = if ai_side { Order::AttackMove(self.spawn_pos[(r.side as usize + 1) % self.sides.len()]) } else { Order::Idle };
            }
            if self.config.record_events {
                self.events.push(Event::ReinforcementsArrived { side: r.side, ships: ids });
            }
        }
    }

    /// `Projectile_Combat_Mod`: the ship hit and every ship within the blast range of the impact get the timed change (see [`CombatMod`]).
    fn apply_combat_mod(&mut self, p: &Projectile, pd: &ProjDef, hit: u32, pos: Vec3) {
        let Some(m) = &pd.combat_mod else { return };
        let cat = self.catalog.clone();
        for (i, s) in self.ships.iter_mut().enumerate() {
            let near = i as u32 == hit || (pd.blast_range > 0.0 && s.pos.dist(pos) - cat.ship(s.def).radius <= pd.blast_range);
            let wanted = if m.value < 0.0 { s.side != p.side } else { s.side == p.side };
            if !near || !wanted || !s.alive || s.retreated || (m.classes != 0 && m.classes & cat.ship(s.def).class.bit() == 0) {
                continue;
            }
            let slot = &mut s.mods[m.kind as usize];
            if m.value.abs() >= slot.0.abs() || slot.1 <= 0.0 {
                *slot = (m.value, m.duration);
            }
        }
    }

    /// `Redirect_Blaster_Ability`: does ship `ship` block or reflect the shot `p`? Only direct shots from an enemy ship in the
    /// reaction arc count; the roll is deterministic per tick, ship and shot.
    fn redirect_roll(&self, ship: u32, p: &Projectile) -> Deflect {
        let cat = &self.catalog;
        let s = &self.ships[ship as usize];
        let def = cat.ship(s.def);
        if def.redirects.is_empty() {
            return Deflect::None;
        }
        let Some(shooter) = self.ships.get(p.owner as usize).filter(|o| o.alive) else { return Deflect::None };
        let class = cat.ship(shooter.def).class;
        let speed = p.vel.len();
        let incoming = if speed > 1.0 { (-p.vel.y).atan2(-p.vel.x) } else { s.yaw };
        let off = crate::math::wrap_angle(incoming - s.yaw).abs();
        let mut rng = Rng::for_entity(self.config.seed, self.tick, ship, 0xB10C0000 + p.owner as u64 * 31 + p.def as u64);
        for r in def.redirects.iter().filter(|r| off <= r.half_arc && (r.classes == 0 || r.classes & class.bit() != 0)) {
            let roll = rng.range(0.0, 1.0);
            if roll < r.redirect {
                return Deflect::Reflected;
            }
            if roll < r.redirect + r.block {
                return Deflect::Blocked;
            }
        }
        Deflect::None
    }

    /// Applies a projectile hit (and its area damage) to ships.
    fn apply_hit(&mut self, p: &Projectile, ship: u32, pos: Vec3, snaps: &[Snap]) {
        let cat = self.catalog.clone();
        let pd = &cat.projectiles[p.def as usize];
        match self.redirect_roll(ship, p) {
            Deflect::Blocked => return,
            Deflect::Reflected => {
                self.damage_ship(p.owner, ship, pd, p.def, pd.damage, pos, NO_HP);
                return;
            }
            Deflect::None => {}
        }
        self.damage_ship(ship, p.owner, pd, p.def, pd.damage, pos, p.aim_hp);
        self.apply_combat_mod(p, pd, ship, pos);
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
        if self.cheats.is_invulnerable(self.ships[id as usize].side) {
            return;
        }
        let cat = self.catalog.clone();
        let record = self.config.record_events;
        let (atk_damage, _, _) = self.side_mult(self.ships.get(attacker as usize).map_or(u8::MAX, |a| a.side));
        let atk_damage = atk_damage * self.ships.get(attacker as usize).map_or(1.0, |a| a.bonus.damage);
        let atk_cause_damage: f32 = if attacker != NONE && let Some(att) = self.ships.get(attacker as usize) {
            let att_def = cat.ship(att.def);
            att_def
                .abilities
                .iter()
                .zip(&att.abilities)
                .filter(|(_, st)| st.active > 0.0)
                .map(|(a, _)| a.cause_damage_mult)
                .product::<f32>()
                * (1.0 + att.mods[ModKind::CauseDamage as usize].0)
        } else {
            1.0
        };
        let (_, tgt_health, tgt_shield) = self.side_mult(self.ships[id as usize].side);
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
        s.since_hit = 0.0;
        let dmg = raw * take * atk_damage * atk_cause_damage / (1.0 + s.mods[ModKind::Defense as usize].0);
        let mut hull_part = 0.0;
        let mut shield_hit = false;
        let mut dead_hps = Vec::new();
        if s.shield > 0.0 && pd.shield_damage {
            let sd = dmg * cat.damage.multiplier(pd.damage_type, def.shield_armor) / tgt_shield * s.bonus.taken_shield / (1.0 + s.mods[ModKind::Shield as usize].0);
            if sd <= s.shield {
                s.shield -= sd;
                if s.shield <= 0.0 {
                    s.shield_lock = cat.depleted_shield_disable;
                }
                shield_hit = true;
                if record {
                    self.events.push(Event::Hit { ship: id, attacker, projectile: proj, pos, kind: HitKind::Shield, damage: sd });
                }
            } else {
                let overflow = (sd - s.shield) / sd;
                s.shield = 0.0;
                s.shield_lock = cat.depleted_shield_disable;
                hull_part = dmg * overflow;
            }
        } else if s.shield <= 0.0 || !pd.shield_damage {
            hull_part = dmg;
        }
        if !shield_hit && pd.hull_damage && hull_part > 0.0 && (s.shield <= 0.0 || !pd.shield_damage) {
            let h = hull_part * cat.damage.multiplier(pd.damage_type, def.armor) / tgt_health * s.bonus.taken_hull / (1.0 + s.mods[ModKind::Health as usize].0);
            // User rule (U47): a unit with destroyable hardpoints has no hull pool of its own, its hit points are the sum of its hardpoints and it dies when
            // the last one is destroyed. The hit goes to the aimed hardpoint, else the one nearest to the impact; damage beyond what the hardpoint had
            // spills over to the next nearest ones, halving at every step (100000, 50000, 25000 ... until it is below 1), all in whole numbers. Units
            // without destroyable hardpoints (fighters, small craft) keep a plain hull.
            let local = (pos - s.pos).rotate_yaw(-s.yaw);
            let mut kind = HitKind::Hull;
            if !def.hardpoints.iter().any(|d| d.destroyable) {
                s.hull -= h;
            } else {
                let mut order: Vec<(f32, usize)> = def
                    .hardpoints
                    .iter()
                    .enumerate()
                    .filter(|(i, d)| d.destroyable && s.hps[*i].alive)
                    .map(|(i, d)| (if aim_hp == i as u16 { -1.0 } else { d.offset.dist(local) }, i))
                    .collect();
                order.sort_by(|a, b| a.0.total_cmp(&b.0));
                let mut incoming = h.round();
                for (n, &(_, i)) in order.iter().enumerate() {
                    if incoming < 1.0 {
                        break;
                    }
                    if n == 0 {
                        kind = HitKind::Hardpoint(i as u16);
                    }
                    let hp = &mut s.hps[i];
                    let over = (incoming - hp.health.max(0.0)).max(0.0);
                    hp.health -= incoming;
                    if hp.health <= 0.0 {
                        hp.alive = false;
                        hp.pulses_left = 0;
                        dead_hps.push(i as u16);
                        let hk = def.hardpoints[i].kind;
                        match hk {
                            HpKind::ShieldGenerator => {
                                s.shield_gen_ok = def.hardpoints.iter().enumerate().any(|(j, d)| d.kind == HpKind::ShieldGenerator && s.hps[j].alive);
                                if !s.shield_gen_ok {
                                    s.shield = 0.0;
                                }
                            }
                            HpKind::Engine => {
                                let (alive, total) = def.hardpoints.iter().enumerate().filter(|(_, d)| d.kind == HpKind::Engine).fold(
                                    (0, 0),
                                    |(a, t), (j, _)| (a + usize::from(s.hps[j].alive), t + 1),
                                );
                                s.engine_factor = if total == 0 { 1.0 } else { 0.20 + 0.80 * (alive as f32 / total as f32) };
                            }
                            _ => {}
                        }
                        if record {
                            self.events.push(Event::HardpointDestroyed { ship: id, hardpoint: i as u16, kind: hk });
                        }
                    }
                    incoming = (over / 2.0).floor();
                }
                // The hull figure everything else reads (bars, retreat and targeting decisions) is the share of hardpoint health that is left.
                let (left, total) = def
                    .hardpoints
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| d.destroyable)
                    .fold((0.0f32, 0.0f32), |(l, t), (i, d)| (l + if s.hps[i].alive { s.hps[i].health.max(0.0) } else { 0.0 }, t + d.health));
                let any_alive = def.hardpoints.iter().enumerate().any(|(i, d)| d.destroyable && s.hps[i].alive);
                s.hull = if any_alive { (def.hull * left / total.max(1.0)).max(0.01) } else { 0.0 };
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
        if !dead_hps.is_empty() {
            for other in &mut self.ships {
                if other.target == id && other.target_hp != NO_HP && dead_hps.contains(&other.target_hp) {
                    other.target_hp = NO_HP;
                }
            }
        }
    }

    /// True while an enemy of `side` holds a gravity well: a station or ship with `Is_Interdictor`, or a ship whose `INTERDICT` ability is on.
    /// The side cannot order a retreat, and a retreat that is running is cancelled.
    pub fn interdicted(&self, side: u8) -> bool {
        self.ships.iter().any(|s| {
            let d = self.catalog.ship(s.def);
            s.alive
                && !s.retreated
                && s.side != side
                && (d.interdictor || d.abilities.iter().zip(&s.abilities).any(|(a, st)| a.interdict && st.active > 0.0))
        })
    }

    /// Seconds until the retreat jump of `side` (`None` when it has not ordered a retreat or the jump is done). For the countdown on the tactical screen.
    pub fn retreat_jump_in(&self, side: u8) -> Option<f32> {
        self.retreat_jump_at.get(side as usize).copied().flatten().map(|t| (t - self.time).max(0.0))
    }

    /// Returns whether the ship can escape during a retreat jump (has working hyperdrive engines,
    /// or its carrier has them). Stations, garrison units, and ships with destroyed engines cannot jump.
    pub fn can_retreat(&self, ship: u32) -> bool {
        let Some(s) = self.ships.get(ship as usize) else { return false; };
        let cat = &self.catalog;
        let d = cat.ship(s.def);
        if d.is_static || !s.alive || s.retreated {
            return false;
        }
        let has_drive = |s: &Ship| {
            let d = cat.ship(s.def);
            !d.is_static && s.alive && !s.retreated && {
                let engines: Vec<usize> = d.hardpoints.iter().enumerate().filter(|(_, h)| h.kind == HpKind::Engine).map(|(i, _)| i).collect();
                engines.is_empty() || engines.iter().any(|&i| s.hps.get(i).map_or(false, |h| h.alive))
            }
        };
        let reinforced_squadron = s.carrier == NONE && s.squad != NONE && self.groups.get(s.group as usize).is_some_and(|g| g.tag == REINFORCE_TAG);
        if s.carrier != NONE {
            self.ships.get(s.carrier as usize).map_or(false, has_drive)
        } else {
            !reinforced_squadron && has_drive(s)
        }
    }

    /// The retreat jump of every side whose timer ran out: each living mobile unit with at least one living engine hardpoint (or none by design)
    /// escapes (`retreated`). Units launched by a carrier escape only with a carrier that escapes; squadrons called in as reinforcements, and everything
    /// that cannot jump (dead drives, stations' fighters), are lost. Stations stay in the battle.
    fn retreat_jumps(&mut self) {
        // A gravity well that comes up while the fleet charges its hyperdrives cancels the jump (Remake `RetreatPlan.lua`: "Retreat was aborted").
        for side in 0..self.retreat_jump_at.len() as u8 {
            if self.retreat_jump_at[side as usize].is_some() && self.interdicted(side) {
                self.retreat_jump_at[side as usize] = None;
                self.retreat_ordered[side as usize] = false;
                for s in self.ships.iter_mut().filter(|s| s.side == side && s.alive && matches!(s.order, Order::Retreat)) {
                    s.order = Order::Idle;
                }
                if self.config.record_events {
                    self.events.push(Event::RetreatAborted { side });
                }
            }
        }
        let due: Vec<u8> = (0..self.retreat_jump_at.len() as u8).filter(|&k| self.retreat_jump_at[k as usize].is_some_and(|t| self.time >= t)).collect();
        for side in due {
            self.retreat_jump_at[side as usize] = None;
            let cat = self.catalog.clone();
            let fate: Vec<(u32, bool)> = self
                .ships
                .iter()
                .filter(|s| s.side == side && s.alive && !s.retreated && !cat.ship(s.def).is_static)
                .map(|s| (s.id, self.can_retreat(s.id)))
                .collect();
            let record = self.config.record_events;
            for (id, escapes) in fate {
                let s = &mut self.ships[id as usize];
                if escapes {
                    s.retreated = true;
                    if record {
                        self.events.push(Event::ShipRetreated { ship: id });
                    }
                } else {
                    s.alive = false;
                    if record {
                        self.events.push(Event::ShipDestroyed { ship: id, pos: s.pos });
                    }
                }
            }
        }
    }

    /// Strength of a side for the retreat decision: sum of `sqrt((hull + shield) x damage rate)` over its living ships (estimate of the
    /// engine's `FriendlyForceUnnormalized`; the thresholds below come from `Retreat_From_Space`).
    fn side_power(&self, side: u8) -> f32 {
        let cat = &self.catalog;
        self.ships
            .iter()
            .filter(|s| s.side == side && s.alive && !s.retreated)
            .map(|s| {
                let d = cat.ship(s.def);
                ((s.hull.max(0.0) + s.shield.max(0.0)) * crate::defs::dps_estimate(d, cat)).sqrt()
            })
            .sum()
    }

    /// `Retreat_From_Space` of the Remake AI (Remake_SpaceCombatEquations.xml): an AI side that is clearly outgunned orders the retreat. Attacker: enemy
    /// force at least 2x its own after 120 s; defender: at least 3x (4x with a station) after 120 s, or after 30 s without any station.
    /// U94: Emergency tactical retreat when critical loss (< 25% survival chance, theirs >= 3x mine) threatens surviving capital ships and heroes.
    /// Not for sides that may not retreat (`BattleConfig::retreat_allowed`, the galaxy says whether the faction holds more than one planet),
    /// not before the retreat countdown, and not while an interdictor blocks it.
    fn ai_retreat(&mut self) {
        let n = self.sides.len();
        for side in 0..n as u8 {
            if !self.is_ai(side) || self.retreat_ordered[side as usize] || !self.config.retreat_allowed.get(side as usize).copied().unwrap_or(true) {
                continue;
            }
            if self.time < self.config.retreat_countdown.max(30.0) {
                continue;
            }
            let cat = self.catalog.clone();
            let stations = self.ships.iter().filter(|s| s.side == side && s.alive && cat.ship(s.def).is_static).count();
            let defender = !self.sides[side as usize].structures.is_empty();

            let mine = self.side_power(side);
            let theirs: f32 = (0..n as u8).filter(|&o| o != side).map(|o| self.side_power(o)).sum();

            // U94: Emergency retreat when overwhelming superiority (< 25% survival chance) threatens to wipe out surviving capital ships
            let has_valuable_ships = self.ships.iter().any(|s| {
                s.side == side && s.alive && !s.retreated && !cat.ship(s.def).is_static
                    && (cat.ship(s.def).class as u8 >= TargetClass::Capital as u8 || cat.ship(s.def).population >= 15)
            });
            let critical_loss = theirs > 0.0 && mine / (mine + theirs) < 0.25;
            let emergency_retreat = has_valuable_ships && critical_loss && (stations == 0 || self.time > 60.0);

            let age_ok = if emergency_retreat {
                self.time >= self.config.retreat_countdown.max(30.0)
            } else if defender {
                if stations == 0 { self.time > 30.0 } else { self.time > 120.0 }
            } else {
                self.time > 120.0
            };
            if !age_ok {
                continue;
            }
            let factor = if defender { 3.0 + f32::from(u8::from(stations > 0)) } else { 2.0 };
            if (theirs > 0.0 && theirs >= factor * mine) || emergency_retreat {
                let _ = self.command(side, Command::Retreat);
            }
        }
    }

    fn upkeep(&mut self, dt: f32) {
        let cat = self.catalog.clone();
        let tick = self.tick;
        if tick.is_multiple_of(60) {
            self.ai_retreat();
        }
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
                s.since_hit += dt;
                for m in &mut s.mods {
                    m.1 = (m.1 - dt).max(0.0);
                    if m.1 == 0.0 {
                        m.0 = 0.0;
                    }
                }
                s.shield_lock = (s.shield_lock - dt).max(0.0);
                if s.shield_gen_ok && def.shield > 0.0 && s.shield < def.shield && s.shield_lock <= 0.0 && s.since_hit >= self.config.shield_regen_delay {
                    let mult: f32 = def.abilities.iter().zip(&s.abilities).filter(|(_, st)| st.active > 0.0).map(|(a, _)| a.shield_regen_mult).product();
                    s.shield = (s.shield + def.shield_regen / cat.shield_interval * mult * dt).min(def.shield);
                }
            }
            // AI sides use weapon abilities when they have something to shoot at.
            let s = &self.ships[i];
            let ai_side = self.is_ai(s.side);
            if (ai_side || s.abilities.iter().any(|a| a.autocast)) && s.target != NONE && s.stance != Stance::HoldFire && (tick + i as u64).is_multiple_of(15) {
                let t = &self.ships[s.target as usize];
                let dist = t.pos.dist(s.pos);
                for (k, a) in def.abilities.iter().enumerate() {
                    let boosts_weapons = a.salvo || a.rocket || a.weapon_delay_mult < 0.99;
                    // Rocket salvos are kept for frigates and bigger.
                    if a.rocket && (cat.ship(t.def).class as u8) < TargetClass::Frigate as u8 {
                        continue;
                    }
                    let effective_range = if a.salvo { (def.preferred_range * 1.15).min(3800.0).max(2500.0) } else { def.max_weapon_range };
                    if boosts_weapons && (ai_side || s.abilities[k].autocast) && s.abilities[k].cooldown <= 0.0 && s.abilities[k].active <= 0.0 && dist <= effective_range {
                        auto_use.push(k);
                        break;
                    }
                }
            }
            if (ai_side || self.ships[i].abilities.iter().any(|a| a.autocast)) && (tick + i as u64).is_multiple_of(15) {
                let s = &self.ships[i];
                for (k, a) in def.abilities.iter().enumerate() {
                    if !(ai_side || s.abilities[k].autocast) {
                        continue;
                    }
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
                    } else if !a.toggle && a.speed_mult > 1.05 && (matches!(s.order, Order::Retreat) || (s.target != NONE && matches!(s.order, Order::AttackMove(_) | Order::Move(_)) && self.ships[s.target as usize].pos.dist(s.pos) > def.max_weapon_range * 2.0)) {
                        // Speed abilities: flee, or close a large gap while on the move.
                        auto_use.push(k);
                    } else if !a.toggle && s.since_hit < 3.0 && ((a.shield_regen_mult > 1.05 && def.shield > 0.0 && s.shield < 0.5 * def.shield) || (a.take_damage_mult < 0.95 && s.hull < 0.6 * def.hull)) {
                        // Shield and damage-reduction abilities: when hurt and under fire.
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
        // Orphaned carrier and station squadrons are destroyed when their mother ship is lost
        for i in 0..self.ships.len() {
            let s = &self.ships[i];
            if s.alive && !s.retreated && s.carrier != NONE {
                if !self.ships[s.carrier as usize].alive || self.ships[s.carrier as usize].retreated {
                    self.ships[i].alive = false;
                    if self.config.record_events {
                        self.events.push(Event::ShipDestroyed { ship: i as u32, pos: self.ships[i].pos });
                    }
                }
            }
        }
        if self.tick.is_multiple_of(15) {
            self.ai_special_abilities();
            self.run_hunts();
            self.run_lures();
        }
        self.run_healing(dt);
        self.run_zones(dt);
        self.run_lasers();
        self.run_drains(dt);
        if self.tick.is_multiple_of(30) {
            self.run_bonuses();
        }
    }

    /// AI use of the abilities that need a place or a reason: sensor jamming against missiles homing on the ship, an area storm on the
    /// densest group of enemies (three or more, or a capital ship), the super laser on the biggest visible enemy, and a reveal ping into
    /// the fog where the side is searching.
    fn ai_special_abilities(&mut self) {
        let cat = self.catalog.clone();
        let mut uses: Vec<(u8, u32, u16, Option<Vec3>)> = Vec::new();
        for side in 0..self.sides.len().min(4) as u8 {
            let ai_side = self.is_ai(side);
            if !ai_side && !self.ships.iter().any(|s| s.side == side && s.alive && s.abilities.iter().any(|a| a.autocast)) {
                continue;
            }
            let enemies: Vec<u32> = self
                .ships
                .iter()
                .filter(|o| o.alive && !o.retreated && o.side != side && self.is_visible_to(side, o.id))
                .map(|o| o.id)
                .collect();
            // `Enemy_Retreating`: the other side has ordered its retreat (the jump is charging).
            let enemy_retreating = (0..self.retreat_jump_at.len() as u8).any(|o| o != side && self.retreat_jump_at[o as usize].is_some());
            for s in self.ships.iter().filter(|s| s.alive && !s.retreated && s.side == side && s.stance != Stance::HoldFire) {
                let def = cat.ship(s.def);
                for (k, a) in def.abilities.iter().enumerate() {
                    let st = &s.abilities[k];
                    if st.cooldown > 0.0 || st.active > 0.0 || !(ai_side || st.autocast) {
                        continue;
                    }
                    let k = k as u16;
                    if a.replenish {
                        // `ObjectScript_VaderTIESquadron`: used when fewer than two fighters of the squadron are left.
                        let (alive, dead) = self.wingmen(s.id);
                        if alive.is_empty() && !dead.is_empty() {
                            uses.push((side, s.id, k, None));
                        }
                    } else if let Some(l) = &a.lure {
                        // Estimate: draw fire when two or more mobile enemies of the listed types are inside the radius.
                        let near = enemies
                            .iter()
                            .filter(|&&e| {
                                let o = &self.ships[e as usize];
                                let d = cat.ship(o.def);
                                !d.is_static && d.max_speed > 0.0 && o.pos.dist(s.pos) <= l.radius && (l.types.is_empty() || l.types.contains(&d.name.to_ascii_lowercase()))
                            })
                            .count();
                        if near >= 2 {
                            uses.push((side, s.id, k, None));
                        }
                    } else if let Some(w) = &a.weaken {
                        // Estimate: the AI weakens the densest group of enemies (three or more, a capital counts as three) within 4000 units.
                        let weight = |id: u32| if cat.ship(self.ships[id as usize].def).class as u8 >= TargetClass::Capital as u8 { 3 } else { 1 };
                        let best = enemies
                            .iter()
                            .filter(|&&e| self.ships[e as usize].pos.dist(s.pos) <= 4000.0)
                            .map(|&e| {
                                let c = self.ships[e as usize].pos;
                                (enemies.iter().filter(|&&o| self.ships[o as usize].pos.dist(c) <= w.radius).map(|&o| weight(o)).sum::<u32>(), e)
                            })
                            .max_by_key(|&(n, e)| (n, std::cmp::Reverse(e)));
                        if let Some((n, e)) = best
                            && n >= 3
                        {
                            uses.push((side, s.id, k, Some(self.ships[e as usize].pos)));
                        }
                    } else if a.interdict {
                        // The interdictor scripts switch the field on against a retreating enemy and off again after `AI_INTERDICT_SECS`.
                        if enemy_retreating && ai_side {
                            uses.push((side, s.id, k, None));
                        }
                    } else if a.jam {
                        let homing = self.projectiles.iter().filter(|p| p.target == s.id && p.side != side && p.pos.dist(s.pos) < 2500.0).count();
                        if homing > 0 {
                            uses.push((side, s.id, k, None));
                        }
                    } else if let Some(z) = &a.zone {
                        // The enemy whose surroundings hold the most enemies (a capital counts as three).
                        let weight = |id: u32| if cat.ship(self.ships[id as usize].def).class as u8 >= TargetClass::Capital as u8 { 3 } else { 1 };
                        let best = enemies
                            .iter()
                            .filter(|&&e| self.ships[e as usize].pos.dist(s.pos) <= z.max_range)
                            .map(|&e| {
                                let c = self.ships[e as usize].pos;
                                (enemies.iter().filter(|&&o| self.ships[o as usize].pos.dist(c) <= z.radius).map(|&o| weight(o)).sum::<u32>(), e)
                            })
                            .max_by_key(|&(n, e)| (n, std::cmp::Reverse(e)));
                        if let Some((n, e)) = best
                            && n >= 3
                        {
                            uses.push((side, s.id, k, Some(self.ships[e as usize].pos)));
                        }
                    } else if let Some(c) = &a.corrupt {
                        // The enemy with the most shield left inside the range.
                        let target = enemies
                            .iter()
                            .copied()
                            .filter(|&e| s.pos.dist(self.ships[e as usize].pos) <= c.max_range * 0.9 && self.ships[e as usize].shield > 0.0)
                            .max_by(|&x, &y| self.ships[x as usize].shield.total_cmp(&self.ships[y as usize].shield).then(y.cmp(&x)));
                        if let Some(e) = target {
                            uses.push((side, s.id, k, Some(self.ships[e as usize].pos)));
                        }
                    } else if let Some(l) = &a.super_laser {
                        let target = enemies
                            .iter()
                            .copied()
                            .filter(|&e| l.classes == 0 || l.classes & cat.ship(self.ships[e as usize].def).class.bit() != 0)
                            .max_by(|&x, &y| cat.ship(self.ships[x as usize].def).hull.total_cmp(&cat.ship(self.ships[y as usize].def).hull).then(y.cmp(&x)));
                        if let Some(e) = target {
                            uses.push((side, s.id, k, Some(self.ships[e as usize].pos)));
                        }
                    } else if a.reveal_radius > 0.0 && self.config.fog_of_war && !enemies.iter().any(|&e| !cat.ship(self.ships[e as usize].def).is_static) {
                        let (idx, _) = self.search[side as usize];
                        let enemy = self.spawn_pos[(side as usize + 1) % self.sides.len()];
                        uses.push((side, s.id, k, Some(search_point(idx + 1, enemy, self.config.half_size))));
                    }
                }
            }
        }
        for (side, ship, ability, at) in uses {
            let result = match at {
                Some(pos) => self.command(side, Command::UseAbilityAt { ship, ability, pos }),
                None => self.command(side, Command::UseAbility { ship, ability }),
            };
            if result.is_ok() && self.catalog.ship(self.ships[ship as usize].def).abilities[ability as usize].interdict {
                self.ships[ship as usize].abilities[ability as usize].active = AI_INTERDICT_SECS;
            }
        }
    }

    /// `WEAKEN_ENEMY`: the enemies of `ship`'s side inside the area (centred on the aimed point, else on the ship) deal less damage and move
    /// slower for the duration; the strongest value of each kind counts.
    fn weaken_area(&mut self, ship: u32, w: &WeakenDef) {
        let side = self.ships[ship as usize].side;
        let at = self.ability_at.unwrap_or(self.ships[ship as usize].pos);
        let cat = self.catalog.clone();
        for s in self.ships.iter_mut().filter(|s| s.alive && !s.retreated && s.side != side && s.pos.dist(at) - cat.ship(s.def).radius <= w.radius) {
            for (kind, value) in [(ModKind::CauseDamage, -w.cause_reduction), (ModKind::Speed, -w.speed_cut), (ModKind::Defense, -w.take_increase)] {
                let slot = &mut s.mods[kind as usize];
                if value != 0.0 && (value.abs() >= slot.0.abs() || slot.1 <= 0.0) {
                    *slot = (value, w.duration);
                }
            }
        }
    }

    /// `LURE`: every 0.5 s the enemy ships of the listed types within the radius of an active lure are ordered to attack the caster; when the
    /// lure ends those still on it go back to idle (AI sides pick a new target with their next search).
    fn run_lures(&mut self) {
        let cat = self.catalog.clone();
        let mut lures: Vec<(u32, u8, Vec3, LureDef)> = Vec::new();
        for s in self.ships.iter().filter(|s| s.alive && !s.retreated) {
            for (a, st) in cat.ship(s.def).abilities.iter().zip(&s.abilities) {
                if let Some(l) = &a.lure
                    && st.active > 0.0
                {
                    lures.push((s.id, s.side, s.pos, l.clone()));
                }
            }
        }
        // Lures that ended release their victims.
        let released: Vec<(u32, u32)> = self.lured.iter().copied().filter(|&(_, caster)| !lures.iter().any(|l| l.0 == caster)).collect();
        self.lured.retain(|p| !released.contains(p));
        for (id, caster) in released {
            let s = &mut self.ships[id as usize];
            if s.order == Order::Attack(caster) {
                s.order = Order::Idle;
            }
        }
        for (caster, side, pos, l) in &lures {
            for s in self.ships.iter_mut().filter(|s| s.alive && !s.retreated && s.side != *side && s.stance != Stance::HoldFire) {
                let d = cat.ship(s.def);
                if d.is_static || d.max_speed <= 0.0 || matches!(s.order, Order::Retreat) || s.pos.dist(*pos) > l.radius {
                    continue;
                }
                if !l.types.is_empty() && !l.types.contains(&d.name.to_ascii_lowercase()) {
                    continue;
                }
                s.order = Order::Attack(*caster);
                s.target = *caster;
                s.target_hp = NO_HP;
                if !self.lured.contains(&(s.id, *caster)) {
                    self.lured.push((s.id, *caster));
                }
            }
        }
    }

    /// The other members of a ship's squadron: (alive, lost).
    fn wingmen(&self, leader: u32) -> (Vec<u32>, Vec<u32>) {
        let squad = self.ships[leader as usize].squad;
        if squad == NONE {
            return (Vec::new(), Vec::new());
        }
        let (mut alive, mut dead) = (Vec::new(), Vec::new());
        for &m in self.squads[squad as usize].iter().filter(|&&m| m != leader) {
            let s = &self.ships[m as usize];
            if s.retreated {
                continue;
            }
            if s.alive { alive.push(m) } else { dead.push(m) }
        }
        (alive, dead)
    }

    /// `REPLENISH_WINGMEN`: every lost wingman is replaced by a new fighter of the same type at its formation slot behind the leader.
    fn replenish_wingmen(&mut self, leader: u32) {
        let (_, dead) = self.wingmen(leader);
        let (pos, yaw, speed, side, group, squad, order) = {
            let l = &self.ships[leader as usize];
            (l.pos, l.yaw, l.speed, l.side, l.group, l.squad, l.order)
        };
        for d in dead {
            let (def, formation, carrier) = (self.ships[d as usize].def, self.ships[d as usize].formation, self.ships[d as usize].carrier);
            let id = self.spawn_ship(def, side, pos + formation.rotate_yaw(yaw), yaw, group, squad, formation);
            let s = &mut self.ships[id as usize];
            s.order = order;
            s.carrier = carrier;
            s.speed = speed;
            self.squads[squad as usize].push(id);
            if let Some(g) = self.groups.get_mut(group as usize) {
                g.members.push(id);
            }
        }
    }

    /// `HUNT` ("Hunt for Enemies": scout the map for hostile targets, destroy the ones found). Every hunter flies alone, out of its squadron
    /// formation: it takes the visible enemy its `Targeting_Priority_Set` ranks best (then the nearest, with a penalty for each other hunter
    /// already on that enemy, so the group spreads over the enemy fleet) and chases it across the map until it is dead. With nothing in
    /// sight it searches the map cell by cell, the nearest cell no hunter of the side has searched yet (estimate: the data names no search
    /// pattern; cell size, the search time limit and the spreading penalty are mine). Runs twice a second.
    fn run_hunts(&mut self) {
        let cat = self.catalog.clone();
        let (half, fog, step) = (self.config.half_size, self.config.fog_of_war, self.config.dt * 15.0);
        let hunters: Vec<u32> = self
            .ships
            .iter()
            .filter(|s| {
                let d = cat.ship(s.def);
                s.alive && !s.retreated && !d.is_static && s.stance != Stance::HoldFire && !matches!(s.order, Order::Retreat)
                    && d.abilities.iter().zip(&s.abilities).any(|(a, st)| a.hunt && st.active > 0.0)
            })
            .map(|s| s.id)
            .collect();
        if hunters.is_empty() {
            return;
        }
        let cell_size = 2.0 * half / HUNT_GRID as f32;
        let cell_of = |p: Vec3| -> u8 {
            let c = |v: f32| (((v + half) / cell_size).floor().max(0.0) as usize).min(HUNT_GRID - 1);
            (c(p.y) * HUNT_GRID + c(p.x)) as u8
        };
        let centre_of = |c: u8| Vec3::new(-half + ((c as usize % HUNT_GRID) as f32 + 0.5) * cell_size, -half + ((c as usize / HUNT_GRID) as f32 + 0.5) * cell_size, 0.0);
        let mut on_target = vec![0u32; self.ships.len()];
        let mut claimed = [0u64; 4];
        for &h in &hunters {
            let s = &self.ships[h as usize];
            let side = (s.side as usize).min(3);
            // Cells and targets already taken by hunters that keep their choice.
            if let Order::Attack(t) = s.order {
                on_target[t as usize] += 1;
            } else if s.hunt_cell != NO_CELL {
                claimed[side] |= 1u64 << s.hunt_cell;
            }
        }
        for &h in &hunters {
            let (side, pos, cur) = {
                let s = &self.ships[h as usize];
                (s.side, s.pos, if let Order::Attack(t) = s.order { Some(t) } else { None })
            };
            let sd = (side as usize).min(3);
            let def = cat.ship(self.ships[h as usize].def);
            let set = def.priority.and_then(|i| cat.priority_sets.get(i as usize));
            if let Some(t) = cur {
                on_target[t as usize] = on_target[t as usize].saturating_sub(1);
            }
            let mut best: Option<(f32, u32)> = None;
            for t in self.ships.iter().filter(|t| t.alive && !t.retreated && t.side != side && (!fog || self.is_visible_to(side, t.id))) {
                let tdef = cat.ship(t.def);
                if tdef.special_weapon.is_some() || tdef.background {
                    continue;
                }
                let weight = match set {
                    Some(p) => {
                        if p.excluded & tdef.class.bit() != 0 {
                            continue;
                        }
                        p.weight(tdef.class, tdef.golan).min(1000.0)
                    }
                    None => f32::from(target_priority(def.class, tdef.class)) * 10.0,
                };
                let mut score = weight * HUNT_WEIGHT_UNITS + t.pos.dist(pos) + on_target[t.id as usize] as f32 * HUNT_CROWD_UNITS;
                if tdef.is_static {
                    score += HUNT_STATIC_UNITS;
                }
                if cur == Some(t.id) {
                    score -= HUNT_STICKY_UNITS;
                }
                if best.is_none_or(|b| score < b.0) {
                    best = Some((score, t.id));
                }
            }
            if let Some((_, t)) = best {
                let s = &mut self.ships[h as usize];
                if cur != Some(t) {
                    s.order = Order::Attack(t);
                    s.target = t;
                    s.target_hp = NO_HP;
                }
                s.hunt_cell = NO_CELL;
                on_target[t as usize] += 1;
                continue;
            }
            // Nothing in sight: search the map.
            let here = cell_of(pos);
            self.hunt_seen[sd] |= 1u64 << here;
            let s = &mut self.ships[h as usize];
            s.hunt_since += step;
            let arrived = s.hunt_cell != NO_CELL && (s.hunt_cell == here || pos.dist(centre_of(s.hunt_cell)) < 0.25 * cell_size);
            if s.hunt_cell != NO_CELL && !arrived && s.hunt_since < HUNT_CELL_SECS && matches!(s.order, Order::AttackMove(_)) {
                continue;
            }
            if s.hunt_cell != NO_CELL {
                self.hunt_seen[sd] |= 1u64 << s.hunt_cell;
                claimed[sd] &= !(1u64 << s.hunt_cell);
            }
            if self.hunt_seen[sd] == u64::MAX {
                self.hunt_seen[sd] = 1u64 << here;
            }
            let taken = self.hunt_seen[sd] | claimed[sd];
            let pick = (0..(HUNT_GRID * HUNT_GRID) as u8)
                .filter(|&c| taken & (1u64 << c) == 0)
                .min_by(|&a, &b| centre_of(a).dist(pos).total_cmp(&centre_of(b).dist(pos)).then(a.cmp(&b)))
                .unwrap_or(here);
            claimed[sd] |= 1u64 << pick;
            s.hunt_cell = pick;
            s.hunt_since = 0.0;
            s.order = Order::AttackMove(centre_of(pick));
            s.target_hp = NO_HP;
        }
    }

    /// Switches `HUNT` off on a ship that got an order of its own, or whose player toggled it off: the hunter stops searching.
    fn stop_hunt(&mut self, ship: u32) {
        let def = self.catalog.ship(self.ships[ship as usize].def);
        let s = &mut self.ships[ship as usize];
        for (a, st) in def.abilities.iter().zip(&mut s.abilities) {
            if a.hunt {
                st.active = 0.0;
            }
        }
        s.hunt_cell = NO_CELL;
    }

    /// `CORRUPT_SYSTEMS`: the shield of a corrupted ship drains until the effect ends (a depleted shield stays down for the usual time).
    fn run_drains(&mut self, dt: f32) {
        let now = self.time;
        self.drains.retain(|d| d.2 > now);
        let shield_lock = self.catalog.depleted_shield_disable;
        for &(t, rate, _) in &self.drains {
            let s = &mut self.ships[t as usize];
            if s.alive && s.shield > 0.0 {
                s.shield = (s.shield - rate * dt).max(0.0);
                if s.shield <= 0.0 {
                    s.shield_lock = shield_lock;
                }
            }
        }
    }

    /// `SUPER_LASER` shots that reach their firing time destroy the target if the shooter still lives.
    fn run_lasers(&mut self) {
        if self.lasers.is_empty() {
            return;
        }
        let now = self.time;
        let due: Vec<(u32, u32, f32)> = self.lasers.iter().copied().filter(|l| l.2 <= now).collect();
        self.lasers.retain(|l| l.2 > now);
        for (shooter, target, _) in due {
            if !self.ships[shooter as usize].alive {
                continue;
            }
            let t = &mut self.ships[target as usize];
            if t.alive {
                t.alive = false;
                t.hull = 0.0;
                let pos = t.pos;
                if self.config.record_events {
                    self.events.push(Event::SuperLaserFired { ship: shooter, target, at: pos });
                    self.events.push(Event::ShipDestroyed { ship: target, pos });
                }
            }
        }
    }

    /// Damaging areas (`BUZZ_DROIDS`): enemy ships inside lose `dps` per second, shield first, then hull.
    fn run_zones(&mut self, dt: f32) {
        let now = self.time;
        self.zones.retain(|z| z.4 > now);
        if self.zones.is_empty() {
            return;
        }
        let record = self.config.record_events;
        for z in self.zones.clone() {
            for s in self.ships.iter_mut().filter(|s| s.alive && !s.retreated && s.side != z.0 && s.pos.dist(z.1) <= z.2) {
                let mut dmg = z.3 * dt * s.bonus.taken_hull;
                let soak = dmg.min(s.shield.max(0.0));
                s.shield -= soak;
                dmg -= soak;
                s.hull -= dmg;
                if s.hull <= 0.0 {
                    s.alive = false;
                    if record {
                        self.events.push(Event::ShipDestroyed { ship: s.id, pos: s.pos });
                    }
                }
            }
        }
    }

    /// `Combat_Bonus_Ability` auras: per side, the strongest bonus of each stacking category counts, categories add up.
    fn run_bonuses(&mut self) {
        let cat = self.catalog.clone();
        if !self.ships.iter().any(|s| !cat.ship(s.def).bonuses.is_empty()) {
            return;
        }
        // (side, stacking category) -> strongest source bonus per stat.
        let mut best: std::collections::BTreeMap<(u8, u8), Vec<&BonusDef>> = std::collections::BTreeMap::new();
        for s in self.ships.iter().filter(|s| s.alive && !s.retreated) {
            for b in &cat.ship(s.def).bonuses {
                best.entry((s.side, b.stacking)).or_default().push(b);
            }
        }
        for s in self.ships.iter_mut() {
            let class = cat.ship(s.def).class;
            let (mut dmg, mut health, mut shield, mut defense, mut range, mut speed) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for ((side, _), list) in &best {
                if *side != s.side {
                    continue;
                }
                let hit = list.iter().filter(|b| b.classes == 0 || b.classes & class.bit() != 0);
                let max = |f: fn(&BonusDef) -> f32| hit.clone().map(|b| f(b)).fold(0.0f32, |m, v| if v.abs() > m.abs() { v } else { m });
                dmg += max(|b| b.damage);
                health += max(|b| b.health);
                shield += max(|b| b.shield);
                defense += max(|b| b.defense);
                range += max(|b| b.range);
                speed += max(|b| b.speed);
            }
            s.bonus = Bonus { damage: 1.0 + dmg, taken_hull: 1.0 / ((1.0 + health) * (1.0 + defense)), taken_shield: 1.0 / ((1.0 + shield) * (1.0 + defense)), range: 1.0 + range, speed: 1.0 + speed };
        }
    }

    /// `Force_Healing_Ability` pulses: self repair and repair auras.
    fn run_healing(&mut self, dt: f32) {
        let cat = self.catalog.clone();
        let mut pulses: Vec<(usize, usize)> = Vec::new();
        for (i, s) in self.ships.iter_mut().enumerate() {
            if !s.alive || s.retreated {
                continue;
            }
            for (k, h) in cat.ship(s.def).heals.iter().take(2).enumerate() {
                s.heal_in[k] -= dt;
                if s.heal_in[k] <= 0.0 {
                    s.heal_in[k] = h.interval;
                    pulses.push((i, k));
                }
            }
        }
        for (i, k) in pulses {
            let h = &cat.ship(self.ships[i].def).heals[k];
            let (pos, side) = (self.ships[i].pos, self.ships[i].side);
            let mut targets: Vec<usize> = if h.range <= 0.0 {
                vec![i]
            } else {
                (0..self.ships.len())
                    .filter(|&j| {
                        let o = &self.ships[j];
                        let od = cat.ship(o.def);
                        o.alive && !o.retreated && o.side == side && o.pos.dist(pos) <= h.range && (h.classes == 0 || h.classes & od.class.bit() != 0)
                    })
                    .collect()
            };
            if h.single {
                let hurt = |j: &usize| {
                    let o = &self.ships[*j];
                    o.hull / cat.ship(o.def).hull.max(1.0)
                };
                targets.sort_by(|a, b| hurt(a).total_cmp(&hurt(b)));
                targets.truncate(1);
            }
            for j in targets {
                let max = cat.ship(self.ships[j].def).hull;
                let o = &mut self.ships[j];
                if max > 0.0 && o.hull < max {
                    o.hull = (o.hull + h.amount + h.percent * max).min(max);
                }
            }
        }
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
                // Initial staged launch from hangar:
                if self.ships[c].slots[k].members.is_empty() {
                    self.ships[c].slots[k].respawn_in -= dt;
                    if self.ships[c].slots[k].respawn_in <= 0.0 && bay_ok {
                        let sq = self.ships[c].slots[k].def;
                        let members = self.launch_squadron(c as u32, sq);
                        self.ships[c].slots[k].members = members;
                        self.ships[c].slots[k].respawn_in = -1.0;
                    }
                    continue;
                }
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
        // A side is in the fight while anything it brought into the battle lives: the ships, squadrons, star base and structures of the setup (and
        // called-in reinforcements). Units spawned during the battle (a station's garrison, squadrons a carrier launches from its reserve) do not count: EaW
        // ends the battle when everything the sides brought is destroyed or has retreated, and the spawned units are removed with the loser.
        // A side without any setup unit (hand-made scenarios) is in the fight while any of its ships lives.
        let mut had_brought = vec![false; n];
        let mut brought_alive = vec![false; n];
        let mut any_alive = vec![false; n];
        for s in &self.ships {
            let i = s.side as usize;
            let vic = self.catalog.ship(s.def).special_weapon.as_ref().map_or(true, |sw| sw.victory_relevant);
            if s.group != NONE && vic {
                had_brought[i] = true;
            }
            if s.alive && !s.retreated {
                any_alive[i] = true;
                if vic {
                    brought_alive[i] |= s.group != NONE;
                }
            }
        }
        let mut active: Vec<bool> = (0..n).map(|i| if had_brought[i] { brought_alive[i] } else { any_alive[i] }).collect();
        for r in &self.pending {
            active[r.side as usize] = true;
        }
        let left: Vec<usize> = (0..n).filter(|&i| active[i]).collect();
        if left.len() <= 1 {
            self.over = true;
            self.winner = if let Some(&w) = left.first() {
                Some(w as u8)
            } else {
                let any_survivors: Vec<usize> = (0..n).filter(|&i| any_alive[i]).collect();
                if any_survivors.len() == 1 {
                    Some(any_survivors[0] as u8)
                } else {
                    None
                }
            };
            // The defeated side's remaining fighters are lost with it (ground structures survive on the planet).
            if let Some(w) = self.winner {
                for s in self.ships.iter_mut().filter(|s| s.side != w && !s.retreated) {
                    if self.catalog.ship(s.def).special_weapon.as_ref().map_or(true, |sw| sw.victory_relevant) {
                        s.alive = false;
                    }
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
    let sees = |t: &Snap| s.side >= 4 || t.seen_by & (1 << s.side) != 0;
    let target_ok = s.target != NONE && snaps[s.target as usize].alive && sees(&snaps[s.target as usize]);
    if s.scan_in > 0.0 && (target_ok || s.target == NONE) {
        return;
    }
    s.scan_in = cfg.retarget_interval;
    let def = cat.ship(s.def);
    let range = match (s.stance, s.order) {
        (Stance::Aggressive, Order::AttackMove(_) | Order::Idle) => f32::MAX,
        (Stance::Aggressive, Order::Guard(_)) => 5000.0,
        (Stance::Defensive, Order::Guard(_)) => 4000.0,
        _ => (def.max_weapon_range.max(def.targeting_range) * 1.25).max(400.0),
    };
    let me = snaps[s.id as usize];
    // Priority table: AI attack-move orders use the `<set>_Attack_Move` variant.
    let set = if cfg.attack_move_sets && matches!(s.order, Order::AttackMove(_)) { def.priority_attack_move } else { def.priority }
        .and_then(|i| cat.priority_sets.get(i as usize));
    let mut found: Vec<(f32, f32, u32)> = Vec::with_capacity(32);
    for (i, t) in snaps.iter().enumerate() {
        if !t.alive || t.side == s.side || t.special || !sees(t) {
            continue;
        }
        let d = t.pos.dist(me.pos) - t.radius;
        if d > range {
            continue;
        }
        let mut weight = match set {
            Some(p) => {
                if p.excluded & t.class.bit() != 0 && explicit != Some(i as u32) {
                    continue;
                }
                p.weight(t.class, t.golan).min(1000.0) + 1.0
            }
            None => f32::from(target_priority(me.class, t.class)) * 10.0 + 1.0,
        };
        // Intercept: fighters near their carrier go for enemy bombers that come within 3500 units of it (estimate; the priority sets already rank bombers).
        if me.class == TargetClass::Fighter && t.class == TargetClass::Bomber && s.carrier != NONE && snaps[s.carrier as usize].alive && snaps[s.carrier as usize].pos.dist(t.pos) < 3500.0 {
            weight *= 0.3;
        }
        // U93: Flanking corvettes and gunships prioritize rear-echelon units: carriers and support vessels.
        if def.is_flanker() {
            let tdef = cat.ship(t.def);
            if tdef.is_carrier() || tdef.is_support() {
                weight *= 0.25;
            }
        }
        // Finish off weak ships in reach first (AotR_Focus_Weak_Unit: health below 0.33; the factor 0.5 on the weight is an estimate).
        if t.health < 0.33 && !matches!(t.class, TargetClass::Fighter | TargetClass::Bomber) && d <= def.max_weapon_range * 1.1 {
            weight *= 0.5;
        }
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
    if def.class == TargetClass::Bomber && s.target != NONE && explicit.is_none() {
        let tid = s.target as usize;
        let t = snaps[tid];
        if (t.class as u8 >= TargetClass::Frigate as u8 || cat.ship(t.def).is_static) && t.hp_alive.any() {
            if s.target_hp == NO_HP || !t.hp_alive.test(s.target_hp as usize) {
                let tdef = cat.ship(t.def);
                let hp_pri = |k: HpKind| match k {
                    HpKind::ShieldGenerator => 0,
                    HpKind::Engine => 1,
                    HpKind::FighterBay => 2,
                    HpKind::Weapon => 3,
                    _ => 4,
                };
                let best_hp = tdef.hardpoints.iter().enumerate()
                    .filter(|(idx, _)| t.hp_alive.test(*idx))
                    .min_by_key(|(_, h)| (hp_pri(h.kind), (h.offset.len() * 10.0) as u32));
                if let Some((idx, _)) = best_hp {
                    s.target_hp = idx as u16;
                }
            }
        }
    }
}

fn turn_towards(s: &mut Ship, desired: f32, rate: f32, dt: f32) -> f32 {
    let diff = wrap_angle(desired - s.yaw);
    if diff.abs() < 0.003 {
        return 0.0;
    }
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
/// What a big ship navigates around.
struct Nav<'a> {
    /// Distance field of rocks and stations a path goes around.
    grid: Option<&'a crate::nav::NavGrid>,
    /// All big ships and stations (ids), for local steering and for keeping fighters out of hulls.
    hulls: &'a [u32],
}

#[allow(clippy::too_many_arguments)]
fn mov(s: &mut Ship, snaps: &[Snap], cat: &Catalog, cfg: &BattleConfig, spawn: &[Vec3], leaders: &[u32], obstacles: &[crate::arena::Obstacle], big: &[u32], nav: &Nav, rng: &mut Rng) {
    let def = cat.ship(s.def);
    if def.is_static {
        return;
    }
    let dt = cfg.dt;
    // A fighter that just left its hangar flies straight out for `Fighter_Bay_Flyout_Distance` before it follows orders.
    if s.flyout_left > 0.0 {
        let step = (def.max_speed * 0.5).max(10.0) * dt;
        s.pos = s.pos + Vec3::from_yaw(s.yaw) * step;
        s.flyout_left -= step;
        s.speed = step / dt;
        return;
    }
    let speed_mult = ability_product(def, s, |a| a.speed_mult) * (1.0 - s.stun_speed_cut) * (1.0 - s.tractor_cut) * s.engine_factor;
    let pinned = def.abilities.iter().zip(&s.abilities).any(|(a, st)| a.disable_movement && st.active > 0.0);
    let max_speed = if pinned { 0.0 } else { def.max_speed * speed_mult * s.bonus.speed * (1.0 + s.mods[ModKind::Speed as usize].0) };
    let fighter = matches!(def.class, TargetClass::Fighter | TargetClass::Bomber);
    let small = |c: TargetClass| matches!(c, TargetClass::Fighter | TargetClass::Bomber);
    // Big ships position themselves against the best non-fighter target: fast fighters cannot be chased or tracked, the guns just fire at them
    // while the ship keeps station (unless it was explicitly ordered to attack one).
    let mover = if fighter || matches!(s.order, Order::Attack(_)) || s.target == NONE || !small(snaps[s.target as usize].class) {
        s.target
    } else {
        s.candidates.iter().copied().find(|&c| !small(snaps[c as usize].class)).unwrap_or(NONE)
    };
    let target = (mover != NONE && snaps[mover as usize].alive).then(|| snaps[mover as usize]);
    let aggressive = s.stance == Stance::Aggressive;

    // (point, stop radius, face-target-when-arrived)
    enum Goal {
        Free(f32),
        Point(Vec3, f32),
        Hold,
    }
    let mut face: Option<Vec3> = None;
    // Members of a squadron fly in formation: the slot is the leader's position plus the data offset (`Squadron_Offsets`) turned with the leader;
    // the leader's speed is the speed to match.
    let slot: Option<(Vec3, f32)> = if s.squad != NONE {
        let l = leaders[s.squad as usize];
        (l != NONE && l != s.id && snaps[l as usize].alive).then(|| (snaps[l as usize].pos + s.formation.rotate_yaw(snaps[l as usize].yaw), snaps[l as usize].vel.len()))
    } else {
        None
    };
    if def.class == TargetClass::Bomber && s.squad != NONE {
        let l = leaders[s.squad as usize];
        if l != NONE && l != s.id && snaps[l as usize].alive {
            let l_hp = snaps[l as usize].target_hp;
            if l_hp != NO_HP && (s.target_hp == NO_HP || target.is_some_and(|t| !t.hp_alive.test(s.target_hp as usize))) {
                s.target_hp = l_hp;
            }
        }
    }
    // A fighter that only holds a place (formation slot, carrier escort) stops there instead of flying past it for ever.
    let mut settle = false;
    let mut follow_speed: Option<f32> = None;
    let goal = match s.order {
        Order::Retreat => {
            // Turn round and head for the own spawn side at full throttle past the boundary.
            let spawn_pt = spawn.get(s.side as usize).copied().unwrap_or(Vec3::ZERO);
            let dir = if spawn_pt.len() > 10.0 { spawn_pt.normalized() } else { Vec3::new(if s.side == 0 { -1.0 } else { 1.0 }, 0.0, 0.0) };
            Goal::Point(spawn_pt + dir * 10000.0, 50.0)
        }
        Order::Move(p) => {
            if let Some((q, v)) = slot {
                settle = true;
                follow_speed = Some(v);
                Goal::Point(q, 50.0)
            } else {
                settle = true;
                Goal::Point(p, def.radius.max(50.0))
            }
        }
        Order::Attack(_) | Order::AttackMove(_) if target.is_some() && (aggressive || matches!(s.order, Order::Attack(_))) => {
            let t = target.expect("checked");
            let tdef = cat.ship(t.def);
            let hp_aim = if s.target_hp != NO_HP && t.hp_alive.test(s.target_hp as usize) {
                tdef.hardpoints.get(s.target_hp as usize).map(|h| t.pos + h.offset.rotate_yaw(t.yaw))
            } else {
                None
            };
            let (tgt_point, radius_slack) = match hp_aim {
                Some(p) => (p, 0.0),
                None => (t.pos, t.radius * 0.5),
            };
            let d = tgt_point.dist(s.pos);
            if fighter {
                let is_dogfight = matches!(t.class, TargetClass::Fighter | TargetClass::Bomber) && d < 650.0;
                let close_run = d < (70.0 + radius_slack).max(180.0);
                if s.phase > 0.0 {
                    Goal::Free(s.yaw + s.phase_turn * dt)
                } else if close_run {
                    if def.class == TargetClass::Bomber && (t.class as u8 >= TargetClass::Frigate as u8) {
                        // Bombing run: fly through, then straight away for a few seconds before the next pass (estimate: 3-5 s, almost no turn).
                        s.phase = rng.range(3.0, 5.0);
                        s.phase_turn = rng.range(-0.15, 0.15);
                    } else {
                        s.phase = rng.range(1.0, 2.5);
                        s.phase_turn = rng.range(-0.8, 0.8);
                    }
                    Goal::Free(s.yaw)
                } else if let Some((q, v)) = slot && !is_dogfight {
                    // Members hold their formation slot while approaching or escorting
                    settle = false;
                    follow_speed = Some(v);
                    Goal::Point(q, 30.0)
                } else {
                    Goal::Point(tgt_point, 0.0)
                }
            } else {
                let stand = if def.preferred_range > 0.0 {
                    (def.preferred_range * cfg.engage_range_scale + radius_slack).max(def.radius + t.radius + 50.0)
                } else {
                    0.0
                };
                // Hysteresis: a ship that has stopped only starts closing again when the target is clearly out of its standoff.
                let resume = if s.speed < 0.05 * def.max_speed.max(1.0) { 1.12 } else { 1.05 };
                if d > stand * resume {
                    // Line abreast while the enemy is still far away (attack-move only).
                    let mut q = tgt_point;
                    let is_flanker = def.is_flanker();
                    if is_flanker && d > (stand * 1.5).max(1800.0) {
                        // U93: Fast corvettes and gunships execute wide flanking maneuvers around the enemy line.
                        let flank_sign = if s.id % 2 == 0 { 1.0 } else { -1.0 };
                        let to = tgt_point - s.pos;
                        let perp = Vec3::new(-to.y, to.x, 0.0).normalized() * flank_sign;
                        let flank_dist = (3200.0 + ((s.id % 3) as f32 * 600.0)).min(d * 0.7);
                        q = tgt_point + perp * flank_dist - to.normalized() * (stand * 0.5);
                    } else if s.lane != 0.0 && matches!(s.order, Order::AttackMove(_)) && d > 2500.0 {
                        let to = tgt_point - s.pos;
                        q = tgt_point + Vec3::new(-to.y, to.x, 0.0) * (s.lane / d);
                    } else if d < stand * 2.2 && def.preferred_range > 0.0 {
                        // Broadside maneuvering: if guns favour broadside angles, curve approach tangentially
                        let flank_sign = if s.lane != 0.0 { s.lane.signum() } else if s.id % 2 == 0 { 1.0 } else { -1.0 };
                        let to = tgt_point - s.pos;
                        let tangent = Vec3::new(-to.y, to.x, 0.0).normalized() * flank_sign;
                        let blend = ((stand * 2.2 - d) / (stand * 1.2)).clamp(0.0, 1.0);
                        q = tgt_point + tangent * (stand * 0.45 * blend);
                    }
                    face = Some(tgt_point);
                    Goal::Point(q, stand)
                } else {
                    face = Some(tgt_point);
                    Goal::Hold
                }
            }
        }
        Order::AttackMove(p) => {
            if let Some((q, v)) = slot {
                settle = true;
                follow_speed = Some(v);
                Goal::Point(q, 50.0)
            } else {
                settle = true;
                // Advance line abreast or along wide flanking vector:
                let mut q = p;
                let to = p - s.pos;
                let dist = to.len();
                if def.is_flanker() && dist > 2000.0 {
                    // U93: Flankers advance along wide flanking vectors
                    let flank_sign = if s.id % 2 == 0 { 1.0 } else { -1.0 };
                    let perp = Vec3::new(-to.y, to.x, 0.0).normalized() * flank_sign;
                    let flank_width = 3500.0 + ((s.id % 4) as f32 * 500.0);
                    q = p + perp * flank_width;
                } else if s.lane != 0.0 && !fighter && dist > 2500.0 {
                    q = p + Vec3::new(-to.y, to.x, 0.0) * (s.lane / dist);
                }
                Goal::Point(q, def.radius.max(100.0))
            }
        }
        Order::Guard(p) => {
            if let Some(t) = target {
                let dist = t.pos.dist(s.pos);
                if dist < (def.max_weapon_range * 1.3).max(2500.0) || t.pos.dist(p) < 4500.0 {
                    face = Some(t.pos);
                    let stand = if def.preferred_range > 0.0 {
                        (def.preferred_range * cfg.engage_range_scale).max(def.radius + t.radius + 50.0)
                    } else {
                        (def.max_weapon_range * 0.8).max(def.radius + t.radius + 50.0)
                    };
                    if dist > stand * 1.1 {
                        Goal::Point(t.pos, stand)
                    } else {
                        Goal::Hold
                    }
                } else {
                    Goal::Point(p, (def.radius + 300.0).max(200.0))
                }
            } else {
                Goal::Point(p, (def.radius + 300.0).max(200.0))
            }
        }
        Order::Idle | Order::Attack(_) => {
            // Fighters stay with their squadron leader or carrier.
            if fighter {
                settle = true;
                if let Some((q, v)) = slot {
                    follow_speed = Some(v);
                    Goal::Point(q, 50.0)
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

    // Cohesion: while advancing without a target a big ship does not outrun the slower ones around it (within 2500 units).
    let max_speed = if target.is_none_or(|t| t.pos.dist(s.pos) > (def.max_weapon_range * 2.0).max(1500.0)) && matches!(s.order, Order::AttackMove(_)) && !fighter {
        let me = snaps[s.id as usize];
        big.iter()
            .filter(|&&j| j != s.id && snaps[j as usize].side == s.side && snaps[j as usize].pos.dist(me.pos) < 2500.0)
            .map(|&j| cat.ship(snaps[j as usize].def).max_speed)
            .fold(max_speed, f32::min)
            .max(max_speed * 0.4)
    } else {
        max_speed
    };
    if !matches!(goal, Goal::Point(..)) {
        s.path.clear();
    }
    match goal {
        Goal::Free(yaw) => {
            let rate = def.turn_rate;
            turn_towards(s, yaw, rate, dt);
            let want = if def.class == TargetClass::Bomber && s.phase > 0.0 {
                max_speed * 1.35
            } else {
                max_speed
            };
            s.speed += (want - s.speed).clamp(-def.accel * dt * 2.0, def.accel * dt * 1.5);
            if s.phase > 0.0 {
                s.phase -= dt;
            }
        }
        Goal::Point(p, stop) => {
            // A place inside a rock is approached to the rock's edge.
            let p = match nav.grid {
                Some(g) if !fighter && target.is_none_or(|t| t.pos.dist(p) > 1.0) => g.free_point(p, def.radius * 0.75 + 40.0),
                _ => p,
            };
            let to = p - s.pos;
            let dist = to.len();
            // Big ships follow a planned path around rocks and stations and steer around the hulls ahead of them.
            let mut aim = p;
            let mut slow = 1.0;
            let mut avoid = (0.0f32, 0.0f32);
            if !fighter {
                s.path_age += dt;
                if s.path_age >= PATH_REPLAN_SECS || s.path_goal.dist(p) > 300.0 || s.path.is_empty() {
                    s.path = nav.grid.map_or_else(|| vec![p], |g| g.plan(s.pos, p, def.radius * 0.75 + 40.0));
                    s.path_goal = p;
                    s.path_age = 0.0;
                } else if let Some(last) = s.path.last_mut() {
                    *last = p;
                }
                while s.path.len() > 1 && s.path[0].dist(s.pos) < def.radius.max(120.0) {
                    s.path.remove(0);
                }
                if s.path.len() > 1 {
                    aim = s.path[0];
                }
                let heading = (s.yaw.cos(), s.yaw.sin());
                let lookahead = (s.speed * 4.0).max(def.radius * 2.0);
                let hulls = nav.hulls.iter().filter(|&&j| j != s.id && j != s.target && j != s.carrier && snaps[j as usize].carrier != s.id).map(|&j| crate::nav::Blocker { pos: snaps[j as usize].pos, radius: snaps[j as usize].radius });
                let st = if cfg.navigation {
                    crate::nav::steer_around(s.pos, heading, snaps[s.id as usize].radius, lookahead, hulls)
                } else {
                    crate::nav::Steer { avoid: (0.0, 0.0), slow: 1.0 }
                };
                avoid = st.avoid;
                slow = st.slow;
                // Escorts calculate a repulsion vector around capital ships to glide around them (U65).
                if matches!(def.class, TargetClass::Corvette | TargetClass::Frigate) {
                    for &j in nav.hulls {
                        if j == s.id || j == s.target || j == s.carrier {
                            continue;
                        }
                        let oj = &snaps[j as usize];
                        if !oj.alive || !matches!(oj.class, TargetClass::Capital | TargetClass::Super | TargetClass::Structure) {
                            continue;
                        }
                        let cap_safe = oj.radius + def.radius + 150.0;
                        let rx = s.pos.x - oj.pos.x;
                        let ry = s.pos.y - oj.pos.y;
                        let dist2 = rx * rx + ry * ry;
                        if dist2 < cap_safe * cap_safe && dist2 > 1e-4 {
                            let d = dist2.sqrt();
                            let factor = ((cap_safe - d) / cap_safe).clamp(0.0, 1.0);
                            avoid.0 += ((rx / d) * factor * 2.0).clamp(-1.5, 1.5);
                            avoid.1 += ((ry / d) * factor * 2.0).clamp(-1.5, 1.5);
                            if d < oj.radius + def.radius + 50.0 {
                                slow = slow.min(0.5 + 0.5 * (d / (oj.radius + def.radius + 50.0)));
                            }
                        }
                    }
                }
            } else {
                s.path.clear();
            }
            let to_aim = aim - s.pos;
            let unit = to_aim.normalized();
            let desired = if fighter && follow_speed.is_some_and(|v| v > 15.0) && s.squad != NONE {
                let l = leaders[s.squad as usize];
                if l != NONE && snaps[l as usize].alive {
                    let lyaw = snaps[l as usize].yaw;
                    let ldir = Vec3::new(lyaw.cos(), lyaw.sin(), 0.0);
                    let to_slot = aim - s.pos;
                    let dist_slot = to_slot.len();
                    // Smooth slot attraction: smoothly align with leader heading when close,
                    // without flip-flopping unit vectors near the slot.
                    let slot_corr = if dist_slot > 1.0 {
                        let w = (dist_slot / 60.0).clamp(0.0, 1.5);
                        to_slot * (w / dist_slot)
                    } else {
                        Vec3::ZERO
                    };
                    let blend_dir = ldir + slot_corr;
                    blend_dir.y.atan2(blend_dir.x)
                } else {
                    (unit.y + avoid.1).atan2(unit.x + avoid.0)
                }
            } else {
                (unit.y + avoid.1).atan2(unit.x + avoid.0)
            };
            let braking = s.speed * s.speed / (2.0 * def.accel.max(1.0));
            // Closing on a target (capitals and frigates): in the last stretch before the standoff the ship already turns the bearing at which most weapons bear
            // towards it (the speed profile below brakes it to a stop at the standoff), instead of pointing its bow at it until it has stopped.
            let err = match face {
                Some(f) if !fighter && aim == p && dist <= stop + (braking * 2.5 + def.radius * 1.5).max(400.0) => {
                    align_to_target(s, def, f, dt);
                    0.0
                }
                _ => turn_towards(s, desired, def.turn_rate, dt),
            };
            let mut want = max_speed * slow;
            if err > 1.2 {
                want *= 0.35;
            } else if err > 0.5 {
                want *= 0.7;
            }
            if let Some(v) = follow_speed {
                let target_speed = if v > 10.0 {
                    let fwd = if s.squad != NONE && leaders[s.squad as usize] != NONE && snaps[leaders[s.squad as usize] as usize].alive {
                        let lyaw = snaps[leaders[s.squad as usize] as usize].yaw;
                        let ldir = Vec3::new(lyaw.cos(), lyaw.sin(), 0.0);
                        to_aim.x * ldir.x + to_aim.y * ldir.y
                    } else {
                        dist
                    };
                    if fwd < -5.0 {
                        (v + fwd * 1.5).max(v * 0.3)
                    } else {
                        v + dist * 2.0
                    }
                } else {
                    dist * 2.0
                };
                want = want.min(target_speed);
            }
            if fighter {
                if dist <= stop + braking * 0.8 && settle && aim == p {
                    if follow_speed.is_some_and(|v| v > 10.0) {
                        want = follow_speed.unwrap();
                    } else {
                        want = 0.0;
                    }
                }
            } else if aim == p {
                // Arrival profile of a big ship: the speed that still stops exactly at the stop radius with the braking rate used below (no overshoot into the target).
                let room = (dist - stop).max(0.0);
                want = want.min((2.0 * def.accel.max(1.0) * 1.5 * 0.9 * room).sqrt());
                if room <= 1.0 {
                    want = 0.0;
                }
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
    let top_clamp = if def.class == TargetClass::Bomber && s.phase > 0.0 { max_speed * 1.35 } else { max_speed };
    s.speed = s.speed.clamp(0.0, top_clamp.max(0.0));
    let fwd = Vec3::from_yaw(s.yaw);
    s.pos += fwd * (s.speed * dt);
    // Flight is planar (yaw only), so height differences from launch bays and spawn points would never close and ships could circle a target
    // out of weapon range for ever: mobile ships climb or sink towards their target's height (or the battle plane) at half their top speed.
    if !def.is_static {
        if def.class == TargetClass::Bomber && s.phase > 0.0 {
            let z_dir = if s.id % 2 == 0 { 1.0 } else { -1.0 };
            s.pos.z += z_dir * (def.max_speed * 0.7 * dt);
        } else {
            let z_goal = target.map_or(0.0, |t| t.pos.z);
            let step = def.max_speed * 0.5 * dt;
            s.pos.z += (z_goal - s.pos.z).clamp(-step, step);
        }
    }
    // Separation: mobile big ships do not sit on top of each other (each pair is resolved by both, half each; estimate 0.85 of the radii).
    if !fighter {
        let me = snaps[s.id as usize];
        let cap = def.max_speed.max(60.0) * dt;
        for &j in nav.hulls {
            if j == s.id {
                continue;
            }
            let o = &snaps[j as usize];
            if j == s.carrier || o.carrier == s.id {
                continue;
            }
            let is_escort = matches!(def.class, TargetClass::Corvette | TargetClass::Frigate);
            let is_cap = matches!(o.class, TargetClass::Capital | TargetClass::Super | TargetClass::Structure);
            let am_cap = matches!(def.class, TargetClass::Capital | TargetClass::Super | TargetClass::Structure);
            let o_escort = matches!(o.class, TargetClass::Corvette | TargetClass::Frigate);
            if am_cap && o_escort {
                // Capital ships hold course; escorts yield to them
                continue;
            }
            let fixed = cat.ship(o.def).is_static;
            let min = if is_escort && is_cap {
                (me.radius + o.radius + 150.0).max((me.radius + o.radius) * 0.85)
            } else {
                (me.radius + o.radius) * 0.85
            };
            let (dx, dy) = (s.pos.x - o.pos.x, s.pos.y - o.pos.y);
            if dx.abs() < min && dy.abs() < min {
                let d = (dx * dx + dy * dy).sqrt();
                if d < min {
                    let (ux, uy) = if d > 1e-3 { (dx / d, dy / d) } else { (1.0 - 2.0 * f32::from(s.id as u8 & 1), 0.0) };
                    let push = ((min - d) * if fixed || (is_escort && is_cap) { 1.0 } else { 0.5 }).min(cap);
                    s.pos.x += ux * push;
                    s.pos.y += uy * push;
                }
            }
        }
    }
    // Fighters fly almost anywhere but not through a hull (their own carrier excepted, they dock there).
    if fighter && cfg.navigation {
        for &j in nav.hulls {
            if j == s.carrier {
                continue;
            }
            let o = &snaps[j as usize];
            let min = o.radius * 0.45 + def.radius;
            let (dx, dy) = (s.pos.x - o.pos.x, s.pos.y - o.pos.y);
            if dx.abs() < min && dy.abs() < min {
                let d = (dx * dx + dy * dy).sqrt();
                if d < min {
                    let (ux, uy) = if d > 1e-3 { (dx / d, dy / d) } else { (1.0, 0.0) };
                    s.pos.x += ux * (min - d);
                    s.pos.y += uy * (min - d);
                }
            }
        }
    }
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

/// Checks whether a hardpoint at `muzzle` could shoot at target `t`, optionally aiming at a specific point `aim`.
#[allow(clippy::too_many_arguments)]
fn can_fire(hp: &HpDef, ship_yaw: f32, range_mult: f32, muzzle: Vec3, t: &Snap, proj: &ProjDef, aim: Option<Vec3>) -> bool {
    let target_pos = aim.unwrap_or(t.pos);
    let to = target_pos - muzzle;
    let dist = to.len();
    let reach = if aim.is_some() { dist } else { (dist - t.radius * 0.6).max(0.0) };
    if reach > hp.range * range_mult || reach < hp.min_range {
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
    let slack = if aim.is_some() { 0.05 } else { (t.radius / dist.max(1.0)).atan() };
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
        for (i, h) in tdef.hardpoints.iter().enumerate().take(HP_MASK_BITS) {
            if h.kind != *kind || !h.targetable || !h.destroyable || !target.hp_alive.test(i) {
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

/// Launch direction for homing missiles and torpedoes: launches with an upward loft and slight lateral spread
/// so they arc in curved 3D trajectories toward their target (Remake / EaW hallmark, U25/U33).
fn missile_launch_dir(base_dir: Vec3, dist: f32, rng: &mut Rng) -> Vec3 {
    let dist_factor = (dist / 1500.0).clamp(0.2, 1.0);
    let loft = 0.45 * dist_factor;
    let mut perp = Vec3::new(-base_dir.y, base_dir.x, 0.0).normalized();
    if perp.len_sq() < 0.5 {
        perp = Vec3::new(0.0, 1.0, 0.0);
    }
    let side = (rng.unit() * 0.4 - 0.2) * dist_factor;
    (base_dir + Vec3::new(0.0, 0.0, loft) + perp * side).normalized()
}

fn weapons(s: &mut Ship, snaps: &[Snap], cat: &Catalog, cfg: &BattleConfig, rng: &mut Rng, out: &mut Vec<FireReq>) {
    let dt = cfg.dt;
    let def = cat.ship(s.def);
    let explicit_attack = matches!(s.order, Order::Attack(_));
    let may_start = s.stance != Stance::HoldFire || explicit_attack;
    let rate_cut = if s.stun > 0.0 { 1.0 / (1.0 - s.stun_rate_cut).max(0.1) } else { 1.0 };
    let mut delay_mult = ability_product(def, s, |a| a.weapon_delay_mult) * rate_cut / (1.0 + s.mods[ModKind::FireRate as usize].0);
    // Hardpoint priorities of the ship's targeting set (AI attack-move sets name shield generators,
    // engines, hangars ... in order).
    let hp_set = if cfg.attack_move_sets && matches!(s.order, Order::AttackMove(_)) { def.priority_attack_move } else { def.priority }
        .and_then(|i| cat.priority_sets.get(i as usize))
        .filter(|p| !p.hardpoints.is_empty());
    let rocket_active = def.abilities.iter().zip(&s.abilities).any(|(a, st)| a.rocket && st.active > 0.0);
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
                let tdef = cat.ship(t.def);
                let targeted_hp = if s.target_hp != NO_HP && tgt == s.target && snaps[tgt as usize].hp_alive.test(s.target_hp as usize) {
                    s.target_hp
                } else if let Some(set) = hp_set {
                    auto_hardpoint(set, tdef, t, muzzle)
                } else {
                    NO_HP
                };
                let specific_aim = if targeted_hp != NO_HP && let Some(h) = tdef.hardpoints.get(targeted_hp as usize) {
                    Some(t.pos + h.offset.rotate_yaw(t.yaw))
                } else {
                    None
                };
                let (allowed, aim_hp) = if can_fire(hp, s.yaw, s.bonus.range, muzzle, t, pd, specific_aim) {
                    (true, targeted_hp)
                } else if can_fire(hp, s.yaw, s.bonus.range, muzzle, t, pd, None) {
                    (true, NO_HP)
                } else {
                    (false, NO_HP)
                };
                if allowed {
                    let mut aim = lead(muzzle, t, pd.speed);
                    if aim_hp != NO_HP
                        && let Some(h) = tdef.hardpoints.get(aim_hp as usize)
                    {
                        // Aim at the hardpoint, but keep the aim point inside the hit sphere.
                        let mut off = h.offset.rotate_yaw(t.yaw);
                        let max = t.radius * 0.85;
                        if off.len() > max {
                            off = off * (max / off.len());
                        }
                        aim += off;
                    } else {
                        aim += rng.in_ball() * (t.radius * 0.45);
                    }
                    aim += rng.in_ball() * hp.inaccuracy[t.class as usize];
                    let base_dir = (aim - muzzle).normalized();
                    let dir = if pd.turn_rate > 0.0 && tgt != NONE && matches!(pd.category.as_str(), "missile" | "torpedo" | "rocket") {
                        missile_launch_dir(base_dir, (aim - muzzle).len(), rng)
                    } else {
                        base_dir
                    };
                    out.push(FireReq { ship: s.id, hp: i as u16, proj: pid, from: muzzle, vel: dir * pd.speed, target: tgt, aim_hp });
                }
                st.pulses_left -= 1;
                st.pulse_timer = if salvo_active { hp.pulse_delay * hp.full_salvo_mult } else { hp.pulse_delay };
            }
            if st.pulses_left == 0 {
                let mut m = delay_mult;
                if salvo_active {
                    m *= hp.full_salvo_mult;
                }
                if power_active {
                    m *= hp.power_weapons_mult;
                }
                if rocket_active && matches!(pd.category.as_str(), "missile" | "rocket" | "torpedo") {
                    m *= ROCKET_RELOAD_MULT;
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
            let can_shoot = if s.target_hp != NO_HP && c == s.target && t.hp_alive.test(s.target_hp as usize) {
                let tdef = cat.ship(t.def);
                let p = tdef.hardpoints.get(s.target_hp as usize).map(|h| t.pos + h.offset.rotate_yaw(t.yaw));
                can_fire(hp, s.yaw, s.bonus.range, muzzle, t, pd, p) || can_fire(hp, s.yaw, s.bonus.range, muzzle, t, pd, None)
            } else {
                can_fire(hp, s.yaw, s.bonus.range, muzzle, t, pd, None)
            };
            if t.alive && can_shoot {
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
                        let aim = lead(muzzle, t, pd.speed);
                        let base_dir = (aim - muzzle).normalized();
                        let dir = if pd.turn_rate > 0.0 && tgt != NONE && matches!(pd.category.as_str(), "missile" | "torpedo" | "rocket") {
                            missile_launch_dir(base_dir, (aim - muzzle).len(), rng)
                        } else {
                            base_dir
                        };
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

    // Ground-to-space special weapon (Hypervelocity gun, Seismic cannon, Ion cannon).
    if let Some(sp) = &def.special_weapon {
        let pd = &cat.projectiles[sp.projectile as usize];
        if s.special_pulses > 0 {
            s.special_timer -= dt;
            if s.special_timer <= 0.0 {
                let tgt = s.special_target;
                if tgt != NONE && snaps[tgt as usize].alive {
                    let t = &snaps[tgt as usize];
                    let muzzle = s.pos;
                    let aim = lead(muzzle, t, pd.speed);
                    let dir = (aim - muzzle).normalized();
                    out.push(FireReq { ship: s.id, hp: NO_HP, proj: sp.projectile, from: muzzle, vel: dir * pd.speed, target: tgt, aim_hp: NO_HP });
                }
                s.special_pulses -= 1;
                if s.special_pulses == 0 {
                    s.special_cd = sp.recharge;
                } else {
                    s.special_timer = sp.pulse_delay;
                }
            }
        } else {
            s.special_cd -= dt;
            if s.special_cd <= 0.0 && may_start {
                let mut best_target = NONE;
                let mut best_score = -1.0f32;
                for (other_id, other) in snaps.iter().enumerate() {
                    if other.side != s.side && other.alive && !other.special {
                        if (sp.valid_targets & other.class.bit()) != 0 {
                            let class_weight = match other.class {
                                TargetClass::Super => 10_000_000.0,
                                TargetClass::Capital => 1_000_000.0,
                                TargetClass::Frigate => 100_000.0,
                                TargetClass::Corvette => 10_000.0,
                                _ => 1_000.0,
                            };
                            let score = class_weight + other.health * 1000.0;
                            if score > best_score {
                                best_score = score;
                                best_target = other_id as u32;
                            }
                        }
                    }
                }
                if best_target != NONE {
                    s.special_target = best_target;
                    s.special_pulses = sp.pulse_count.max(1);
                    let t = &snaps[best_target as usize];
                    let muzzle = s.pos;
                    let aim = lead(muzzle, t, pd.speed);
                    let dir = (aim - muzzle).normalized();
                    out.push(FireReq { ship: s.id, hp: NO_HP, proj: sp.projectile, from: muzzle, vel: dir * pd.speed, target: best_target, aim_hp: NO_HP });
                    s.special_pulses -= 1;
                    if s.special_pulses == 0 {
                        s.special_cd = sp.recharge;
                    } else {
                        s.special_timer = sp.pulse_delay;
                    }
                } else {
                    s.special_cd = 2.0;
                }
            }
        }
    }
}

/// Estimate: change of a rocket's heading (radians) over `step` units of flight after `flown` units. After `Projectile_Rocket_Straight_Distance`
/// the heading deviation follows one full sine of amplitude `Projectile_Rocket_Curve_Offset` over `Projectile_Rocket_Curve_Distance`, so the
/// rocket weaves in an S and ends on its original line (the data names the three values but not the curve).
fn rocket_weave_turn(pd: &ProjDef, flown: f32, step: f32) -> f32 {
    let dev = |s: f32| pd.rocket_weave * (2.0 * PI * (s / pd.rocket_curve).clamp(0.0, 1.0)).sin();
    let s0 = flown - pd.rocket_straight;
    dev(s0 + step) - dev(s0)
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
        if t.jammed {
            p.target = NONE;
        } else if t.alive {
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
    if pd.rocket_weave > 0.0 {
        let side = if (p.owner ^ p.target ^ p.def) & 1 == 0 { 1.0 } else { -1.0 };
        let flown = pd.max_distance - p.range_left;
        let turn = rocket_weave_turn(pd, flown, p.vel.len() * dt);
        if turn != 0.0 {
            p.vel = p.vel.rotate_yaw(side * turn);
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
            if d <= s.hit_radius && best.is_none_or(|(bt, _)| t < bt) {
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

/// What a `Redirect_Blaster_Ability` did to a shot.
enum Deflect {
    None,
    Blocked,
    Reflected,
}

/// Reload delay multiplier of missile weapons while `ROCKET_ATTACK` is active (estimate).
const ROCKET_RELOAD_MULT: f32 = 0.4;

/// `HUNT` target score (estimates): game units one step of priority weight is worth against distance, the cost of each other hunter already
/// on the enemy, the cost of a station or structure (hunters go for ships first), and the bonus of the enemy the hunter already chases.
const HUNT_WEIGHT_UNITS: f32 = 2500.0;
const HUNT_CROWD_UNITS: f32 = 1800.0;
const HUNT_STATIC_UNITS: f32 = 1.0e7;
const HUNT_STICKY_UNITS: f32 = 2000.0;
/// Seconds after which a hunter gives up on an unreachable search cell (estimate).
const HUNT_CELL_SECS: f32 = 60.0;

/// Seconds an AI interdictor keeps its field on (`Register_Timer(Cancel_Interdiction, 20)` in `ObjectScript_Interdict_Layer_Z.lua`).
const AI_INTERDICT_SECS: f32 = 20.0;

/// The `n`-th place an AI side searches for hidden enemies: their start, the middle, then the four quarters.
fn search_point(n: u32, enemy_start: Vec3, half: f32) -> Vec3 {
    match n % 6 {
        0 => enemy_start,
        1 => Vec3::ZERO,
        2 => Vec3::new(half * 0.5, half * 0.5, 0.0),
        3 => Vec3::new(-half * 0.5, half * 0.5, 0.0),
        4 => Vec3::new(-half * 0.5, -half * 0.5, 0.0),
        _ => Vec3::new(half * 0.5, -half * 0.5, 0.0),
    }
}

/// Seconds between replans of a moving big ship's path (also replanned when its destination moves).
const PATH_REPLAN_SECS: f32 = 0.5;

/// Squadron definitions with counts.
type Squadrons = Vec<(ShipId, u32)>;

#[cfg(test)]
mod rocket_tests {
    use super::*;

    #[test]
    fn rocket_weaves_between_straight_and_curve_distance_and_ends_on_its_line() {
        let pd = ProjDef { rocket_weave: 0.2, rocket_straight: 400.0, rocket_curve: 800.0, ..ProjDef::default() };
        assert_eq!(rocket_weave_turn(&pd, 0.0, 10.0), 0.0, "straight at first");
        assert!(rocket_weave_turn(&pd, 410.0, 10.0).abs() > 0.0, "weaves after the straight distance");
        assert_eq!(rocket_weave_turn(&pd, 1300.0, 10.0), 0.0, "straight again after the curve");
        // Summed over the whole curve the heading returns to where it was, and the sideways drift cancels.
        let (mut heading, mut lateral, mut flown) = (0.0f32, 0.0f32, 0.0f32);
        while flown < 1500.0 {
            heading += rocket_weave_turn(&pd, flown, 1.0);
            lateral += heading.sin();
            flown += 1.0;
        }
        assert!(heading.abs() < 1e-3, "heading back on the line: {heading}");
        assert!(lateral.abs() < 20.0, "net drift small: {lateral}");
    }
}
