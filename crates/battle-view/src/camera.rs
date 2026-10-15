//! EaW tactical camera: an angled view over the battle plane with wheel zoom, edge / keyboard
//! panning and rotation, clamped to the map bounds.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowMode};

use crate::control::{FocusCamera, Selection};
use crate::scene::SkyFollow;
use crate::ships::RenderedShips;
use alamo_render::AlamoModelLoaded;

/// Orbit camera state; the [`Transform`] is derived from it every frame.
#[derive(Component, Debug, Clone)]
pub struct BattleCamera {
    /// Point on the battle plane the camera looks at (smoothed).
    pub focus: Vec3,
    /// Focus the smoothed value converges to.
    pub target_focus: Vec3,
    /// Distance from the focus (smoothed).
    pub distance: f32,
    /// Distance the zoom converges to.
    pub target_distance: f32,
    /// Rotation about the vertical axis in radians (0 looks along map +Y, i.e. Bevy -Z).
    pub yaw: f32,
    /// Yaw the rotation converges to.
    pub target_yaw: f32,
    /// Elevation above the plane in radians at medium zoom.
    pub pitch: f32,
    /// Allowed focus rectangle (Bevy x, z).
    pub bounds_min: Vec2,
    /// Allowed focus rectangle (Bevy x, z).
    pub bounds_max: Vec2,
    /// Closest zoom distance.
    pub min_distance: f32,
    /// Farthest zoom distance.
    pub max_distance: f32,
    /// Disables smoothing (offscreen runs set the pose directly).
    pub snap: bool,
}

impl Default for BattleCamera {
    fn default() -> Self {
        Self {
            focus: Vec3::ZERO,
            target_focus: Vec3::ZERO,
            distance: 4000.0,
            target_distance: 4000.0,
            yaw: 0.0,
            target_yaw: 0.0,
            // `Space_Mode` of TacticalCameras.xml: Pitch_Default 60 (0 looks across the world, 90 straight down).
            pitch: 60f32.to_radians(),
            bounds_min: Vec2::splat(-10_000.0),
            bounds_max: Vec2::splat(10_000.0),
            min_distance: 500.0,
            // `Space_Mode` allows 1000..2000 only; its overview camera (`Tactical_Overview_Distance2`) reaches 6000. The wheel stops there, so
            // zooming out never turns every ship into a dot.
            max_distance: 6_000.0,
            snap: false,
        }
    }
}

impl BattleCamera {
    /// Centres the camera on a point and picks a distance that shows a sphere of `radius`.
    pub fn frame(&mut self, center: Vec3, radius: f32, fov: f32) {
        let d = (radius * 1.15 / (fov * 0.5).tan() / self.pitch.sin().abs().max(0.3))
            .clamp(self.min_distance, self.max_distance);
        self.focus = center;
        self.target_focus = center;
        self.distance = d;
        self.target_distance = d;
    }

    /// Camera position for the current pose.
    pub fn eye(&self) -> (Vec3, Quat) {
        let zoom_t = ((self.distance - self.min_distance) / (self.max_distance - self.min_distance))
            .clamp(0.0, 1.0);
        // Zooming in tilts the view flatter, like the original game.
        // Smoothly fade out the pitch adjustment near horizontal (pitch 0) to avoid any discontinuity / hitch.
        let pitch_factor = (self.pitch / 1.0).clamp(-1.0, 1.0);
        let pitch_adj = (1.0 - zoom_t) * 0.18 * pitch_factor;
        let pitch = (self.pitch - pitch_adj).clamp(-1.50, 1.50);
        let rot = Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(-pitch);
        (self.focus + rot * Vec3::Z * self.distance, rot)
    }
}

const ZOOM_STEP: f32 = 0.86;
const ZOOM_SMOOTH: f32 = 10.0;
const PAN_SMOOTH: f32 = 12.0;
/// `Tactical_Min_Scroll_Speed` / `Tactical_Max_Scroll_Speed` (Gameconstants.xml), world units per second: the
/// closest zoom scrolls at the minimum, the farthest at the maximum (the interpolation between them is an estimate).
const SCROLL_SPEED_MIN: f32 = 1000.0;
const SCROLL_SPEED_MAX: f32 = 4000.0;
/// `Push_Scroll_Speed_Modifier` (Gameconstants.xml): Shift speeds scrolling up.
const PUSH_SCROLL_MODIFIER: f32 = 2.0;
/// `Tactical_Edge_Scroll_Region` (Gameconstants.xml) in pixels.
const EDGE_PIXELS: f32 = 2.0;
const ROTATE_SPEED: f32 = 1.6;
/// Tilt limits of the base pitch in radians, changed by middle-drag up / down: allows full orbit including viewing from below (-1.50 to 1.50).
const PITCH_RANGE: (f32, f32) = (-1.50, 1.50);

