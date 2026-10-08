//! 3D stations next to planets on the galaxy map: the `Galactic_Model_Name` of the planet's star base level
//! (`Empire_Starbase_LVL_n_gc.ALO` and kin) standing on a small faction-coloured disc, slowly turning.

use std::collections::HashMap;

use alamo_render::{AlamoFs, AlamoModel, AlamoModelLoaded};
use bevy::prelude::*;
use galaxy_play::{GalaxyLayout, PlayCatalog, PlayerFaction, SimState};

/// Draws the stations.
pub struct GalaxyModelsPlugin;

impl Plugin for GalaxyModelsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Stations>().add_systems(
            Update,
            (sync_stations, fit_stations)
                .chain()
                .after(galaxy_play::PlaySet::Visuals)
                .run_if(resource_exists::<SimState>)
                .run_if(resource_exists::<PlayCatalog>)
                .run_if(resource_exists::<GalaxyLayout>)
                .run_if(resource_exists::<AlamoFs>),
        );
    }
}

/// Station root per planet, with the star base level it shows.
#[derive(Resource, Default)]
struct Stations(HashMap<u32, (Entity, u32)>);

/// Root of a station (turns slowly).
#[derive(Component)]
struct Station;

/// The model child of a station.
#[derive(Component)]
struct StationModel;

/// Angle (XZ plane) of the station slot around a planet: left of the three fleet slots.
const STATION_ANGLE: f32 = -3.45;

#[allow(clippy::too_many_arguments)]
fn sync_stations(
    mut commands: Commands,
    mut stations: ResMut<Stations>,
    state: Res<SimState>,
    catalog: Res<PlayCatalog>,
    layout: Res<GalaxyLayout>,
    player: Res<PlayerFaction>,
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut roots: Query<&mut Transform, With<Station>>,
) {
    let g = state.galaxy();
    for (i, p) in g.planets.iter().enumerate() {
        let id = i as u32;
        let model = p.starbase.and_then(|s| catalog.starbase_models.get(s as usize).cloned().flatten());
        let level = p.starbase.map_or(0, |s| s + 1);
        let want = model.is_some().then_some(level);
        match (stations.0.get(&id).copied(), want) {
            (Some((e, l)), Some(w)) if l == w => {
                if let Ok(mut tf) = roots.get_mut(e) {
                    tf.rotation = Quat::from_rotation_y(time.elapsed_secs() * 0.25);
                }
            }
            (old, Some(w)) => {
                if let Some((e, _)) = old {
                    commands.entity(e).despawn();
                }
                let c = layout.positions[i];
                let r = layout.orbit_radius(i as u32);
                let pos = c + Vec3::new(STATION_ANGLE.cos() * r, layout.unit * 0.25, STATION_ANGLE.sin() * r);
                let own = p.owner.is_some() && p.owner == player.0;
                let col = if own { Color::srgb(0.1, 0.9, 0.2) } else if p.owner.is_some() { Color::srgb(0.9, 0.2, 0.15) } else { Color::srgb(0.5, 0.5, 0.5) };
                let lin = col.to_linear();
                let root = commands.spawn((Name::new("station"), Station, Transform::from_translation(pos), Visibility::default())).id();
                commands.spawn((
                    Mesh3d(meshes.add(Circle::new(1.0).mesh().resolution(48))),
                    MeshMaterial3d(mats.add(StandardMaterial {
                        base_color: Color::LinearRgba(LinearRgba::new(lin.red * 0.12, lin.green * 0.12, lin.blue * 0.05, 0.88)),
                        unlit: true,
                        depth_bias: 400.0,
                        alpha_mode: AlphaMode::Blend,
                        cull_mode: None,
                        ..default()
                    })),
                    // The disc stays flat: it is a sibling of the turning root, not a child.
                    Transform::from_translation(pos - Vec3::Y * layout.unit * 0.12)
                        .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                        .with_scale(Vec3::splat(layout.unit * 0.7)),
                    StationDisc(root),
                ));
                if let Some(m) = model {
                    commands.spawn((AlamoModel::new(m).with_team_color(col), Transform::default(), StationModel, ChildOf(root)));
                }
                stations.0.insert(id, (root, w));
            }
            (Some((e, _)), None) => {
                commands.entity(e).despawn();
                stations.0.remove(&id);
            }
            (None, None) => {}
        }
    }
}

/// Flat disc under a station; removed with it.
#[derive(Component)]
struct StationDisc(Entity);

/// Scales freshly loaded station models to a fixed size and centres them.
fn fit_stations(
    mut commands: Commands,
    layout: Res<GalaxyLayout>,
    mut loaded: Query<(&AlamoModelLoaded, &mut Transform), (With<StationModel>, Added<AlamoModelLoaded>)>,
    discs: Query<(Entity, &StationDisc)>,
    roots: Query<(), With<Station>>,
) {
    for (m, mut tf) in &mut loaded {
        let s = if m.radius > 0.0 { layout.unit * 0.6 / m.radius } else { 1.0 };
        tf.scale = Vec3::splat(s);
        tf.translation = -m.center * s;
    }
    for (e, d) in &discs {
        if !roots.contains(d.0) {
            commands.entity(e).despawn();
        }
    }
}
