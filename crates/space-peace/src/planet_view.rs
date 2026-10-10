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
    selection: Res<Selection>,
    bounds: Res<CameraBounds>,
    view: Res<GalaxyView>,
    mut cam: Single<&mut GalaxyCamera>,
    mut state: ResMut<PlanetViewState>,
) {
    if !state.active {
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
                cam.target_distance = bounds.min_distance;
                state.active = true;
                state.planet = Some(p.0);
            }
        }
    } else if keys.just_pressed(KeyCode::Escape) || (scroll.delta.y < 0.0 && state.blend >= 1.0) {
        // Scrolling out only leaves once the view has fully opened: a trailing wheel tick must not close it mid-flight.
        state.active = false;
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
    let ground_ring = meshes.add(Annulus::new(0.125, 0.14));
    let space_ring = meshes.add(Annulus::new(0.085, 0.1));
    for (at, mesh, lift) in state.ground_slots.iter().map(|a| (a, &ground_ring, 0.1)).chain(state.orbital_slots.iter().map(|a| (a, &space_ring, 0.0))) {
        commands.spawn((
            ViewEntity,
            DynEntity,
            Name::new("slot ring"),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(ring.clone()),
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
/// Phase 1 (0.0 .. 0.5): camera swoops from galaxy orbit down onto the planet globe, closing in so the 3D planet fills the screen.
/// Phase 2 (0.5 .. 1.0): camera transitions from the planet close-up to the overview framing (platform & station).
pub(crate) fn camera_transition_pose(
    orbit_pos: Vec3,
    orbit_rot: Quat,
    planet_pos: Vec3,
    planet_radius: f32,
    blend: f32,
) -> Transform {
    let b = blend.clamp(0.0, 1.0);
    let overview_pose = Transform::from_translation(ORIGIN + Vec3::new(0.0, 0.0, -1.8))
        .looking_at(ORIGIN + Vec3::new(0.0, -1.95, -10.0), Vec3::Y);

    let dir = (orbit_rot * Vec3::Z).normalize_or_zero();
    let dir = if dir.length_squared() < 0.1 { Vec3::new(0.0, 0.707, 0.707) } else { dir };

    // 1. Galaxy close-up: camera dives onto the planet sphere
    let galaxy_close_dist = (planet_radius * 2.2).max(6.0);
    let galaxy_close_pos = planet_pos + dir * galaxy_close_dist;
    let galaxy_close_pose = Transform::from_translation(galaxy_close_pos).looking_at(planet_pos, Vec3::Y);

    // 2. Planet view scene close-up: matches the planet scale & viewing direction
    let pv_planet_pos = ORIGIN + Vec3::new(-15.8, -1.0, -15.0);
    let pv_close_pos = pv_planet_pos + dir * (24.0 * 2.2);
    let pv_close_pose = Transform::from_translation(pv_close_pos).looking_at(pv_planet_pos, Vec3::Y);

    if b < 0.5 {
        let t = b / 0.5;
        let t = t * t * (3.0 - 2.0 * t);
        Transform {
            translation: orbit_pos.lerp(galaxy_close_pose.translation, t),
            rotation: orbit_rot.slerp(galaxy_close_pose.rotation, t),
            scale: Vec3::ONE,
        }
    } else {
        let t = (b - 0.5) / 0.5;
        let t = t * t * (3.0 - 2.0 * t);
        Transform {
            translation: pv_close_pose.translation.lerp(overview_pose.translation, t),
            rotation: pv_close_pose.rotation.slerp(overview_pose.rotation, t),
            scale: Vec3::ONE,
        }
    }
}

fn view_camera(
    mut state: ResMut<PlanetViewState>,
    view: Res<GalaxyView>,
    radii: Option<Res<PlanetRadii>>,
    mut cam: Single<&mut Transform, With<GalaxyCamera>>,
) {
    if state.blend <= 0.0 {
        return;
    }
    if state.active && state.start_pos.is_none() {
        state.start_pos = Some(cam.translation);
        state.start_rot = Some(cam.rotation);
    }
    let overview_pose = Transform::from_translation(ORIGIN + Vec3::new(0.0, 0.0, -1.8))
        .looking_at(ORIGIN + Vec3::new(0.0, -1.95, -10.0), Vec3::Y);

    let Some((idx, p)) = state.planet.and_then(|i| view.planets.get(i).map(|p| (i, p))) else {
        let t = state.blend * state.blend * (3.0 - 2.0 * state.blend);
        let start_pos = state.start_pos.unwrap_or(cam.translation);
        let start_rot = state.start_rot.unwrap_or(cam.rotation);
        cam.translation = start_pos.lerp(overview_pose.translation, t);
        cam.rotation = start_rot.slerp(overview_pose.rotation, t);
        return;
    };

    let planet_pos = crate::galaxy_map::galaxy_to_world(p.position);
    let r = radii.as_ref().and_then(|rad| rad.0.get(idx)).copied().unwrap_or(8.0).max(4.0);

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
    let alpha = (std::f32::consts::PI * state.blend).sin().powi(4) * 0.25;
    let visible = state.blend > 0.0 && state.blend < 1.0;
    match quad.single_mut() {
        Ok((e, mut tf, mat)) => {
            if !visible {
                commands.entity(e).despawn();
                return;
            }
            tf.translation = cam.translation + cam.rotation * Vec3::NEG_Z * 1.6;
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
    t.push_str(&format!("\n@Action\tRight-click to sell for {:.0} credits", (st.cost / 3.0).floor()));
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
    app.init_resource::<OrbitSlots>().add_systems(Update, (view_tooltip.after(crate::play_ui::fleet_tooltip), sell_click).run_if(in_state(AppScreen::Galaxy)));
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

/// Right click on a built ground / orbital structure in the planet view sells it for a third of its cost (a journaled sim command).
#[allow(clippy::too_many_arguments)]
fn sell_click(
    mouse: Res<ButtonInput<MouseButton>>,
    state: Res<PlanetViewState>,
    sim: Option<Res<SimState>>,
    catalog: Option<Res<PlayCatalog>>,
    bridge: Option<Res<crate::play::PlayBridge>>,
    window: Option<Single<&Window, With<bevy::window::PrimaryWindow>>>,
    cam: Single<(&Camera, &GlobalTransform), With<GalaxyCamera>>,
    mut actions: MessageWriter<galaxy_play::PlayAction>,
    mut toast: ResMut<crate::play_ui::Toast>,
) {
    if !mouse.just_pressed(MouseButton::Right) || !state.active || state.blend < 1.0 {
        return;
    }
    let (Some(sim), Some(cat), Some(bridge)) = (sim, catalog, bridge) else { return };
    let Some(sp) = state.planet.and_then(|p| bridge.planet_to_sim.get(p).copied().flatten()) else { return };
    let Some(c) = window.and_then(|w| w.cursor_position()) else { return };
    let (camera, gt) = *cam;
    let project = |p: Vec3| camera.world_to_viewport(gt, p).ok();
    let mut points: Vec<(Hover, Vec2)> = Vec::new();
    points.extend(state.ground_slots.iter().enumerate().filter_map(|(i, p)| project(*p + Vec3::new(0.0, 0.2, 0.0)).map(|v| (Hover::Ground(i), v))));
    points.extend(state.orbital_slots.iter().enumerate().filter_map(|(i, p)| project(*p + Vec3::new(0.0, 0.15, 0.0)).map(|v| (Hover::Orbital(i), v))));
    let (orbital, n) = match pick(&points, c, 30.0) {
        Some(Hover::Ground(i)) => (false, i),
        Some(Hover::Orbital(i)) => (true, i),
        _ => return,
    };
    let Some(ps) = sim.galaxy().planets.get(sp as usize) else { return };
    let built = ps.structures.iter().copied().filter(|&s| sim.world.structures.get(s as usize).is_some_and(|d| d.orbital == orbital)).nth(n);
    if let Some(s) = built {
        actions.write(galaxy_play::PlayAction::SellStructure { planet: sp, structure: s });
        toast.0 = format!("{} sold for {:.0} credits", cat.structure(s), sim.world.structures[s as usize].cost / 3.0);
        toast.1 = 3.0;
    }
}