/// Reads keyboard, edge scrolling, wheel and drag input.
#[allow(clippy::too_many_arguments)]
pub fn camera_input(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    armed: Res<crate::control::ArmedOrder>,
    mut cam: Single<(&mut BattleCamera, &Projection)>,
) {
    let dt = time.delta_secs().min(0.1);
    let (cam, projection) = &mut *cam;

    let notches = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
    };
    // While a reinforcement ghost is carried the wheel turns it (+-15 degrees) instead of zooming.
    if notches != 0.0 && armed.0 != Some(crate::control::ArmedKind::Reinforce) {
        cam.target_distance =
            (cam.target_distance * ZOOM_STEP.powf(notches)).clamp(cam.min_distance, cam.max_distance);
    }

    let (sin, cos) = cam.yaw.sin_cos();
    let right = Vec3::new(cos, 0.0, -sin);
    let forward = Vec3::new(-sin, 0.0, -cos);
    let key = |a: KeyCode, b: KeyCode| keys.pressed(a) || keys.pressed(b);
    let mut dir = Vec2::ZERO;
    if key(KeyCode::KeyW, KeyCode::ArrowUp) {
        dir.y += 1.0;
    }
    if key(KeyCode::KeyS, KeyCode::ArrowDown) {
        dir.y -= 1.0;
    }
    if key(KeyCode::KeyD, KeyCode::ArrowRight) {
        dir.x += 1.0;
    }
    if key(KeyCode::KeyA, KeyCode::ArrowLeft) {
        dir.x -= 1.0;
    }
    let window_height = window.as_ref().map_or(1080.0, |w| w.height());
    if let Some(window) = &window
        && window.focused
        && matches!(
            window.mode,
            WindowMode::BorderlessFullscreen(_) | WindowMode::Fullscreen(..)
        )
        && let Some(p) = window.cursor_position()
    {
        if p.x <= EDGE_PIXELS {
            dir.x -= 1.0;
        } else if p.x >= window.width() - EDGE_PIXELS {
            dir.x += 1.0;
        }
        if p.y <= EDGE_PIXELS {
            dir.y += 1.0;
        } else if p.y >= window.height() - EDGE_PIXELS {
            dir.y -= 1.0;
        }
    }
    let fast = if keys.pressed(KeyCode::ShiftLeft) { PUSH_SCROLL_MODIFIER } else { 1.0 };
    let zoom_t = ((cam.distance - cam.min_distance) / (cam.max_distance - cam.min_distance)).clamp(0.0, 1.0);
    let speed = SCROLL_SPEED_MIN + (SCROLL_SPEED_MAX - SCROLL_SPEED_MIN) * zoom_t;
    let mut pan = (right * dir.x + forward * dir.y).normalize_or_zero() * speed * dt * fast;

    // Rotate with Q / E or by dragging with the middle mouse button.
    let mut rot = 0.0;
    if keys.pressed(KeyCode::KeyQ) {
        rot -= 1.0;
    }
    if keys.pressed(KeyCode::KeyE) {
        rot += 1.0;
    }
    cam.target_yaw += rot * ROTATE_SPEED * dt;
    if buttons.pressed(MouseButton::Middle) && motion.delta != Vec2::ZERO {
        let fov = match projection {
            Projection::Perspective(p) => p.fov,
            _ => 45f32.to_radians(),
        };
        if keys.pressed(KeyCode::ControlLeft) {
            // Ctrl + middle drag grabs the map instead.
            let per_px = 2.0 * cam.distance * (fov * 0.5).tan() / window_height.max(1.0);
            let grab = (-right * motion.delta.x + forward * motion.delta.y) * per_px
                / cam.pitch.sin().abs().max(0.3);
            cam.target_focus += grab;
            cam.focus += grab;
        } else {
            cam.target_yaw -= motion.delta.x * 0.005;
            cam.yaw = cam.target_yaw;
            cam.pitch = (cam.pitch + motion.delta.y * 0.003).clamp(PITCH_RANGE.0, PITCH_RANGE.1);
        }
    }
    if pan != Vec3::ZERO {
        cam.target_focus += std::mem::take(&mut pan);
    }
}

