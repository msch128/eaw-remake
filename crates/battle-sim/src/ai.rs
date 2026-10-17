//! Side-level tactical AI after the Remake's space scripts (`Data/Scripts/AI/SpaceMode`, `FreeStore/SpaceTacticalFreeStore.lua`):
//! * the big ships advance on the enemy as one formation at the speed of the slowest (`MoveToLocation.lua`, goal `AotR_Beginning_Assault`:
//!   the free units move as one task force until it is `ASSAULT_RANGE` from the enemy location; a ship with an enemy warship in weapon
//!   range leaves it and fights), and they all attack together once one of them meets an enemy warship near its fighting range; fighters
//!   escort the formation until an enemy comes within their aggro range, and at contact the corvettes of an attacker escort the big ships
//!   (`Service_Guard`);
//! * the side focuses one priority target: the weakest enemy of the most important class near its fleet, or the nearest threat to its
//!   station (`SpaceTacticalFreeStore.lua`: `priority_target_preference`, `Service_Attack`, `Service_Move`); every unit outside the
//!   formation gets the free store's own target (`Service_Attack`: capital ships take frigates, frigates corvettes, corvettes fighters);
//! * the plan knows the enemies the side sees and those that hit one of its units within the fog's regrow time (`Default_Unit_Damaged`
//!   hands the Remake AI the attacker);
//! * a defender guards its station until the free store's aggressive mode starts (station below 70%, enemies near the station or near its
//!   big ships, or a fleet twice the enemy's after 120 s: `Allowed_As_Defender_Space_Untargeted`);
//! * reserve units are called in one at a time while the population cap has room (`ReinforceSpace.lua`, `Want_To_Reinforce_Units`), and
//!   the formation waits for units that are arriving next to it.
//!
//! Distances named in the scripts are data; the formation geometry, the catch-up rule, the contact rule and the focus weight are
//! estimates (RULES.md 9c, 9e).

use std::f32::consts::FRAC_PI_2;

use crate::battle::*;
use crate::defs::*;
use crate::formation::{Placed, ROW_GAP, Room, band, half_across, row_key};
use crate::math::{Vec3, wrap_angle};

/// `FREE_STORE_ATTACK_RANGE`: ships this close to the priority target attack it; an enemy this close to the protected station is its threat.
pub(crate) const FREE_STORE_ATTACK_RANGE: f32 = 3600.0;
/// `FREE_STORE_DEFEND_RANGE`: an own big ship below 70% this close to the enemy makes the side aggressive.
const FREE_STORE_DEFEND_RANGE: f32 = 6200.0;
/// `FIGHTER_AGGRO_RANGE`: fighters leave the formation for enemies this close.
const FIGHTER_AGGRO_RANGE: f32 = 3000.0;
/// `SHIP_AGGRO_RANGE`: the aggro range of a ship is its longest weapon range, at least this (`Get_Aggro_Range`).
const SHIP_AGGRO_RANGE: f32 = 3500.0;
/// `MoveToLocation.lua`: the fleet moves as one task force until it is this close to the enemy location.
const ASSAULT_RANGE: f32 = 5000.0;
/// Estimate: a free ship this close to the formation joins it; ships farther away catch up first.
const JOIN_RANGE: f32 = 3000.0;
/// Estimate: the enemy location is the enemy with the most enemy strength within this radius.
const CLUSTER_RADIUS: f32 = 2500.0;
/// Estimate: the formation slows down while its worst ship is more than `LAG_FREE` units behind its slot, down to `LAG_MIN` of its
/// speed at `LAG_FREE + LAG_SPAN`.
const LAG_FREE: f32 = 500.0;
const LAG_SPAN: f32 = 1500.0;
const LAG_MIN: f32 = 0.25;
/// Estimate: a ship that left the formation to fight counts as fighting until no enemy is within this multiple of its reach.
const ENGAGED_HYSTERESIS: f32 = 2.0;
/// Estimate: the whole formation attacks once an enemy warship is within this multiple of one of its ships' standoff distance.
const CONTACT_STANDOFF_MULT: f32 = 1.5;
/// Estimate: when the goal lies more than this angle (radians) off the formation's heading, the line is laid out anew facing it.
const REFORM_TURN: f32 = 1.0;
/// Estimate: a search point's time limit starts again whenever the fleet got this much closer to it.
const SEARCH_PROGRESS: f32 = 500.0;
/// Estimate: weight factor of the side's priority target in a ship's target choice (a lower weight is preferred).
pub(crate) const FOCUS_WEIGHT: f32 = 0.5;
/// Estimate: reinforcements are not called to an arrival point with a visible enemy this close (`Space_Area_Is_Friendly` / `_Is_Empty`).
const REINFORCE_CLEAR: f32 = 3000.0;
/// Estimate: when enemies hold the reinforcement point, units arrive this far behind the own fleet instead.
const REINFORCE_BEHIND: f32 = 1500.0;

