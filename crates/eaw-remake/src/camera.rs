//! Empire-at-War style galaxy camera: angled top-down orbit with smooth zoom and clamped panning.

use crate::flow::{AppScreen, not_paused};
use crate::galaxy_view::GalaxyCameraView;
use crate::settings::Settings;
use bevy::{
    camera::{Hdr, ImageRenderTarget, RenderTarget},
    core_pipeline::tonemapping::Tonemapping,
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    post_process::bloom::Bloom,
    prelude::*,
    window::PrimaryWindow,
};

/// Camera setup and control.
pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraBounds>()
            .add_systems(Startup, spawn_camera)
            .add_systems(
                Update,
                (control_camera.run_if(not_paused), apply_camera)
                    .chain()
                    .run_if(in_state(AppScreen::Galaxy)),
            );
    }
}

/// World-space rectangle (XZ plane) the camera focus point is clamped to.
#[derive(Resource, Debug, Clone, Copy)]
pub struct CameraBounds {
    /// Minimum focus corner (x, z).
    pub min: Vec2,
    /// Maximum focus corner (x, z).
    pub max: Vec2,
    /// Closest allowed camera distance.
    pub min_distance: f32,
    /// Farthest allowed camera distance.
    pub max_distance: f32,
    /// Distance used when (re)framing the galaxy.
    pub initial_distance: f32,
    /// Set whenever the bounds change so the camera can re-frame the galaxy.
    pub dirty: bool,
}

