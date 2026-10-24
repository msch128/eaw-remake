//! The planet view: scrolling all the way in on the selected planet (or Esc / scrolling out to leave) flies the
//! camera into a separate 3D scene far below the galaxy: a large planet sphere on the left, the planet's star
//! base model, and the tan base-layout platform (`W_BuildBaseFOC.alo`). The 2D UI around it (orbit boxes,
//! description) hangs off [`PlanetViewState`].

use alamo_render::{AlamoFs, AlamoModel, AlamoModelLoaded};
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use galaxy_play::{PlayCatalog, SimState};

use crate::camera::{CameraBounds, GalaxyCamera};
use crate::flow::AppScreen;
use crate::galaxy_map::Selection;
use crate::galaxy_view::GalaxyView;
use crate::planet_visual::PlanetRadii;

/// Shared state of the planet view.
#[derive(Resource, Debug, Default)]
pub struct PlanetViewState {
    /// The view is open (or flying in).
    pub active: bool,
    /// Planet index in `GalaxyView::planets`.
    pub planet: Option<usize>,
    /// 0 = galaxy map, 1 = fully in the planet view (eased).
    pub blend: f32,
    /// World positions of the three rectangular frames of the upper pad (left to right); they stay empty.
    pub base_pads: Vec<Vec3>,
    /// World positions of the round ground-building slots of the lower plate, one per `Special_Structures_Land` (left to right).
    pub ground_slots: Vec<Vec3>,
    /// World positions of the round orbital-building slots left of the platform, one per `Special_Structures_Space`.
    pub orbital_slots: Vec<Vec3>,
    /// Initial camera position when zoom transition begins.
    pub start_pos: Option<Vec3>,
    /// Initial camera rotation when zoom transition begins.
    pub start_rot: Option<Quat>,
    /// Galaxy zoom (camera target distance) before the dive; leaving the view returns to it.
    pub return_distance: Option<f32>,
}

/// Registers the planet view.
pub struct PlanetViewPlugin;

impl Plugin for PlanetViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlanetViewState>().add_systems(
            Update,
            (view_input, view_scene, view_dynamic, view_camera.after(crate::camera::apply_camera), view_fade, fit_models)
                .chain()
                .run_if(in_state(AppScreen::Galaxy)),
        );
        register_tooltip(app);
        app.add_systems(Update, (wheel_test, rebuild_test).run_if(in_state(AppScreen::Galaxy)));
    }
}

/// Origin of the planet-view scene, far below the galaxy plane.
const ORIGIN: Vec3 = Vec3::new(0.0, -1000.0, 0.0);

#[derive(Component)]
struct ViewEntity;

/// A model to scale to `radius` and centre (planet, station, platform).
#[derive(Component)]
struct FitModel {
    radius: f32,
    spin: f32,
}

/// Wheel-in opens the planet view once the camera has already homed in on the selected planet: within [`OPEN_DISTANCE`] times the
/// closest zoom (about four notches of the wheel) and with the planet near the view centre.
const OPEN_DISTANCE: f32 = 2.0;

/// True if a wheel tick toward the planet should open its view. `distance` is the camera's target distance in units of the closest
/// allowed one, `offset` the focus-to-planet distance in units of the camera distance.
fn wheel_opens(notches: f32, distance: f32, offset: f32) -> bool {
    notches > 0.0 && distance <= OPEN_DISTANCE && offset <= 0.5
}