/// Tactical AI state of one side.
#[derive(Debug, Clone, Default)]
pub(crate) struct AiSide {
    /// The advancing formation: ship and slot offset (x ahead of the anchor, y to its left; the ships of a row share the x).
    group: Vec<(u32, Vec3)>,
    /// Formation reference point and its heading (radians).
    anchor: Vec3,
    heading: f32,
    /// Where the formation is going (the enemy location or a search point).
    goal: Vec3,
    /// Priority target (`NONE` = none).
    focus: u32,
    /// Units fighting outside the formation (sorted ids).
    engaged: Vec<u32>,
    /// Closest any big ship came to the current search point so far.
    search_best: f32,
    /// Units waiting outside the battle (definition, count), called in this order.
    pub(crate) reserve: Vec<(ShipId, u32)>,
    /// Highest force of the side's ships (fighters and bombers left out) in this battle so far, for the retreat rule.
    pub(crate) peak: f32,
}

impl AiSide {
    /// A side whose AI may call in `reserve`.
    pub(crate) fn new(reserve: Vec<(ShipId, u32)>) -> Self {
        Self { focus: NONE, search_best: f32::MAX, reserve, ..Self::default() }
    }
}

fn small(d: &ShipDef) -> bool {
    matches!(d.class, TargetClass::Fighter | TargetClass::Bomber)
}

/// `Get_Shield_And_Health` (`PGCommands.lua`): the hull share, or the shield share when the ship has a shield that is lower than its hull.
fn shield_and_health(s: &Ship, d: &ShipDef) -> f32 {
    let hull = (s.hull / d.hull.max(1.0)).clamp(0.0, 1.0);
    let shield = if d.shield > 0.0 { (s.shield / d.shield).clamp(0.0, 1.0) } else { 0.0 };
    if shield == 0.0 || hull < shield { hull } else { shield }
}

impl Battle {
    /// Weight of a unit where the plan looks for the enemy's and its own fleet: `sqrt((hull + shield) x best-bearing damage rate)` (estimate).
    pub(crate) fn unit_power(&self, s: &Ship) -> f32 {
        let d = self.catalog.ship(s.def);
        ((s.hull.max(0.0) + s.shield.max(0.0)) * crate::defs::dps_estimate(d, &self.catalog)).sqrt()
    }

    /// Force of one unit as the Remake AI counts it (`FriendlyForceUnnormalized`): its `AI_Combat_Power` (data), weighted by the share of hull
    /// and shield it has left (estimate: the engine's weighting is not visible).
    pub(crate) fn unit_force(&self, s: &Ship) -> f32 {
        let d = self.catalog.ship(s.def);
        d.ai_power * ((s.hull.max(0.0) + s.shield.max(0.0)) / (d.hull + d.shield).max(1.0)).min(1.0)
    }

    /// Force of a side: all its units in the battle.
    pub(crate) fn side_force(&self, side: u8) -> f32 {
        self.ships.iter().filter(|s| s.alive && !s.retreated && s.side == side).map(|s| self.unit_force(s)).sum()
    }

    /// Force of the side's ships and structures in the battle without fighters and bombers (their squadrons come and go).
    pub(crate) fn fleet_force(&self, side: u8) -> f32 {
        self.ships.iter().filter(|s| s.alive && !s.retreated && s.side == side && !small(self.catalog.ship(s.def))).map(|s| self.unit_force(s)).sum()
    }

    /// Whether the AI plan may (re)order a unit: its standing orders are its own, not a lure's, a hunt's or a retreat's (a fighter's move
    /// order is the plan's escort order).
    fn planned(&self, s: &Ship) -> bool {
        let d = self.catalog.ship(s.def);
        s.alive
            && !s.retreated
            && !d.is_static
            && (matches!(s.order, Order::Idle | Order::AttackMove(_) | Order::Guard(_)) || (small(d) && matches!(s.order, Order::Move(_))))
            && s.hunt_cell == NO_CELL
            && !d.abilities.iter().zip(&s.abilities).any(|(a, st)| a.hunt && st.active > 0.0)
    }

    /// Once a second: every AI side updates its plan (see the module docs).
    pub(crate) fn ai_plan(&mut self) {
        for side in 0..self.sides.len().min(4) as u8 {
            if self.is_ai(side) {
                self.plan_side(side);
            } else {
                self.dissolve(side);
            }
        }
    }

    /// Ends the formation of a side: its ships fly on their own.
    fn dissolve(&mut self, side: u8) {
        let group = std::mem::take(&mut self.ai[side as usize].group);
        for (id, ..) in group {
            self.ships[id as usize].group_slot = None;
        }
    }

