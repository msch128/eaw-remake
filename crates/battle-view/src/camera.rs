//! EaW tactical camera: an angled view over the battle plane with wheel zoom, edge / keyboard
//! panning and rotation, clamped to the map bounds.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowMode};

use crate::scene::SkyFollow;
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
            pitch: 50f32.to_radians(),
            bounds_min: Vec2::splat(-10_000.0),
            bounds_max: Vec2::splat(10_000.0),
            min_distance: 500.0,
            max_distance: 16_000.0,
            snap: false,
        }
    }
}

impl BattleCamera {
    /// Centres the camera on a point and picks a distance that shows a sphere of `radius`.
    pub fn frame(&mut self, center: Vec3, radius: f32, fov: f32) {
        let d = (radius * 1.15 / (fov * 0.5).tan() / self.pitch.sin().max(0.5))
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
        let pitch = (self.pitch - (1.0 - zoom_t) * 0.18).clamp(0.2, 1.45);
        let rot = Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(-pitch);
        (self.focus + rot * Vec3::Z * self.distance, rot)
    }
}

const ZOOM_STEP: f32 = 0.86;
const ZOOM_SMOOTH: f32 = 10.0;
const PAN_SMOOTH: f32 = 12.0;
const KEY_PAN_SPEED: f32 = 0.9;
const EDGE_PIXELS: f32 = 3.0;
const ROTATE_SPEED: f32 = 1.6;

/// Reads keyboard, edge scrolling, wheel and drag input.
#[allow(clippy::too_many_arguments)]
pub fn camera_input(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    mut cam: Single<(&mut BattleCamera, &Projection)>,
) {
    let dt = time.delta_secs().min(0.1);
    let (cam, projection) = &mut *cam;

    let notches = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
    };
    if notches != 0.0 {
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
    let fast = if keys.pressed(KeyCode::ShiftLeft) { 2.5 } else { 1.0 };
    let mut pan = (right * dir.x + forward * dir.y) * KEY_PAN_SPEED * cam.distance * dt * fast;

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
                / cam.pitch.sin().max(0.3);
            cam.target_focus += grab;
            cam.focus += grab;
        } else {
            cam.target_yaw -= motion.delta.x * 0.005;
            cam.yaw = cam.target_yaw;
        }
    }
    if pan != Vec3::ZERO {
        cam.target_focus += std::mem::take(&mut pan);
    }
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
        cam.yaw += (cam.target_yaw - cam.yaw) * pan_k;
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
