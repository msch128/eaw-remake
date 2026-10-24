//! Picking a fleet up: while the player drags a fleet, the fleet under the press hangs exactly at the cursor (the centre of its model on
//! the cursor ray, a little above the map and in front of the planet it is held over) and the rest of the carried group beside it, like
//! dragging a picture from one folder to another. Everything keeps its world size at every zoom (nothing in the world changes size with
//! the zoom). The parked spot is marked by a dashed ring with a tether to the carried fleet, and on drop the fleet glides to where the
//! simulation puts it. A fleet that was just clicked pops once. All numbers are estimates: no video frame of the Remake shows a drag.

use std::collections::{BTreeSet, HashMap};
use std::f32::consts::PI;

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use galaxy_sim::FleetId;

use crate::geometry::GalaxyLayout;
use crate::input::DragState;
use crate::state::{PlayerFaction, PlayerSelection, SimState};
use crate::visuals::{FLEET_MODEL_LIFT, FleetEntities, FleetVisual, map_color};

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

/// Exponential rate (per second) of the pickup: the carried model grows to [`CARRY_SCALE`] within about a fifth of a second.
const LIFT_RATE: f32 = 22.0;
/// Rate of the glide to the parking spot after the drop.
const DROP_RATE: f32 = 14.0;
/// Model scale of a carried fleet (it is "lifted"), the same at every zoom.
const CARRY_SCALE: f32 = 1.5;
/// Height of the carried model's centre above the map, in pad radii (where the centre was before at the carry scale).
const CARRY_HEIGHT: f32 = 2.2;
/// Gap between neighbouring carried fleets, in map units.
const CARRY_STEP: f32 = 1.3;
/// Seconds of the click pop and its extra scale at the peak.
const POP_SECONDS: f32 = 0.3;
const POP_SCALE: f32 = 0.28;

/// A lift this close to 1 counts as over (and is dropped), so a finished fleet keeps no entry.
const LIFT_DONE: f32 = 1e-3;

impl CarryAnim {
    /// The model scale ("lift") of fleet `id` in this frame: up to [`CARRY_SCALE`] while it is `held`, back to 1 after the drop. The way
    /// back runs on its own clock ([`DROP_RATE`]) whether the fleet still glides to its parking spot or not: it used to be tied to the
    /// glide, which restarted a finished lift at the carry scale when the glide outlasted it (a drag over a long way: 0.45 s after the drop the
    /// fleet popped to 1.4 times its size in one frame, then shrank again and snapped to 1 when the glide ended).
    fn lift_of(&mut self, id: FleetId, held: bool, dt: f32) -> f32 {
        if held {
            let l = self.lift.entry(id).or_insert(1.0);
            *l += (CARRY_SCALE - *l) * (1.0 - (-LIFT_RATE * dt).exp());
            return *l;
        }
        let Some(l) = self.lift.get_mut(&id) else { return 1.0 };
        *l += (1.0 - *l) * (1.0 - (-DROP_RATE * dt).exp());
        if (*l - 1.0).abs() > LIFT_DONE {
            return *l;
        }
        self.lift.remove(&id);
        1.0
    }
}

/// Moves `cur` towards `target` by an exponential step that does not depend on the frame rate.
pub(crate) fn approach(cur: Vec3, target: Vec3, rate: f32, dt: f32) -> Vec3 {
    cur + (target - cur) * (1.0 - (-rate * dt).exp())
}

/// Places of the `n` carried fleets along the screen's horizontal, in steps: the held fleet (the first) at the cursor, the others
/// alternately right and left of it.
pub(crate) fn fan_steps(n: usize) -> Vec<f32> {
    (0..n).map(|i| if i % 2 == 1 { (i / 2 + 1) as f32 } else { -((i / 2) as f32) }).collect()
}

/// Distance along the ray (`origin`, unit `dir`) at which it enters the sphere (`centre`, `radius`), if it does in front of `origin`.
pub(crate) fn ray_enters_sphere(origin: Vec3, dir: Vec3, centre: Vec3, radius: f32) -> Option<f32> {
    let oc = origin - centre;
    let b = oc.dot(dir);
    let disc = b * b - (oc.length_squared() - radius * radius);
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    (t > 0.0).then_some(t)
}