impl Default for CameraBounds {
    fn default() -> Self {
        Self {
            min: Vec2::splat(-100.0),
            max: Vec2::splat(100.0),
            min_distance: 40.0,
            max_distance: 650.0,
            initial_distance: 400.0,
            dirty: true,
        }
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

/// Radians of turn per pixel of middle-button drag.
const ROTATE_PER_PIXEL: f32 = 0.005;
/// Tilt limits of the galaxy view.
const MIN_PITCH: f32 = 0.35;
const MAX_PITCH: f32 = 1.45;
/// Page Up / Page Down turn speed in radians per second.
const PAGE_TURN_SPEED: f32 = 1.2;
const ZOOM_STEP: f32 = 0.88;
const ZOOM_SMOOTH: f32 = 12.0;
const PAN_SMOOTH: f32 = 14.0;
const KEY_PAN_SPEED: f32 = 1.1;
const EDGE_PIXELS: f32 = 3.0;

fn spawn_camera(mut commands: Commands, offscreen: Option<Res<crate::offscreen::OffscreenTarget>>) {
    let mut camera = commands.spawn((
        Camera3d::default(),
        IsDefaultUiCamera,
        Hdr,
        // The Alamo shaders already output display-referred colour: no tonemapping.
        Tonemapping::None,
        Bloom {
            intensity: 0.12,
            ..Bloom::NATURAL
        },
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
            pitch: GalaxyCameraView::default().pitch_deg.to_radians(), // the data's value is read when a galaxy loads (`frame_galaxy`)
        },
        Transform::default(),
    ));
    if let Some(target) = offscreen {
        camera.insert(RenderTarget::Image(ImageRenderTarget {
            handle: target.image.clone(),
            scale_factor: target.scale_factor,
        }));
    }
}

#[allow(clippy::too_many_arguments)]
fn control_camera(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut bounds: ResMut<CameraBounds>,
    settings: Res<Settings>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    mut cam: Single<(&mut GalaxyCamera, &Projection)>,
    selection: Option<Res<crate::galaxy_map::Selection>>,
    hover: Option<Res<crate::galaxy_map::Hover>>,
    view: Option<Res<crate::galaxy_view::GalaxyView>>,
    planet_view: Option<Res<crate::planet_view::PlanetViewState>>,
) {
    let dt = time.delta_secs();
    let (cam, projection) = &mut *cam;

    // Re-frame the whole galaxy when the bounds are (re)defined.
    if bounds.dirty {
        bounds.dirty = false;
        frame_galaxy(cam, &bounds, view.as_deref().map_or_else(GalaxyCameraView::default, |v| v.camera));
    }

    // Smooth zoom: each wheel notch scales the target distance.
    let notches = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
    };
    // The planet view owns the wheel while it is open or fading: the galaxy zoom it returns to stays where the player left it.
    let notches = if planet_view.is_some_and(|v| v.active || v.blend > 0.0) { 0.0 } else { notches };
    if notches != 0.0 {
        let old_distance = cam.target_distance;
        cam.target_distance = (old_distance * ZOOM_STEP.powf(notches)).clamp(bounds.min_distance, bounds.max_distance);
        let fov = match projection {
            Projection::Perspective(p) => p.fov,
            _ => 40f32.to_radians(),
        };
        let cursor = window.as_ref().and_then(|w| w.cursor_position().map(|c| (c, Vec2::new(w.width(), w.height()))));
        let hovered = view.as_deref().zip(hover.as_deref().and_then(|h| h.0)).and_then(|(v, i)| v.planets.get(i));
        // Like the original, the wheel zooms toward what is under the cursor: the point of the map under the pointer stays
        // put on screen while the camera moves in or out.
        if let Some((cursor, size)) = cursor
            && let Some(ground) = cursor_ground_point(cam.target_focus, old_distance, cam.yaw, cam.pitch, fov, cursor, size)
        {
            let ratio = cam.target_distance / old_distance.max(1e-3);
            cam.target_focus = ground + (cam.target_focus - ground) * ratio;
        }
        if notches > 0.0 {
            // Zooming in on a hovered planet also centres it; without a pointer a selected planet is homed in on.
            let planet = hovered.or_else(|| selection.as_deref().zip(view.as_deref()).and_then(|(s, v)| s.0.and_then(|i| v.planets.get(i))));
            if let Some(p) = planet {
                let w = crate::galaxy_map::galaxy_to_world(p.position);
                let k = (notches * if hovered.is_some() { 0.3 } else { 0.18 }).clamp(0.0, 1.0);
                cam.target_focus = cam.target_focus.lerp(Vec3::new(w.x, 0.0, w.z), k);
            }
        }
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

    // Screen edge scrolling (can be switched off in the options).
    let window_height = window.as_ref().map_or(1080.0, |w| w.height());
    if let Some(window) = &window
        && window.focused
        && settings.edge_scroll
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
    let speed = if keys.pressed(KeyCode::ShiftLeft) {
        2.5
    } else {
        1.0
    };
    pan += (right * dir.x + forward * dir.y) * KEY_PAN_SPEED * settings.camera_speed * cam.distance * dt * speed;

    // Middle mouse drag turns the view like the original: sideways = yaw around the focus, up/down = tilt.
    if buttons.pressed(MouseButton::Middle) && motion.delta != Vec2::ZERO {
        cam.yaw += motion.delta.x * ROTATE_PER_PIXEL;
        cam.pitch = (cam.pitch + motion.delta.y * ROTATE_PER_PIXEL).clamp(MIN_PITCH, MAX_PITCH);
    }
    // Page Up / Page Down turn the view.
    let turn = f32::from(u8::from(keys.pressed(KeyCode::PageUp))) - f32::from(u8::from(keys.pressed(KeyCode::PageDown)));
    cam.yaw += turn * PAGE_TURN_SPEED * dt;

    // Right mouse drag: grab the map. World units per pixel from the projection.
    if buttons.pressed(MouseButton::Right) && motion.delta != Vec2::ZERO {
        let fov = match projection {
            Projection::Perspective(p) => p.fov,
            _ => 40f32.to_radians(),
        };
        let world_per_px = 2.0 * cam.distance * (fov * 0.5).tan() / window_height.max(1.0);
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

/// Puts the camera on the whole galaxy: over the middle of the bounds, at the initial distance, tilted as the data says. Called when a
/// galaxy is (re)built, so the tilt the player gave the last one with the middle button does not carry over.
fn frame_galaxy(cam: &mut GalaxyCamera, bounds: &CameraBounds, data: GalaxyCameraView) {
    let center = (bounds.min + bounds.max) * 0.5;
    cam.target_focus = Vec3::new(center.x, 0.0, center.y);
    cam.focus = cam.target_focus;
    cam.target_distance = bounds.initial_distance.clamp(bounds.min_distance, bounds.max_distance);
    cam.distance = cam.target_distance;
    cam.pitch = data.pitch_deg.to_radians().clamp(MIN_PITCH, MAX_PITCH);
}

/// Camera position and rotation for a focus, distance and orbit angles. The tilt is the same at every distance: the Remake's galaxy
/// camera has one pitch (`GMC_InitialPitchAngleDegrees` -50); its flatter `GMC_ZoomedPitchAngleDegrees` (-20) belongs to the planet
/// zoom, which is the planet view's dive here (it turns the camera from this pose into the planet scene's).
fn camera_pose(focus: Vec3, distance: f32, yaw: f32, pitch: f32) -> (Vec3, Quat) {
    let rot = Quat::from_rotation_y(-yaw) * Quat::from_rotation_x(-pitch);
    (focus + rot * Vec3::Z * distance, rot)
}

/// The point of the galaxy plane (y = 0) under `cursor` (logical pixels in a window of `size`) for the given camera state.
#[allow(clippy::too_many_arguments)]
fn cursor_ground_point(focus: Vec3, distance: f32, yaw: f32, pitch: f32, fov: f32, cursor: Vec2, size: Vec2) -> Option<Vec3> {
    let (pos, rot) = camera_pose(focus, distance, yaw, pitch);
    let half = (fov * 0.5).tan();
    let ndc = Vec2::new(cursor.x / size.x.max(1.0) * 2.0 - 1.0, 1.0 - cursor.y / size.y.max(1.0) * 2.0);
    let dir = rot * Vec3::new(ndc.x * half * size.x / size.y.max(1.0), ndc.y * half, -1.0);
    (dir.y < -1e-4).then(|| pos + dir * (-pos.y / dir.y))
}

pub(crate) fn apply_camera(mut q: Query<(&GalaxyCamera, &mut Transform)>) {
    for (cam, mut t) in &mut q {
        let (translation, rotation) = camera_pose(cam.focus, cam.distance, cam.yaw, cam.pitch);
        t.rotation = rotation;
        t.translation = translation;
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    const FOV: f32 = 0.7;
    const SIZE: Vec2 = Vec2::new(1280.0, 720.0);

    #[test]
    fn the_screen_centre_looks_at_the_focus() {
        let focus = Vec3::new(30.0, 0.0, -20.0);
        let p = cursor_ground_point(focus, 300.0, 0.4, 1.0, FOV, SIZE * 0.5, SIZE).unwrap();
        assert!(p.distance(focus) < 0.01);
    }

    #[test]
    fn the_point_under_the_cursor_stays_put_while_zooming() {
        let (focus, yaw, pitch, cursor) = (Vec3::new(10.0, 0.0, 5.0), 0.3, 1.0, Vec2::new(900.0, 200.0));
        let before = cursor_ground_point(focus, 300.0, yaw, pitch, FOV, cursor, SIZE).unwrap();
        let (old, new) = (300.0, 300.0 * ZOOM_STEP.powf(3.0));
        let moved = before + (focus - before) * (new / old);
        let after = cursor_ground_point(moved, new, yaw, pitch, FOV, cursor, SIZE).unwrap();
        // The tilt is the same at every distance, so the point does not drift at all (it drifted by up to 8 % while the tilt eased with the zoom).
        assert!(after.distance(before) < 1e-3, "{before} vs {after}");
    }

    #[test]
    fn a_cursor_above_the_horizon_has_no_ground_point() {
        assert!(cursor_ground_point(Vec3::ZERO, 300.0, 0.0, 0.4, 1.4, Vec2::new(640.0, 0.0), SIZE).is_none());
    }

    /// Where `p` of the galaxy plane is on screen (logical px), the inverse of `cursor_ground_point`.
    fn screen_of(p: Vec3, focus: Vec3, distance: f32, yaw: f32, pitch: f32) -> Vec2 {
        let (pos, rot) = camera_pose(focus, distance, yaw, pitch);
        let local = rot.inverse() * (p - pos);
        let half = (FOV * 0.5).tan();
        let ndc = Vec2::new(local.x / -local.z / (half * SIZE.x / SIZE.y), local.y / -local.z / half);
        Vec2::new((ndc.x * 0.5 + 0.5) * SIZE.x, (0.5 - ndc.y * 0.5) * SIZE.y)
    }

    #[test]
    fn the_view_has_one_tilt_at_every_zoom() {
        let b = CameraBounds::default();
        let pitch = GalaxyCameraView::default().pitch_deg.to_radians();
        for k in 0..=10 {
            let distance = b.min_distance + (b.max_distance - b.min_distance) * k as f32 / 10.0;
            let (pos, rot) = camera_pose(Vec3::new(5.0, 0.0, 8.0), distance, 0.3, pitch);
            let down = (-(rot * Vec3::Z)).y.asin().to_degrees();
            assert!((down + 50.0).abs() < 1e-3, "the camera looks {down} degrees below the horizon at distance {distance}");
            assert!((pos.y - distance * pitch.sin()).abs() < 1e-3, "it hangs {} above the plane at distance {distance}", pos.y);
        }
    }

    #[test]
    fn a_circle_on_the_map_is_as_flat_as_the_tilt_says_at_every_zoom() {
        // Round planets, rings and fleet pads are laid out on the plane: at the middle of the screen a circle of the plane is sin(tilt) as tall as
        // it is wide, 0.77 at the Remake's 50 degrees (it was 0.44 at the closest zoom with the old flattening, 25 degrees).
        let pitch = GalaxyCameraView::default().pitch_deg.to_radians();
        let focus = Vec3::new(-12.0, 0.0, 30.0);
        for distance in [CameraBounds::default().min_distance, 90.0, 240.0, 650.0] {
            let at = |d: Vec3| screen_of(focus + d, focus, distance, 0.0, pitch);
            let (wide, tall) = ((at(Vec3::X * 0.5) - at(-Vec3::X * 0.5)).length(), (at(Vec3::Z * 0.5) - at(-Vec3::Z * 0.5)).length());
            let ratio = tall / wide;
            assert!((ratio - pitch.sin()).abs() < 0.01, "ratio {ratio} at distance {distance}, wanted {}", pitch.sin());
        }
    }

    #[test]
    fn screen_and_ground_agree_whatever_the_yaw() {
        let (focus, pitch) = (Vec3::new(3.0, 0.0, -7.0), 50f32.to_radians());
        for yaw in [0.0, 0.8, -2.0] {
            let p = Vec3::new(20.0, 0.0, 11.0);
            let px = screen_of(p, focus, 160.0, yaw, pitch);
            let back = cursor_ground_point(focus, 160.0, yaw, pitch, FOV, px, SIZE).unwrap();
            assert!(back.distance(p) < 1e-2, "{p} -> {px} -> {back}");
        }
    }

    #[test]
    fn a_galaxy_is_framed_at_the_tilt_of_the_data_and_the_middle_button_tilt_is_forgotten() {
        let bounds = CameraBounds { min: Vec2::new(-40.0, -10.0), max: Vec2::new(60.0, 30.0), min_distance: 40.0, max_distance: 800.0, initial_distance: 700.0, dirty: true };
        let mut cam = GalaxyCamera { focus: Vec3::new(9.0, 0.0, 9.0), target_focus: Vec3::new(9.0, 0.0, 9.0), distance: 50.0, target_distance: 50.0, yaw: 0.7, pitch: 1.3 };
        frame_galaxy(&mut cam, &bounds, GalaxyCameraView { pitch_deg: 50.0 });
        assert!((cam.pitch.to_degrees() - 50.0).abs() < 1e-4, "{}", cam.pitch.to_degrees());
        assert_eq!((cam.focus, cam.target_focus), (Vec3::new(10.0, 0.0, 10.0), Vec3::new(10.0, 0.0, 10.0)));
        assert_eq!((cam.distance, cam.target_distance), (700.0, 700.0));
        // Another value in the data is followed, within the limits the middle button has.
        frame_galaxy(&mut cam, &bounds, GalaxyCameraView { pitch_deg: 35.0 });
        assert!((cam.pitch.to_degrees() - 35.0).abs() < 1e-4);
        frame_galaxy(&mut cam, &bounds, GalaxyCameraView { pitch_deg: 89.0 });
        assert_eq!(cam.pitch, MAX_PITCH);
    }
}
