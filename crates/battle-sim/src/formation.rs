//! Formations of group moves. In EaW the units ordered somewhere together end up side by side in rows that face the move (design rule
//! 2026-10-17: 20 Star Destroyers stand in a tight row, if needed in 2-3 rows). [`layout`] places the slots; the
//! destination formations of `Command::Move` / `Command::AttackMove` ([`Formation`], tight: [`Room::Tight`]) and the AI's advancing line
//! ([`crate::ai`], with the start layout's room: [`Room::Start`]) share it. A unit of a destination formation flies its own route to its
//! slot (`mov`: the nav grid's A*, `steer_around` and the hull separation), the group keeps the pace of its slowest member, and a ship that
//! has parked turns to the formation's facing.
//!
//! Data: `BetweenFormationSpacing` (GameConstants) is the clearance between neighbours beyond what keeps their hulls apart, and a unit's
//! `FormationSpacing` multiplies it; `FormationOrder` puts lower orders further ahead; within its band a higher `Formation_Priority` fills
//! the front rows first; `FormationGrouping` `Solo` units do not join; a unit with `WaitsForFormationFormup` false flies at its own speed;
//! `Rotate_Formation_Facing_Moves` turns the formation to face its move; a ship that parks less than `FinalFormationFacingMinimumAngle`
//! off that facing stays as it is. Estimates (RULES.md 9d): the escort band ahead of the big ships, the row width, the pace rule, the way
//! in to a slot (lining up behind it, turning on the spot) and the stop and hold distances.

use std::f32::consts::FRAC_PI_2;

use crate::battle::*;
use crate::defs::*;
use crate::math::{Vec3, wrap_angle};

/// Estimate: a band of a formation wider than this in one row splits into rows of equal count (20 Star Destroyers stand in 2 rows).
const ROW_WIDTH: f32 = 6000.0;
/// Estimate (as the AI line): a fighter squadron takes at least the room of a unit of radius 90 every way (more where its members' places
/// reach farther).
const SQUADRON_REACH: f32 = 90.0;
/// Estimate: rows of the AI's advancing line stand this far apart (the start layout's row spacing).
pub(crate) const ROW_GAP: f32 = 700.0;
/// Estimate: a big ship stops this close to its slot.
pub(crate) const SLOT_STOP: f32 = 20.0;
/// A fighter that holds a place (its slot, its place in the squadron) stops within this distance of it (as `mov` for every fighter slot,
/// estimate).
pub(crate) const FIGHTER_HOLD: f32 = 50.0;
/// Estimate: a ship counts as parked once it is this much farther than `SLOT_STOP` from its slot or nearer, and stays parked while it is
/// within `SLOT_HOLD` (a push by a passing hull does not send it round again).
const SLOT_ARRIVE: f32 = 10.0;
const SLOT_HOLD: f32 = 120.0;
/// Estimate: no member of a waiting formation is slowed below this share of its top speed (one that is stuck does not stop the others).
const PACE_FLOOR: f32 = 0.25;
/// Estimates: a big ship turns on the spot before it moves on when it faces more than `TURN_ON_SPOT` (radians; the angle at which a ship
/// slows to a third to turn) away from where it steers, or more than `TURN_FIRST` while it is closer to its slot than its turning radius
/// at top speed or stands at the start of the move; it then turns until it is within `TURN_ALIGNED`. So a formation sent behind itself
/// turns round where it stands instead of swinging out across the others' way, and nobody circles a slot that lies inside its turn.
pub(crate) const TURN_FIRST: f32 = 0.35;
const TURN_ON_SPOT: f32 = 1.2;
const TURN_ALIGNED: f32 = 0.05;
/// Estimate: a big ship on its way to its slot steers at the point of the slot's line along the formation's heading that lies this share
/// of the way from the ship's place along that line to the slot. It lines up behind its slot on the way (the lateral offset shrinks with
/// the square of the distance left) and comes in straight instead of from the side.
const APPROACH_SHARE: f32 = 0.5;

