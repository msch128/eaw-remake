//! Navigation for big ships: waypoint paths around static obstacles (asteroids, stations) and local steering around other hulls.
//!
//! Everything here is a pure function of its inputs, so battles stay deterministic.

use crate::math::Vec3;

/// A circular obstacle in the battle plane.
#[derive(Debug, Clone, Copy)]
pub struct Blocker {
    /// Centre.
    pub pos: Vec3,
    /// Radius.
    pub radius: f32,
}

/// Longest detour recursion: a path has at most `2^MAX_DEPTH` waypoints.
const MAX_DEPTH: u32 = 5;

fn flat(v: Vec3) -> (f32, f32) {
    (v.x, v.y)
}

/// Distance from `c` to the segment `a`-`b` in the plane and the parameter of the closest point.
fn segment_distance(a: Vec3, b: Vec3, c: Vec3) -> (f32, f32) {
    let ((ax, ay), (bx, by), (cx, cy)) = (flat(a), flat(b), flat(c));
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 < 1e-6 { 0.0 } else { (((cx - ax) * dx + (cy - ay) * dy) / len2).clamp(0.0, 1.0) };
    let (px, py) = (ax + dx * t, ay + dy * t);
    (((cx - px) * (cx - px) + (cy - py) * (cy - py)).sqrt(), t)
}

/// Waypoints from `from` to `to` (the last one is `to`) that keep `clearance` away from every blocker. Blockers that already
/// contain the start or the destination within their clearance are ignored (a ship pushed against a rock must still be able to leave,
/// and a target sitting on a station is approached, not circled).
pub fn plan_path(from: Vec3, to: Vec3, clearance: f32, blockers: &[Blocker]) -> Vec<Vec3> {
    let mut out = Vec::new();
    route(from, to, clearance, blockers, MAX_DEPTH, &mut out);
    out
}

fn route(from: Vec3, to: Vec3, clearance: f32, blockers: &[Blocker], depth: u32, out: &mut Vec<Vec3>) {
    if depth == 0 {
        out.push(to);
        return;
    }
    // First blocker along the segment.
    let mut hit: Option<(f32, usize)> = None;
    for (i, b) in blockers.iter().enumerate() {
        let need = b.radius + clearance;
        if from.dist(b.pos) < need * 0.98 || to.dist(b.pos) < need * 0.98 {
            continue;
        }
        let (d, t) = segment_distance(from, to, b.pos);
        if d < need && hit.is_none_or(|(ht, _)| t < ht) {
            hit = Some((t, i));
        }
    }
    let Some((_, i)) = hit else {
        out.push(to);
        return;
    };
    let dir = Vec3::new(to.x - from.x, to.y - from.y, 0.0).normalized();
    let n = Vec3::new(-dir.y, dir.x, 0.0);
    let (left, right) = detour_points(blockers, i, clearance, from, to, dir, n);
    let cost = |w: Vec3| from.dist(w) + w.dist(to);
    // Ties go to the right, so two ships meeting head on pass each other on the same side.
    let via = if cost(left) < cost(right) - 1.0 { left } else { right };
    route(from, via, clearance, blockers, depth - 1, out);
    route(via, to, clearance, blockers, depth - 1, out);
}

/// The two waypoints (left and right of the line) that clear blocker `i` and every blocker overlapping it with the clearance (they form one
/// wall): the wall's extent across the line of travel, at its average distance along the line.
fn detour_points(blockers: &[Blocker], i: usize, clearance: f32, from: Vec3, to: Vec3, dir: Vec3, n: Vec3) -> (Vec3, Vec3) {
    let mut members = vec![i];
    let mut grew = true;
    while grew {
        grew = false;
        for (j, o) in blockers.iter().enumerate() {
            if !members.contains(&j) && members.iter().any(|&m| blockers[m].pos.dist(o.pos) < blockers[m].radius + o.radius + 2.0 * clearance) {
                members.push(j);
                grew = true;
            }
        }
    }
    let rel = |b: &Blocker| ((b.pos.x - from.x) * dir.x + (b.pos.y - from.y) * dir.y, (b.pos.x - from.x) * n.x + (b.pos.y - from.y) * n.y);
    let margin = clearance * 1.15;
    let along = members.iter().map(|&m| rel(&blockers[m]).0).sum::<f32>() / members.len() as f32;
    let hi = members.iter().map(|&m| rel(&blockers[m]).1 + blockers[m].radius).fold(f32::MIN, f32::max) + margin;
    let lo = members.iter().map(|&m| rel(&blockers[m]).1 - blockers[m].radius).fold(f32::MAX, f32::min) - margin;
    let at = |lat: f32| Vec3::new(from.x + dir.x * along + n.x * lat, from.y + dir.y * along + n.y * lat, to.z);
    (at(hi), at(lo))
}

