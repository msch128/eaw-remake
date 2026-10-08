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
}

/// Registers the planet view.
pub struct PlanetViewPlugin;

impl Plugin for PlanetViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlanetViewState>().add_systems(
            Update,
            (view_input, view_scene, view_camera.after(crate::camera::apply_camera), fit_models)
                .chain()
                .run_if(in_state(AppScreen::Galaxy)),
        );
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

fn view_input(
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    selection: Res<Selection>,
    bounds: Res<CameraBounds>,
    cam: Single<&GalaxyCamera>,
    mut state: ResMut<PlanetViewState>,
) {
    if !state.active {
        if std::env::var_os("SP_PLANET_VIEW").is_some() && selection.0.is_some() {
            state.active = true;
            state.planet = selection.0;
            return;
        }
        let at_min = cam.target_distance <= bounds.min_distance * 1.02;
        if scroll.delta.y > 0.0 && at_min && let Some(p) = selection.0 {
            state.active = true;
            state.planet = Some(p);
        }
    } else if keys.just_pressed(KeyCode::Escape) || scroll.delta.y < 0.0 {
        state.active = false;
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
    state.blend = if target > state.blend { (state.blend + step).min(1.0) } else { (state.blend - step).max(0.0) };
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
        commands.spawn((ViewEntity, AlamoModel::new(model.clone()), FitModel { radius: 13.0, spin: 0.01 }, Transform::from_translation(ORIGIN + Vec3::new(-17.5, -1.0, -15.0))));
    }
    // Station: the model of the planet's star base level.
    if let (Some(sim), Some(cat)) = (sim, catalog)
        && let Some(sb) = sim.galaxy().planets.get(p.0).and_then(|s| s.starbase)
        && let Some(Some(m)) = cat.starbase_models.get(sb as usize)
    {
        commands.spawn((ViewEntity, AlamoModel::new(m.clone()), FitModel { radius: 1.1, spin: 0.15 }, Transform::from_translation(ORIGIN + Vec3::new(-2.6, -1.5, -9.0))));
    }
    if let Some(tex) = textures.as_mut().and_then(|t| t.get(&mut images, "i_galactic_organize_land.dds")) {
        spawn_platform(&mut commands, &mut meshes, &mut mats, tex.handle, land_cell(p.1.terrain.as_deref()), ORIGIN + Vec3::new(0.0, -2.5, -10.0));
    }
}

fn view_camera(state: Res<PlanetViewState>, mut cam: Single<&mut Transform, With<GalaxyCamera>>) {
    if state.blend <= 0.0 {
        return;
    }
    let t = state.blend * state.blend * (3.0 - 2.0 * state.blend);
    let target = Transform::from_translation(ORIGIN).looking_at(ORIGIN + Vec3::new(0.0, -1.6, -10.0), Vec3::Y);
    cam.translation = cam.translation.lerp(target.translation, t);
    cam.rotation = cam.rotation.slerp(target.rotation, t);
}

fn fit_models(time: Res<Time>, mut models: Query<(&FitModel, &mut Transform, Option<Ref<AlamoModelLoaded>>)>) {
    for (f, mut tf, loaded) in &mut models {
        if let Some(m) = loaded
            && m.is_added()
        {
            tf.scale = Vec3::splat(if m.radius > 0.0 { f.radius / m.radius } else { 1.0 });
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