/// A unit's place in a destination formation: its slot, the speed it keeps (the group's pace, `f32::MAX` = its own), the formation's
/// heading, the heading it turns to while parked, whether it turns on the spot or has parked at the slot, and the formation (index in
/// `Battle::formations` this step).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MoveSlot {
    pub pos: Vec3,
    pub pace: f32,
    pub heading: f32,
    pub facing: f32,
    pub on_spot: bool,
    pub parked: bool,
    pub formation: u32,
}

impl MoveSlot {
    /// Where a unit at `pos` that flies to this slot steers on the last, straight leg: see [`approach`].
    pub(crate) fn approach(&self, pos: Vec3) -> Vec3 {
        approach(self.pos, self.heading, pos)
    }
}

/// Where a unit at `pos` that flies to `slot` of a formation facing `heading` steers on the last, straight leg: on the line through the
/// slot along the heading, `APPROACH_SHARE` of the way from the unit's place along that line to the slot, the slot itself once the unit is
/// level with it. A squadron leader comes in the same way: its squadron, which keeps the leader's heading at rest, then faces the move.
fn approach(slot: Vec3, heading: f32, pos: Vec3) -> Vec3 {
    let fwd = Vec3::from_yaw(heading);
    slot + fwd * ((pos - slot).dot(fwd) * (1.0 - APPROACH_SHARE)).min(0.0)
}

/// A group move under way: the units of one `Command::Move` / `Command::AttackMove`.
#[derive(Debug, Clone)]
pub(crate) struct Formation {
    /// The ordered point (the formation's centre), the facing of its rows and the kind of order.
    to: Vec3,
    heading: f32,
    attack: bool,
    units: Vec<FormUnit>,
}

#[derive(Debug, Clone, Copy)]
struct FormUnit {
    /// A ship, or with `squad` a fighter squadron's index in `Battle::squads` (its leader, `Battle::squad_leader`, flies the slot, the others
    /// keep their places in the squadron around it).
    id: u32,
    squad: bool,
    /// Slot offset from the formation's centre (x ahead, y to the left).
    offset: Vec3,
    /// Turning on the spot before it moves on; parked at the slot, and turning to the formation's facing there; past the first step.
    on_spot: bool,
    parked: bool,
    turning: bool,
    started: bool,
}

fn small(d: &ShipDef) -> bool {
    matches!(d.class, TargetClass::Fighter | TargetClass::Bomber)
}

/// Formation band of a class within a `FormationOrder`: the escorts (fighters, bombers, corvettes, transports) stand ahead of the frigates
/// and capital ships.
pub(crate) fn row_key(class: TargetClass) -> u8 {
    match class {
        TargetClass::Fighter | TargetClass::Bomber | TargetClass::Corvette | TargetClass::Transport => 0,
        _ => 1,
    }
}

/// Band of a unit: `FormationOrder`, then [`row_key`]; lower bands stand further ahead.
pub(crate) fn band(d: &ShipDef) -> (u8, u8) {
    (d.formation_order, row_key(d.class))
}

/// Centre distance below which the hull separation in `mov` pushes two big ships apart (0.85 of their snapshot radii, three quarters of
/// the collision radius from frigates up; an escort keeps `ESCORT_GAP` more from a capital ship), and the clearance it keeps between
/// their hulls. Fighters keep none.
fn keep_apart(a: &ShipDef, b: &ShipDef) -> (f32, f32) {
    if small(a) || small(b) {
        return (0.0, 0.0);
    }
    let radius = |d: &ShipDef| if d.class as u8 >= TargetClass::Frigate as u8 { d.radius * 0.75 } else { d.radius };
    let escort = |d: &ShipDef| matches!(d.class, TargetClass::Corvette | TargetClass::Frigate);
    let capital = |d: &ShipDef| matches!(d.class, TargetClass::Capital | TargetClass::Super | TargetClass::Structure);
    let r = radius(a) + radius(b);
    if (escort(a) && capital(b)) || (escort(b) && capital(a)) {
        ((r + crate::sim::ESCORT_GAP).max(r * 0.85), crate::sim::ESCORT_GAP)
    } else {
        (r * 0.85, 0.0)
    }
}