/// Result of [`steer_around`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Steer {
    /// Sideways correction to add to the unit vector towards the aim point (zero = clear).
    pub avoid: (f32, f32),
    /// Speed factor, below 1 when a hull is close ahead.
    pub slow: f32,
}

/// Local avoidance: looks `lookahead` units ahead along `heading` (unit vector) for hulls the ship would run into and returns a sideways
/// correction away from them. `hulls` are `(position, radius, speed along the heading is not needed)`; the ship's own entry must not be in it.
pub fn steer_around(pos: Vec3, heading: (f32, f32), radius: f32, lookahead: f32, hulls: impl Iterator<Item = Blocker>) -> Steer {
    let mut out = Steer { avoid: (0.0, 0.0), slow: 1.0 };
    let left = (-heading.1, heading.0);
    for h in hulls {
        let (rx, ry) = (h.pos.x - pos.x, h.pos.y - pos.y);
        let ahead = rx * heading.0 + ry * heading.1;
        let need = (radius + h.radius) * 1.05;
        if ahead < -need * 0.2 || ahead > lookahead + need {
            continue;
        }
        let lat = rx * left.0 + ry * left.1;
        if lat.abs() >= need * 1.25 {
            continue;
        }
        // Weight grows as the hull gets closer and more central in the path.
        let closeness = 1.0 - (ahead.max(0.0) / (lookahead + need)).clamp(0.0, 1.0);
        let centred = 1.0 - (lat.abs() / (need * 1.25)).clamp(0.0, 1.0);
        let w = closeness * (0.4 + 0.6 * centred);
        // Away from the hull; a hull dead ahead is passed on the right.
        let side = if lat > need * 0.05 { -1.0 } else if lat < -need * 0.05 { 1.0 } else { -1.0 };
        out.avoid.0 += left.0 * side * w * 2.0;
        out.avoid.1 += left.1 * side * w * 2.0;
        if ahead < need * 1.4 && lat.abs() < need {
            out.slow = out.slow.min(0.4 + 0.6 * (ahead / (need * 1.4)).clamp(0.0, 1.0));
        }
    }
    out
}

/// Plan view of a hull: a capsule along the bow axis. The hull reaches `front` ahead of the ship's centre and `back` behind it, and
/// `half_width` to each side; the capsule's core segment runs between the two ends pulled in by `half_width`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hull {
    /// Reach ahead of the centre.
    pub front: f32,
    /// Reach behind the centre.
    pub back: f32,
    /// Half of the width (the capsule radius).
    pub half_width: f32,
}

impl Hull {
    /// A round hull of radius `r`.
    pub fn round(r: f32) -> Self {
        Hull { front: r, back: r, half_width: r }
    }

    /// Farthest point of the hull from the centre.
    pub fn reach(&self) -> f32 {
        self.front.max(self.back).max(self.half_width)
    }

    /// End points of the core segment for a ship at `pos` with heading `yaw` (one point for a hull no longer than it is wide).
    fn core(&self, pos: Vec3, yaw: f32) -> ((f32, f32), (f32, f32)) {
        let (mut lo, mut hi) = (self.half_width - self.back, self.front - self.half_width);
        if lo > hi {
            let mid = (self.front - self.back) * 0.5;
            (lo, hi) = (mid, mid);
        }
        let (c, s) = (yaw.cos(), yaw.sin());
        ((pos.x + c * lo, pos.y + s * lo), (pos.x + c * hi, pos.y + s * hi))
    }
}

