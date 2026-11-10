//! Units that object scripts bring into a battle ([`SpawnScriptDef`], read from the mod's Lua by `scripts.rs`). A script runs when a unit
//! that has one enters the battle (hero escorts) or when a hardpoint whose breakoff prop has one is destroyed (Subjugator). Its roll picks
//! one of its lists, and the list's units arrive one by one at the script's markers: each `hyperspace_in` seconds after its spawn, with the
//! script's pause between two spawns. Rolls and pauses come from the battle seed, so a battle replays the same from its commands.
//!
//! Scripted units belong to the battle only: they get no results group (nothing is returned to the galaxy) and, like a station's garrison,
//! do not keep their side in the fight (docs/RULES.md 2.8).
//!
//! The same path places the helper objects of abilities ([`HelperDef`]: repair droids, missile jamming pods): they appear in place, hold
//! position and fire, and wear out by `Ship::decay` (docs/RULES.md 2.6).

use crate::battle::*;
use crate::defs::*;
use crate::math::Vec3;
use crate::rng::Rng;

/// Where a scripted unit arrives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum SpawnAt {
    /// At a unit, facing its way (its place when the new unit arrives).
    Unit(u32),
    /// At a map marker: place and heading.
    Place(Vec3, f32),
}

/// A unit an object script spawned that has not arrived yet.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScriptedArrival {
    /// Battle time of the arrival.
    pub at: f32,
    pub side: u8,
    pub ship: ShipId,
    pub place: SpawnAt,
    /// Drops out of hyperspace (else it appears where it is placed).
    pub hyperspace: bool,
    /// The unit whose script (or ability) spawned it.
    pub by: u32,
    /// Hit points per second it loses on top of its definition's `decay` (helper objects).
    pub decay: f32,
}

impl Battle {
    /// Runs the spawn scripts of the units that entered the battle since the last step (all units on the first step).
    pub(crate) fn start_scripts(&mut self) {
        while self.scripts_seen < self.ships.len() {
            let id = self.scripts_seen as u32;
            self.scripts_seen += 1;
            if let Some(script) = self.catalog.ship(self.ships[id as usize].def).spawn_script.clone() {
                self.run_spawn_script(id, &script, 0);
            }
        }
    }

    /// `State_Init` of a spawn script owned by unit `owner`: rolls the list and schedules its units. `salt` tells apart several scripts of
    /// one unit in one step (hardpoint index + 1 for breakoff props).
    pub(crate) fn run_spawn_script(&mut self, owner: u32, script: &SpawnScriptDef, salt: u64) {
        if script.choices.is_empty() {
            return;
        }
        let mut rng = Rng::for_entity(self.config.seed, self.tick, owner, 0x5C21_0000 ^ salt);
        let units = &script.choices[(rng.next_u64() % script.choices.len() as u64) as usize];
        let owner_side = self.ships[owner as usize].side;
        // `Find_Player(faction)`: the side of that faction; a faction that is not in the battle gets the units of its owner.
        let side = self.sides.iter().position(|s| s.name.eq_ignore_ascii_case(&script.faction)).map_or(owner_side, |i| i as u8);
        let markers = self.spawn_markers(&script.place);
        if markers.is_empty() {
            return;
        }
        let mut t = self.time + script.delay;
        for (k, &ship) in units.iter().enumerate() {
            self.scripted.push(ScriptedArrival { at: t + script.hyperspace_in, side, ship, place: markers[k % markers.len()], hyperspace: true, by: owner, decay: 0.0 });
            let g = script.gap;
            let roll = if g.hi > g.lo { g.lo + (rng.next_u64() % (g.hi - g.lo + 1) as u64) as i32 } else { g.lo };
            t += g.base + roll as f32 / g.div;
        }
    }

    /// `Find_All_Objects_Of_Type` of a spawn script: the living units of a type in id order, or the map's entry markers (the start places
    /// of side 0, the attacker, or side 1, the defender, without them).
    fn spawn_markers(&self, place: &SpawnPlace) -> Vec<SpawnAt> {
        let fixed = |list: Option<&Vec<(Vec3, f32)>>, side: usize| -> Vec<SpawnAt> {
            let list = list.filter(|l| !l.is_empty()).or(self.side_places.get(side)).cloned().unwrap_or_default();
            list.into_iter().map(|(p, y)| SpawnAt::Place(p, y)).collect()
        };
        match place {
            SpawnPlace::ObjectsOfType(name) => self
                .ships
                .iter()
                .filter(|s| s.alive && !s.retreated && self.catalog.ship(s.def).name.eq_ignore_ascii_case(name))
                .map(|s| SpawnAt::Unit(s.id))
                .collect(),
            SpawnPlace::AttackerEntries => fixed(self.config.arena.as_ref().map(|a| &a.attacker_entries), 0),
            SpawnPlace::DefenderPositions => fixed(self.config.arena.as_ref().map(|a| &a.defender_positions), 1),
        }
    }