/// Room a unit takes from its centre: ahead, behind and to each side.
pub(crate) type Reach = (f32, f32, f32);

/// A unit to place in a [`layout`]: its definition, where it is now, and the room it takes where that is not its hull's (a fighter
/// squadron: its members' places around the leader, see [`Battle::form_up`]).
pub(crate) type Placed = (ShipId, Vec3, Option<Reach>);

/// Reach of a unit's hull ahead of its centre, behind it and to each side (the plan-view hull); a fighter squadron without its places counts as
/// round.
fn reach(d: &ShipDef) -> Reach {
    if small(d) {
        (SQUADRON_REACH, SQUADRON_REACH, SQUADRON_REACH)
    } else {
        let h = d.footprint();
        (h.front, h.back, h.half_width)
    }
}

/// How much room the units of a formation take.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Room {
    /// A group move's formation: neighbours stand `BetweenFormationSpacing` apart beyond what keeps their hulls apart.
    Tight,
    /// The AI's advancing line (`ai.rs`): the start layout's room, `radius x 2.2 + 60` across per unit and rows `ROW_GAP` apart (estimates),
    /// so that the fleet advances from its start line without closing up first (a tight line made the AI meet the enemy 5-9 s later).
    Start,
}

/// Half the room a unit takes across a row.
pub(crate) fn half_across(d: &ShipDef, room: Room) -> f32 {
    match room {
        Room::Tight => reach(d).2,
        Room::Start => ((if small(d) { SQUADRON_REACH } else { d.radius }) * 2.2 + 60.0) * 0.5,
    }
}

/// `BetweenFormationSpacing` between two neighbours, times the larger `FormationSpacing` of the two.
fn clearance(cat: &Catalog, a: &ShipDef, b: &ShipDef) -> f32 {
    let mult = |d: &ShipDef| if d.formation_spacing > 0.0 { d.formation_spacing } else { 1.0 };
    cat.formation.spacing * mult(a).max(mult(b))
}

/// Distance between the centres of two neighbours in a row (side by side, both facing the formation's way), each given by its definition
/// and the room it takes.
fn across(cat: &Catalog, (a, ra): (&ShipDef, Reach), (b, rb): (&ShipDef, Reach), room: Room) -> f32 {
    match room {
        Room::Tight => {
            let (keep, margin) = keep_apart(a, b);
            (ra.2 + rb.2 + margin).max(keep) + clearance(cat, a, b)
        }
        Room::Start => half_across(a, room) + half_across(b, room),
    }
}

/// Distance between the centre of a unit and the centre of a unit right behind it.
fn along(cat: &Catalog, (front, rf): (&ShipDef, Reach), (back, rb): (&ShipDef, Reach), room: Room) -> f32 {
    match room {
        Room::Tight => {
            let (keep, margin) = keep_apart(front, back);
            (rf.1 + rb.0 + margin).max(keep) + clearance(cat, front, back)
        }
        Room::Start => ROW_GAP,
    }
}