fn view_input(
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    mut selection: ResMut<Selection>,
    hover: Res<crate::galaxy_map::Hover>,
    bounds: Res<CameraBounds>,
    view: Res<GalaxyView>,
    mut cam: Single<&mut GalaxyCamera>,
    mut state: ResMut<PlanetViewState>,
    dialog: Res<SellDialog>,
) {
    if !state.active {
        // The wheel zooms toward the planet under the cursor (see `camera`), so zooming in on a hovered planet opens its view
        // without a click first.
        if scroll.delta.y > 0.0 && hover.0.is_some() && hover.0 != selection.0 && cam.target_distance <= bounds.min_distance * OPEN_DISTANCE * 2.0 {
            selection.0 = hover.0;
        }
        if std::env::var_os("SP_PLANET_VIEW").is_some() && selection.0.is_some() {
            state.active = true;
            state.planet = selection.0;
            return;
        }
        if let Some(p) = selection.0.and_then(|i| view.planets.get(i).map(|pl| (i, pl))) {
            let at = crate::galaxy_map::galaxy_to_world(p.1.position);
            let offset = cam.focus.distance(at) / cam.distance.max(1e-3);
            let distance = cam.target_distance / bounds.min_distance.max(1e-3);
            if wheel_opens(scroll.delta.y, distance, offset) {
                // The dive finishes while the view fades in: the camera keeps flying to the planet.
                cam.target_focus = at;
                state.return_distance = Some(cam.target_distance);
                cam.target_distance = bounds.min_distance;
                state.active = true;
                state.planet = Some(p.0);
            }
        }
    } else if (keys.just_pressed(KeyCode::Escape) && !dialog.is_open()) || (scroll.delta.y < 0.0 && state.blend >= 1.0) {
        // Scrolling out only leaves once the view has fully opened: a trailing wheel tick must not close it mid-flight.
        state.active = false;
        if let Some(d) = state.return_distance.take() {
            cam.target_distance = d.clamp(bounds.min_distance, bounds.max_distance);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orbital_slots_sit_left_of_the_platform_without_overlap() {
        for n in 0..=10 {
            let c = space_slots(n);
            assert_eq!(c.len(), n);
            assert!(c.iter().all(|o| o.x < -1.9), "left of the platform: {c:?}");
            for (i, a) in c.iter().enumerate() {
                assert!(c.iter().skip(i + 1).all(|b| a.distance(*b) > 0.25), "no overlap: {c:?}");
            }
        }
    }

    #[test]
    fn ground_circles_follow_the_planet_slot_count() {
        for n in 0..=8 {
            let c = circle_slots(n);
            assert_eq!(c.len(), n, "one circle per ground slot");
            assert!(c.iter().all(|o| o.x.abs() <= 1.03), "inside the plate: {c:?}");
            for (i, a) in c.iter().enumerate() {
                assert!(c.iter().skip(i + 1).all(|b| a.distance(*b) > 0.25), "circles do not overlap: {c:?}");
            }
        }
        assert!(circle_slots(4).iter().all(|o| (o.y + 0.74).abs() < 1e-6), "up to six sit in the strip");
    }

    #[test]
    fn a_small_wheel_zoom_near_the_planet_opens_the_view() {
        assert!(wheel_opens(1.0, 1.0, 0.0), "at the closest zoom");
        assert!(wheel_opens(1.0, 1.9, 0.3), "a few notches out, planet near the centre");
        assert!(!wheel_opens(1.0, 6.0, 0.0), "far away: the wheel only zooms the map");
        assert!(!wheel_opens(1.0, 1.0, 2.0), "the selected planet is not where the camera looks");
        assert!(!wheel_opens(-1.0, 1.0, 0.0), "wheel out never opens");
        assert!(!wheel_opens(0.0, 1.0, 0.0), "no wheel, no view");
    }

    #[test]
    fn the_dive_ends_where_the_real_planet_looks_like_the_scene_planet() {
        let (planet_pos, radius) = (Vec3::new(50.0, 2.0, 50.0), 3.0);
        let dive = dive_pose(planet_pos, radius, Quat::IDENTITY);
        let scene = overview_pose();
        let s = radius / SCENE_PLANET_RADIUS;
        // Same distance (in planet radii) and the same direction to the planet centre as in the scene.
        let scene_to_planet = (ORIGIN + SCENE_PLANET) - scene.translation;
        let dive_to_planet = planet_pos - dive.translation;
        assert!((dive_to_planet.length() - scene_to_planet.length() * s).abs() < 1e-4);
        assert!(dive_to_planet.normalize().dot(scene_to_planet.normalize()) > 0.9999);
        assert!(dive.rotation.dot(scene.rotation).abs() > 0.9999);
    }

    #[test]
    fn the_dive_follows_the_camera_yaw() {
        let yaw = Quat::from_rotation_y(0.7);
        let dive = dive_pose(Vec3::ZERO, 3.0, yaw);
        let plain = dive_pose(Vec3::ZERO, 3.0, Quat::IDENTITY);
        assert!((dive.translation - yaw * plain.translation).length() < 1e-4);
    }
    #[test]
    fn planet_zoom_camera_dives_onto_planet_before_overview() {
        let orbit_pos = Vec3::new(50.0, 40.0, 80.0);
        let orbit_rot = Quat::from_rotation_y(0.2);
        let planet_pos = Vec3::new(50.0, 0.0, 50.0);
        let planet_radius = 8.0;

        let at_0 = camera_transition_pose(orbit_pos, orbit_rot, planet_pos, planet_radius, 0.0);
        assert!((at_0.translation - orbit_pos).length() < 1e-4, "at blend 0 camera starts at orbit");

        let at_quarter = camera_transition_pose(orbit_pos, orbit_rot, planet_pos, planet_radius, 0.25);
        assert!(at_quarter.translation.y > 0.0, "during phase 1 camera stays in galaxy space");
        assert!(at_quarter.translation.distance(planet_pos) < orbit_pos.distance(planet_pos), "camera moves closer to planet");

        let at_1 = camera_transition_pose(orbit_pos, orbit_rot, planet_pos, planet_radius, 1.0);
        let expected_overview = Transform::from_translation(ORIGIN + Vec3::new(0.0, 0.0, -1.8))
            .looking_at(ORIGIN + Vec3::new(0.0, -1.95, -10.0), Vec3::Y);
        assert!((at_1.translation - expected_overview.translation).length() < 1e-4, "at blend 1 camera is at overview pose");
        assert!((at_1.rotation.dot(expected_overview.rotation)).abs() > 0.9999, "at blend 1 rotation matches overview");
    }

    /// Where the planet's centre is in the view of the dive at `blend` (a 40 degree 16:9 camera): `(x, y)` in -1..1 across the picture, `y` up, and the planet's
    /// radius in the same units of the picture's height; `None` behind the camera.
    fn planet_in_view(orbit_pos: Vec3, orbit_rot: Quat, planet_pos: Vec3, radius: f32, blend: f32) -> Option<(f32, f32, f32)> {
        let pose = camera_transition_pose(orbit_pos, orbit_rot, planet_pos, radius, blend);
        let local = pose.rotation.inverse() * (planet_pos - pose.translation);
        let (depth, half_v) = (-local.z, 20f32.to_radians().tan());
        (depth > 0.0).then(|| (local.x / depth / (half_v * 16.0 / 9.0), local.y / depth / half_v, radius / depth / half_v))
    }

    #[test]
    fn the_planet_stays_in_view_above_the_command_bar_through_the_dive_from_every_tilt_of_the_galaxy_camera() {
        let (planet_pos, radius) = (Vec3::new(30.0, 0.0, -20.0), 6.0);
        // The command bar covers the picture below this (the panel of the 1080p picture: the bar starts 905 px down).
        let bar_top = 1.0 - 2.0 * 905.0 / 1080.0;
        for tilt in [25.0f32, 35.0, 50.0, 65.0, 80.0] {
            // The camera hangs 80 units from the planet, as when the wheel opens the view at the closest zoom, centred on it.
            let orbit_rot = Quat::from_rotation_y(-0.4) * Quat::from_rotation_x(-tilt.to_radians());
            let orbit_pos = planet_pos + orbit_rot * Vec3::Z * 80.0;
            for step in 0..=77 {
                let blend = step as f32 / 100.0;
                let (x, y, r) = planet_in_view(orbit_pos, orbit_rot, planet_pos, radius, blend).expect("in front of the camera");
                // While the planet is small (before it swells into the scene's big planet at the left) its centre is on the free part of the picture.
                if r < 0.5 {
                    assert!(x.abs() <= 1.0 && y <= 1.0 && y >= bar_top, "tilt {tilt}: at blend {blend} the planet's centre is at ({x:.2}, {y:.2}) of the picture, the free part ends at y {bar_top:.2}");
                }
            }
        }
    }

    #[test]
    fn the_swing_keeps_its_ends_and_closes_in_evenly() {
        let (centre, from, to) = (Vec3::new(1.0, 0.0, 2.0), Vec3::new(1.0, 40.0, 32.0), Vec3::new(6.0, 0.3, 6.4));
        assert!(swing(centre, from, to, 0.0).distance(from) < 1e-4);
        assert!(swing(centre, from, to, 1.0).distance(to) < 1e-4);
        let (ra, rb) = ((from - centre).length(), (to - centre).length());
        let mid = swing(centre, from, to, 0.5);
        assert!(((mid - centre).length() - (ra + rb) * 0.5).abs() < 1e-3, "the distance to the planet is the mean at the middle");
        // A camera at the planet's centre has no direction to swing round: it goes straight.
        assert!(swing(centre, centre, to, 0.5).distance(centre.lerp(to, 0.5)) < 1e-4);
    }
}

/// Offsets (from the platform centre) of the ground building circles on the lower plate: up to six in one row spread over the plate, extras
/// (planets with 7 or 8 slots) in a second row just above it.
fn circle_slots(n: usize) -> Vec<Vec3> {
    let row = |count: usize, y: f32| -> Vec<Vec3> {
        let spacing = if count > 1 { (2.05 / (count - 1) as f32).min(0.41) } else { 0.0 };
        (0..count).map(|k| Vec3::new((k as f32 - (count as f32 - 1.0) * 0.5) * spacing, y, 0.05)).collect()
    };
    let mut out = row(n.min(6), -0.74);
    if n > 6 {
        out.extend(row(n - 6, -0.42));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn view_scene(
    mut commands: Commands,
    time: Res<Time>,
    mut state: ResMut<PlanetViewState>,
    view: Res<GalaxyView>,
    alamo: Option<Res<AlamoFs>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut textures: Option<ResMut<eaw_ui::textures::UiTextures>>,
    existing: Query<Entity, With<ViewEntity>>,
) {
    let target = if state.active { 1.0 } else { 0.0 };
    let step = time.delta_secs() / 0.9;
    state.blend = if target > state.blend {
        (state.blend + step).min(1.0)
    } else if target < state.blend {
        (state.blend - step).max(0.0)
    } else {
        state.blend
    };
    // Test hook: `SP_PLANET_VIEW_BLEND=0.4` freezes the fly-in at that blend so an offscreen capture shows the transition.
    if state.active
        && let Some(cap) = std::env::var("SP_PLANET_VIEW_BLEND").ok().and_then(|v| v.parse::<f32>().ok())
    {
        state.blend = state.blend.min(cap);
    }
    let building = state.active && existing.is_empty();
    if !state.active && state.blend == 0.0 {
        for e in &existing {
            commands.entity(e).despawn();
        }
        state.planet = None;
        state.start_pos = None;
        state.start_rot = None;
        return;
    }
    if !building || alamo.is_none() {
        return;
    }
    let Some(p) = state.planet.and_then(|i| view.planets.get(i).map(|pl| (i, pl))) else { return };
    // Black backdrop: hides the galaxy behind the scene once the camera is inside.
    commands.spawn((
        ViewEntity,
        Name::new("planet view backdrop"),
        Mesh3d(meshes.add(Sphere::new(150.0).mesh().uv(32, 16))),
        MeshMaterial3d(mats.add(StandardMaterial { base_color: Color::BLACK, unlit: true, cull_mode: None, ..default() })),
        Transform::from_translation(ORIGIN),
    ));
    commands.spawn((
        ViewEntity,
        Name::new("planet view sun"),
        DirectionalLight { illuminance: 8000.0, ..default() },
        Transform::from_translation(ORIGIN).looking_to(Vec3::new(-0.5, -0.25, -0.8), Vec3::Y),
    ));
    if let Some(model) = &p.1.model {
        commands.spawn((ViewEntity, AlamoModel::new(model.clone()), FitModel { radius: 24.0, spin: 0.01 }, Visibility::Hidden, Transform::from_translation(ORIGIN + Vec3::new(-15.8, -1.0, -15.0))));
    } else {
        commands.spawn((
            ViewEntity,
            Mesh3d(meshes.add(Sphere::new(24.0).mesh().uv(32, 16))),
            MeshMaterial3d(mats.add(StandardMaterial { base_color: Color::srgb(0.5, 0.5, 0.55), perceptual_roughness: 0.75, ..default() })),
            Transform::from_translation(ORIGIN + Vec3::new(-15.8, -1.0, -15.0)),
        ));
    }
    if let Some(tex) = textures.as_mut().and_then(|t| t.get(&mut images, "i_galactic_organize_land.dds")) {
        spawn_platform(&mut commands, &mut meshes, &mut mats, tex.handle, land_cell(p.1.terrain.as_deref()), PLAT);
    }
    // The three rectangular frames of the upper pad stay empty (EaW's base pads; base editing is out of scope).
    state.base_pads = [-0.9f32, 0.0, 0.95].iter().map(|x| PLAT + Vec3::new(*x, -0.07, 0.05)).collect();
}

/// Centre of the platform quad (3.8 x 1.9, facing +Z).
const PLAT: Vec3 = Vec3::new(0.0, ORIGIN.y - 2.5, ORIGIN.z - 10.0);
/// Where the star base model hangs, left of the platform (offset from [`PLAT`]).
const STATION_AT: Vec3 = Vec3::new(-2.78, 1.2, 1.0);

/// Offsets (from the platform centre) of the orbital building slots: two rows of up to five small circles under the station, on the left of the platform.
fn space_slots(n: usize) -> Vec<Vec3> {
    (0..n.min(10)).map(|k| Vec3::new(-3.5 + (k % 5) as f32 * 0.36, 0.45 - (k / 5) as f32 * 0.4, 0.05)).collect()
}

/// Everything that depends on what stands on the planet: the star base, the ground and orbital buildings and their slot circles. Rebuilt
/// whenever that list changes, so a building finished while the view is open appears at once (the planet, platform and light stay).
#[derive(Component)]
struct DynEntity;

#[allow(clippy::too_many_arguments)]
fn view_dynamic(
    mut commands: Commands,
    mut state: ResMut<PlanetViewState>,
    sim: Option<Res<SimState>>,
    catalog: Option<Res<PlayCatalog>>,
    bridge: Option<Res<crate::play::PlayBridge>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    scene: Query<(), (With<ViewEntity>, Without<DynEntity>)>,
    dynamic: Query<Entity, With<DynEntity>>,
    mut built: Local<Option<(usize, Option<u32>, Vec<u32>, i32, i32)>>,
) {
    let Some(view_planet) = state.planet.filter(|_| state.active && !scene.is_empty()) else {
        *built = None;
        return;
    };
    let (Some(sim), Some(cat)) = (sim.as_deref(), catalog.as_deref()) else { return };
    let Some(sim_planet) = bridge.as_deref().and_then(|b| b.planet_to_sim.get(view_planet).copied().flatten()) else { return };
    let Some(ps) = sim.galaxy().planets.get(sim_planet as usize) else { return };
    let cap = sim.world.planet_capacity(sim.galaxy(), sim_planet);
    let key = (view_planet, ps.starbase, ps.structures.clone(), cap.ground.total, cap.space.total);
    if built.as_ref() == Some(&key) {
        return;
    }
    *built = Some(key);
    for e in &dynamic {
        commands.entity(e).despawn();
    }
    // Station: the model of the planet's star base level.
    if let Some(sb) = ps.starbase
        && let Some(Some(m)) = cat.starbase_models.get(sb as usize)
    {
        commands.spawn((ViewEntity, DynEntity, AlamoModel::new(m.clone()), FitModel { radius: 0.5, spin: 0.15 }, Visibility::Hidden, Transform::from_translation(PLAT + STATION_AT)));
    }
    // The circles are the slots of the planet, as many as the data says (`planet_capacity`): ground on the lower plate, orbital left of it.
    state.ground_slots = circle_slots(cap.ground.total.max(0) as usize).into_iter().map(|o| PLAT + o).collect();
    state.orbital_slots = space_slots(cap.space.total.max(0) as usize).into_iter().map(|o| PLAT + o).collect();
    let ring = mats.add(StandardMaterial { base_color: Color::srgb(0.85, 0.78, 0.55), unlit: true, depth_bias: 50.0, cull_mode: None, ..default() });
    // The Remake planet scene (CS9VdxglewY f0009) shows no rings next to the station, only the board; the orbital slot rings stay as a
    // faint hint because they are the drop targets of orbital structures.
    let faint_ring = mats.add(StandardMaterial { base_color: Color::srgba(0.85, 0.78, 0.55, 0.16), unlit: true, alpha_mode: AlphaMode::Blend, depth_bias: 50.0, cull_mode: None, ..default() });
    let ground_ring = meshes.add(Annulus::new(0.125, 0.14));
    let space_ring = meshes.add(Annulus::new(0.085, 0.1));
    for (at, mesh, lift, material) in state.ground_slots.iter().map(|a| (a, &ground_ring, 0.1, &ring)).chain(state.orbital_slots.iter().map(|a| (a, &space_ring, 0.0, &faint_ring))) {
        commands.spawn((
            ViewEntity,
            DynEntity,
            Name::new("slot ring"),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(*at + Vec3::new(0.0, lift, 0.03)).with_scale(Vec3::new(1.0, 0.8, 1.0)),
        ));
    }
    let (mut g, mut o) = (0usize, 0usize);
    for &s in &ps.structures {
        let Some(def) = sim.world.structures.get(s as usize) else { continue };
        let Some(Some(model)) = cat.structure_models.get(s as usize) else { continue };
        let (slot, radius) = if def.orbital {
            o += 1;
            (state.orbital_slots.get(o - 1), 0.17)
        } else {
            g += 1;
            (state.ground_slots.get(g - 1), 0.22)
        };
        if let Some(at) = slot {
            commands.spawn((ViewEntity, DynEntity, AlamoModel::new(model.clone()), FitModel { radius, spin: 0.3 }, Visibility::Hidden, Transform::from_translation(*at + Vec3::new(0.0, 0.12, 0.06))));
        }
    }
}

/// Computes the camera transform for the planet view transition at a given blend (0.0 = galaxy orbit, 1.0 = full overview).
///
/// Moves in one continuous fluid motion: swoops from orbit down towards the planet and into the
/// overview pose without stopping or hinging midway.
pub(crate) fn camera_transition_pose(
    orbit_pos: Vec3,
    orbit_rot: Quat,
    planet_pos: Vec3,
    planet_radius: f32,
    blend: f32,
) -> Transform {
    let b = blend.clamp(0.0, 1.0);
    let overview_pose = overview_pose();
    if b >= DIVE_CUT {
        return overview_pose;
    }
    // One continuous dive onto the real planet on the galaxy map; it ends at the pose from which the real planet looks exactly like the
    // planet of the view scene does from the overview pose, so the cut to the scene (hidden by a very short dip) shows the same picture.
    let dive = dive_pose(planet_pos, planet_radius, orbit_rot);
    let k = b / DIVE_CUT;
    let t = k * k * (3.0 - 2.0 * k);
    Transform { translation: swing(planet_pos, orbit_pos, dive.translation, t), rotation: orbit_rot.slerp(dive.rotation, t), scale: Vec3::ONE }
}

/// The point that goes from `from` to `to` by swinging round `centre` while its distance from it changes evenly, `t` of the way (0 to 1).
/// The planet stays in front of the camera however far apart the galaxy view and the scene view are: the straight line between the two
/// poses cut across the planet's side and, from the galaxy camera's 50 degree tilt to the scene's flat view, took the planet out of the
/// frame for the middle of the dive.
fn swing(centre: Vec3, from: Vec3, to: Vec3, t: f32) -> Vec3 {
    let (a, b) = (from - centre, to - centre);
    let (ra, rb) = (a.length(), b.length());
    if ra < 1e-4 || rb < 1e-4 {
        return from.lerp(to, t);
    }
    let turn = Quat::from_rotation_arc(a / ra, b / rb);
    centre + Quat::IDENTITY.slerp(turn, t) * (a / ra) * (ra + (rb - ra) * t)
}

/// The camera pose of the planet view: low in front of the planet-view scene, looking at the platform.
fn overview_pose() -> Transform {
    Transform::from_translation(ORIGIN + Vec3::new(0.0, 0.0, -1.8)).looking_at(ORIGIN + Vec3::new(0.0, -1.95, -10.0), Vec3::Y)
}

/// Centre of the planet of the view scene and its radius (the fit size of the planet model).
const SCENE_PLANET: Vec3 = Vec3::new(-15.8, -1.0, -15.0);
const SCENE_PLANET_RADIUS: f32 = 24.0;

/// Blend at which the camera leaves the galaxy map and the planet-view scene takes over (the rest of the blend holds the overview pose
/// while the view's panels fade in).
pub(crate) const DIVE_CUT: f32 = 0.78;

/// Pose in galaxy space that shows the real planet (model radius `planet_radius`) as the scene planet is seen from [`overview_pose`]:
/// the scene is mapped onto the galaxy by a scale of `planet_radius / 24` about the planet, turned with the galaxy camera's yaw.
pub(crate) fn dive_pose(planet_pos: Vec3, planet_radius: f32, orbit_rot: Quat) -> Transform {
    let s = planet_radius / SCENE_PLANET_RADIUS;
    let yaw = Quat::from_rotation_y(orbit_rot.to_euler(EulerRot::YXZ).0);
    let scene = overview_pose();
    Transform {
        translation: planet_pos + yaw * ((scene.translation - (ORIGIN + SCENE_PLANET)) * s),
        rotation: yaw * scene.rotation,
        scale: Vec3::ONE,
    }
}

fn view_camera(
    mut state: ResMut<PlanetViewState>,
    view: Res<GalaxyView>,
    planet_models: Query<(&crate::planet_visual::PlanetModel, &AlamoModelLoaded, &Transform), Without<GalaxyCamera>>,
    mut cam: Single<(&mut Transform, &mut Projection), With<GalaxyCamera>>,
) {
    let (cam, projection) = &mut *cam;
    // The dive ends close to the planet: the usual near plane would cut its near side.
    let near = if state.blend > 0.0 { 0.05 } else { 1.0 };
    if let Projection::Perspective(p) = &mut **projection
        && p.near != near
    {
        p.near = near;
    }
    if state.blend <= 0.0 {
        return;
    }
    if state.active && state.start_pos.is_none() {
        state.start_pos = Some(cam.translation);
        state.start_rot = Some(cam.rotation);
    }
    let overview_pose = overview_pose();

    let Some((idx, p)) = state.planet.and_then(|i| view.planets.get(i).map(|p| (i, p))) else {
        let t = state.blend * state.blend * (3.0 - 2.0 * state.blend);
        let start_pos = state.start_pos.unwrap_or(cam.translation);
        let start_rot = state.start_rot.unwrap_or(cam.rotation);
        cam.translation = start_pos.lerp(overview_pose.translation, t);
        cam.rotation = start_rot.slerp(overview_pose.rotation, t);
        return;
    };

    let planet_pos = crate::galaxy_map::galaxy_to_world(p.position);
    // The real model radius (the view scene fits the same model to 24 units), not the clamped pick radius.
    let r = planet_models.iter().find(|(m, _, _)| m.0 == idx).map_or(2.0, |(_, loaded, tf)| (loaded.radius * tf.scale.x).max(0.5));

    let start_pos = state.start_pos.unwrap_or(cam.translation);
    let start_rot = state.start_rot.unwrap_or(cam.rotation);
    let target = camera_transition_pose(start_pos, start_rot, planet_pos, r, state.blend);
    cam.translation = target.translation;
    cam.rotation = target.rotation;
}

fn fit_models(time: Res<Time>, mut models: Query<(&FitModel, &mut Transform, &mut Visibility, Option<Ref<AlamoModelLoaded>>)>) {
    for (f, mut tf, mut vis, loaded) in &mut models {
        if let Some(m) = loaded
            && m.is_added()
        {
            tf.scale = Vec3::splat(if m.radius > 0.0 { f.radius / m.radius } else { 1.0 });
            // Shown only now: before the fit a model is drawn at its raw size (a planet-sized flash while opening).
            *vis = Visibility::Inherited;
        }
        if f.spin != 0.0 {
            tf.rotate_y(f.spin * time.delta_secs());
        }
    }
}

/// Cell (column, row) of `i_galactic_organize_land.dds` for a terrain: the original draws the base layout as
/// a pre-rendered platform per climate (2 columns x 4 rows, `land_art_ALT0..7` of `I_Galactic_Organize_New`).
fn land_cell(terrain: Option<&str>) -> (u32, u32) {
    let idx = match terrain.map(str::to_ascii_lowercase).as_deref() {
        Some("desert") => 1,
        Some("swamp") => 2,
        Some("volcanic") => 3,
        Some("arctic") => 4,
        Some("forest") => 5,
        Some("urban") => 6,
        Some("barren") => 7,
        _ => 0,
    };
    (idx % 2, idx / 2)
}

/// The base-layout platform: the original art quad (terrain variant) facing the camera.
fn spawn_platform(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    texture: Handle<Image>,
    cell: (u32, u32),
    at: Vec3,
) {
    let mat = mats.add(StandardMaterial {
        base_color_texture: Some(texture),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        uv_transform: bevy::math::Affine2::from_scale_angle_translation(
            Vec2::new(0.5, 0.25),
            0.0,
            Vec2::new(cell.0 as f32 * 0.5, cell.1 as f32 * 0.25),
        ),
        ..default()
    });
    commands.spawn((ViewEntity, Name::new("base layout"), Mesh3d(meshes.add(Rectangle::new(3.8, 1.9))), MeshMaterial3d(mat), Transform::from_translation(at)));
}

/// Soft vignette in front of the camera: smooths the handoff between the galaxy close-up and
/// the planet-view scene without pitch-black void.
#[derive(Component)]
struct FadeQuad;

fn view_fade(
    mut commands: Commands,
    state: Res<PlanetViewState>,
    cam: Single<&Transform, With<GalaxyCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut quad: Query<(Entity, &mut Transform, &MeshMaterial3d<StandardMaterial>), (With<FadeQuad>, Without<GalaxyCamera>)>,
) {
    // A very short dip to black around the cut from the galaxy to the view scene (the star sky is not part of the scene).
    let alpha = (1.0 - (state.blend - DIVE_CUT).abs() / 0.05).clamp(0.0, 1.0).powi(2) * 0.9;
    let visible = state.blend > 0.0 && state.blend < 1.0;
    match quad.single_mut() {
        Ok((e, mut tf, mat)) => {
            if !visible {
                commands.entity(e).despawn();
                return;
            }
            tf.translation = cam.translation + cam.rotation * Vec3::NEG_Z * 0.3;
            tf.rotation = cam.rotation;
            if let Some(mut m) = mats.get_mut(&mat.0) {
                m.base_color = Color::srgba(0.0, 0.0, 0.0, alpha);
            }
        }
        Err(_) if visible => {
            commands.spawn((
                FadeQuad,
                Mesh3d(meshes.add(Rectangle::new(8.0, 8.0))),
                MeshMaterial3d(mats.add(StandardMaterial { base_color: Color::srgba(0.0, 0.0, 0.0, 0.0), unlit: true, alpha_mode: AlphaMode::Blend, cull_mode: None, ..default() })),
                Transform::default(),
            ));
        }
        Err(_) => {}
    }
}

/// Opt-in self test of the wheel path (`SP_PLANET_VIEW_TEST=1`, run with `--offscreen` and `--select <planet> --frames 900`): writes real
/// `MouseWheel` messages that the input systems accumulate, like the window system would. Checks that a wheel tick far from the planet does
/// not open the view, that one near it does (and flies the camera to the planet), and that a wheel tick out leaves once it is open.
/// The verdict is one log line `planet-view-test: ... PASS|FAIL`.
#[derive(Default)]
struct WheelTest {
    frame: u32,
    opened: bool,
    opened_at: u32,
    results: Vec<(&'static str, bool)>,
    done: bool,
}

#[allow(clippy::too_many_arguments)]
fn wheel_test(
    mut st: Local<WheelTest>,
    state: ResMut<PlanetViewState>,
    selection: Res<Selection>,
    bounds: Res<CameraBounds>,
    view: Res<GalaxyView>,
    mut cam: Single<&mut GalaxyCamera>,
    mut wheel: MessageWriter<bevy::input::mouse::MouseWheel>,
) {
    if st.done || std::env::var("SP_PLANET_VIEW_TEST").is_ok_and(|v| v == "rebuild") || std::env::var_os("SP_PLANET_VIEW_TEST").is_none() {
        return;
    }
    let Some(p) = selection.0 else { return };
    let Some(pl) = view.planets.get(p) else { return };
    let tick = |wheel: &mut MessageWriter<bevy::input::mouse::MouseWheel>, y: f32| {
        wheel.write(bevy::input::mouse::MouseWheel { unit: bevy::input::mouse::MouseScrollUnit::Line, x: 0.0, y, window: Entity::PLACEHOLDER, phase: bevy::input::touch::TouchPhase::Moved });
    };
    st.frame += 1;
    let at = crate::galaxy_map::galaxy_to_world(pl.position);
    let at = Vec3::new(at.x, 0.0, at.z);
    match st.frame {
        // Far out the wheel only zooms the map.
        20 => {
            cam.target_distance = bounds.min_distance * 8.0;
            cam.distance = cam.target_distance;
            cam.target_focus = at;
            cam.focus = at;
            tick(&mut wheel, 1.0);
        }
        25 => {
            let ok = !state.active;
            st.results.push(("wheel-far-from-planet-keeps-the-map", ok));
            // Close by: one notch opens the view and the camera dives onto the planet.
            cam.target_distance = bounds.min_distance * 1.4;
            cam.distance = cam.target_distance;
            tick(&mut wheel, 1.0);
        }
        30 => {
            let ok = state.active && state.planet == Some(p) && (cam.target_distance - bounds.min_distance).abs() < 1e-3;
            st.results.push(("small-wheel-zoom-near-planet-opens-view", ok));
        }
        // Wheel out while the view is still flying in does not close it.
        32 => tick(&mut wheel, -1.0),
        36 => {
            let ok = state.active;
            st.results.push(("wheel-out-mid-flight-keeps-view", ok));
        }
        f if f > 36 && !st.opened && state.blend >= 1.0 => {
            st.opened = true;
            st.opened_at = f;
            st.results.push(("view-fully-open", true));
            tick(&mut wheel, -1.0);
        }
        f if st.opened && f == st.opened_at + 10 => {
            let ok = !state.active;
            st.results.push(("wheel-out-when-open-leaves", ok));
            let pass = st.results.iter().all(|r| r.1);
            for (n, ok) in &st.results {
                info!("planet-view-test: {n} {}", if *ok { "ok" } else { "FAILED" });
            }
            info!("planet-view-test: {} checks {}", st.results.len(), if pass { "PASS" } else { "FAIL" });
            st.done = true;
        }
        1400 if !st.opened => {
            st.results.push(("view-fully-open", false));
            st.done = true;
            info!("planet-view-test: view never fully opened FAIL");
        }
        _ => {}
    }
}

#[derive(Default)]
struct RebuildTest {
    frame: u32,
    models_before: Option<usize>,
    structures_before: usize,
    ordered: bool,
    settle: u32,
    done: bool,
}

/// Opt-in self test of the live rebuild (`SP_PLANET_VIEW_TEST=rebuild`, run with `SP_PLANET_VIEW=1 --select <owned planet> --frames 6000`): orders a structure
/// through the normal build action, speeds the sim up and checks that its model appears in the open view without reopening.
#[allow(clippy::too_many_arguments)]
fn rebuild_test(
    mut st: Local<RebuildTest>,
    state: Res<PlanetViewState>,
    sim: Option<Res<SimState>>,
    catalog: Option<Res<PlayCatalog>>,
    bridge: Option<Res<crate::play::PlayBridge>>,
    model: Option<Res<galaxy_play::PlayUiModel>>,
    mut actions: MessageWriter<galaxy_play::PlayAction>,
    models: Query<&FitModel, With<DynEntity>>,
) {
    if st.done || std::env::var("SP_PLANET_VIEW_TEST").ok().as_deref() != Some("rebuild") {
        return;
    }
    let (Some(sim), Some(cat), Some(bridge)) = (sim, catalog, bridge) else { return };
    let Some(sp) = state.planet.and_then(|p| bridge.planet_to_sim.get(p).copied().flatten()).filter(|_| state.active && state.blend >= 1.0) else { return };
    st.frame += 1;
    let count = models.iter().count();
    if st.models_before.is_none() && st.frame > 30 {
        st.models_before = Some(count);
        st.structures_before = sim.galaxy().planets[sp as usize].structures.len();
        let pick = model.as_ref().and_then(|m| m.planet.as_ref()).and_then(|m| {
            m.options.iter().find_map(|o| match o.target {
                galaxy_play::BuildTarget::Structure(s)
                    if o.availability.is_available() && !sim.world.structures[s as usize].orbital && matches!(cat.structure_models.get(s as usize), Some(Some(_))) =>
                {
                    Some(o.target)
                }
                _ => None,
            })
        });
        let Some(target) = pick else {
            info!("planet-view-test: rebuild: no orderable ground structure with a model FAIL");
            st.done = true;
            return;
        };
        info!("planet-view-test: rebuild: {} dynamic models, {} structures; ordering {target:?}", count, st.structures_before);
        actions.write(galaxy_play::PlayAction::Build { planet: sp, target });
        actions.write(galaxy_play::PlayAction::SetSpeedMultiplier(3000.0));
        st.ordered = true;
        return;
    }
    if st.ordered && sim.galaxy().planets[sp as usize].structures.len() > st.structures_before {
        st.settle += 1;
        if st.settle == 20 {
            let before = st.models_before.unwrap_or(0);
            let ok = count == before + 1;
            info!("planet-view-test: rebuild: {before} -> {count} dynamic models with the view still open: {}", if ok { "PASS" } else { "FAIL" });
            st.done = true;
        }
    } else if st.frame > 5000 {
        info!("planet-view-test: rebuild: the structure never finished FAIL");
        st.done = true;
    }
}

/// Unit object name behind each unit icon of the orbit boxes (`q1_0000` ...); filled by `play_ui::planet_view_ui`, read by the hover tooltip.
#[derive(Resource, Default)]
pub(crate) struct OrbitSlots(pub std::collections::HashMap<String, String>);

/// What the cursor points at in the planet view's 3D part.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Hover {
    Station,
    Ground(usize),
    Orbital(usize),
}

/// The pointed-at thing: the closest screen point within `radius`.
fn pick(points: &[(Hover, Vec2)], cursor: Vec2, radius: f32) -> Option<Hover> {
    points.iter().map(|(h, p)| (*h, p.distance(cursor))).filter(|(_, d)| *d <= radius).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(h, _)| h)
}

/// Tooltip of a structure type: name, encyclopedia text, cost and what it does for the owner (same format as the build pads).
fn structure_tip(world: &galaxy_sim::World, cat: &PlayCatalog, game: Option<&alamo_data::GameData>, s: u32) -> String {
    let st = &world.structures[s as usize];
    let mut t = cat.structure(s).to_uppercase();
    let description = crate::play_ui::lore(game, &st.name);
    if let Some(d) = &description {
        t.push('\n');
        t.push_str(d.trim());
    }
    t.push_str(&format!("\n@Cost\t{:.0} credits", st.cost));
    let mentioned = description.as_deref().is_some_and(|d| d.to_ascii_lowercase().contains("upkeep"));
    if st.income_abs < -0.5 && !mentioned {
        t.push_str(&format!("\n@Upkeep\t{:.0} credits per week", -st.income_abs));
    } else if st.income_abs > 0.5 {
        t.push_str(&format!("\n@Income\t+{:.0} credits per week", st.income_abs));
    }
    if st.income_pct.abs() > 1e-4 {
        t.push_str(&format!("\n@Planet income\t{:+.0}%", st.income_pct * 100.0));
    }
    if st.population_capacity > 0 {
        t.push_str(&format!("\n@Population cap\t+{}", st.population_capacity));
    }
    t.push_str(&format!("\n@Action\tRight-click to sell for {:.0} credits", world.sell_value(s).floor()));
    t
}

/// Tooltip of a unit type shown by an orbit-box icon.
fn unit_tip(world: &galaxy_sim::World, cat: &PlayCatalog, game: Option<&alamo_data::GameData>, name: &str) -> String {
    if name == "!hyperspace" {
        return "HYPERSPACE SLOT
Drag a ship here: only this ship jumps into the battle first, the rest of the fleet waits as reserve.
Drag it out to empty the slot.".to_owned();
    }
    let Some(id) = world.unit_id(name) else { return "UNKNOWN FORCES\nOnly seen from afar: strength and composition are not known.".to_owned() };
    let ut = &world.units[id as usize];
    let mut t = cat.unit(id).to_uppercase();
    let description = crate::play_ui::lore(game, &ut.name);
    if let Some(d) = &description {
        t.push('\n');
        t.push_str(d.trim());
    }
    t.push_str(&format!("\n@Cost\t{:.0} credits", ut.cost));
    if ut.population > 0 {
        t.push_str(&format!("\n@Population\t{}", ut.population));
    }
    let mentioned = description.as_deref().is_some_and(|d| d.to_ascii_lowercase().contains("upkeep"));
    if ut.upkeep < -0.5 && !mentioned {
        t.push_str(&format!("\n@Upkeep\t{:.0} credits per week", -ut.upkeep));
    }
    t
}

/// Hover tooltips of the planet view on the shared world tooltip (next to the advisor droid): the buildings, the station and the empty
/// slots of the 3D scene, and the unit icons of the orbit boxes. `SP_PLANET_VIEW_HOVER=ground0|orbital0|station|unit:<Name>` forces a
/// target without a cursor (offscreen checks).
#[allow(clippy::too_many_arguments)]
fn view_tooltip(
    state: Res<PlanetViewState>,
    sim: Option<Res<SimState>>,
    catalog: Option<Res<PlayCatalog>>,
    bridge: Option<Res<crate::play::PlayBridge>>,
    assets: Option<Res<eaw_ui::EawUiAssets>>,
    orbit: Option<Res<OrbitSlots>>,
    window: Option<Single<&Window, With<bevy::window::PrimaryWindow>>>,
    cam: Single<(&Camera, &GlobalTransform), With<GalaxyCamera>>,
    slot_nodes: Query<(&eaw_ui::panels::PanelSlotName, &ComputedNode, &UiGlobalTransform, &InheritedVisibility)>,
    mut tip: ResMut<eaw_ui::WorldTooltip>,
) {
    if !state.active || state.blend < 1.0 {
        return;
    }
    let (Some(sim), Some(cat), Some(bridge)) = (sim, catalog, bridge) else { return };
    let Some(sp) = state.planet.and_then(|p| bridge.planet_to_sim.get(p).copied().flatten()) else { return };
    let game = assets.as_deref().and_then(|a| a.game.as_deref());
    let forced = std::env::var("SP_PLANET_VIEW_HOVER").ok();
    let mut text = None;

    // Unit icons of the orbit boxes.
    let unit_name = match (&forced, window.as_ref().and_then(|w| w.physical_cursor_position())) {
        (Some(f), _) => f.strip_prefix("unit:").map(str::to_owned),
        (None, Some(c)) => slot_nodes
            .iter()
            .find(|(_, n, g, v)| v.get() && n.size().x > 4.0 && n.contains_point(**g, c))
            .and_then(|(name, ..)| orbit.as_ref().and_then(|o| o.0.get(&name.0.to_ascii_lowercase()).cloned())),
        _ => None,
    };
    if let Some(n) = unit_name {
        text = Some(unit_tip(&sim.world, &cat, game, &n));
    }

    // Buildings, station and free slots of the scene.
    if text.is_none() {
        let hover = match (&forced, window.as_ref().and_then(|w| w.cursor_position())) {
            (Some(f), _) => match f.as_str() {
                "station" => Some(Hover::Station),
                s if s.starts_with("ground") => s[6..].parse().ok().map(Hover::Ground),
                s if s.starts_with("orbital") => s[7..].parse().ok().map(Hover::Orbital),
                _ => None,
            },
            (None, Some(c)) => {
                let (camera, gt) = *cam;
                let project = |p: Vec3| camera.world_to_viewport(gt, p).ok();
                let mut points: Vec<(Hover, Vec2)> = Vec::new();
                points.extend(state.ground_slots.iter().enumerate().filter_map(|(i, p)| project(*p + Vec3::new(0.0, 0.2, 0.0)).map(|v| (Hover::Ground(i), v))));
                points.extend(state.orbital_slots.iter().enumerate().filter_map(|(i, p)| project(*p + Vec3::new(0.0, 0.15, 0.0)).map(|v| (Hover::Orbital(i), v))));
                points.extend(project(PLAT + STATION_AT).map(|v| (Hover::Station, v)));
                pick(&points, c, 30.0)
            }
            _ => None,
        };
        let ps = sim.galaxy().planets.get(sp as usize);
        let cap = sim.world.planet_capacity(sim.galaxy(), sp);
        let nth = |orbital: bool, n: usize| {
            ps.and_then(|p| p.structures.iter().copied().filter(|&s| sim.world.structures.get(s as usize).is_some_and(|d| d.orbital == orbital)).nth(n))
        };
        text = match hover {
            Some(Hover::Ground(i)) => Some(match nth(false, i) {
                Some(s) => structure_tip(&sim.world, &cat, game, s),
                None => format!(
                    "FREE GROUND SLOT\nBuild a ground structure here from the production bar.\n@Ground slots\t{} of {} used",
                    cap.ground.built + cap.ground.queued,
                    cap.ground.total
                ),
            }),
            Some(Hover::Orbital(i)) => Some(match nth(true, i) {
                Some(s) => structure_tip(&sim.world, &cat, game, s),
                None => format!(
                    "FREE ORBITAL SLOT\nBuild a space structure here from the space production tab.\n@Space slots\t{} of {} used",
                    cap.space.built + cap.space.queued,
                    cap.space.total
                ),
            }),
            Some(Hover::Station) => ps.and_then(|p| p.starbase).map(|sb| {
                let b = &sim.world.starbases[sb as usize];
                let mut t = cat.starbase(sb).to_uppercase();
                if let Some(d) = crate::play_ui::lore(game, &b.name) {
                    t.push('\n');
                    t.push_str(d.trim());
                }
                t.push_str(&format!("\n@Level\t{} of {}", cap.starbase_level, cap.starbase_max));
                if b.income_abs.abs() > 0.5 {
                    t.push_str(&format!("\n@Income\t{:+.0} credits per week", b.income_abs));
                }
                if b.population_capacity > 0 {
                    t.push_str(&format!("\n@Population cap\t+{}", b.population_capacity));
                }
                t
            }),
            None => None,
        };
    }
    if let Some(t) = text
        && tip.text.as_deref() != Some(t.as_str())
    {
        tip.text = Some(t);
    }
}

/// Runs after the generic hover text, which clears the tooltip whenever the map has nothing under the cursor.
fn register_tooltip(app: &mut App) {
    app.init_resource::<OrbitSlots>().init_resource::<SellDialog>().add_systems(
        Update,
        (view_tooltip.after(crate::play_ui::fleet_tooltip), sell_dialog_input.after(crate::planet_fleets::OrganizerSet), sell_open.after(sell_dialog_input), sell_dialog_ui.after(sell_open), sell_button_look)
            .run_if(in_state(AppScreen::Galaxy)),
    );
    app.add_systems(Last, sell_dialog_escape);
}

#[cfg(test)]
mod tooltip_tests {
    use super::*;

    #[test]
    fn the_closest_point_within_the_radius_is_picked() {
        let pts = [(Hover::Ground(0), Vec2::new(100.0, 100.0)), (Hover::Ground(1), Vec2::new(140.0, 100.0)), (Hover::Station, Vec2::new(400.0, 100.0))];
        assert_eq!(pick(&pts, Vec2::new(125.0, 100.0), 30.0), Some(Hover::Ground(1)));
        assert_eq!(pick(&pts, Vec2::new(250.0, 100.0), 30.0), None, "between the things: no tooltip");
        assert_eq!(pick(&pts, Vec2::new(420.0, 110.0), 30.0), Some(Hover::Station));
    }
}

/// What a sell box would sell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SellWhat {
    /// A built structure of a planet (a ground or orbital slot).
    Structure { planet: u32, structure: u32 },
    /// One ship of a fleet that waits at the planet (a fleet box icon).
    Unit { fleet: u32, unit: u32 },
}

/// Where a sell box is open: what it sells, the credits it pays and the window point (logical pixels) it hangs from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SellTarget {
    what: SellWhat,
    value: i64,
    at: Vec2,
}

/// The sell box of the planet view, like the Empire at War command menu: right click on a structure or ship opens one line "Sell for <credits> N" at
/// the cursor; only a left click on that line sells.
#[derive(Resource, Default, Debug, PartialEq)]
pub(crate) struct SellDialog(Option<SellTarget>);

impl SellDialog {
    pub(crate) fn is_open(&self) -> bool {
        self.0.is_some()
    }

    /// A left click: on the line it sells (and closes), anywhere else it only closes. Returns what to sell.
    fn left_click(&mut self, on_button: bool) -> Option<SellWhat> {
        let t = self.0.take()?;
        on_button.then_some(t.what)
    }
}

/// The `n`-th built structure of the slot kind (ground or orbital), in build order: the same order the slots are filled in.
fn built_structure(structures: &[u32], is_orbital: impl Fn(u32) -> bool, orbital: bool, n: usize) -> Option<u32> {
    structures.iter().copied().filter(|&s| is_orbital(s) == orbital).nth(n)
}

/// Top-left corner of the box (UI pixels) for a cursor at `at` (window pixels): at the cursor tip, kept on the screen.
fn dialog_corner(at: Vec2, scale: f32, size: Vec2, screen: Vec2) -> Vec2 {
    let scale = scale.max(1e-3);
    let p = at / scale + Vec2::new(2.0, 2.0);
    p.clamp(Vec2::ZERO, (screen / scale - size).max(Vec2::ZERO))
}

#[derive(Component)]
struct SellDialogRoot;

#[derive(Component)]
struct SellButton;

const DIALOG_BG: Color = Color::srgba(0.04, 0.09, 0.13, 0.82);
const DIALOG_BORDER: Color = Color::srgba(0.0, 0.76, 0.99, 0.7);
const BUTTON_HOT: Color = Color::srgba(0.1, 0.4, 0.55, 0.9);
/// Estimated size of the box (UI pixels), only used to keep it on the screen.
const DIALOG_SIZE: Vec2 = Vec2::new(118.0, 24.0);

/// The sellable ship behind slot `k` of fleet box `b`: `(fleet, unit)`. Heroes are not for sale.
fn sellable_unit(boxes: &[crate::planet_fleets::BoxContent; 3], world: &galaxy_sim::World, b: usize, k: usize) -> Option<(u32, u32)> {
    let content = boxes.get(b)?;
    let unit = if k == 0 { content.hyper } else { content.stacks.get(k - 1).map(|s| s.0) }?;
    (!world.units[unit as usize].hero).then_some((content.fleet?, unit))
}

/// Right click on a built ground / orbital structure (3D slot) or on a ship icon of a fleet box opens the sell box at the cursor; a right click on
/// anything else closes it. `SP_SELL_DIALOG=ground:<n>|orbital:<n>|unit:<box>:<slot>` opens it on that thing without a cursor (offscreen checks).
#[allow(clippy::too_many_arguments)]
fn sell_open(
    mouse: Res<ButtonInput<MouseButton>>,
    state: Res<PlanetViewState>,
    sim: Option<Res<SimState>>,
    bridge: Option<Res<crate::play::PlayBridge>>,
    player: Option<Res<galaxy_play::PlayerFaction>>,
    window: Option<Single<&Window, With<bevy::window::PrimaryWindow>>>,
    cam: Single<(&Camera, &GlobalTransform), With<GalaxyCamera>>,
    slots: Query<(&eaw_ui::panels::PanelSlotName, &ComputedNode, &UiGlobalTransform, &InheritedVisibility)>,
    mut dialog: ResMut<SellDialog>,
    mut forced: Local<bool>,
) {
    let (Some(sim), Some(bridge)) = (sim, bridge) else { return };
    let Some(sp) = state.planet.and_then(|p| bridge.planet_to_sim.get(p).copied().flatten()).filter(|_| state.active && state.blend >= 1.0) else { return };
    let wanted = std::env::var("SP_SELL_DIALOG").ok().filter(|_| !*forced);
    let right = mouse.just_pressed(MouseButton::Right);
    if wanted.is_none() && !right {
        return;
    }
    *forced |= wanted.is_some();
    // Physical pixels of a UI node to the logical pixels of the camera target (the window, or the image of an offscreen render).
    let scale_factor = cam.0.target_scaling_factor().unwrap_or(1.0);
    let boxes = player.and_then(|p| p.0).map(|me| crate::planet_fleets::box_contents(sim.galaxy(), me, sp));
    let unit_value = |u: u32| sim.world.unit_sell_value(u).floor() as i64;
    let structure_value = |s: u32| sim.world.sell_value(s).floor() as i64;
    let mut target = None;

    // Ship icons of the fleet boxes (UI nodes, physical pixels).
    let ship_at = |phys: Option<Vec2>, want: Option<(usize, usize)>| {
        let boxes = boxes.as_ref()?;
        slots.iter().find_map(|(name, n, g, v)| {
            let (b, k) = crate::planet_fleets::parse_slot(&name.0)?;
            let hit = match (want, phys) {
                (Some(w), _) => w == (b, k),
                (None, Some(c)) => n.contains_point(*g, c),
                _ => false,
            };
            (v.get() && n.size().x > 4.0 && hit).then_some(())?;
            let (fleet, unit) = sellable_unit(boxes, &sim.world, b, k)?;
            Some(SellTarget { what: SellWhat::Unit { fleet, unit }, value: unit_value(unit), at: g.translation / scale_factor })
        })
    };
    let forced_unit = wanted.as_deref().and_then(|f| {
        let mut it = f.strip_prefix("unit:")?.split(':');
        Some((it.next()?.parse::<usize>().ok()?, it.next()?.parse::<usize>().ok()?))
    });
    if forced_unit.is_some() {
        target = ship_at(None, forced_unit);
    } else if wanted.is_none() {
        let phys = window.as_ref().and_then(|w| w.physical_cursor_position());
        target = ship_at(phys, None).map(|t| SellTarget { at: window.as_ref().and_then(|w| w.cursor_position()).unwrap_or(t.at), ..t });
    }

    // Structure slots (3D scene, projected to window pixels).
    if target.is_none() && forced_unit.is_none() {
        let (camera, gt) = *cam;
        let project = |p: Vec3| camera.world_to_viewport(gt, p).ok();
        let mut points: Vec<(Hover, Vec2)> = Vec::new();
        points.extend(state.ground_slots.iter().enumerate().filter_map(|(i, p)| project(*p + Vec3::new(0.0, 0.2, 0.0)).map(|v| (Hover::Ground(i), v))));
        points.extend(state.orbital_slots.iter().enumerate().filter_map(|(i, p)| project(*p + Vec3::new(0.0, 0.15, 0.0)).map(|v| (Hover::Orbital(i), v))));
        let hit = match wanted.as_deref().and_then(|f| f.split_once(':')).and_then(|(k, n)| Some((k, n.parse::<usize>().ok()?))) {
            Some((k, n)) => {
                let want = if k == "orbital" { Hover::Orbital(n) } else { Hover::Ground(n) };
                points.iter().find(|(h, _)| *h == want).copied()
            }
            None if wanted.is_some() => None,
            None => {
                let Some(c) = window.as_ref().and_then(|w| w.cursor_position()) else { return };
                pick(&points, c, 30.0).map(|h| (h, c))
            }
        };
        let structures = sim.galaxy().planets.get(sp as usize).map(|p| p.structures.as_slice());
        target = hit.and_then(|(h, at)| {
            let (orbital, n) = match h {
                Hover::Ground(i) => (false, i),
                Hover::Orbital(i) => (true, i),
                _ => return None,
            };
            let built = built_structure(structures?, |s| sim.world.structures.get(s as usize).is_some_and(|d| d.orbital), orbital, n)?;
            Some(SellTarget { what: SellWhat::Structure { planet: sp, structure: built }, value: structure_value(built), at })
        });
    }
    // A right click on nothing closes an open box, a right click on another thing moves it there.
    dialog.0 = target;
}

/// Builds or removes the box when [`SellDialog`] changes: a tiny semi-transparent line "Sell for <credit icon> N" at the cursor.
#[allow(clippy::too_many_arguments)]
fn sell_dialog_ui(
    dialog: Res<SellDialog>,
    assets: Option<Res<eaw_ui::EawUiAssets>>,
    settings: Option<Res<eaw_ui::EawUiSettings>>,
    mut textures: Option<ResMut<eaw_ui::textures::UiTextures>>,
    mut images: ResMut<Assets<Image>>,
    scale: Res<UiScale>,
    cam: Single<&Camera, With<GalaxyCamera>>,
    roots: Query<Entity, With<SellDialogRoot>>,
    mut commands: Commands,
) {
    if !dialog.is_changed() {
        return;
    }
    for e in &roots {
        commands.entity(e).despawn();
    }
    let Some(t) = dialog.0 else { return };
    let sell = assets.as_deref().and_then(|a| a.text.get("TEXT_BUTTON_SELL").filter(|s| !s.is_empty())).unwrap_or("Sell").to_owned();
    let family = settings.as_ref().and_then(|s| s.tooltip_font_family.clone());
    let font = |size: f32| match &family {
        Some(f) => TextFont { font: FontSource::Family(f.as_str().into()), font_size: FontSize::Px(size), ..default() },
        None => TextFont::from_font_size(size),
    };
    let icon = textures.as_mut().and_then(|tx| tx.credit_icon(&mut images)).map(|t| t.handle);
    let screen = cam.logical_viewport_size().unwrap_or(Vec2::new(1920.0, 1080.0));
    let corner = dialog_corner(t.at, scale.0, DIALOG_SIZE, screen);
    let white = Color::srgb(0.95, 0.97, 1.0);
    let mut root = commands.spawn((
        SellDialogRoot,
        SellButton,
        Button,
        Name::new("Sell box"),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(corner.x),
            top: Val::Px(corner.y),
            align_items: AlignItems::Center,
            column_gap: Val::Px(4.0),
            padding: UiRect::axes(Val::Px(7.0), Val::Px(3.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(DIALOG_BG),
        BorderColor::all(DIALOG_BORDER),
        bevy::ui::FocusPolicy::Block,
        GlobalZIndex(1100),
    ));
    if let Some(c) = settings.as_ref().and_then(|s| s.camera) {
        root.insert(UiTargetCamera(c));
    }
    root.with_children(|p| {
        p.spawn((Text::new(format!("{sell} for")), font(12.0), TextColor(white), bevy::ui::FocusPolicy::Pass));
        if let Some(h) = icon {
            p.spawn((ImageNode::new(h), Node { width: Val::Px(12.0), height: Val::Px(12.0), ..default() }, bevy::ui::FocusPolicy::Pass));
        }
        p.spawn((Text::new(t.value.to_string()), font(12.0), TextColor(white), bevy::ui::FocusPolicy::Pass));
    });
}

/// Hover look of the sell line.
fn sell_button_look(mut buttons: Query<(&Interaction, &mut BackgroundColor), (With<SellButton>, Changed<Interaction>)>) {
    for (i, mut bg) in &mut buttons {
        bg.0 = if *i == Interaction::None { DIALOG_BG } else { BUTTON_HOT };
    }
}

/// A left click sells when it lands on the sell line and only closes the box anywhere else. Scrolling or leaving the view also closes it.
#[allow(clippy::too_many_arguments)]
fn sell_dialog_input(
    mouse: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    state: Res<PlanetViewState>,
    sim: Option<Res<SimState>>,
    catalog: Option<Res<PlayCatalog>>,
    buttons: Query<&Interaction, With<SellButton>>,
    mut dialog: ResMut<SellDialog>,
    mut actions: MessageWriter<galaxy_play::PlayAction>,
    mut toast: ResMut<crate::play_ui::Toast>,
) {
    let Some(t) = dialog.0 else { return };
    if !state.active || scroll.delta.y != 0.0 {
        dialog.0 = None;
        return;
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let on_button = buttons.iter().any(|i| *i == Interaction::Pressed);
    let Some(what) = dialog.left_click(on_button) else { return };
    let name = match what {
        SellWhat::Structure { planet, structure } => {
            actions.write(galaxy_play::PlayAction::SellStructure { planet, structure });
            catalog.as_ref().map(|c| c.structure(structure).to_owned())
        }
        SellWhat::Unit { fleet, unit } => {
            actions.write(galaxy_play::PlayAction::SellUnit { fleet, unit });
            sim.as_ref().map(|s| s.world.units[unit as usize].name.replace('_', " "))
        }
    };
    if let Some(n) = name {
        toast.0 = format!("{n} sold for {} credits", t.value);
        toast.1 = 3.0;
    }
}

/// Esc closes the box. Runs last in the frame so the view and the pause menu, which also react to Esc, still see it open and leave Esc alone.
fn sell_dialog_escape(keys: Res<ButtonInput<KeyCode>>, mut dialog: ResMut<SellDialog>) {
    if keys.just_pressed(KeyCode::Escape) && dialog.is_open() {
        dialog.0 = None;
    }
}

#[cfg(test)]
mod sell_tests {
    use super::*;

    fn target() -> SellTarget {
        SellTarget { what: SellWhat::Structure { planet: 3, structure: 7 }, value: 2250, at: Vec2::new(500.0, 300.0) }
    }

    #[test]
    fn built_structure_counts_only_its_slot_kind_in_build_order() {
        let orbital = |s: u32| s >= 100;
        let built = [5, 100, 6, 101, 7];
        assert_eq!(built_structure(&built, orbital, false, 0), Some(5));
        assert_eq!(built_structure(&built, orbital, false, 2), Some(7));
        assert_eq!(built_structure(&built, orbital, true, 1), Some(101));
        assert_eq!(built_structure(&built, orbital, true, 2), None, "an empty slot sells nothing");
    }

    #[test]
    fn a_click_on_the_line_sells_and_closes() {
        let mut d = SellDialog(Some(target()));
        assert_eq!(d.left_click(true), Some(SellWhat::Structure { planet: 3, structure: 7 }));
        assert!(!d.is_open());
    }

    #[test]
    fn a_click_elsewhere_closes_without_selling() {
        let mut d = SellDialog(Some(target()));
        assert_eq!(d.left_click(false), None);
        assert!(!d.is_open());
    }

    #[test]
    fn a_click_with_no_box_sells_nothing() {
        let mut d = SellDialog::default();
        assert_eq!(d.left_click(true), None);
    }

    #[test]
    fn the_box_hangs_at_the_cursor_tip_and_stays_on_the_screen() {
        let size = Vec2::new(100.0, 24.0);
        let screen = Vec2::new(1000.0, 600.0);
        assert_eq!(dialog_corner(Vec2::new(400.0, 300.0), 1.0, size, screen), Vec2::new(402.0, 302.0), "at the tip");
        assert_eq!(dialog_corner(Vec2::new(990.0, 590.0), 1.0, size, screen), Vec2::new(900.0, 576.0), "pushed back inside at the lower right corner");
        assert_eq!(dialog_corner(Vec2::new(400.0, 300.0), 2.0, size, screen), Vec2::new(202.0, 152.0), "window pixels are divided by the UI scale");
    }

    /// The box flow in a tiny app with the real input systems.
    fn flow_app(what: SellWhat) -> App {
        let mut app = App::new();
        app.init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<AccumulatedMouseScroll>()
            .init_resource::<crate::play_ui::Toast>()
            .insert_resource(PlanetViewState { active: true, ..default() })
            .insert_resource(SellDialog(Some(SellTarget { what, ..target() })))
            .add_message::<galaxy_play::PlayAction>()
            .add_systems(Update, sell_dialog_input)
            .add_systems(Last, sell_dialog_escape);
        app.world_mut().spawn((SellButton, Interaction::None));
        app
    }

    fn sent(app: &App) -> Vec<galaxy_play::PlayAction> {
        let msgs = app.world().resource::<Messages<galaxy_play::PlayAction>>();
        msgs.iter_current_update_messages().cloned().collect()
    }

    fn press_button(app: &mut App) {
        let mut q = app.world_mut().query_filtered::<&mut Interaction, With<SellButton>>();
        *q.single_mut(app.world_mut()).unwrap() = Interaction::Pressed;
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Left);
    }

    #[test]
    fn a_left_click_off_the_line_closes_and_sells_nothing() {
        let mut app = flow_app(SellWhat::Structure { planet: 3, structure: 7 });
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Left);
        app.update();
        assert!(!app.world().resource::<SellDialog>().is_open());
        assert!(sent(&app).is_empty());
    }

    #[test]
    fn a_left_click_on_the_line_sells_the_structure_once_and_closes() {
        let mut app = flow_app(SellWhat::Structure { planet: 3, structure: 7 });
        press_button(&mut app);
        app.update();
        assert!(!app.world().resource::<SellDialog>().is_open());
        assert!(matches!(sent(&app).as_slice(), [galaxy_play::PlayAction::SellStructure { planet: 3, structure: 7 }]));
    }

    #[test]
    fn a_left_click_on_the_line_sells_one_ship_and_closes() {
        let mut app = flow_app(SellWhat::Unit { fleet: 9, unit: 4 });
        press_button(&mut app);
        app.update();
        assert!(!app.world().resource::<SellDialog>().is_open());
        assert!(matches!(sent(&app).as_slice(), [galaxy_play::PlayAction::SellUnit { fleet: 9, unit: 4 }]));
    }

    #[test]
    fn nothing_is_sold_while_the_box_just_sits_open() {
        let mut app = flow_app(SellWhat::Unit { fleet: 9, unit: 4 });
        app.update();
        assert!(app.world().resource::<SellDialog>().is_open());
        assert!(sent(&app).is_empty());
    }

    #[test]
    fn a_right_button_press_alone_never_sells() {
        let mut app = flow_app(SellWhat::Unit { fleet: 9, unit: 4 });
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Right);
        app.update();
        assert!(app.world().resource::<SellDialog>().is_open());
        assert!(sent(&app).is_empty());
    }

    #[test]
    fn escape_closes_the_box() {
        let mut app = flow_app(SellWhat::Unit { fleet: 9, unit: 4 });
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Escape);
        app.update();
        assert!(!app.world().resource::<SellDialog>().is_open());
        assert!(sent(&app).is_empty());
    }

    #[test]
    fn leaving_the_planet_view_closes_the_box() {
        let mut app = flow_app(SellWhat::Unit { fleet: 9, unit: 4 });
        app.world_mut().resource_mut::<PlanetViewState>().active = false;
        app.update();
        assert!(!app.world().resource::<SellDialog>().is_open());
    }
}