/// Closest points of the plane segments `p0`-`p1` and `q0`-`q1` (Ericson, Real-Time Collision Detection 5.1.9).
fn closest_points(p0: (f32, f32), p1: (f32, f32), q0: (f32, f32), q1: (f32, f32)) -> ((f32, f32), (f32, f32)) {
    let d1 = (p1.0 - p0.0, p1.1 - p0.1);
    let d2 = (q1.0 - q0.0, q1.1 - q0.1);
    let r = (p0.0 - q0.0, p0.1 - q0.1);
    let (a, e, f) = (d1.0 * d1.0 + d1.1 * d1.1, d2.0 * d2.0 + d2.1 * d2.1, d2.0 * r.0 + d2.1 * r.1);
    let (s, t) = if a <= 1e-6 && e <= 1e-6 {
        (0.0, 0.0)
    } else if a <= 1e-6 {
        (0.0, (f / e).clamp(0.0, 1.0))
    } else {
        let c = d1.0 * r.0 + d1.1 * r.1;
        if e <= 1e-6 {
            ((-c / a).clamp(0.0, 1.0), 0.0)
        } else {
            let b = d1.0 * d2.0 + d1.1 * d2.1;
            let denom = a * e - b * b;
            let mut s = if denom > 1e-6 { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
            let mut t = (b * s + f) / e;
            if t < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            }
            (s, t)
        }
    };
    ((p0.0 + d1.0 * s, p0.1 + d1.1 * s), (q0.0 + d2.0 * t, q0.1 + d2.1 * t))
}

/// How deep hull `a` (`(position, heading, hull)`) reaches into hull `b` plus a clearance `margin`, in the battle plane, and the unit
/// direction that moves `a` out of `b`; `None` when they keep the clearance. Two ships on the same spot go apart along x, `a` towards the
/// sign of `tie` (give the two ships of a pair opposite signs).
pub fn hull_overlap((a_pos, a_yaw, a): (Vec3, f32, Hull), (b_pos, b_yaw, b): (Vec3, f32, Hull), margin: f32, tie: f32) -> Option<(f32, (f32, f32))> {
    let (a0, a1) = a.core(a_pos, a_yaw);
    let (b0, b1) = b.core(b_pos, b_yaw);
    let (pa, pb) = closest_points(a0, a1, b0, b1);
    let (dx, dy) = (pa.0 - pb.0, pa.1 - pb.1);
    let d = (dx * dx + dy * dy).sqrt();
    let depth = a.half_width + b.half_width + margin - d;
    if depth <= 0.0 {
        return None;
    }
    // Crossing cores have no closest direction: out along the line between the centres.
    let dir = if d > 1e-3 {
        (dx / d, dy / d)
    } else {
        let (cx, cy) = (a_pos.x - b_pos.x, a_pos.y - b_pos.y);
        let c = (cx * cx + cy * cy).sqrt();
        if c > 1e-3 { (cx / c, cy / c) } else { (tie.signum(), 0.0) }
    };
    Some((depth, dir))
}

/// Distance of the point `p` from the core segment of hull `h` (at `pos`, heading `yaw`) in the battle plane, and the unit direction from
/// the core to the point.
pub fn core_distance(p: Vec3, h: Hull, pos: Vec3, yaw: f32) -> (f32, (f32, f32)) {
    let (c0, c1) = h.core(pos, yaw);
    let (q, _) = closest_points(c0, c1, (p.x, p.y), (p.x, p.y));
    let (dx, dy) = (p.x - q.0, p.y - q.1);
    let d = (dx * dx + dy * dy).sqrt();
    (d, if d > 1e-3 { (dx / d, dy / d) } else { (1.0, 0.0) })
}

/// The volume of a hull in the battle: its plan-view capsule and, where the model gives one, its height range (lowest and highest z
/// relative to the ship's centre).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HullVolume {
    /// Plan-view capsule.
    pub hull: Hull,
    /// Height range of the model (`None`: the hull is as tall as the battle is).
    pub z: Option<(f32, f32)>,
}

impl HullVolume {
    /// The shortest move that takes the point `p` out of this hull, at `pos` heading `yaw`, grown by `gap` on every side: sideways in the
    /// plan, or straight up or down, whichever is shortest. `None` when `p` is already outside.
    pub fn way_out(&self, p: Vec3, pos: Vec3, yaw: f32, gap: f32) -> Option<Vec3> {
        let (cd, (mut ux, mut uy)) = core_distance(p, self.hull, pos, yaw);
        let plan = self.hull.half_width + gap - cd;
        if plan <= 0.0 {
            return None;
        }
        if cd < 1e-3 {
            // On the core line itself: out to the left of the hull, not along it.
            (ux, uy) = (-yaw.sin(), yaw.cos());
        }
        let (mut best, mut by) = (plan, Vec3::new(ux * plan, uy * plan, 0.0));
        if let Some((lo, hi)) = self.z {
            let (over, under) = (pos.z + hi + gap - p.z, p.z - (pos.z + lo - gap));
            if over <= 0.0 || under <= 0.0 {
                return None;
            }
            if over < best {
                (best, by) = (over, Vec3::new(0.0, 0.0, over));
            }
            if under < best {
                by = Vec3::new(0.0, 0.0, -under);
            }
        }
        Some(by)
    }