/// Where the held fleet's model centre goes: on the cursor ray (`origin`, unit `dir`) at `height` above the map plane, or nearer the
/// camera on the same ray where a planet (`spheres`, grown by the model's radius) would hide it. Either way it is drawn exactly under
/// the cursor.
pub(crate) fn held_point(origin: Vec3, dir: Vec3, height: f32, spheres: impl Iterator<Item = (Vec3, f32)>) -> Option<Vec3> {
    if dir.y.abs() < 1e-6 {
        return None;
    }
    let t = (height - origin.y) / dir.y;
    if t <= 0.0 {
        return None;
    }
    let t = spheres.filter_map(|(c, r)| ray_enters_sphere(origin, dir, c, r)).fold(t, f32::min);
    Some(origin + dir * t)
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
    (cfg, hover): (Res<crate::state::PlayConfig>, Res<crate::input::PlayHover>),
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    camera: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut anim: ResMut<CarryAnim>,
    mut fleets: Query<(&FleetVisual, &mut Transform, &Children)>,
    mut pads: Query<&mut Visibility, With<crate::visuals::FleetPad>>,
) {
    let dt = time.delta_secs();
    let carrying = drag.dragging && drag.cursor_ground.is_some() && !drag.carry.is_empty();

    for &id in selection.fleets.difference(&anim.selected.clone()) {
        anim.pop.insert(id, 1.0);
    }
    anim.selected = selection.fleets.clone();

    // The cursor ray of this frame (the drag's ground point is from the input pass of the last frame) and the screen's horizontal on
    // the map, along which the carried group lines up.
    let view = camera.iter().next().and_then(|(cam, cam_tf)| {
        let ray = cam.viewport_to_world(cam_tf, window.as_ref()?.cursor_position()?).ok()?;
        let right = cam_tf.right();
        Some((ray, Vec3::new(right.x, 0.0, right.z).try_normalize().unwrap_or(Vec3::X)))
    });
    let steps = fan_steps(drag.carry.len());
    // Over a fleet pad that takes the drop, the lit pad (`input::draw_drag`) marks the spot instead of the white rings.
    let on_pad = crate::input::drop_pad(&state, player.0, &hover).is_some();
    for (visual, mut tf, children) in &mut fleets {
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
        // The base disc and rim stay behind while the fleet is lifted (they would scale up with it and read as a second selection).
        let carried = slot.is_some() && drag.cursor_ground.is_some();
        for &c in children {
            if let Ok(mut v) = pads.get_mut(c) {
                let want = if carried { Visibility::Hidden } else { Visibility::Inherited };
                if *v != want {
                    *v = want;
                }
            }
        }

        if let (Some(i), Some(ground)) = (slot, drag.cursor_ground) {
            anim.origin.insert(id, sim_pos);
            let team = map_color(&state.world, player.0, visual.owner);
            let lifted = anim.lift_of(id, true, dt);
            scale *= lifted;
            // The held fleet's model centre sits on the cursor ray, so it is drawn exactly under the pointer at any zoom and camera tilt
            // (no follow lag), in front of the planet it is held over; the others hang beside it. The root is below the model's centre
            // and the model's radius is the one `fit_fleet_models` gives it, times the lift.
            let height = visual.pad * CARRY_HEIGHT;
            let model_radius = visual.pad * 2.0 * cfg.fleet_icon_scale * 2.0 * scale;
            let spheres = layout.positions.iter().zip(&layout.radii).map(|(&c, &r)| (c, r + model_radius));
            let right = view.map_or(Vec3::X, |(_, r)| r);
            let fan = right * steps[i] * layout.unit * CARRY_STEP;
            let held = view.and_then(|(ray, _)| held_point(ray.origin, *ray.direction, height, spheres)).unwrap_or(ground + Vec3::Y * height);
            let centre = held + fan;
            let next = centre - Vec3::Y * visual.pad * FLEET_MODEL_LIFT * scale;
            anim.shown.insert(id, next);
            tf.translation = next;

            // A bright ring on the map point behind the carried fleet (under the cursor) shows that it is held there.
            let under = view.and_then(|(ray, _)| held_point(ray.origin, *ray.direction, 0.0, std::iter::empty())).unwrap_or(ground) + fan;
            if !on_pad {
                let shadow = Isometry3d::new(Vec3::new(under.x, 0.08, under.z), Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2));
                gizmos.circle(shadow, visual.pad * lifted * 1.25, Color::WHITE.with_alpha(0.85));
                gizmos.circle(shadow, visual.pad * lifted * 1.55, Color::WHITE.with_alpha(0.35));
            }
            // The spot the fleet came from: a dashed ring with a thin tether to the carried fleet.
            let spot = Vec3::new(sim_pos.x, 0.05, sim_pos.z);
            let ring = visual.pad;
            let segments = 16;
            for k in (0..segments).step_by(2) {
                let (a0, a1) = (k as f32 / segments as f32 * 2.0 * PI, (k + 1) as f32 / segments as f32 * 2.0 * PI);
                let p = |a: f32| spot + Vec3::new(a.cos() * ring, 0.0, a.sin() * ring);
                gizmos.line(p(a0), p(a1), team.with_alpha(0.7));
            }
            let end = Vec3::new(under.x, 0.05, under.z);
            let dir = (end - spot).normalize_or_zero();
            let len = (end - spot).length();
            let dash = layout.unit * 0.5;
            let mut s = ring;
            while s + dash < len {
                gizmos.line(spot + dir * s, spot + dir * (s + dash), team.with_alpha(0.45));
                s += dash * 2.0;
            }
        } else {
            // Dropped: glide from the cursor to the parking spot (or into the lane) the simulation chose, while the lift shrinks back.
            if let Some(&cur) = anim.shown.get(&id) {
                let next = approach(cur, sim_pos, DROP_RATE, dt);
                if next.distance(sim_pos) < visual.pad * 0.05 {
                    anim.shown.remove(&id);
                    anim.origin.remove(&id);
                } else {
                    anim.shown.insert(id, next);
                    tf.translation = next;
                }
            }
            scale *= anim.lift_of(id, false, dt);
        }
        tf.scale = Vec3::splat(scale);
    }
    anim.lift.retain(|id, _| entities.0.contains_key(id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_held_fleet_is_at_the_cursor_and_the_rest_hang_beside_it() {
        assert_eq!(fan_steps(1), vec![0.0]);
        assert_eq!(fan_steps(5), vec![0.0, 1.0, -1.0, 2.0, -2.0]);
        assert!(fan_steps(0).is_empty());
    }

    #[test]
    fn the_carried_model_centre_is_on_the_cursor_ray() {
        // A camera high above the map looking down and forward, like the galaxy camera.
        let origin = Vec3::new(3.0, 80.0, 60.0);
        let dir = (Vec3::new(10.0, 0.0, -5.0) - origin).normalize();
        let p = held_point(origin, dir, 2.2, std::iter::empty()).expect("the ray reaches the map");
        assert!((p.y - 2.2).abs() < 1e-4);
        // On the ray: no distance from the line through the camera along the ray.
        assert!((p - origin).cross(dir).length() < 1e-3);
        // Between the camera and the map, not behind the camera; a ray parallel to the map gives nothing.
        assert!((p - origin).dot(dir) > 0.0);
        assert!(held_point(origin, Vec3::X, 2.2, std::iter::empty()).is_none());
        assert!(held_point(origin, -dir, 2.2, std::iter::empty()).is_none());
    }

    #[test]
    fn a_planet_under_the_cursor_does_not_hide_the_carried_fleet() {
        // Aimed at the centre of a planet of radius 4: at 2.2 above the map the point would be inside the globe.
        let origin = Vec3::new(3.0, 80.0, 60.0);
        let centre = Vec3::new(10.0, 0.0, -5.0);
        let dir = (centre - origin).normalize();
        let planet = (centre, 4.0 + 1.0);
        let p = held_point(origin, dir, 2.2, std::iter::once(planet)).expect("the ray reaches the map");
        // Still on the ray (so still under the cursor), now in front of the grown globe, nearer the camera than the map point.
        assert!((p - origin).cross(dir).length() < 1e-3);
        assert!((p.distance(centre) - 5.0).abs() < 1e-3);
        assert!(p.y > 2.2);
        // A planet beside the ray changes nothing.
        let beside = held_point(origin, dir, 2.2, std::iter::once((centre + Vec3::X * 20.0, 5.0))).expect("the ray reaches the map");
        assert!((beside.y - 2.2).abs() < 1e-4);
        assert_eq!(ray_enters_sphere(origin, dir, centre + Vec3::X * 20.0, 5.0), None);
    }

    #[test]
    fn the_follow_step_does_not_depend_on_the_frame_rate() {
        let (a, b) = (Vec3::ZERO, Vec3::new(10.0, 0.0, 4.0));
        let one = approach(a, b, DROP_RATE, 0.032);
        let two = approach(approach(a, b, DROP_RATE, 0.016), b, DROP_RATE, 0.016);
        assert!(one.distance(two) < 1e-4);
    }

    #[test]
    fn the_lift_settles_within_a_fifth_of_a_second() {
        let b = Vec3::new(10.0, 0.0, 0.0);
        let mut p = Vec3::ZERO;
        for _ in 0..12 {
            p = approach(p, b, LIFT_RATE, 1.0 / 60.0);
        }
        assert!(p.distance(b) < 1.0);
    }

    #[test]
    fn the_click_pop_is_a_bump_that_starts_and_ends_at_rest() {
        assert!(pop_bump(1.0).abs() < 1e-5);
        assert!(pop_bump(0.0).abs() < 1e-5);
        assert!((pop_bump(0.5) - POP_SCALE).abs() < 1e-5);
    }

    /// The lift of fleet 7 in every frame: held for `held` seconds, then dropped for `after` seconds.
    fn lift_frames(anim: &mut CarryAnim, held: f32, after: f32, fps: f32) -> Vec<f32> {
        let dt = 1.0 / fps;
        let mut frames: Vec<f32> = (0..(held * fps) as usize).map(|_| anim.lift_of(7, true, dt)).collect();
        frames.extend((0..(after * fps) as usize).map(|_| anim.lift_of(7, false, dt)));
        frames
    }

    /// Largest change from one frame to the next, as a fraction of the size.
    fn largest_step(frames: &[f32]) -> f32 {
        frames.windows(2).map(|w| (w[1] - w[0]).abs() / w[0]).fold(0.0, f32::max)
    }

    #[test]
    fn a_dropped_fleet_shrinks_back_smoothly_and_never_grows_again() {
        // A drag over a long way: the glide to the parking spot outlasts the lift (0.45 s), which is when the old code lifted the fleet
        // again (39.5 % in one frame at 60 Hz, 0.45 s after the drop) and then snapped it to 1 when the glide ended. Every frame here is
        // asked for with the fleet still gliding: nothing but the lift's own clock may decide.
        let mut anim = CarryAnim::default();
        let frames = lift_frames(&mut anim, 1.0, 3.0, 60.0);
        let dropped = &frames[59..];
        assert!((dropped[0] - CARRY_SCALE).abs() < 0.01, "held at {}", dropped[0]);
        assert!(dropped.windows(2).all(|w| w[1] <= w[0]), "the fleet grew again after the drop: {dropped:?}");
        assert!(largest_step(dropped) < 0.08, "a step of {:.1} % in one frame", largest_step(dropped) * 100.0);
        assert_eq!(dropped.last(), Some(&1.0));
        assert!(anim.lift.is_empty(), "a fleet back at its size keeps no lift entry");
    }

    #[test]
    fn the_lift_follows_the_clock_not_the_frame_count() {
        let (mut slow, mut fast) = (CarryAnim::default(), CarryAnim::default());
        let at = |anim: &mut CarryAnim, fps: f32| *lift_frames(anim, 0.5, 0.1, fps).last().expect("frames");
        assert!((at(&mut slow, 60.0) - at(&mut fast, 240.0)).abs() < 0.01);
    }

    #[test]
    fn a_fleet_picked_up_again_while_it_shrinks_goes_on_from_its_size() {
        let mut anim = CarryAnim::default();
        let before = *lift_frames(&mut anim, 1.0, 0.1, 60.0).last().expect("frames");
        let again = anim.lift_of(7, true, 1.0 / 60.0);
        assert!(before < CARRY_SCALE - 0.05, "it had begun to shrink: {before}");
        // One frame of the pick-up's own (faster) ease from where it was, not a restart from 1 or a jump to the carry scale.
        assert!(again > before && again < before * 1.15, "{before} -> {again}");
    }

    #[test]
    fn a_fleet_that_was_never_carried_has_no_lift() {
        let mut anim = CarryAnim::default();
        assert_eq!(anim.lift_of(3, false, 1.0 / 60.0), 1.0);
        assert!(anim.lift.is_empty());
    }
}
