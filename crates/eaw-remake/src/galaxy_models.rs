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
            (sync_stations, fit_stations, hide_in_planet_view)
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
struct StationModel(f32);

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
                let pad = layout.pad_radius(i as u32);
                let pos = c + Vec3::new(STATION_ANGLE.cos() * r, pad * 0.3, STATION_ANGLE.sin() * r);
                let own = p.owner.is_some() && p.owner == player.0;
                let col = if own { Color::srgb(0.1, 0.9, 0.2) } else if p.owner.is_some() { Color::srgb(0.9, 0.2, 0.15) } else { Color::srgb(0.5, 0.5, 0.5) };
                let lin = col.to_linear();
                let root = commands.spawn((Name::new("station"), Station, Transform::from_translation(pos), Visibility::default())).id();
                commands.spawn((
                    Mesh3d(meshes.add(Circle::new(1.0).mesh().resolution(48))),
                    MeshMaterial3d(mats.add(StandardMaterial {
                        base_color: Color::LinearRgba(LinearRgba::new(lin.red * 0.02, lin.green * 0.07, lin.blue * 0.02, 0.92)),
                        unlit: true,
                        depth_bias: 400.0,
                        alpha_mode: AlphaMode::Blend,
                        cull_mode: None,
                        ..default()
                    })),
                    // The disc stays flat: it is a sibling of the turning root, not a child.
                    Transform::from_translation(pos - Vec3::Y * pad * 0.26)
                        .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                        .with_scale(Vec3::splat(pad)),
                    StationDisc(root),
                ));
                if let Some(m) = model {
                    commands.spawn((station_model(&m, col, pad), ChildOf(root)));
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

/// A station's model, hidden until `fit_stations` has scaled it: alamo-render spawns the parts in PostUpdate, after the fit, so a
/// new or upgraded star base showed at its own size for one frame (like the hero ships that flashed the map white, see
/// `galaxy_play::visuals::spawn_model`).
fn station_model(model: &str, col: Color, pad: f32) -> impl Bundle {
    (AlamoModel::new(model).with_team_color(col), Transform::default(), Visibility::Hidden, StationModel(pad))
}

/// Scales freshly loaded station models to a fixed size, centres them and shows them.
fn fit_stations(
    mut commands: Commands,
    mut loaded: Query<(&AlamoModelLoaded, &mut Transform, &mut Visibility, &StationModel), Added<AlamoModelLoaded>>,
    discs: Query<(Entity, &StationDisc)>,
    roots: Query<(), With<Station>>,
) {
    for (m, mut tf, mut vis, station) in &mut loaded {
        // About 1.6 pad radii of model radius: the station stands on its pad like the ships (it was 0.6 `unit`, far bigger than the pads).
        let s = if m.radius > 0.0 { station.0 * 1.6 / m.radius } else { 1.0 };
        tf.scale = Vec3::splat(s);
        tf.translation = -m.center * s;
        *vis = Visibility::Inherited;
    }
    for (e, d) in &discs {
        if !roots.contains(d.0) {
            commands.entity(e).despawn();
        }
    }
}

/// Hides the map stations while the planet view is open (they would show through into its scene).
fn hide_in_planet_view(
    state: Res<crate::planet_view::PlanetViewState>,
    mut q: Query<&mut Visibility, Or<(With<Station>, With<StationDisc>)>>,
) {
    let want = if state.active || state.blend > 0.0 { Visibility::Hidden } else { Visibility::Inherited };
    for mut v in &mut q {
        if *v != want {
            *v = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stand-in for alamo-render, which finishes a model in PostUpdate, after the Update fit: here every model loads in its second
    /// frame, radius 800 like a star base.
    fn finish_models(mut commands: Commands, models: Query<Entity, (With<AlamoModel>, Without<AlamoModelLoaded>)>, mut frame: Local<u32>) {
        *frame += 1;
        if *frame < 2 {
            return;
        }
        for e in &models {
            let axis = commands.spawn((Transform::default(), Visibility::Inherited, ChildOf(e))).id();
            commands.entity(e).insert(AlamoModelLoaded {
                bones: Vec::new(),
                parts: Vec::new(),
                proxies: Vec::new(),
                axis,
                hardpoints: Vec::new(),
                bone_by_name: Default::default(),
                center: Vec3::ZERO,
                radius: 800.0,
            });
        }
    }

    #[test]
    fn a_new_station_model_never_shows_at_its_own_size() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_systems(Update, fit_stations).add_systems(PostUpdate, finish_models);
        let root = app.world_mut().spawn((Station, Transform::default(), Visibility::default())).id();
        app.world_mut().spawn((station_model("e_station_4.alo", Color::WHITE, 2.0), ChildOf(root)));
        let frames: Vec<(bool, f32)> = (0..6)
            .map(|_| {
                app.update();
                let (vis, tf) = app.world_mut().query_filtered::<(&Visibility, &Transform), With<StationModel>>().single(app.world()).unwrap();
                (*vis != Visibility::Hidden, tf.scale.x)
            })
            .collect();
        // Shown only once fitted to its pad, and shown in the end.
        assert!(frames.iter().all(|&(shown, scale)| !shown || scale < 0.1), "{frames:?}");
        assert!(frames.last().is_some_and(|f| f.0), "{frames:?}");
    }
}