    /// Brings in the scripted units that are due.
    pub(crate) fn scripted_arrivals(&mut self) {
        if !self.scripted.iter().any(|a| a.at <= self.time) {
            return;
        }
        let due: Vec<ScriptedArrival> = self.scripted.iter().copied().filter(|a| a.at <= self.time).collect();
        self.scripted.retain(|a| a.at > self.time);
        for a in due {
            let (origin, yaw) = match a.place {
                SpawnAt::Unit(id) => (self.ships[id as usize].pos, self.ships[id as usize].yaw),
                SpawnAt::Place(p, y) => (p, y),
            };
            // A helper object stays exactly where its ability put it; it does nothing but its own abilities.
            let helper = !a.hyperspace;
            let pos = if helper { origin } else { self.free_spot(origin, a.ship) };
            let ids = self.spawn_battle_unit(a.ship, a.side, pos, yaw);
            let order = if self.is_ai(a.side) && !helper { Order::AttackMove(self.spawn_pos[(a.side as usize + 1) % self.sides.len()]) } else { Order::Idle };
            for &i in &ids {
                let s = &mut self.ships[i as usize];
                s.order = order;
                s.decay += a.decay;
                if helper {
                    s.stance = Stance::HoldFire;
                }
            }
            if self.config.record_events {
                self.events.push(Event::Spawned { side: a.side, ships: ids, by: a.by, hyperspace: a.hyperspace });
            }
        }
    }

    /// The point nearest to `origin` where a unit of type `ship` overlaps no hull: `Teleport_And_Face` puts every unit on its marker and
    /// the engine pushes them apart; we place them side by side instead (rings of `reinforce_clearance` spacing, estimate).
    fn free_spot(&self, origin: Vec3, ship: ShipId) -> Vec3 {
        let hull = |d: &ShipDef| if d.class as u8 >= TargetClass::Frigate as u8 { d.radius * 0.75 } else { d.radius };
        let def = self.catalog.ship(ship);
        let mine = if def.is_squadron { def.offsets.iter().map(|o| o.len()).fold(50.0, f32::max) } else { hull(def) };
        let clear = self.config.reinforce_clearance;
        let taken = |p: Vec3| {
            self.ships.iter().any(|s| {
                let d = self.catalog.ship(s.def);
                s.alive && !s.retreated && !matches!(d.class, TargetClass::Fighter | TargetClass::Bomber) && s.pos.dist(p) < mine + hull(d)
            })
        };
        let step = 2.0 * mine + clear;
        for ring in 0..16u32 {
            let n = if ring == 0 { 1 } else { 6 * ring };
            for j in 0..n {
                let a = std::f32::consts::TAU * j as f32 / n as f32;
                let p = origin + Vec3::new(a.cos(), a.sin(), 0.0) * (ring as f32 * step);
                if !taken(p) {
                    return p;
                }
            }
        }
        origin
    }

    /// Helper ability `ability` (`AbilityDef::helper`) of `ship` was used: its object appears after the ability's delay at the aimed point,
    /// pulled back to the ability's reach, or at the ship. A squadron uses it as one unit: its other craft wait out the same recharge (the
    /// data wants one field per squadron).
    pub(crate) fn place_helper(&mut self, ship: u32, ability: usize, h: &HelperDef) {
        let s = &self.ships[ship as usize];
        let to = self.ability_at.unwrap_or(s.pos) - s.pos;
        let at = if to.len() > h.max_range { s.pos + to.normalized() * h.max_range } else { s.pos + to };
        let (squad, cooldown) = (s.squad, s.abilities[ability].cooldown);
        self.scripted.push(ScriptedArrival { at: self.time + h.delay, side: s.side, ship: h.ship, place: SpawnAt::Place(at, s.yaw), hyperspace: false, by: ship, decay: h.own_dps });
        if squad != NONE {
            for &m in &self.squads[squad as usize] {
                if let Some(st) = self.ships[m as usize].abilities.get_mut(ability) {
                    st.cooldown = st.cooldown.max(cooldown);
                }
            }
        }
    }

    /// Helper objects wear out: `Ship::decay` hit points per second (`ObjectScript_TakeDamage` takes 1 per `ServiceRate` of 1 s from
    /// `Repair_Droids`; the buzz droid objects take their ability's `Own_Damage_Per_Second`). Helpers have no hardpoints, so the hull pays.
    pub(crate) fn run_decay(&mut self, dt: f32) {
        for i in 0..self.ships.len() {
            let s = &mut self.ships[i];
            if !s.alive || s.retreated || s.decay <= 0.0 {
                continue;
            }
            s.hull -= s.decay * dt;
            if s.hull <= 0.0 {
                s.alive = false;
                s.combat_death = true;
                if self.config.record_events {
                    self.events.push(Event::ShipDestroyed { ship: i as u32, pos: s.pos });
                }
            }
        }
    }

    /// Spawns one unit (a squadron expands into its craft) that belongs to the battle only (no results group).
    pub(crate) fn spawn_battle_unit(&mut self, ship: ShipId, side: u8, pos: Vec3, yaw: f32) -> Vec<u32> {
        let cat = self.catalog.clone();
        let def = cat.ship(ship);
        let fighter = matches!(def.class, TargetClass::Fighter | TargetClass::Bomber);
        if !def.is_squadron && !fighter {
            return vec![self.spawn_ship(ship, side, pos, yaw, NONE, NONE, Vec3::ZERO)];
        }
        let squad = self.squads.len() as u32;
        self.squads.push(Vec::new());
        let members: Vec<(ShipId, u32)> = if def.is_squadron { def.members.clone() } else { vec![(ship, 1)] };
        let mut ids = Vec::new();
        for (m, c) in members {
            for _ in 0..c {
                let off = formation_offset(&def.offsets, ids.len());
                let id = self.spawn_ship(m, side, pos + off.rotate_yaw(yaw), yaw, NONE, squad, off);
                self.squads[squad as usize].push(id);
                ids.push(id);
            }
        }
        self.lead_order(squad);
        ids
    }
}