/// Slots for `units` in a formation facing `heading`: offsets from the formation's centre (x ahead, y to the left), in the order of
/// `units`; the units of one row get the same x. The units form bands ([`band`]); a band wider than `ROW_WIDTH` in one row splits into
/// rows of equal count, filled front to back by `Formation_Priority` and by how far ahead the units already are; every row stands in the
/// order its units stand across the line now, so nobody crosses another's way; neighbours stand [`across`] apart and each row [`along`]
/// behind the one ahead, with the `room` given.
pub(crate) fn layout(cat: &Catalog, units: &[Placed], heading: f32, room: Room) -> Vec<Vec3> {
    let n = units.len();
    if n == 0 {
        return Vec::new();
    }
    let (fwd, left) = (Vec3::from_yaw(heading), Vec3::from_yaw(heading + FRAC_PI_2));
    let centre = units.iter().fold(Vec3::ZERO, |a, u| a + u.1) * (1.0 / n as f32);
    let def = |k: usize| cat.ship(units[k].0);
    let room_of = |k: usize| (def(k), units[k].2.unwrap_or_else(|| reach(def(k))));
    let half = |k: usize| match room {
        Room::Tight => room_of(k).1.2,
        Room::Start => half_across(def(k), room),
    };
    let ahead = |k: usize| (units[k].1 - centre).dot(fwd);
    let aside = |k: usize| (units[k].1 - centre).dot(left);
    let by_aside = |a: &usize, b: &usize| aside(*a).total_cmp(&aside(*b)).then(a.cmp(b));
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| band(def(a)).cmp(&band(def(b))).then(def(b).formation_priority.cmp(&def(a).formation_priority)).then(ahead(b).total_cmp(&ahead(a))).then(a.cmp(&b)));
    let mut rows: Vec<Vec<usize>> = Vec::new();
    for members in order.chunk_by(|&a, &b| band(def(a)) == band(def(b))) {
        let mut line = members.to_vec();
        line.sort_by(by_aside);
        let width = half(line[0]) + half(line[line.len() - 1]) + line.windows(2).map(|w| across(cat, room_of(w[0]), room_of(w[1]), room)).sum::<f32>();
        let count = ((width / ROW_WIDTH).ceil() as usize).clamp(1, members.len());
        let (size, extra) = (members.len() / count, members.len() % count);
        let mut at = 0;
        for r in 0..count {
            let mut row = members[at..at + size + usize::from(r < extra)].to_vec();
            at += row.len();
            row.sort_by(by_aside);
            rows.push(row);
        }
    }
    let mut slots = vec![Vec3::ZERO; n];
    let mut x = 0.0f32;
    for (r, row) in rows.iter().enumerate() {
        if r > 0 {
            x -= rows[r - 1].iter().flat_map(|&a| row.iter().map(move |&b| (a, b))).map(|(a, b)| along(cat, room_of(a), room_of(b), room)).fold(0.0, f32::max);
        }
        match room {
            Room::Tight => {
                let mut y = vec![0.0f32; row.len()];
                for k in 1..row.len() {
                    y[k] = y[k - 1] + across(cat, room_of(row[k - 1]), room_of(row[k]), room);
                }
                let mid = 0.5 * (y[0] - half(row[0]) + y[row.len() - 1] + half(row[row.len() - 1]));
                for (k, &u) in row.iter().enumerate() {
                    slots[u] = Vec3::new(x, y[k] - mid, 0.0);
                }
            }
            // The units side by side from the row's left end (the AI line's arithmetic, so that its battles stay the same).
            Room::Start => {
                let mut y = -0.5 * row.iter().map(|&u| 2.0 * half_across(def(u), room)).sum::<f32>();
                for &u in row {
                    let half = half_across(def(u), room);
                    y += half;
                    slots[u] = Vec3::new(x, y, 0.0);
                    y += half;
                }
            }
        }
    }
    for s in &mut slots {
        s.x -= 0.5 * x;
    }
    slots
}

impl Battle {
    /// The ship that flies a unit's slot: the ship, or the leader of a squadron.
    fn unit_ship(&self, u: &FormUnit) -> Option<u32> {
        let up = |i: u32| self.ships[i as usize].alive && !self.ships[i as usize].retreated;
        if u.squad { self.squad_leader(u.id).filter(|&i| up(i)) } else { Some(u.id).filter(|&i| up(i)) }
    }

    /// The ships of a unit.
    fn unit_ships(&self, u: &FormUnit) -> Vec<u32> {
        if u.squad { self.squads[u.id as usize].clone() } else { vec![u.id] }
    }