    /// Whether `p` lies in the hull grown by `gap`.
    pub fn contains(&self, p: Vec3, pos: Vec3, yaw: f32, gap: f32) -> bool {
        self.way_out(p, pos, yaw, gap).is_some()
    }

    /// Distance from `p` to the surface of the hull at `pos` heading `yaw` (0 inside it): the plan distance to the capsule and the height
    /// distance to the slab combined, which is the exact distance to the solid.
    pub fn distance_to(&self, p: Vec3, pos: Vec3, yaw: f32) -> f32 {
        let plan = (core_distance(p, self.hull, pos, yaw).0 - self.hull.half_width).max(0.0);
        let height = self.z.map_or(0.0, |(lo, hi)| (pos.z + lo - p.z).max(p.z - (pos.z + hi)).max(0.0));
        plan.hypot(height)
    }

    /// Where a craft at `from` attacking the point `aim` of this hull runs in: the place where the line from `aim` to the craft leaves
    /// the hull grown by `gap` (the volume is convex, so there is one). `aim` itself when it lies outside the grown hull (a hardpoint on a
    /// boom), and the craft's own place when it is already inside.
    pub fn run_in(&self, aim: Vec3, from: Vec3, pos: Vec3, yaw: f32, gap: f32) -> Vec3 {
        if !self.contains(aim, pos, yaw, gap) {
            return aim;
        }
        if self.contains(from, pos, yaw, gap) {
            return from;
        }
        let (mut inside, mut outside) = (0.0f32, 1.0f32);
        for _ in 0..24 {
            let mid = 0.5 * (inside + outside);
            if self.contains(aim + (from - aim) * mid, pos, yaw, gap) {
                inside = mid;
            } else {
                outside = mid;
            }
        }
        aim + (from - aim) * outside
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rock(x: f32, y: f32, r: f32) -> Blocker {
        Blocker { pos: Vec3::new(x, y, 0.0), radius: r }
    }

    #[test]
    fn a_clear_line_is_one_waypoint() {
        let p = plan_path(Vec3::ZERO, Vec3::new(1000.0, 0.0, 0.0), 100.0, &[rock(500.0, 900.0, 200.0)]);
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn a_rock_on_the_line_is_passed_with_clearance() {
        let rocks = [rock(500.0, 0.0, 200.0)];
        let p = plan_path(Vec3::ZERO, Vec3::new(1000.0, 0.0, 0.0), 100.0, &rocks);
        assert!(p.len() >= 2, "{p:?}");
        let mut a = Vec3::ZERO;
        for &w in &p {
            assert!(segment_distance(a, w, rocks[0].pos).0 >= 299.0, "segment {a:?} -> {w:?} cuts the rock");
            a = w;
        }
        assert_eq!(*p.last().unwrap(), Vec3::new(1000.0, 0.0, 0.0));
    }

    #[test]
    fn a_wall_of_rocks_is_gone_around_without_crossing() {
        let rocks = [rock(500.0, -300.0, 250.0), rock(500.0, 0.0, 250.0), rock(500.0, 300.0, 250.0)];
        let p = plan_path(Vec3::ZERO, Vec3::new(1000.0, 0.0, 0.0), 80.0, &rocks);
        let mut a = Vec3::ZERO;
        for &w in &p {
            for r in &rocks {
                assert!(segment_distance(a, w, r.pos).0 >= r.radius - 1.0, "{a:?} -> {w:?} crosses {r:?}");
            }
            a = w;
        }
    }

    #[test]
    fn the_same_input_gives_the_same_path() {
        let rocks = [rock(500.0, 0.0, 200.0), rock(800.0, 50.0, 150.0)];
        let a = plan_path(Vec3::ZERO, Vec3::new(1200.0, 0.0, 0.0), 100.0, &rocks);
        let b = plan_path(Vec3::ZERO, Vec3::new(1200.0, 0.0, 0.0), 100.0, &rocks);
        assert_eq!(a, b);
    }

    /// The Executor's numbers: 9000 long, 3000 wide, from 230 below to 460 above its centre.
    fn executor() -> HullVolume {
        HullVolume { hull: Hull { front: 4500.0, back: 4500.0, half_width: 1500.0 }, z: Some((-230.0, 460.0)) }
    }

    #[test]
    fn a_point_in_a_hull_is_moved_out_the_shortest_way_sideways_over_or_under_it() {
        let (vol, at) = (executor(), |x: f32, y: f32, z: f32| Vec3::new(x, y, z));
        // Outside the hull grown by 40: beside it, over it, under it, beyond its bow.
        for p in [at(0.0, 1600.0, 0.0), at(0.0, 0.0, 600.0), at(0.0, 0.0, -300.0), at(4600.0, 0.0, 0.0)] {
            assert_eq!(vol.way_out(p, Vec3::ZERO, 0.0, 40.0), None, "{p:?}");
        }
        // 100 under the top: 140 up to clear it (500), not 1340 sideways.
        let up = vol.way_out(at(500.0, 200.0, 360.0), Vec3::ZERO, 0.0, 40.0).expect("inside");
        assert_eq!((up.x, up.y), (0.0, 0.0));
        assert!((up.z - 140.0).abs() < 1e-3, "{up:?}");
        // Low in the hull and near the middle: down and out under it (-270).
        let down = vol.way_out(at(0.0, 0.0, -200.0), Vec3::ZERO, 0.0, 40.0).expect("inside");
        assert!((down.z + 70.0).abs() < 1e-3 && down.x == 0.0, "{down:?}");
        // At mid height next to the wall: sideways, 90 to the grown wall at 1540.
        let side = vol.way_out(at(0.0, 1450.0, 100.0), Vec3::ZERO, 0.0, 40.0).expect("inside");
        assert!(side.z == 0.0 && (side.y - 90.0).abs() < 1e-3 && side.x.abs() < 1e-3, "{side:?}");
        // The hull turns with the ship: at 90 degrees its bow axis is +y, so a point 3000 up the y axis is inside and one 3000 along x is not.
        assert!(vol.contains(at(0.0, 3000.0, 100.0), Vec3::ZERO, std::f32::consts::FRAC_PI_2, 40.0));
        assert!(!vol.contains(at(3000.0, 0.0, 100.0), Vec3::ZERO, std::f32::consts::FRAC_PI_2, 40.0));
        // The ship's own height moves the slab with it.
        assert!(vol.contains(at(0.0, 0.0, 1000.0), at(0.0, 0.0, 800.0), 0.0, 40.0));
        assert!(!vol.contains(at(0.0, 0.0, 1000.0), at(0.0, 0.0, 400.0), 0.0, 40.0));
        // A hull without a height range is as tall as the battle: only sideways; on its core line out to its left.
        let flat = HullVolume { z: None, ..vol };
        assert_eq!(flat.way_out(Vec3::ZERO, Vec3::ZERO, 0.0, 40.0), Some(at(0.0, 1540.0, 0.0)));
        let turned = flat.way_out(Vec3::ZERO, Vec3::ZERO, std::f32::consts::FRAC_PI_2, 40.0).expect("inside");
        assert!((turned.x + 1540.0).abs() < 1e-2 && turned.y.abs() < 1e-2, "{turned:?}");
        assert!(flat.contains(at(0.0, 0.0, 5000.0), Vec3::ZERO, 0.0, 40.0));
    }

    #[test]
    fn a_craft_attacking_a_hull_runs_in_where_its_line_meets_the_grown_surface() {
        let (vol, at) = (executor(), |x: f32, y: f32, z: f32| Vec3::new(x, y, z));
        let gap = 120.0;
        let on_the_surface = |p: Vec3| !vol.contains(p, Vec3::ZERO, 0.0, gap) && vol.contains(p + (Vec3::ZERO - p).normalized() * 1.0, Vec3::ZERO, 0.0, gap);
        // From the south-west at the hull's height: on the side wall (1500 + 120 from the axis), on the line to the aim.
        let from = at(-3000.0, -4000.0, 0.0);
        let p = vol.run_in(Vec3::ZERO, from, Vec3::ZERO, 0.0, gap);
        assert!(on_the_surface(p), "{p:?}");
        assert!((p.y + 1620.0).abs() < 1.0, "the wall: {p:?}");
        assert!((p.x / p.y - from.x / from.y).abs() < 1e-3, "on the line from the aim to the craft: {p:?}");
        // From above: over the middle of the hull, 120 over its top.
        let p = vol.run_in(Vec3::ZERO, at(0.0, 0.0, 2000.0), Vec3::ZERO, 0.0, gap);
        assert!(p.x.abs() < 1e-2 && p.y.abs() < 1e-2 && (p.z - 580.0).abs() < 0.5, "{p:?}");
        // A hardpoint aim near the top surface seen from the side meets the side wall, the craft stays level with its line.
        let aim = at(200.0, 100.0, 400.0);
        let p = vol.run_in(aim, at(200.0, -5000.0, 100.0), Vec3::ZERO, 0.0, gap);
        assert!(on_the_surface(p) || !vol.contains(p, Vec3::ZERO, 0.0, gap), "{p:?}");
        assert!(p.y < -1500.0 && p.z > 100.0 && p.z < 400.0, "{p:?}");
        // An aim outside the grown hull is the aim; a craft inside the grown hull runs in where it is.
        assert_eq!(vol.run_in(at(0.0, 2000.0, 0.0), from, Vec3::ZERO, 0.0, gap), at(0.0, 2000.0, 0.0));
        assert_eq!(vol.run_in(Vec3::ZERO, at(0.0, 1600.0, 0.0), Vec3::ZERO, 0.0, gap), at(0.0, 1600.0, 0.0));
    }
}

/// A distance field over the battle area for planning around rocks and stations.
///
/// `sdf` holds, per cell centre, the distance to the nearest blocker edge (negative inside). Paths are found with A* over the cells whose
/// distance exceeds the clearance of the ship and then pulled straight again, so a ship takes the shortest way around a whole field of
/// rocks instead of weaving between detours.
#[derive(Debug, Clone)]
pub struct NavGrid {
    min: f32,
    cell: f32,
    n: usize,
    sdf: Vec<f32>,
}

impl NavGrid {
    /// Grid over `[-half, half]` squared with about `cell` sized cells. `None` when there is nothing to go around.
    pub fn build(half: f32, cell: f32, blockers: &[Blocker]) -> Option<NavGrid> {
        if blockers.is_empty() {
            return None;
        }
        let n = (((2.0 * half) / cell).ceil() as usize).clamp(8, 400);
        let cell = 2.0 * half / n as f32;
        let mut sdf = vec![f32::MAX; n * n];
        for (i, v) in sdf.iter_mut().enumerate() {
            let (cx, cy) = (-half + (i % n) as f32 * cell + cell * 0.5, -half + (i / n) as f32 * cell + cell * 0.5);
            for b in blockers {
                let d = ((cx - b.pos.x).powi(2) + (cy - b.pos.y).powi(2)).sqrt() - b.radius;
                if d < *v {
                    *v = d;
                }
            }
        }
        Some(NavGrid { min: -half, cell, n, sdf })
    }

    fn index(&self, x: f32, y: f32) -> Option<usize> {
        let (cx, cy) = (((x - self.min) / self.cell).floor() as i64, ((y - self.min) / self.cell).floor() as i64);
        (cx >= 0 && cy >= 0 && (cx as usize) < self.n && (cy as usize) < self.n).then(|| cy as usize * self.n + cx as usize)
    }

    /// Distance to the nearest blocker edge at a point (large outside the grid).
    pub fn clearance_at(&self, p: Vec3) -> f32 {
        // Bilinear between the four nearest cell centres (the field of a circle is nearly linear at this scale).
        let (fx, fy) = ((p.x - self.min) / self.cell - 0.5, (p.y - self.min) / self.cell - 0.5);
        if fx < -0.5 || fy < -0.5 || fx > self.n as f32 - 0.5 || fy > self.n as f32 - 0.5 {
            return f32::MAX;
        }
        let (x0, y0) = (fx.floor().max(0.0), fy.floor().max(0.0));
        let (tx, ty) = ((fx - x0).clamp(0.0, 1.0), (fy - y0).clamp(0.0, 1.0));
        let at = |x: f32, y: f32| self.sdf[(y as usize).min(self.n - 1) * self.n + (x as usize).min(self.n - 1)];
        let (a, b, c, d) = (at(x0, y0), at(x0 + 1.0, y0), at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0));
        (a * (1.0 - tx) + b * tx) * (1.0 - ty) + (c * (1.0 - tx) + d * tx) * ty
    }

    fn centre(&self, i: usize) -> Vec3 {
        Vec3::new(self.min + (i % self.n) as f32 * self.cell + self.cell * 0.5, self.min + (i / self.n) as f32 * self.cell + self.cell * 0.5, 0.0)
    }

    /// True if the straight segment keeps `clearance` from every blocker (sampled at half a cell).
    pub fn line_clear(&self, a: Vec3, b: Vec3, clearance: f32) -> bool {
        let len = a.dist(b);
        let steps = (len / (self.cell * 0.5)).ceil().max(1.0) as usize;
        (0..=steps).all(|k| {
            let t = k as f32 / steps as f32;
            // The field is sampled per cell; allow a quarter cell of slack.
            self.clearance_at(Vec3::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t, 0.0)) >= clearance - self.cell * 0.25
        })
    }

