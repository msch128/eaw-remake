//! Picking a fleet up: while the player drags a fleet, the whole group hangs at the cursor (like dragging a picture from one folder to
//! another), its parked spot is marked by a dashed ring with a tether to the carried fleet, and on drop the fleet glides to where the
//! simulation puts it. A fleet that was just clicked pops once. All numbers are estimates: no video frame of the Remake shows a drag.

use std::collections::{BTreeSet, HashMap};
use std::f32::consts::PI;

use bevy::prelude::*;
use galaxy_sim::FleetId;

use crate::geometry::GalaxyLayout;
use crate::input::DragState;
use crate::state::{PlayerFaction, PlayerSelection, SimState};
use crate::visuals::{FleetEntities, FleetVisual, map_color};

/// Per-fleet animation state of carrying, dropping and the click pop.
#[derive(Resource, Default)]
pub struct CarryAnim {
    /// Drawn position of fleets that are carried or still gliding home after a drop.
    shown: HashMap<FleetId, Vec3>,
    /// Where the simulation parks a carried fleet (drawn as the origin ring).
    origin: HashMap<FleetId, Vec3>,
    /// Smoothed model scale of the carried fleets.
    lift: HashMap<FleetId, f32>,
    /// Click pop, 1 right after the click and 0 when it is over.
    pop: HashMap<FleetId, f32>,
    /// Selection of the last frame, to find fresh clicks.
    selected: BTreeSet<FleetId>,
}

/// Exponential approach rate (per second) while a fleet follows the cursor: quick, but with a little weight so it does not jitter.
const FOLLOW_RATE: f32 = 22.0;
/// Rate of the glide to the parking spot after the drop.
const DROP_RATE: f32 = 14.0;
/// Model scale of a carried fleet (it is "lifted").
const CARRY_SCALE: f32 = 1.5;
/// Most the carried group is scaled up, and the pad radius in pixels it is scaled to when the map is zoomed out.
const CARRY_SCALE_MAX: f32 = 9.0;
const CARRY_SCREEN_RADIUS: f32 = 42.0;
/// How far above the map a carried fleet hangs, in pad radii.
const CARRY_HEIGHT: f32 = 1.1;
/// Seconds of the click pop and its extra scale at the peak.
const POP_SECONDS: f32 = 0.3;
const POP_SCALE: f32 = 0.28;

/// Moves `cur` towards `target` by an exponential step that does not depend on the frame rate.
pub(crate) fn approach(cur: Vec3, target: Vec3, rate: f32, dt: f32) -> Vec3 {
    cur + (target - cur) * (1.0 - (-rate * dt).exp())
}

/// Offsets of the `n` carried fleets around the cursor: one row, centred, `step` apart.
pub(crate) fn fan_offsets(n: usize, step: f32) -> Vec<Vec3> {
    (0..n).map(|i| Vec3::new((i as f32 - (n as f32 - 1.0) * 0.5) * step, 0.0, 0.0)).collect()
}

/// Extra scale of the click pop for a pop value that falls from 1 to 0: rises and settles back in one smooth bump.
pub(crate) fn pop_bump(pop: f32) -> f32 {
    POP_SCALE * (pop.clamp(0.0, 1.0) * PI).sin()
}