    /// `ships` were just ordered to `to`: they leave the formations they were in, and if they are two units or more (a fighter squadron is one
    /// unit; static and `Solo` units stay out) they form up there, each unit with its own slot of [`layout`], facing the move
    /// (`Rotate_Formation_Facing_Moves`; else the way the group faces now).
    pub(crate) fn form_up(&mut self, ships: &[u32], to: Vec3, attack: bool) {
        self.leave_formations(ships);
        let cat = self.catalog.clone();
        let mut units: Vec<FormUnit> = Vec::new();
        for &i in ships {
            let s = &self.ships[i as usize];
            let d = cat.ship(s.def);
            if d.is_static || d.formation_solo {
                continue;
            }
            let (id, squad) = if small(d) && s.squad != NONE { (s.squad, true) } else { (i, false) };
            if !units.iter().any(|u| u.id == id && u.squad == squad) {
                units.push(FormUnit { id, squad, offset: Vec3::ZERO, on_spot: false, parked: false, turning: false, started: false });
            }
        }
        // A fighter squadron takes the room of its members' places around its leader (`Ship::formation`, turned with the leader), each member
        // within `FIGHTER_HOLD` of its place, at least `SQUADRON_REACH` every way, so that neighbouring squadrons do not fly into each other.
        let squad_reach = |u: &FormUnit, lead: u32| -> Option<Reach> {
            u.squad.then(|| {
                let l = &self.ships[lead as usize];
                let r = cat.ship(l.def).radius + FIGHTER_HOLD;
                self.squads[u.id as usize].iter().map(|&m| &self.ships[m as usize]).filter(|m| m.alive).fold((SQUADRON_REACH, SQUADRON_REACH, SQUADRON_REACH), |e, m| {
                    let (x, y) = (m.formation.x - l.formation.x, m.formation.y - l.formation.y);
                    (e.0.max(x + r), e.1.max(r - x), e.2.max(y.abs() + r))
                })
            })
        };
        // (unit to place, heading now)
        let placed: Vec<(Placed, f32)> = units.iter().filter_map(|u| self.unit_ship(u).map(|i| (u, i))).map(|(u, i)| ((self.ships[i as usize].def, self.ships[i as usize].pos, squad_reach(u, i)), self.ships[i as usize].yaw)).collect();
        if placed.len() < 2 || placed.len() != units.len() {
            return;
        }
        let centre = placed.iter().fold(Vec3::ZERO, |a, p| a + p.0.1) * (1.0 / placed.len() as f32);
        let heading = if cat.formation.rotate_facing && (to - centre).len() > 1.0 {
            (to - centre).y.atan2((to - centre).x)
        } else {
            let (sin, cos) = placed.iter().fold((0.0f32, 0.0f32), |a, p| (a.0 + p.1.sin(), a.1 + p.1.cos()));
            sin.atan2(cos)
        };
        let offsets = layout(&cat, &placed.iter().map(|p| p.0).collect::<Vec<_>>(), heading, Room::Tight);
        for (u, offset) in units.iter_mut().zip(offsets) {
            u.offset = offset;
        }
        self.formations.push(Formation { to, heading, attack, units });
    }

    /// Takes `ships` out of the destination formations they belong to (they got a new order).
    pub(crate) fn leave_formations(&mut self, ships: &[u32]) {
        let mut cleared: Vec<u32> = ships.to_vec();
        let mut formations = std::mem::take(&mut self.formations);
        for f in &mut formations {
            f.units.retain(|u| {
                let all = self.unit_ships(u);
                let stays = !all.iter().any(|i| ships.contains(i));
                if !stays {
                    cleared.extend(all);
                }
                stays
            });
        }
        formations.retain(|f| !f.units.is_empty());
        self.formations = formations;
        for i in cleared {
            self.ships[i as usize].move_slot = None;
        }
    }