    /// The nearest point to `p` that keeps `clearance` from every blocker (`p` itself when it already does): a place that lies inside a
    /// rock cannot be reached, so ships steer for its edge instead.
    pub fn free_point(&self, p: Vec3, clearance: f32) -> Vec3 {
        if self.clearance_at(p) >= clearance {
            return p;
        }
        for ring in 1..=40 {
            let r = ring as f32 * self.cell;
            let mut best: Option<(f32, Vec3)> = None;
            for k in 0..16 {
                let a = k as f32 * std::f32::consts::TAU / 16.0;
                let q = Vec3::new(p.x + a.cos() * r, p.y + a.sin() * r, p.z);
                if self.clearance_at(q) >= clearance && best.is_none_or(|(c, _)| self.clearance_at(q) < c) {
                    best = Some((self.clearance_at(q), q));
                }
            }
            if let Some((_, q)) = best {
                return q;
            }
        }
        p
    }

    /// Waypoints from `from` to `to` (the last is `to`) keeping `clearance` from the blockers where the start and destination allow it.
    pub fn plan(&self, from: Vec3, to: Vec3, clearance: f32) -> Vec<Vec3> {
        // A ship pressed against a rock, or a destination inside the clearance band, must still be able to leave or arrive.
        let clearance = clearance.min(self.clearance_at(from) - 1.0).min(self.clearance_at(to) - 1.0).max(0.0);
        if self.line_clear(from, to, clearance) {
            return vec![to];
        }
        let (Some(start), Some(goal)) = (self.index(from.x, from.y), self.index(to.x, to.y)) else { return vec![to] };
        let free = |i: usize| self.sdf[i] >= clearance + self.cell * 0.35;
        let n = self.n;
        // A* over the 8-neighbour grid; ties are broken by the cell index so the result is deterministic.
        let mut g = vec![f32::MAX; n * n];
        let mut came = vec![usize::MAX; n * n];
        let mut open = std::collections::BinaryHeap::new();
        let h = |i: usize| self.centre(i).dist(to);
        g[start] = 0.0;
        open.push(std::cmp::Reverse((OrdF(h(start)), start)));
        let mut found = false;
        while let Some(std::cmp::Reverse((_, cur))) = open.pop() {
            if cur == goal {
                found = true;
                break;
            }
            let (cx, cy) = ((cur % n) as i64, (cur / n) as i64);
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let (nx, ny) = (cx + dx, cy + dy);
                    if nx < 0 || ny < 0 || nx >= n as i64 || ny >= n as i64 {
                        continue;
                    }
                    let ni = ny as usize * n + nx as usize;
                    if !free(ni) && ni != goal && ni != start {
                        continue;
                    }
                    // A diagonal step may not squeeze between two blocked cells.
                    if dx != 0 && dy != 0 && (!free(cy as usize * n + nx as usize) || !free(ny as usize * n + cx as usize)) {
                        continue;
                    }
                    let step = if dx != 0 && dy != 0 { std::f32::consts::SQRT_2 } else { 1.0 } * self.cell;
                    let ng = g[cur] + step;
                    if ng < g[ni] {
                        g[ni] = ng;
                        came[ni] = cur;
                        open.push(std::cmp::Reverse((OrdF(ng + h(ni)), ni)));
                    }
                }
            }
        }
        if !found {
            return vec![to];
        }
        let mut cells = vec![goal];
        while *cells.last().unwrap() != start {
            cells.push(came[*cells.last().unwrap()]);
        }
        cells.reverse();
        // Cell centres, then pull the string: from each anchor skip ahead to the farthest cell that is visible from it.
        let mut pts: Vec<Vec3> = cells.iter().map(|&c| self.centre(c)).collect();
        *pts.last_mut().unwrap() = to;
        let mut out = Vec::new();
        let mut anchor = from;
        let mut i = 0;
        while i < pts.len() {
            let mut far = i;
            for j in (i..pts.len()).rev() {
                if self.line_clear(anchor, pts[j], clearance) {
                    far = j;
                    break;
                }
            }
            out.push(pts[far]);
            anchor = pts[far];
            i = far + 1;
        }
        for p in &mut out {
            p.z = to.z;
        }
        out
    }
}