/// Runs after `sync_fleets` (which puts every fleet at its simulated spot) and overrides what the carry animation owns.
#[allow(clippy::too_many_arguments)]
pub fn carry_fleets(
    mut gizmos: Gizmos,
    time: Res<Time>,
    drag: Res<DragState>,
    selection: Res<PlayerSelection>,
    state: Res<SimState>,
    player: Res<PlayerFaction>,
    entities: Res<FleetEntities>,
    layout: Res<GalaxyLayout>,
    camera: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut anim: ResMut<CarryAnim>,
    mut fleets: Query<(&FleetVisual, &mut Transform)>,
) {
    let dt = time.delta_secs();
    let carrying = drag.dragging && drag.cursor_ground.is_some() && !drag.carry.is_empty();

    for &id in selection.fleets.difference(&anim.selected.clone()) {
        anim.pop.insert(id, 1.0);
    }
    anim.selected = selection.fleets.clone();

    let offsets = fan_offsets(drag.carry.len(), layout.unit * 1.3);
    for (visual, mut tf) in &mut fleets {
        let id = visual.id;
        let sim_pos = tf.translation;
        let slot = if carrying { drag.carry.iter().position(|&c| c == id) } else { None };
        let pop = anim.pop.get(&id).copied().unwrap_or(0.0);
        if pop > 0.0 {
            let left = pop - dt / POP_SECONDS;
            if left > 0.0 {
                anim.pop.insert(id, left);
            } else {
                anim.pop.remove(&id);
            }
        }
        let mut scale = 1.0 + pop_bump(pop);

        if let (Some(i), Some(cursor)) = (slot, drag.cursor_ground) {
            anim.origin.insert(id, sim_pos);
            let team = map_color(&state.world, player.0, visual.owner);
            let target = cursor + offsets[i] + Vec3::Y * visual.pad * CARRY_HEIGHT;
            let cur = anim.shown.get(&id).copied().unwrap_or(sim_pos);
            let next = approach(cur, target, FOLLOW_RATE, dt);
            anim.shown.insert(id, next);
            tf.translation = next;
            // At galaxy zoom a pad is a few pixels: the carried group is scaled up to a size that reads as held in the hand.
            let held = camera.iter().next().and_then(|(cam, cam_tf)| {
                let a = cam.world_to_viewport(cam_tf, next).ok()?;
                let b = cam.world_to_viewport(cam_tf, next + cam_tf.right() * visual.pad).ok()?;
                Some((CARRY_SCREEN_RADIUS / a.distance(b).max(1.0)).clamp(CARRY_SCALE, CARRY_SCALE_MAX))
            });
            let want = held.unwrap_or(CARRY_SCALE);
            let l = anim.lift.entry(id).or_insert(1.0);
            *l += (want - *l) * (1.0 - (-FOLLOW_RATE * dt).exp());
            let lifted = *l;
            scale *= *l;

            // A bright shadow ring under the carried fleet shows that it is held.
            let shadow = Isometry3d::new(Vec3::new(next.x, 0.08, next.z), Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2));
            gizmos.circle(shadow, visual.pad * lifted * 1.25, Color::WHITE.with_alpha(0.85));
            gizmos.circle(shadow, visual.pad * lifted * 1.55, team.with_alpha(0.55));
            // The spot the fleet came from: a dashed ring with a thin tether to the carried fleet.
            let spot = Vec3::new(sim_pos.x, 0.05, sim_pos.z);
            let ring = visual.pad;
            let steps = 16;
            for k in (0..steps).step_by(2) {
                let (a0, a1) = (k as f32 / steps as f32 * 2.0 * PI, (k + 1) as f32 / steps as f32 * 2.0 * PI);
                let p = |a: f32| spot + Vec3::new(a.cos() * ring, 0.0, a.sin() * ring);
                gizmos.line(p(a0), p(a1), team.with_alpha(0.7));
            }
            let dir = (next - spot).normalize_or_zero();
            let len = (next - spot).length();
            let dash = layout.unit * 0.5;
            let mut s = ring;
            while s + dash < len {
                gizmos.line(spot + dir * s, spot + dir * (s + dash), team.with_alpha(0.45));
                s += dash * 2.0;
            }
        } else if let Some(&cur) = anim.shown.get(&id) {
            // Dropped: glide from the cursor to the parking spot (or into the lane) the simulation chose.
            let next = approach(cur, sim_pos, DROP_RATE, dt);
            if next.distance(sim_pos) < visual.pad * 0.05 {
                anim.shown.remove(&id);
                anim.origin.remove(&id);
            } else {
                anim.shown.insert(id, next);
                tf.translation = next;
            }
            let l = anim.lift.entry(id).or_insert(CARRY_SCALE);
            *l += (1.0 - *l) * (1.0 - (-DROP_RATE * dt).exp());
            scale *= *l;
        } else {
            anim.lift.remove(&id);
        }
        tf.scale = Vec3::splat(scale);
    }
    anim.lift.retain(|id, l| entities.0.contains_key(id) && (*l - 1.0).abs() > 1e-3);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fan_is_centred_on_the_cursor() {
        let o = fan_offsets(3, 2.0);
        assert_eq!(o.len(), 3);
        assert!((o[0].x + 2.0).abs() < 1e-6 && o[1].x.abs() < 1e-6 && (o[2].x - 2.0).abs() < 1e-6);
        assert!(fan_offsets(1, 2.0)[0].x.abs() < 1e-6);
        assert!(fan_offsets(0, 2.0).is_empty());
    }

    #[test]
    fn the_follow_step_does_not_depend_on_the_frame_rate() {
        let (a, b) = (Vec3::ZERO, Vec3::new(10.0, 0.0, 4.0));
        let one = approach(a, b, FOLLOW_RATE, 0.032);
        let two = approach(approach(a, b, FOLLOW_RATE, 0.016), b, FOLLOW_RATE, 0.016);
        assert!(one.distance(two) < 1e-4);
    }

    #[test]
    fn the_follow_gets_close_within_a_fifth_of_a_second() {
        let b = Vec3::new(10.0, 0.0, 0.0);
        let mut p = Vec3::ZERO;
        for _ in 0..12 {
            p = approach(p, b, FOLLOW_RATE, 1.0 / 60.0);
        }
        assert!(p.distance(b) < 1.0);
    }

    #[test]
    fn the_click_pop_is_a_bump_that_starts_and_ends_at_rest() {
        assert!(pop_bump(1.0).abs() < 1e-5);
        assert!(pop_bump(0.0).abs() < 1e-5);
        assert!((pop_bump(0.5) - POP_SCALE).abs() < 1e-5);
    }
}