    /// `Current_Enemy_Location`: the known enemy (seen, or one that hit the side) with the most enemy strength within `CLUSTER_RADIUS` of it
    /// (fighters only when nothing bigger is known).
    fn enemy_location(&self, enemies: &[u32]) -> Option<Vec3> {
        let cat = &self.catalog;
        let units: Vec<(Vec3, f32, bool)> = enemies.iter().map(|&e| {
            let s = &self.ships[e as usize];
            (s.pos, self.unit_power(s), small(cat.ship(s.def)))
        }).collect();
        let any_big = units.iter().any(|u| !u.2);
        units
            .iter()
            .enumerate()
            .filter(|(_, u)| !any_big || !u.2)
            .map(|(k, u)| (units.iter().filter(|o| o.0.dist(u.0) <= CLUSTER_RADIUS).map(|o| o.1).sum::<f32>(), k))
            .max_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)))
            .map(|(_, k)| units[k].0)
    }

    /// `SpaceTacticalFreeStore.lua` priority enemy: of the first class group with a known enemy (station or capital, frigate, corvette),
    /// the enemy the script's scan ends on: in id order, each enemy that is at least as weak (`Get_Shield_And_Health`) and at least as
    /// close to the own fleet as the one before replaces it.
    fn priority_enemy(&self, enemies: &[u32], friendly: Vec3) -> u32 {
        use TargetClass::*;
        let cat = &self.catalog;
        let groups: [&[TargetClass]; 4] = [&[Structure, Capital, Super], &[Frigate], &[Structure, Frigate], &[Corvette]];
        for classes in groups {
            let mut best = (NONE, 2.0f32, 100_000.0f32);
            for &e in enemies {
                let s = &self.ships[e as usize];
                let d = cat.ship(s.def);
                if !classes.contains(&d.class) {
                    continue;
                }
                let health = shield_and_health(s, d);
                let dist = s.pos.dist(friendly);
                if health <= best.1 && dist <= best.2 {
                    best = (e, health, dist);
                }
            }
            if best.0 != NONE {
                return best.0;
            }
        }
        NONE
    }

    fn plan_side(&mut self, side: u8) {
        let cat = self.catalog.clone();
        let si = side as usize;
        // A side that ordered its retreat only flies home.
        if self.retreat_ordered[si] {
            self.dissolve(side);
            return;
        }
        let time = self.time;
        // The enemies the side sees, and those that hit one of its units within the fog's regrow time although it cannot see them: the shots
        // show where they come from (`Default_Unit_Damaged` hands the Remake AI the attacker).
        let linger = self.config.fog_linger;
        let enemies: Vec<u32> = self
            .ships
            .iter()
            .filter(|o| {
                o.alive
                    && !o.retreated
                    && o.side != side
                    && cat.ship(o.def).special_weapon.is_none()
                    && !cat.ship(o.def).background
                    && (self.is_visible_to(side, o.id) || time - o.hit_side_at[si] <= linger)
            })
            .map(|o| o.id)
            .collect();
        let structures: Vec<u32> = self.ships.iter().filter(|s| s.alive && s.side == side && cat.ship(s.def).is_static && cat.ship(s.def).special_weapon.is_none()).map(|s| s.id).collect();
        // Big ships, gunship pairs and the corvettes a carrier launches included (fighters and bombers follow their squadron leader).
        let warships: Vec<u32> = self.ships.iter().filter(|s| s.alive && !s.retreated && s.side == side && !cat.ship(s.def).is_static && !small(cat.ship(s.def))).map(|s| s.id).collect();

        // Where the enemy is; with nothing in sight the fleet sweeps the map along the search points.
        let seen = self.enemy_location(&enemies);
        let goal = match seen {
            Some(g) => {
                self.search[si] = (0, time);
                self.ai[si].search_best = f32::MAX;
                g
            }
            None => {
                let (idx, mut since) = self.search[si];
                let enemy_start = self.spawn_pos[(si + 1) % self.sides.len()];
                let half = self.config.half_size;
                let cur = crate::sim::search_point(idx, enemy_start, half);
                let closest = warships.iter().map(|&w| self.ships[w as usize].pos.dist(cur)).fold(f32::MAX, f32::min);
                // The search gives a point up when nobody gets there in time; a fleet still closing in on it (a slow formation) keeps it.
                if closest < self.ai[si].search_best - SEARCH_PROGRESS {
                    self.ai[si].search_best = closest;
                    since = time;
                    self.search[si].1 = time;
                }
                let timeout = if idx == 0 { 90.0 } else { 45.0 };
                if closest < 2500.0 || time - since >= timeout {
                    self.search[si] = (idx + 1, time);
                    self.ai[si].search_best = f32::MAX;
                    crate::sim::search_point(idx + 1, enemy_start, half)
                } else {
                    cur
                }
            }
        };

        // Where the own fleet is: the formation, else the strength-weighted centre of the big ships, else the station.
        let friendly = if !self.ai[si].group.is_empty() {
            self.ai[si].anchor
        } else {
            let (mut sum, mut w) = (Vec3::ZERO, 0.0f32);
            for &id in &warships {
                let s = &self.ships[id as usize];
                let p = self.unit_power(s).max(1.0);
                sum += s.pos * p;
                w += p;
            }
            if w > 0.0 { sum * (1.0 / w) } else { structures.first().map_or(self.spawn_pos[si], |&s| self.ships[s as usize].pos) }
        };

        // The structure to protect (the most damaged one, then the one nearest to the enemy) and the enemy nearest to it.
        let protected = structures.iter().copied().min_by(|&a, &b| {
            let (sa, sb) = (&self.ships[a as usize], &self.ships[b as usize]);
            shield_and_health(sa, cat.ship(sa.def)).total_cmp(&shield_and_health(sb, cat.ship(sb.def))).then(sa.pos.dist(goal).total_cmp(&sb.pos.dist(goal))).then(a.cmp(&b))
        });
        let threat = protected.and_then(|p| {
            let at = self.ships[p as usize].pos;
            enemies.iter().copied().map(|e| (self.ships[e as usize].pos.dist(at), e)).filter(|&(d, _)| d < FREE_STORE_ATTACK_RANGE).min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1))).map(|(_, e)| e)
        });

        // `Allowed_As_Defender_Space_Untargeted`: attackers, defenders without structures, and defenders whose fleet is twice the enemy's after 120 s attack.
        let defender = !self.sides[si].structures.is_empty();
        let allowed = !defender || structures.is_empty() || {
            let mine: f32 = self.ships.iter().filter(|s| s.alive && !s.retreated && s.side == side && !cat.ship(s.def).is_static).map(|s| self.unit_force(s)).sum();
            let theirs: f32 = (0..self.sides.len() as u8).filter(|&o| o != side).map(|o| self.side_force(o)).sum();
            time > 120.0 && mine >= 2.0 * theirs
        };
        // Aggressive mode of the free store.
        let near_enemy = seen.is_some()
            && warships.iter().chain(&structures).any(|&id| {
                let s = &self.ships[id as usize];
                let d = cat.ship(s.def);
                let dist = s.pos.dist(goal);
                matches!(d.class, TargetClass::Frigate | TargetClass::Capital | TargetClass::Super | TargetClass::Structure)
                    && ((shield_and_health(s, d) < 0.7 && dist < FREE_STORE_DEFEND_RANGE) || dist < FREE_STORE_ATTACK_RANGE)
            });
        let station_hurt = protected.is_some_and(|p| shield_and_health(&self.ships[p as usize], cat.ship(self.ships[p as usize].def)) < 0.7);
        let aggressive = allowed || station_hurt || near_enemy || threat.is_some();

        // Priority target: the station's threat for a defender that holds back, else the free store's priority enemy, kept until a new
        // one is at least as weak and as close.
        let old = self.ai[si].focus;
        let old_ok = old != NONE && enemies.contains(&old);
        let focus = if !allowed {
            match (threat, old_ok) {
                (Some(t), true) if shield_and_health(&self.ships[old as usize], cat.ship(self.ships[old as usize].def)) <= shield_and_health(&self.ships[t as usize], cat.ship(self.ships[t as usize].def)) => old,
                (Some(t), _) => t,
                (None, _) => NONE,
            }
        } else {
            let new = self.priority_enemy(&enemies, friendly);
            if !old_ok || new == NONE {
                new
            } else {
                let (o, n) = (&self.ships[old as usize], &self.ships[new as usize]);
                if shield_and_health(o, cat.ship(o.def)) >= shield_and_health(n, cat.ship(n.def)) && o.pos.dist(friendly) >= n.pos.dist(friendly) { new } else { old }
            }
        };
        self.ai[si].focus = focus;
        self.ai[si].goal = goal;
        let target_pt = if focus != NONE { self.ships[focus as usize].pos } else { goal };

        // The units the plan orders: big ships and the leader of every fighter or bomber squadron (wingmen follow their leader).
        let leaders: Vec<u32> = self
            .squads
            .iter()
            .filter_map(|m| m.iter().copied().find(|&i| self.ships[i as usize].alive && !self.ships[i as usize].retreated))
            .filter(|&l| self.ships[l as usize].side == side && small(cat.ship(self.ships[l as usize].def)))
            .collect();
        let mut units: Vec<u32> = warships.iter().chain(&leaders).copied().filter(|&id| self.planned(&self.ships[id as usize])).collect();
        units.sort_unstable();
        let given: Vec<u32> = units.iter().map(|&id| self.ships[id as usize].focus).collect();
        for &id in &units {
            self.ships[id as usize].focus = focus;
        }
        self.ai_reinforce(side, &enemies, friendly, goal);

        if !aggressive {
            // `Service_Guard`: hold at the station.
            self.dissolve(side);
            self.ai[si].engaged.clear();
            let at = protected.map_or(self.spawn_pos[si], |p| self.ships[p as usize].pos);
            for &id in &units {
                self.set_order(id, Order::Guard(at));
            }
            return;
        }

        // Who is fighting: a big ship with an enemy warship or structure in weapon range, a fighter with an enemy in its aggro range.
        let was = std::mem::take(&mut self.ai[si].engaged);
        let mut engaged = Vec::new();
        for &id in &units {
            let s = &self.ships[id as usize];
            let d = cat.ship(s.def);
            let hold = if was.binary_search(&id).is_ok() { ENGAGED_HYSTERESIS } else { 1.0 };
            let reach = if small(d) { FIGHTER_AGGRO_RANGE.max(d.max_weapon_range) } else { d.max_weapon_range } * hold;
            let fighting = enemies.iter().any(|&e| {
                let o = &self.ships[e as usize];
                let od = cat.ship(o.def);
                (small(d) || !small(od)) && o.pos.dist(s.pos) - od.radius * 0.75 <= reach
            });
            if fighting {
                engaged.push(id);
            }
        }
        engaged.sort_unstable();
        let free: Vec<u32> = units.iter().copied().filter(|id| engaged.binary_search(id).is_err()).collect();
        self.ai[si].engaged = engaged.clone();

        // The formation of the big ships: the free ones advance together while the enemy is farther than `ASSAULT_RANGE`, and all of them
        // attack together once one of them meets an enemy warship near its fighting range (estimate: 1.5x its standoff; a ship that only
        // has the enemy at the end of its longest gun, like a missile cruiser, leaves the formation alone).
        let free_big: Vec<u32> = free.iter().copied().filter(|&id| !small(cat.ship(self.ships[id as usize].def))).collect();
        let far = |from: Vec3| from.dist(goal) > ASSAULT_RANGE;
        let contact = self.ai[si].group.iter().any(|g| {
            let s = &self.ships[g.0 as usize];
            let reach = cat.ship(s.def).preferred_range * CONTACT_STANDOFF_MULT;
            enemies.iter().any(|&e| {
                let o = &self.ships[e as usize];
                !small(cat.ship(o.def)) && o.pos.dist(s.pos) - cat.ship(o.def).radius * 0.75 <= reach
            })
        });
        let group_alive = !self.ai[si].group.is_empty();
        let turn = {
            let (a, to) = (&self.ai[si], goal - self.ai[si].anchor);
            wrap_angle(to.y.atan2(to.x) - a.heading).abs()
        };
        if group_alive && (contact || !far(self.ai[si].anchor)) {
            self.dissolve(side);
        } else if group_alive && turn > REFORM_TURN {
            // The goal moved far round: a new line facing it instead of wheeling the old one.
            let members: Vec<u32> = self.ai[si].group.iter().map(|g| g.0).collect();
            self.form_group(side, &members, goal);
        } else if !group_alive && !free_big.is_empty() {
            let c = free_big.iter().fold(Vec3::ZERO, |a, &id| a + self.ships[id as usize].pos) * (1.0 / free_big.len() as f32);
            if far(c) {
                // Ships far from the others (a straggler, a unit across the map) catch up and join later.
                let members: Vec<u32> = free_big.iter().copied().filter(|&id| self.ships[id as usize].pos.dist(c) <= 1.5 * JOIN_RANGE).collect();
                self.form_group(side, &members, goal);
            }
        }
        // Ships that started to fight leave it; free ships close to it join, the others catch up.
        let mut in_group: Vec<u32> = Vec::new();
        if !self.ai[si].group.is_empty() {
            let leave: Vec<u32> = self.ai[si].group.iter().map(|g| g.0).filter(|id| engaged.binary_search(id).is_ok() || !units.contains(id)).collect();
            for id in leave {
                self.ai[si].group.retain(|g| g.0 != id);
                self.ships[id as usize].group_slot = None;
            }
            for &id in &free_big {
                if !self.ai[si].group.iter().any(|g| g.0 == id) && self.ships[id as usize].pos.dist(self.ai[si].anchor) <= JOIN_RANGE {
                    self.join_group(side, id);
                }
            }
            in_group = self.ai[si].group.iter().map(|g| g.0).collect();
            if in_group.is_empty() {
                self.dissolve(side);
            } else {
                self.order_rows(side);
            }
        }
        let (anchor, heading) = (self.ai[si].anchor, self.ai[si].heading);
        for &id in &units {
            let s = &self.ships[id as usize];
            let fighting = engaged.binary_search(&id).is_ok();
            let guard = (protected.is_none() && cat.ship(s.def).class == TargetClass::Corvette && !in_group.contains(&id)).then(|| self.corvette_guard(id, &enemies)).flatten();
            let order = if in_group.contains(&id) {
                Order::AttackMove(goal)
            } else if let Some(order) = guard {
                // A corvette of an attacking side escorts the big ships instead of charging the enemy line.
                order
            } else if !in_group.is_empty() && !fighting && small(cat.ship(s.def)) {
                // Fighters escort the formation: carrier squadrons stay with their carrier, the others keep ahead of the line.
                if s.carrier != NONE && self.ships[s.carrier as usize].alive { Order::Idle } else { Order::Move(anchor + Vec3::from_yaw(heading) * (2.0 * ROW_GAP)) }
            } else if !in_group.is_empty() && !fighting {
                // A straggler (or a reinforcement) catches up with the formation.
                Order::AttackMove(anchor)
            } else {
                // `Service_Move`: attack-move on the priority target, else the enemy location.
                Order::AttackMove(target_pt)
            };
            self.set_order(id, order);
        }
        // `On_Unit_Service`: the free store gives every unit outside the advancing formation its own target.
        for (&id, &prev) in units.iter().zip(&given) {
            if !in_group.contains(&id) {
                let t = self.service_target(id, focus, prev, &enemies);
                if t != NONE {
                    self.ships[id as usize].focus = t;
                }
            }
        }
    }

    /// `Service_Attack` of `SpaceTacticalFreeStore.lua` in aggressive mode at difficulty Normal: the side's priority target when it is of the
    /// kind the unit prefers (`Is_Preferred_Target`: capital ships take frigates, frigates corvettes, corvettes and anti-fighter ships fighters,
    /// bombers and transports, fighters fighters and bombers) and within `FREE_STORE_ATTACK_RANGE`; else, while the unit's own target is not
    /// of that kind, the nearest enemy of it within the unit's aggro range (`Get_Preferred_Target_In_Range`); else the priority target within
    /// `FREE_STORE_ATTACK_RANGE`. A target given before (`prev`) stays while it is of the preferred kind and within the aggro range: an attack
    /// order holds until the unit's next service. `NONE` when none of these applies (the unit keeps its own choice).
    fn service_target(&self, id: u32, priority: u32, prev: u32, enemies: &[u32]) -> u32 {
        use TargetClass::*;
        let cat = &self.catalog;
        let s = &self.ships[id as usize];
        let d = cat.ship(s.def);
        let kind = |t: u32| cat.ship(self.ships[t as usize].def).class;
        let preferred = |c: TargetClass| {
            if d.anti_fighter {
                matches!(c, Fighter | Bomber | Transport)
            } else {
                match d.class {
                    Capital | Super => c == Frigate,
                    Frigate => c == Corvette,
                    Corvette => matches!(c, Fighter | Bomber | Transport),
                    Fighter => matches!(c, Fighter | Bomber),
                    _ => false,
                }
            }
        };
        let aggro = if small(d) { FIGHTER_AGGRO_RANGE } else { d.max_weapon_range.max(SHIP_AGGRO_RANGE) };
        let dist = |t: u32| self.ships[t as usize].pos.dist(s.pos);
        if priority != NONE && preferred(kind(priority)) && dist(priority) < FREE_STORE_ATTACK_RANGE {
            return priority;
        }
        if prev != NONE && prev != priority && enemies.contains(&prev) && preferred(kind(prev)) && dist(prev) < aggro {
            return prev;
        }
        if s.target == NONE || !preferred(kind(s.target)) {
            let near = enemies.iter().copied().filter(|&e| preferred(kind(e))).map(|e| (dist(e), e)).filter(|&(x, _)| x < aggro).min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            if let Some((_, e)) = near {
                return e;
            }
        }
        if priority != NONE && dist(priority) < FREE_STORE_ATTACK_RANGE { priority } else { NONE }
    }

    /// `Service_Guard` of `SpaceTacticalFreeStore.lua` for a corvette of an attacking side: it attacks fighters, bombers and transports within its
    /// aggro range (`Get_Preferred_Target_In_Range`), else the enemy nearest to the own frigate, capital ship or station nearest to it
    /// (`Get_Guard_Targets`, `Find_Nearest_Smart`: stations and capital ships first, then frigates, then corvettes) once that enemy is within
    /// its aggro range of it, else it stays beside that ship. `None` without a ship to guard.
    fn corvette_guard(&self, id: u32, enemies: &[u32]) -> Option<Order> {
        use TargetClass::*;
        let cat = &self.catalog;
        let s = &self.ships[id as usize];
        let d = cat.ship(s.def);
        let aggro = d.max_weapon_range.max(SHIP_AGGRO_RANGE);
        let nearest = |from: Vec3, classes: &[TargetClass]| {
            enemies
                .iter()
                .copied()
                .filter(|&e| classes.contains(&cat.ship(self.ships[e as usize].def).class))
                .map(|e| (self.ships[e as usize].pos.dist(from), e))
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
        };
        if let Some((dist, e)) = nearest(s.pos, &[Fighter, Bomber, Transport])
            && dist < aggro
        {
            return Some(Order::AttackMove(self.ships[e as usize].pos));
        }
        let guarded = self
            .ships
            .iter()
            .filter(|o| o.alive && !o.retreated && o.side == s.side && matches!(cat.ship(o.def).class, Frigate | Capital | Super | Structure))
            .map(|o| (o.pos.dist(s.pos), o.id))
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))?
            .1;
        let g = &self.ships[guarded as usize];
        let groups: [&[TargetClass]; 4] = [&[Structure, Capital, Super], &[Frigate], &[Structure, Frigate], &[Corvette]];
        match groups.iter().find_map(|c| nearest(g.pos, c)) {
            Some((dist, e)) if dist < aggro => Some(Order::AttackMove(self.ships[e as usize].pos)),
            _ => {
                // Beside the guarded ship, on the side the corvette is on.
                let away = s.pos - g.pos;
                let side_dir = if away.len() > 1.0 { away.normalized() } else { Vec3::from_yaw(g.yaw + FRAC_PI_2) };
                Some(Order::Guard(g.pos + side_dir * (cat.ship(g.def).radius + d.radius + 150.0)))
            }
        }
    }

    /// Gives a planned unit (and the wingmen of a fighter squadron's leader) a standing order.
    fn set_order(&mut self, id: u32, order: Order) {
        let squad = self.ships[id as usize].squad;
        if squad == NONE || !small(self.catalog.ship(self.ships[id as usize].def)) {
            self.ships[id as usize].order = order;
            return;
        }
        for k in 0..self.squads[squad as usize].len() {
            let m = self.squads[squad as usize][k] as usize;
            if self.ships[m].alive && !self.ships[m].retreated && !matches!(self.ships[m].order, Order::Attack(_) | Order::Retreat) {
                self.ships[m].order = order;
            }
        }
    }

    /// Lays out a new formation facing the goal with the slot layout of a group move ([`crate::formation::layout`]: the escorts in a row
    /// ahead of the big ships, a band wider than a row can be in several rows, each row ordered by where its ships stand across the line of
    /// advance) and the start layout's room ([`Room::Start`]). The anchor stands on the front row of big ships. The slots start at or ahead
    /// of every ship, so the ships in front wait while the others close up and nobody has to turn round.
    fn form_group(&mut self, side: u8, members: &[u32], goal: Vec3) {
        if members.is_empty() {
            return;
        }
        let units: Vec<Placed> = members.iter().map(|&id| (self.ships[id as usize].def, self.ships[id as usize].pos, None)).collect();
        let centre = units.iter().fold(Vec3::ZERO, |a, u| a + u.1) * (1.0 / units.len() as f32);
        let to = goal - centre;
        let heading = to.y.atan2(to.x);
        let fwd = Vec3::from_yaw(heading);
        let mut offsets = crate::formation::layout(&self.catalog, &units, heading, Room::Start);
        let big = |k: usize| row_key(self.catalog.ship(units[k].0).class) == 1;
        let front = (0..units.len()).filter(|&k| big(k)).map(|k| offsets[k].x).fold(f32::MIN, f32::max);
        let shift = if front > f32::MIN { -front } else { ROW_GAP - offsets.iter().map(|o| o.x).fold(f32::MAX, f32::min) };
        for o in &mut offsets {
            o.x += shift;
        }
        let ahead = units.iter().zip(&offsets).map(|(u, o)| (u.1 - centre).dot(fwd) - o.x).fold(f32::MIN, f32::max);
        let ai = &mut self.ai[side as usize];
        ai.anchor = centre + fwd * ahead;
        ai.heading = heading;
        ai.goal = goal;
        ai.group = members.iter().copied().zip(offsets).collect();
    }

    /// Keeps every row ordered by where its ships stand across the line of advance: ships that were pushed past each other swap slots
    /// instead of crossing back. The row keeps its centre; each ship keeps its own width.
    fn order_rows(&mut self, side: u8) {
        let cat = self.catalog.clone();
        let ships = &self.ships;
        let ai = &mut self.ai[side as usize];
        let left = Vec3::from_yaw(ai.heading + FRAC_PI_2);
        let mut depths: Vec<f32> = ai.group.iter().map(|g| g.1.x).collect();
        depths.sort_by(f32::total_cmp);
        depths.dedup();
        for depth in depths {
            // (across the line now, index in the group, half the room the ship takes)
            let mut row: Vec<(f32, usize, f32)> = ai.group.iter().enumerate().filter(|(_, g)| g.1.x == depth).map(|(k, g)| ((ships[g.0 as usize].pos - ai.anchor).dot(left), k, half_across(cat.ship(ships[g.0 as usize].def), Room::Start))).collect();
            if row.len() < 2 {
                continue;
            }
            let lo = row.iter().map(|r| ai.group[r.1].1.y - r.2).fold(f32::MAX, f32::min);
            let hi = row.iter().map(|r| ai.group[r.1].1.y + r.2).fold(f32::MIN, f32::max);
            row.sort_by(|a, b| a.0.total_cmp(&b.0).then(ai.group[a.1].0.cmp(&ai.group[b.1].0)));
            let mut y = 0.5 * (lo + hi) - 0.5 * row.iter().map(|r| 2.0 * r.2).sum::<f32>();
            for (_, k, half) in row {
                ai.group[k].1.y = y + half;
                y += 2.0 * half;
            }
        }
    }

    /// Adds a ship to a row of its band, the one with the fewest ships (the rear one of equals), at the end nearer to where the ship is; a
    /// ship of a band the formation does not have yet starts a row `ROW_GAP` ahead of it or behind it.
    fn join_group(&mut self, side: u8, id: u32) {
        let cat = self.catalog.clone();
        let d = cat.ship(self.ships[id as usize].def);
        let half = half_across(d, Room::Start);
        let ships = &self.ships;
        let ai = &mut self.ai[side as usize];
        let lateral = (ships[id as usize].pos - ai.anchor).rotate_yaw(-ai.heading).y;
        let def = |g: &(u32, Vec3)| cat.ship(ships[g.0 as usize].def);
        let mut depths: Vec<f32> = ai.group.iter().map(|g| g.1.x).collect();
        depths.sort_by(f32::total_cmp);
        depths.dedup();
        let count = |x: f32| ai.group.iter().filter(|g| g.1.x == x).count();
        let mine = depths.iter().copied().filter(|&x| ai.group.iter().any(|g| g.1.x == x && band(def(g)) == band(d))).min_by(|&a, &b| count(a).cmp(&count(b)).then(a.total_cmp(&b)));
        let slot = match mine {
            Some(x) => {
                let row: Vec<(f32, f32)> = ai.group.iter().filter(|g| g.1.x == x).map(|g| (g.1.y, half_across(def(g), Room::Start))).collect();
                let hi = row.iter().map(|r| r.0 + r.1).fold(f32::MIN, f32::max);
                let lo = row.iter().map(|r| r.0 - r.1).fold(f32::MAX, f32::min);
                Vec3::new(x, if lateral >= 0.5 * (hi + lo) { hi + half } else { lo - half }, 0.0)
            }
            None => {
                let (rear, front) = (depths[0], depths[depths.len() - 1]);
                if ai.group.iter().any(|g| g.1.x == front && band(d) < band(def(g))) { Vec3::new(front + ROW_GAP, 0.0, 0.0) } else { Vec3::new(rear - ROW_GAP, 0.0, 0.0) }
            }
        };
        ai.group.push((id, slot));
    }

    /// Every step: the formation's anchor advances on its goal at the speed of the slowest big ship (slower while ships lag behind their
    /// slots, standing while reinforcements arrive next to it), turning no faster than its slowest ship and its outer slots allow; every
    /// member gets its slot and the group's speed.
    pub(crate) fn ai_formation(&mut self, dt: f32) {
        let cat = self.catalog.clone();
        for si in 0..self.ai.len() {
            if self.ai[si].group.is_empty() {
                continue;
            }
            let gone: Vec<u32> = self.ai[si].group.iter().map(|g| g.0).filter(|&id| {
                let s = &self.ships[id as usize];
                !s.alive || s.retreated || !matches!(s.order, Order::AttackMove(_))
            }).collect();
            for id in gone {
                self.ai[si].group.retain(|g| g.0 != id);
                self.ships[id as usize].group_slot = None;
            }
            let ai = &self.ai[si];
            let big: Vec<(u32, f32, f32)> = ai.group.iter().filter(|g| !small(cat.ship(self.ships[g.0 as usize].def))).map(|g| {
                let s = &self.ships[g.0 as usize];
                let d = cat.ship(s.def);
                (g.0, d.max_speed * s.engine_factor, d.turn_rate)
            }).collect();
            if big.is_empty() {
                self.dissolve(si as u8);
                continue;
            }
            let speed = big.iter().map(|b| b.1).fold(f32::MAX, f32::min).max(0.0);
            let lag = ai.group.iter().map(|g| self.ships[g.0 as usize].pos.dist(ai.anchor + g.1.rotate_yaw(ai.heading))).fold(0.0f32, f32::max);
            let factor = (1.0 - (lag - LAG_FREE) / LAG_SPAN).clamp(LAG_MIN, 1.0);
            // Units called in that drop out of hyperspace next to the formation are waited for.
            let side = si as u8;
            let arriving = self.pending.iter().any(|r| r.side == side && r.pos.unwrap_or(self.spawn_pos[si]).dist(ai.anchor) <= JOIN_RANGE);
            let v = if arriving { 0.0 } else { speed * factor };
            let reach = ai.group.iter().map(|g| g.1.len()).fold(1.0f32, f32::max);
            let turn = big.iter().map(|b| b.2).fold(f32::MAX, f32::min).min(speed / reach) * factor;
            let to = ai.goal - ai.anchor;
            let want = to.y.atan2(to.x);
            let heading = wrap_angle(ai.heading + wrap_angle(want - ai.heading).clamp(-turn * dt, turn * dt));
            let anchor = if to.len() > ASSAULT_RANGE * 0.5 { ai.anchor + Vec3::from_yaw(heading) * (v * dt) } else { ai.anchor };
            let slots: Vec<(u32, Vec3)> = ai.group.iter().map(|g| (g.0, anchor + g.1.rotate_yaw(heading))).collect();
            let ai = &mut self.ai[si];
            ai.anchor = anchor;
            ai.heading = heading;
            for (id, p) in slots {
                self.ships[id as usize].group_slot = Some((p, v, heading));
            }
        }
    }

    /// `ReinforceSpace.lua` / `Want_To_Reinforce_Units`: while the population cap has room, the next unit of the reserve that fits is called
    /// in, one at a time, at the side's reinforcement point; when enemies hold that point it arrives behind the own fleet instead.
    fn ai_reinforce(&mut self, side: u8, enemies: &[u32], friendly: Vec3, enemy_at: Vec3) {
        let si = side as usize;
        if self.ai[si].reserve.is_empty() {
            return;
        }
        let room = self.config.unit_cap - self.pop_in_play(side);
        let Some(k) = self.ai[si].reserve.iter().position(|&(ship, n)| n > 0 && self.catalog.ship(ship).population <= room) else { return };
        let ship = self.ai[si].reserve[k].0;
        let clear = |b: &Battle, p: Vec3| !enemies.iter().any(|&e| b.ships[e as usize].pos.dist(p) < REINFORCE_CLEAR);
        let called = if clear(self, self.spawn_pos[si]) {
            self.command(side, Command::Reinforce { ship, count: 1 }).is_ok()
        } else {
            let back = friendly - enemy_at;
            let pos = friendly + if back.len() > 1.0 { back.normalized() * REINFORCE_BEHIND } else { Vec3::ZERO };
            clear(self, pos) && self.command(side, Command::ReinforceAt { ship, count: 1, pos, yaw_offset: 0.0 }).is_ok()
        };
        if called {
            let r = &mut self.ai[si].reserve;
            r[k].1 -= 1;
            if r[k].1 == 0 {
                r.remove(k);
            }
        }
    }
}