/// Totally ordered f32 for the open list.
#[derive(Debug, Clone, Copy, PartialEq)]
struct OrdF(f32);
impl Eq for OrdF {}
impl PartialOrd for OrdF {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for OrdF {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&o.0)
    }
}

#[cfg(test)]
mod grid_tests {
    use super::*;

    #[test]
    fn a_wall_is_gone_around_without_crossing_a_rock() {
        // A wall of rocks across x=0 from y=-3000 to y=2000; the open end is at y>2400.
        let rocks: Vec<Blocker> = (-12..=8).map(|k| Blocker { pos: Vec3::new(0.0, k as f32 * 250.0, 0.0), radius: 200.0 }).collect();
        let g = NavGrid::build(5000.0, 100.0, &rocks).unwrap();
        let from = Vec3::new(-2000.0, 0.0, 0.0);
        let to = Vec3::new(2000.0, 0.0, 0.0);
        let p = g.plan(from, to, 100.0);
        assert!(p.len() >= 2, "{p:?}");
        let mut a = from;
        for &w in &p {
            for r in &rocks {
                assert!(super::segment_distance(a, w, r.pos).0 >= r.radius - 1.0, "{a:?} -> {w:?} crosses {r:?}");
            }
            a = w;
        }
        assert_eq!(*p.last().unwrap(), to);
        assert_eq!(p, g.plan(from, to, 100.0), "deterministic");
    }

