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
}
