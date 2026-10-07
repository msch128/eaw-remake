//! Empire-at-War style galaxy camera: angled top-down orbit with smooth zoom and clamped panning.

use bevy::{
    camera::Hdr,
    core_pipeline::tonemapping::Tonemapping,
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    post_process::bloom::Bloom,
    prelude::*,
    window::{PrimaryWindow, WindowMode},
};

/// Camera setup and control.
pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraBounds>()
            .add_systems(Startup, spawn_camera)
            .add_systems(Update, (control_camera, apply_camera).chain());
    }
}

/// World-space rectangle (XZ plane) the camera focus point is clamped to.
#[derive(Resource, Debug, Clone, Copy)]
pub struct CameraBounds {
    /// Minimum focus corner (x, z).
    pub min: Vec2,
    /// Maximum focus corner (x, z).
    pub max: Vec2,
    /// Set whenever the bounds change so the camera can re-frame the galaxy.
    pub dirty: bool,
}

impl Default for CameraBounds {
    fn default() -> Self {
        Self { min: Vec2::splat(-100.0), max: Vec2::splat(100.0), dirty: true }
    }
}

/// Orbit camera state; the actual [`Transform`] is derived from it every frame.
#[derive(Component, Debug)]
pub struct GalaxyCamera {
    /// Point on the galaxy plane the camera looks at (smoothed value).
    pub focus: Vec3,
    /// Target focus the smoothed value converges to.
    pub target_focus: Vec3,
    /// Smoothed distance from the focus.
    pub distance: f32,
    /// Target distance (driven by the mouse wheel).
    pub target_distance: f32,
    /// Rotation around the vertical axis, radians.
    pub yaw: f32,
    /// Elevation above the galaxy plane, radians.
    pub pitch: f32,
}

const MIN_DISTANCE: f32 = 40.0;
const MAX_DISTANCE: f32 = 650.0;
const ZOOM_STEP: f32 = 0.88;
const ZOOM_SMOOTH: f32 = 12.0;
const PAN_SMOOTH: f32 = 14.0;
const KEY_PAN_SPEED: f32 = 1.1;
const EDGE_PIXELS: f32 = 3.0;

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Hdr,
        Tonemapping::TonyMcMapface,
        Bloom { intensity: 0.22, ..Bloom::NATURAL },
        Projection::Perspective(PerspectiveProjection {
            fov: 40f32.to_radians(),
            near: 1.0,
            far: 20_000.0,
            ..default()
        }),
        GalaxyCamera {
            focus: Vec3::ZERO,
            target_focus: Vec3::ZERO,
            distance: 400.0,
            target_distance: 400.0,
            yaw: 0.0,
            pitch: 58f32.to_radians(),
        },
        Transform::default(),
    ));
}

#[allow(clippy::too_many_arguments)]
fn control_camera(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut bounds: ResMut<CameraBounds>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut cam: Single<(&mut GalaxyCamera, &Projection)>,
) {
    let dt = time.delta_secs();
    let (cam, projection) = &mut *cam;

    // Re-frame the whole galaxy when the bounds are (re)defined.
    if bounds.dirty {
        bounds.dirty = false;
        let center = (bounds.min + bounds.max) * 0.5;
        cam.target_focus = Vec3::new(center.x, 0.0, center.y);
        cam.focus = cam.target_focus;
        let half = (bounds.max - bounds.min).max_element() * 0.5;
        cam.target_distance = (half * 2.3).clamp(MIN_DISTANCE, MAX_DISTANCE);
        cam.distance = cam.target_distance;
    }

    // Smooth zoom: each wheel notch scales the target distance.
    let notches = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
    };
    if notches != 0.0 {
        cam.target_distance =
            (cam.target_distance * ZOOM_STEP.powf(notches)).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    // Pan basis on the galaxy plane, rotated by yaw.
    let (sin, cos) = cam.yaw.sin_cos();
    let right = Vec3::new(cos, 0.0, sin);
    let forward = Vec3::new(sin, 0.0, -cos);
    let mut pan = Vec3::ZERO;

    // Keyboard.
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

    // Screen edge scrolling (only in fullscreen, so windowed use stays comfortable).
    if window.focused
        && matches!(window.mode, WindowMode::BorderlessFullscreen(_) | WindowMode::Fullscreen(..))
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
    let speed = if keys.pressed(KeyCode::ShiftLeft) { 2.5 } else { 1.0 };
    pan += (right * dir.x + forward * dir.y) * KEY_PAN_SPEED * cam.distance * dt * speed;

    // Right / middle mouse drag: grab the map. World units per pixel from the projection.
    if (buttons.pressed(MouseButton::Right) || buttons.pressed(MouseButton::Middle))
        && motion.delta != Vec2::ZERO
    {
        let fov = match projection {
            Projection::Perspective(p) => p.fov,
            _ => 40f32.to_radians(),
        };
        let world_per_px = 2.0 * cam.distance * (fov * 0.5).tan() / window.height().max(1.0);
        // Dragging down moves the map down, i.e. the camera moves "up" the screen.
        pan += (-right * motion.delta.x + forward * motion.delta.y) * world_per_px
            / cam.pitch.sin().max(0.3);
        // Dragging is direct manipulation: skip smoothing for the grabbed amount.
        cam.target_focus += pan;
        cam.focus += pan;
        pan = Vec3::ZERO;
    }

    cam.target_focus += pan;
    cam.target_focus.x = cam.target_focus.x.clamp(bounds.min.x, bounds.max.x);
    cam.target_focus.z = cam.target_focus.z.clamp(bounds.min.y, bounds.max.y);
    cam.focus.x = cam.focus.x.clamp(bounds.min.x, bounds.max.x);
    cam.focus.z = cam.focus.z.clamp(bounds.min.y, bounds.max.y);

    // Exponential smoothing, frame-rate independent.
    let zoom_k = 1.0 - (-ZOOM_SMOOTH * dt).exp();
    let pan_k = 1.0 - (-PAN_SMOOTH * dt).exp();
    cam.distance += (cam.target_distance - cam.distance) * zoom_k;
    let delta = cam.target_focus - cam.focus;
    cam.focus += delta * pan_k;
}

fn apply_camera(mut q: Query<(&GalaxyCamera, &mut Transform)>) {
    for (cam, mut t) in &mut q {
        // Zooming in tilts the view slightly flatter, like the original game.
        let zoom_t = ((cam.distance - MIN_DISTANCE) / (MAX_DISTANCE - MIN_DISTANCE)).clamp(0.0, 1.0);
        let pitch = cam.pitch - (1.0 - zoom_t) * 0.2;
        let rot = Quat::from_rotation_y(-cam.yaw) * Quat::from_rotation_x(-pitch);
        t.rotation = rot;
        t.translation = cam.focus + rot * Vec3::Z * cam.distance;
    }
}