    #[test]
    fn a_ship_pressed_against_a_rock_can_leave_and_a_clear_line_stays_one_hop() {
        let g = NavGrid::build(5000.0, 100.0, &[Blocker { pos: Vec3::new(0.0, 0.0, 0.0), radius: 300.0 }]).unwrap();
        assert_eq!(g.plan(Vec3::new(0.0, 4000.0, 0.0), Vec3::new(1000.0, 4000.0, 0.0), 100.0).len(), 1);
        let p = g.plan(Vec3::new(310.0, 0.0, 0.0), Vec3::new(-2000.0, 0.0, 0.0), 150.0);
        assert!(!p.is_empty());
    }

    #[test]
    fn long_hulls_overlap_along_their_length_and_not_beyond_their_width() {
        // A 9000 long, 2700 wide hull (the Executor) and a round 200 hull.
        let big = Hull { front: 4500.0, back: 4500.0, half_width: 1350.0 };
        let small = Hull::round(200.0);
        // 3000 ahead of the centre on the bow axis is deep inside the long hull (a circle of the radius would not reach it).
        let at = |x: f32, y: f32| Vec3::new(x, y, 0.0);
        let (depth, dir) = hull_overlap((at(3000.0, 0.0), 0.0, small), (Vec3::ZERO, 0.0, big), 0.0, 1.0).expect("inside");
        assert!((depth - 1550.0).abs() < 1.0, "{depth}");
        assert!(dir.1.abs() > 0.99 || dir.0 > 0.99, "{dir:?}");
        // Beside the hull, clear of the width: no overlap; with a clearance margin it counts.
        assert!(hull_overlap((at(0.0, 1600.0), 0.0, small), (Vec3::ZERO, 0.0, big), 0.0, 1.0).is_none());
        assert!(hull_overlap((at(0.0, 1600.0), 0.0, small), (Vec3::ZERO, 0.0, big), 100.0, 1.0).is_some());
        // The hull turns with the ship: at 90 degrees the bow axis is +y.
        assert!(hull_overlap((at(0.0, 3000.0), 0.0, small), (Vec3::ZERO, std::f32::consts::FRAC_PI_2, big), 0.0, 1.0).is_some());
        assert!(hull_overlap((at(3000.0, 0.0), 0.0, small), (Vec3::ZERO, std::f32::consts::FRAC_PI_2, big), 0.0, 1.0).is_none());
        // Two parallel long hulls side by side: the push is sideways.
        let (_, dir) = hull_overlap((at(500.0, 2000.0), 0.0, big), (Vec3::ZERO, 0.0, big), 0.0, 1.0).expect("side by side");
        assert!(dir.1 > 0.99, "{dir:?}");
        // Round hulls behave like circles; two on the same spot go opposite ways by the tie sign.
        assert!((hull_overlap((at(300.0, 0.0), 0.0, small), (Vec3::ZERO, 0.0, small), 0.0, 1.0).unwrap().0 - 100.0).abs() < 1e-3);
        assert_eq!(hull_overlap((Vec3::ZERO, 0.0, small), (Vec3::ZERO, 0.0, small), 0.0, -1.0).unwrap().1, (-1.0, 0.0));
        let (d, _) = core_distance(at(0.0, 2000.0), big, Vec3::ZERO, 0.0);
        assert!((d - 2000.0).abs() < 1e-3);
    }
}
