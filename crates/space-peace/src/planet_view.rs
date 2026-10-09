//! The planet view: scrolling all the way in on the selected planet (or Esc / scrolling out to leave) flies the
//! camera into a separate 3D scene far below the galaxy: a large planet sphere on the left, the planet's star
//! base model, and the tan base-layout platform (`W_BuildBaseFOC.alo`). The 2D UI around it (orbit boxes,
//! description, "Base layout" button) hangs off [`PlanetViewState`].

use alamo_render::{AlamoFs, AlamoModel, AlamoModelLoaded};
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use galaxy_play::{PlayCatalog, SimState};

use crate::camera::{CameraBounds, GalaxyCamera};
use crate::flow::AppScreen;
use crate::galaxy_map::Selection;
use crate::galaxy_view::GalaxyView;

/// Shared state of the planet view.
#[derive(Resource, Debug, Default)]
pub struct PlanetViewState {
    /// The view is open (or flying in).
    pub active: bool,
    /// Planet index in `GalaxyView::planets`.
    pub planet: Option<usize>,
    /// 0 = galaxy map, 1 = fully in the planet view (eased).
    pub blend: f32,
    /// World positions of the three rectangular ground-structure frames on the base layout (left to right).
    pub ground_slots: Vec<Vec3>,
    /// World positions of the six round orbital/defence slots on the front strip (left to right).
    pub orbital_slots: Vec<Vec3>,
}

/// Registers the planet view.
pub struct PlanetViewPlugin;

impl Plugin for PlanetViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlanetViewState>().add_systems(
            Update,
            (view_input, view_scene, view_camera.after(crate::camera::apply_camera), view_fade, fit_models)
                .chain()
                .run_if(in_state(AppScreen::Galaxy)),
        );
        app.add_systems(Update, wheel_test.before(view_input).run_if(in_state(AppScreen::Galaxy)));
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
            let at = Vec3::new(at.x, 0.0, at.z);
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
    fn a_small_wheel_zoom_near_the_planet_opens_the_view() {
        assert!(wheel_opens(1.0, 1.0, 0.0), "at the closest zoom");
        assert!(wheel_opens(1.0, 1.9, 0.3), "a few notches out, planet near the centre");
        assert!(!wheel_opens(1.0, 6.0, 0.0), "far away: the wheel only zooms the map");
        assert!(!wheel_opens(1.0, 1.0, 2.0), "the selected planet is not where the camera looks");
        assert!(!wheel_opens(-1.0, 1.0, 0.0), "wheel out never opens");
        assert!(!wheel_opens(0.0, 1.0, 0.0), "no wheel, no view");
    }
}

#[allow(clippy::too_many_arguments)]
fn view_scene(
    mut commands: Commands,
    time: Res<Time>,
    mut state: ResMut<PlanetViewState>,
    view: Res<GalaxyView>,
    sim: Option<Res<SimState>>,
    catalog: Option<Res<PlayCatalog>>,
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
    }
    // Station: the model of the planet's star base level.
    let (sim_for_structs, cat_for_structs) = (sim.as_deref(), catalog.as_deref());
    if let (Some(sim), Some(cat)) = (sim_for_structs, cat_for_structs)
        && let Some(sb) = sim.galaxy().planets.get(p.0).and_then(|s| s.starbase)
        && let Some(Some(m)) = cat.starbase_models.get(sb as usize)
    {
        commands.spawn((ViewEntity, AlamoModel::new(m.clone()), FitModel { radius: 1.1, spin: 0.15 }, Visibility::Hidden, Transform::from_translation(ORIGIN + Vec3::new(-2.6, -1.5, -9.0))));
    }
    if let Some(tex) = textures.as_mut().and_then(|t| t.get(&mut images, "i_galactic_organize_land.dds")) {
        spawn_platform(&mut commands, &mut meshes, &mut mats, tex.handle, land_cell(p.1.terrain.as_deref()), ORIGIN + Vec3::new(0.0, -2.5, -10.0));
    }
    // Slot positions on the platform quad (centre `plat`, 3.8 x 1.9, facing +Z): read off the art.
    let plat = ORIGIN + Vec3::new(0.0, -2.5, -10.0);
    state.ground_slots = [-0.9f32, 0.0, 0.95].iter().map(|x| plat + Vec3::new(*x, -0.07, 0.05)).collect();
    state.orbital_slots = (0..6).map(|k| plat + Vec3::new((k as f32 - 2.5) * 0.62, -0.74, 0.05)).collect();
    // Orbital slot circles of the second pad (the art is an empty strip).
    let ring = mats.add(StandardMaterial { base_color: Color::srgb(0.85, 0.78, 0.55), unlit: true, depth_bias: 50.0, cull_mode: None, ..default() });
    for at in &state.orbital_slots {
        commands.spawn((
            ViewEntity,
            Name::new("orbital slot ring"),
            Mesh3d(meshes.add(Annulus::new(0.125, 0.14))),
            MeshMaterial3d(ring.clone()),
            Transform::from_translation(plat + Vec3::new(at.x - plat.x, 0.0, 0.0) * 0.66 + Vec3::new(0.0, -0.64, 0.08)).with_scale(Vec3::new(1.0, 0.8, 1.0)),
        ));
    }
    if let Some(sim) = sim_for_structs {
        let cat_for_structs = cat_for_structs;
        let (mut g, mut o) = (0usize, 0usize);
        for &s in &sim.galaxy().planets.get(p.0).map(|pl| pl.structures.clone()).unwrap_or_default() {
            let Some(def) = sim.world.structures.get(s as usize) else { continue };
            let Some(Some(model)) = cat_for_structs.and_then(|c| c.structure_models.get(s as usize)) else { continue };
            let slot = if def.orbital { o += 1; state.orbital_slots.get(o - 1) } else { g += 1; state.ground_slots.get(g - 1) };
            if let Some(at) = slot {
                commands.spawn((ViewEntity, AlamoModel::new(model.clone()), FitModel { radius: if def.orbital { 0.22 } else { 0.5 }, spin: 0.3 }, Visibility::Hidden, Transform::from_translation(*at + Vec3::Y * 0.2)));
            }
        }
    }
}

fn view_camera(state: Res<PlanetViewState>, mut cam: Single<&mut Transform, With<GalaxyCamera>>) {
    if state.blend <= 0.0 {
        return;
    }
    let t = state.blend * state.blend * (3.0 - 2.0 * state.blend);
    let target = Transform::from_translation(ORIGIN + Vec3::new(0.0, 0.0, -1.8)).looking_at(ORIGIN + Vec3::new(0.0, -1.95, -10.0), Vec3::Y);
    cam.translation = cam.translation.lerp(target.translation, t);
    cam.rotation = cam.rotation.slerp(target.rotation, t);
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

/// Black quad in front of the camera: the galaxy fades to black while the camera flies in, and the planet
/// scene fades in (and the reverse when leaving).
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
    let alpha = (std::f32::consts::PI * state.blend).sin().powi(2).powf(0.7);
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
    if st.done || std::env::var_os("SP_PLANET_VIEW_TEST").is_none() {
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