    /// Every step: drops the units of the destination formations that died, left or got another order, and gives every other unit its
    /// slot, its pace and the facing it turns to once parked. The pace brings the waiting units in together: each flies the share of its
    /// top speed it needs to arrive when the last one can (turning on the spot counts; a fighting unit and a parked one do not hold the
    /// group up).
    pub(crate) fn move_formations(&mut self) {
        if self.formations.is_empty() {
            return;
        }
        let cat = self.catalog.clone();
        let mut formations = std::mem::take(&mut self.formations);
        let mut gone: Vec<u32> = Vec::new();
        for f in &mut formations {
            let order = if f.attack { Order::AttackMove(f.to) } else { Order::Move(f.to) };
            f.units.retain(|u| {
                let keep = self.unit_ship(u).is_some_and(|i| self.ships[i as usize].order == order);
                if !keep {
                    gone.extend(self.unit_ships(u));
                }
                keep
            });
        }
        for i in gone {
            self.ships[i as usize].move_slot = None;
        }
        formations.retain(|f| !f.units.is_empty());
        for (index, f) in formations.iter_mut().enumerate() {
            // (ship, slot, distance to fly, seconds to turn first, top speed, waits for the group)
            let mut legs: Vec<(u32, Vec3, f32, f32, f32, bool)> = Vec::with_capacity(f.units.len());
            for u in &mut f.units {
                let i = self.unit_ship(u).expect("kept");
                let s = &self.ships[i as usize];
                let d = cat.ship(s.def);
                let slot = f.to + u.offset.rotate_yaw(f.heading);
                let dist = slot.dist(s.pos);
                if !small(d) {
                    u.parked = if u.parked { dist <= SLOT_HOLD } else { dist <= SLOT_STOP + SLOT_ARRIVE };
                }
                // Along the planned path (it ends where the ship lines up behind its slot, see `MoveSlot::approach`).
                let route = match s.path.last() {
                    Some(&last) => s.pos.dist(s.path[0]) + s.path.windows(2).map(|w| w[0].dist(w[1])).sum::<f32>() + last.dist(slot),
                    None => dist,
                };
                let bearing = (slot - s.pos).y.atan2((slot - s.pos).x);
                let turn = if small(d) || u.parked || dist <= SLOT_STOP { 0.0 } else { wrap_angle(bearing - s.yaw).abs() / d.turn_rate.max(0.01) };
                // Turning on the spot (`mov` steers at the next waypoint, else at `approach`): decided here, where it is kept from one step to
                // the next, so that the hull avoidance of a ship that starts to move cannot flip it back and forth at the limit.
                if small(d) || u.parked {
                    u.on_spot = false;
                } else {
                    let aim = if s.path.len() > 1 { s.path[0] } else { approach(slot, f.heading, s.pos) };
                    let off = if aim.dist(s.pos) > 1.0 { wrap_angle((aim - s.pos).y.atan2((aim - s.pos).x) - s.yaw).abs() } else { 0.0 };
                    let near = dist < d.max_speed / d.turn_rate.max(0.01);
                    let standing = !u.started && s.speed < 0.1 * d.max_speed.max(1.0);
                    u.on_spot = if u.on_spot { off > TURN_ALIGNED } else { off > TURN_ON_SPOT || (off > TURN_FIRST && (near || standing)) };
                }
                u.started = true;
                let fighting = f.attack && s.target != NONE && self.ships[s.target as usize].alive;
                let top = (d.max_speed * s.engine_factor).max(1.0);
                legs.push((i, slot, if u.parked { 0.0 } else { route }, turn, top, !d.formation_no_wait && !fighting));
            }
            let last = legs.iter().filter(|l| l.5).map(|l| l.3 + l.2 / l.4).fold(0.0f32, f32::max);
            for (u, &(i, slot, route, turn, top, waits)) in f.units.iter_mut().zip(&legs) {
                let pace = if waits && last > 0.0 { (route / (last - turn).max(route / top).max(1e-3)).clamp(top * PACE_FLOOR, top) } else { f32::MAX };
                let s = &self.ships[i as usize];
                // `FinalFormationFacingMinimumAngle`: a parked ship turns to the formation's facing when it is off by more, and then all the way.
                let off = wrap_angle(f.heading - s.yaw).abs();
                if !u.parked {
                    u.turning = false;
                } else if off > cat.formation.facing_min_angle {
                    u.turning = true;
                } else if off < 0.01 {
                    u.turning = false;
                }
                let facing = if u.turning { f.heading } else { s.yaw };
                let (parked, on_spot) = (u.parked, u.on_spot);
                for m in self.unit_ships(u) {
                    self.ships[m as usize].move_slot = (m == i).then_some(MoveSlot { pos: slot, pace, heading: f.heading, facing, on_spot, parked, formation: index as u32 });
                }
            }
        }
        self.formations = formations;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn unit(name: &str, class: TargetClass, radius: f32) -> ShipDef {
        ShipDef { name: name.into(), class, hull: 1000.0, radius, max_speed: 100.0, accel: 100.0, turn_rate: 1.5, population: 1, ..ShipDef::default() }
    }

    /// A battle of side 0 with `units` of the definitions, and one ship of side 1 far away.
    fn battle(defs: Vec<ShipDef>, units: &[(ShipId, u32)]) -> Battle {
        let cat = Arc::new(Catalog::from_defs(defs, vec![]));
        let side = |name: &str, units: Vec<UnitSetup>| SideSetup { name: name.into(), units, structures: vec![], ai: false };
        let mine = units.iter().map(|&(ship, count)| UnitSetup { ship, count, tag: 1 }).collect();
        let mut b = Battle::new(BattleSetup { catalog: cat, sides: vec![side("a", mine), side("b", vec![UnitSetup { ship: 0, count: 1, tag: 1 }])], config: BattleConfig { half_size: 20000.0, record_events: false, ..BattleConfig::default() } });
        for s in b.ships.iter_mut().filter(|s| s.side == 1) {
            s.pos = Vec3::new(60000.0, 0.0, 0.0);
        }
        b
    }

    #[test]
    fn a_wide_band_stands_in_rows_of_equal_count_behind_the_escorts() {
        let cat = Catalog::from_defs(vec![unit("big", TargetClass::Capital, 400.0), unit("escort", TargetClass::Corvette, 40.0)], vec![]);
        // 20 big ships side by side and 3 escorts, all across the line now (x = 0).
        let units: Vec<Placed> = (0..23).map(|k| (ShipId::from(k >= 20), Vec3::new(0.0, 700.0 * k as f32, 0.0), None)).collect();
        let slots = layout(&cat, &units, 0.0, Room::Tight);
        let mut rows: Vec<f32> = slots.iter().map(|s| s.x).collect();
        rows.sort_by(|a, b| b.total_cmp(a));
        rows.dedup();
        assert_eq!(rows.len(), 3, "the escorts' row and the big ships in two rows: {slots:?}");
        let in_row = |x: f32| slots.iter().filter(|s| s.x == x).count();
        assert_eq!((in_row(rows[0]), in_row(rows[1]), in_row(rows[2])), (3, 10, 10));
        assert!(slots[20..].iter().all(|s| s.x == rows[0]), "the escorts stand ahead");
        // Neighbours: 0.85 of the two snapshot radii (0.75 x 400 each) keep the hulls apart, plus `BetweenFormationSpacing`.
        let mut row: Vec<f32> = slots.iter().filter(|s| s.x == rows[1]).map(|s| s.y).collect();
        row.sort_by(f32::total_cmp);
        assert!(row.windows(2).all(|w| (w[1] - w[0] - (0.85 * 600.0 + 50.0)).abs() < 0.1), "{row:?}");
        // The order across the line is kept: the units further left get the slots further left.
        assert!(slots[..10].windows(2).chain(slots[10..20].windows(2)).all(|w| w[0].x != w[1].x || w[0].y < w[1].y), "{slots:?}");
    }

    #[test]
    fn the_start_room_is_the_ai_lines_room() {
        let cat = Catalog::from_defs(vec![unit("big", TargetClass::Capital, 400.0), unit("escort", TargetClass::Corvette, 40.0)], vec![]);
        let (big, escort) = (cat.ship(0), cat.ship(1));
        assert_eq!(across(&cat, (big, reach(big)), (escort, reach(escort)), Room::Start), (400.0 * 2.2 + 60.0) * 0.5 + (40.0 * 2.2 + 60.0) * 0.5);
        assert_eq!(along(&cat, (escort, reach(escort)), (big, reach(big)), Room::Start), ROW_GAP);
    }

    #[test]
    fn solo_units_stay_out_and_a_new_order_takes_a_unit_out() {
        let mut solo = unit("solo", TargetClass::Frigate, 50.0);
        solo.formation_solo = true;
        let mut b = battle(vec![unit("a", TargetClass::Frigate, 50.0), solo], &[(0, 3), (1, 1)]);
        let mine: Vec<u32> = b.ships.iter().filter(|s| s.side == 0).map(|s| s.id).collect();
        b.command(0, Command::Move { ships: mine.clone(), to: Vec3::new(-5000.0, 0.0, 0.0) }).unwrap();
        b.step();
        assert_eq!(b.formations.len(), 1);
        assert_eq!(b.formations[0].units.len(), 3, "the Solo unit does not join");
        let solo_id = mine.iter().copied().find(|&i| b.ships[i as usize].def == 1).unwrap();
        assert!(mine.iter().all(|&i| b.ships[i as usize].move_slot.is_some() == (i != solo_id)));
        // One unit stops: it leaves the formation, the others keep their slots.
        b.command(0, Command::Stop { ships: vec![mine[0]] }).unwrap();
        b.step();
        assert_eq!(b.formations[0].units.len(), 2);
        assert!(b.ships[mine[0] as usize].move_slot.is_none() && b.ships[mine[1] as usize].move_slot.is_some());
        // A single unit is not a formation.
        b.command(0, Command::Move { ships: vec![mine[0]], to: Vec3::ZERO }).unwrap();
        b.step();
        assert_eq!(b.formations.len(), 1);
        assert!(b.ships[mine[0] as usize].move_slot.is_none());
    }

    #[test]
    fn a_squadron_is_one_unit_with_the_room_of_its_members() {
        // Three squadrons of three fighters 150 apart side by side: each squadron gets one slot, flown by its leader (the middle fighter),
        // and neighbouring squadrons stand far enough apart for their members: 2 x (150 + 10 + `FIGHTER_HOLD` 50) + `BetweenFormationSpacing` 50.
        let offsets = vec![Vec3::ZERO, Vec3::new(0.0, 150.0, 0.0), Vec3::new(0.0, -150.0, 0.0)];
        let squad = ShipDef { name: "squad".into(), is_squadron: true, members: vec![(0, 3)], class: TargetClass::Fighter, offsets, ..ShipDef::default() };
        let mut b = battle(vec![unit("fighter", TargetClass::Fighter, 10.0), squad], &[(1, 3)]);
        let mine: Vec<u32> = b.ships.iter().filter(|s| s.side == 0).map(|s| s.id).collect();
        assert_eq!(mine.len(), 9);
        b.command(0, Command::Move { ships: mine, to: Vec3::new(-5000.0, 0.0, 0.0) }).unwrap();
        b.step();
        let f = &b.formations[0];
        assert_eq!(f.units.len(), 3, "one unit per squadron");
        let holders: Vec<&Ship> = b.ships.iter().filter(|s| s.move_slot.is_some()).collect();
        assert_eq!(holders.len(), 3, "one ship of each squadron flies its slot");
        assert!(holders.iter().all(|s| b.squad_leader(s.squad) == Some(s.id)), "the squadron's leader");
        let left = Vec3::from_yaw(f.heading + FRAC_PI_2);
        let mut across: Vec<f32> = holders.iter().map(|s| (s.move_slot.expect("slot").pos - f.to).dot(left)).collect();
        across.sort_by(f32::total_cmp);
        assert!(across.windows(2).all(|w| (w[1] - w[0] - (2.0 * (150.0 + 10.0 + FIGHTER_HOLD) + 50.0)).abs() < 0.1), "{across:?}");
    }
}
