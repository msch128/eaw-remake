//! Heroes next to the fleets of the galaxy map: like the original, a hero is its normal unit model
//! (`Land_Model_Name`, or the space model for ship heroes) scaled down on its own disc left and right
//! of the fleet, turning slowly.

use std::collections::HashMap;

use alamo_render::{AlamoModel, AlamoModelLoaded};
use bevy::prelude::*;
use galaxy_play::{FleetEntities, GalaxyLayout, PlayCatalog, PlayerFaction, SimState};

/// Draws the heroes.
pub struct HeroHeadsPlugin;

impl Plugin for HeroHeadsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HeroHeads>().add_systems(
            Update,
            (update_hero_heads, fit_heroes)
                .chain()
                .after(galaxy_play::PlaySet::Visuals)
                .run_if(resource_exists::<SimState>)
                .run_if(resource_exists::<PlayCatalog>)
                .run_if(resource_exists::<GalaxyLayout>),
        );
    }
}

/// Hero root entity by (fleet id, unit type).
#[derive(Resource, Default)]
struct HeroHeads(HashMap<(u32, u32), Entity>);

/// Root of a hero figure.
#[derive(Component)]
struct HeroRoot;

/// Model child of a hero root.
#[derive(Component)]
struct HeroModel;

#[allow(clippy::too_many_arguments)]
fn update_hero_heads(
    mut commands: Commands,
    mut heads: ResMut<HeroHeads>,
    state: Res<SimState>,
    catalog: Res<PlayCatalog>,
    layout: Res<GalaxyLayout>,
    player: Res<PlayerFaction>,
    time: Res<Time>,
    entities: Option<Res<FleetEntities>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    transforms: Query<&GlobalTransform, With<galaxy_play::FleetVisual>>,
    mut roots: Query<&mut Transform, (With<HeroRoot>, Without<galaxy_play::FleetVisual>)>,
) {
    let Some(entities) = entities else { return };
    let g = state.galaxy();
    let fake = std::env::var_os("SP_HERO_FAKE").is_some();
    let mut alive: Vec<(u32, u32)> = Vec::new();
    for f in g.fleets.values() {
        let Some(&e) = entities.0.get(&f.id) else { continue };
        let Ok(tf) = transforms.get(e) else { continue };
        let heroes: Vec<u32> = f
            .units
            .iter()
            .filter(|&(&u, &n)| n > 0 && (fake || state.world.units.get(u as usize).is_some_and(|d| d.hero)))
            .map(|(&u, _)| u as u32)
            .take(4)
            .collect();
        for (k, &u) in heroes.iter().enumerate() {
            let key = (f.id as u32, u);
            alive.push(key);
            // Left and right of the fleet base, further out for more heroes.
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            let pos = tf.translation() + Vec3::new(side * layout.unit * 1.5 * (1.0 + (k / 2) as f32), 0.0, 0.0);
            if let Some(&root) = heads.0.get(&key) {
                if let Ok(mut t) = roots.get_mut(root) {
                    t.translation = pos;
                    t.rotation = Quat::from_rotation_y(time.elapsed_secs() * 0.4);
                }
                continue;
            }
            let model = catalog
                .unit_land_models
                .get(u as usize)
                .cloned()
                .flatten()
                .or_else(|| catalog.unit_space_models.get(u as usize).cloned().flatten());
            let Some(model) = model else { continue };
            let own = player.0 == Some(f.owner);
            let col = if own { Color::srgb(0.1, 0.9, 0.2) } else { Color::srgb(0.9, 0.2, 0.15) };
            let lin = col.to_linear();
            let root = commands
                .spawn((Name::new("hero"), HeroRoot, Transform::from_translation(pos), Visibility::default()))
                .id();
            commands.spawn((
                Mesh3d(meshes.add(Circle::new(1.0).mesh().resolution(48))),
                MeshMaterial3d(mats.add(StandardMaterial {
                    base_color: Color::LinearRgba(LinearRgba::new(lin.red * 0.2, lin.green * 0.2, lin.blue * 0.2, 0.75)),
                    unlit: true,
                    alpha_mode: AlphaMode::Blend,
                    cull_mode: None,
                    ..default()
                })),
                Transform::from_xyz(0.0, -layout.unit * 0.12, 0.0)
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                    .with_scale(Vec3::splat(layout.unit * 0.7)),
                ChildOf(root),
            ));
            commands.spawn((AlamoModel::new(model).with_team_color(col), Transform::default(), HeroModel, ChildOf(root)));
            heads.0.insert(key, root);
        }
    }
    heads.0.retain(|key, e| {
        if alive.contains(key) {
            true
        } else {
            commands.entity(*e).despawn();
            false
        }
    });
}

/// Scales freshly loaded hero models to a fixed size, centred on the disc.
fn fit_heroes(layout: Res<GalaxyLayout>, mut loaded: Query<(&AlamoModelLoaded, &mut Transform), (With<HeroModel>, Added<AlamoModelLoaded>)>) {
    for (m, mut tf) in &mut loaded {
        let s = if m.radius > 0.0 { layout.unit * 0.6 / m.radius } else { 1.0 };
        tf.scale = Vec3::splat(s);
        tf.translation = -m.center * s;
    }
}