/// The "battle cam" (`b_camera_t`, tooltip `TEXT_MOVIE_MODE`): the camera follows the selected unit (or the player's fleet) at a
/// fixed distance and slowly circles it. The game's own text says "Selecting a Unit and hitting C will make it the focus of
/// BattleCam. You can rotate around it at a fixed distance in this mode" (`TEXT_NEW_INTRO_MISC_9`).
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct ActionCam {
    /// The battle cam is on.
    pub active: bool,
    /// The followed ship; the whole fleet when `None`.
    pub focus: Option<u32>,
    /// Cinematic mode (C): lower, faster orbit than the Space battle cam.
    pub cinematic: bool,
    /// A button asked to toggle it.
    pub request: bool,
}

/// Seconds of a full turn of the battle cam (estimate).
const ACTION_CAM_TURN_SECONDS: f32 = 60.0;
/// Pitch of the battle cam in radians.
const ACTION_CAM_PITCH: f32 = 0.38;

/// Runs the battle cam: C or the camera button toggles it, any scroll key or the space bar ends it.
pub fn action_cam(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    settings: Res<crate::BattleViewSettings>,
    sel: Res<Selection>,
    ships: Res<RenderedShips>,
    mut action: ResMut<ActionCam>,
    mut cam: Single<&mut BattleCamera>,
) {
    // Space = Movie Mode (battle cam), C = cinematic camera; the same key again ends it, the other key switches mode.
    let want = if keys.just_pressed(KeyCode::KeyC) {
        Some(true)
    } else if std::mem::take(&mut action.request) || keys.just_pressed(KeyCode::Space) {
        Some(false)
    } else {
        None
    };
    let toggle = want.is_some();
    if let Some(cinematic) = want {
        action.active = !action.active || action.cinematic != cinematic;
        action.cinematic = cinematic;
    }
    if toggle {
        if action.active {
            action.focus = sel.ships.iter().copied().find(|id| ships.0.contains_key(id));
            if action.focus.is_none() {
                // Focus intense combat zone / damaged ship
                let in_combat = ships
                    .0
                    .values()
                    .filter(|s| s.side == settings.player_side)
                    .min_by(|a, b| (a.hull + a.shield).total_cmp(&(b.hull + b.shield)))
                    .map(|s| s.id);
                action.focus = in_combat.or_else(|| ships.0.keys().next().copied());
            }
        }
    }
    if !action.active {
        return;
    }
    let moving = [
        KeyCode::KeyW, KeyCode::KeyA, KeyCode::KeyS, KeyCode::KeyD,
        KeyCode::ArrowUp, KeyCode::ArrowDown, KeyCode::ArrowLeft, KeyCode::ArrowRight,
    ]
    .iter()
    .any(|k| keys.pressed(*k))
        || scroll.delta != Vec2::ZERO;
    if moving {
        action.active = false;
        return;
    }
    let (centre, distance, target_yaw) = match action.focus.and_then(|id| ships.0.get(&id)) {
        Some(s) => {
            let fast = s.squad != u32::MAX || s.radius < 40.0;
            let d = if fast {
                (s.radius * 3.5).clamp(45.0, 180.0)
            } else {
                (s.radius * 1.8).clamp(220.0, 1200.0)
            };
            let behind_yaw = s.yaw + std::f32::consts::PI * 0.9;
            (s.pos + Vec3::new(0.0, s.radius * 0.35, 0.0), d, Some(behind_yaw))
        }
        None => {
            let own: Vec<&crate::ships::ShipRender> = ships.0.values().filter(|s| s.side == settings.player_side).collect();
            if own.is_empty() {
                action.active = false;
                return;
            }
            let c = own.iter().map(|s| s.pos).sum::<Vec3>() / own.len() as f32;
            let spread = own.iter().map(|s| (s.pos - c).length()).fold(0.0f32, f32::max);
            (c, (spread * 2.2).max(cam.min_distance * 2.0), None)
        }
    };
    let dt = time.delta_secs().min(0.1);
    cam.target_focus = centre;
    cam.target_distance = distance.clamp(cam.min_distance, cam.max_distance);
    if let Some(tyaw) = target_yaw {
        let diff = (tyaw - cam.target_yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        cam.target_yaw += diff * (1.0 - (-3.0 * dt).exp()) + (std::f32::consts::TAU / ACTION_CAM_TURN_SECONDS * dt * 0.3);
    } else {
        cam.target_yaw += std::f32::consts::TAU / ACTION_CAM_TURN_SECONDS * dt * if action.cinematic { 2.0 } else { 1.0 };
    }
    let pitch = if action.cinematic { ACTION_CAM_PITCH * 0.5 } else { ACTION_CAM_PITCH };
    cam.pitch += (pitch - cam.pitch) * (1.0 - (-3.0 * dt).exp());
}

/// Camera hotkeys on the battle plane: Page Up centres on the selection (also a [`FocusCamera`] request), Home on the fleet, End resets the view.
pub fn focus_camera(
    keys: Res<ButtonInput<KeyCode>>,
    mut requests: MessageReader<FocusCamera>,
    sel: Res<Selection>,
    ships: Res<RenderedShips>,
    action: Res<ActionCam>,
    settings: Res<crate::BattleViewSettings>,
    mut cam: Single<&mut BattleCamera>,
) {
    let asked = requests.read().count() > 0;
    if action.active {
        return;
    }
    // Page Up: centre on the selection. Home: centre on the player's fleet. End: reset the view (heading, tilt, zoom) over the fleet.
    let page_up = keys.just_pressed(KeyCode::PageUp);
    let home = keys.just_pressed(KeyCode::Home);
    let end = keys.just_pressed(KeyCode::End);
    if end {
        let d = BattleCamera::default();
        cam.target_yaw = d.target_yaw;
        cam.yaw = d.yaw;
        cam.pitch = d.pitch;
        cam.target_distance = cam.target_distance.min(1300.0).max(cam.min_distance);
    }
    if home || end {
        let own: Vec<Vec3> = ships.0.values().filter(|s| s.side == settings.player_side).map(|s| s.pos).collect();
        if !own.is_empty() {
            let c = own.iter().copied().sum::<Vec3>() / own.len() as f32;
            cam.target_focus = Vec3::new(c.x, cam.target_focus.y, c.z);
        }
    }
    if !(asked || page_up) {
        return;
    }
    let pts: Vec<Vec3> = sel.ships.iter().filter_map(|id| ships.0.get(id)).map(|s| s.pos).collect();
    if pts.is_empty() {
        return;
    }
    let center = pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
    cam.target_focus = Vec3::new(center.x, cam.target_focus.y, center.z);
}

/// Smooths the camera pose, clamps it to the bounds and writes the transform.
pub fn apply_camera(
    time: Res<Time>,
    mut cam: Single<(&mut BattleCamera, &mut Transform)>,
) {
    let dt = time.delta_secs().min(0.1);
    let (cam, tf) = &mut *cam;
    cam.target_focus.x = cam.target_focus.x.clamp(cam.bounds_min.x, cam.bounds_max.x);
    cam.target_focus.z = cam.target_focus.z.clamp(cam.bounds_min.y, cam.bounds_max.y);
    if cam.snap {
        cam.focus = cam.target_focus;
        cam.distance = cam.target_distance;
        cam.yaw = cam.target_yaw;
    } else {
        let zoom_k = 1.0 - (-ZOOM_SMOOTH * dt).exp();
        let pan_k = 1.0 - (-PAN_SMOOTH * dt).exp();
        cam.distance += (cam.target_distance - cam.distance) * zoom_k;
        let yaw_diff = (cam.target_yaw - cam.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        cam.yaw += yaw_diff * pan_k;
        let delta = cam.target_focus - cam.focus;
        cam.focus += delta * pan_k;
    }
    cam.focus.x = cam.focus.x.clamp(cam.bounds_min.x, cam.bounds_max.x);
    cam.focus.z = cam.focus.z.clamp(cam.bounds_min.y, cam.bounds_max.y);
    let (eye, rot) = cam.eye();
    tf.translation = eye;
    tf.rotation = rot;
}

/// Keeps sky models centred on the camera and scaled to the far plane.
pub fn sky_follow(
    cam: Single<&Transform, (With<BattleCamera>, Without<SkyFollow>)>,
    mut sky: Query<(&SkyFollow, &AlamoModelLoaded, &mut Transform), Without<BattleCamera>>,
) {
    for (follow, loaded, mut tf) in &mut sky {
        let scale = follow.radius / loaded.radius.max(1.0);
        tf.scale = Vec3::splat(scale);
        // The model's bounding sphere is centred on `loaded.center` (model root space).
        tf.translation = cam.translation - tf.rotation * (loaded.center * scale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_battle_camera_eye_pitch_and_zoom() {
        let mut cam = BattleCamera::default();
        cam.snap = true;
        let (eye_far, _) = cam.eye();
        cam.target_distance = 1000.0;
        cam.distance = 1000.0;
        let (eye_near, _) = cam.eye();
        assert!(eye_near.y < eye_far.y, "zoomed-in camera has lower elevation");
    }
}

