//! Heroes next to the fleets of the galaxy map: like the original, a hero is its normal unit model
//! (`Land_Model_Name`, or the space model for ship heroes) scaled down on its own disc left and right
//! of the fleet, turning slowly.

use std::collections::HashMap;

use bevy::prelude::*;
use galaxy_play::{GalaxyLayout, PlayCatalog, PlayerFaction, SimState};

/// Draws the heroes.
pub struct HeroHeadsPlugin;

impl Plugin for HeroHeadsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HeroHeads>().add_systems(
            Update,
            (update_hero_heads, face_camera)
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
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    textures: Option<ResMut<eaw_ui::textures::UiTextures>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut roots: Query<&mut Transform, With<HeroRoot>>,
) {
    let mut textures = textures;
    let cam_rot = cameras.iter().next().map(|c| c.rotation());
    let g = state.galaxy();
    let fake = std::env::var_os("SP_HERO_FAKE").is_some();
    let mut alive: Vec<(u32, u32)> = Vec::new();
    let mut per_planet: HashMap<u32, usize> = HashMap::new();
    for f in g.fleets.values() {
        let Some(p) = f.planet() else { continue };
        let n_here = per_planet.entry(p).or_insert(0usize);
        let base = layout.positions[p as usize];
        let heroes: Vec<u32> = f
            .units
            .iter()
            .filter(|&(&u, &n)| n > 0 && (fake || state.world.units.get(u as usize).is_some_and(|d| d.hero)))
            .map(|(&u, _)| u)
            .take(4)
            .collect();
        for (k, &u) in heroes.iter().enumerate() {
            let key = (f.id as u32, u);
            alive.push(key);
            // Left and right of the fleet base, further out for more heroes.
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            let _ = (side, k);
            // Heroes stand right of the fleet slots above the planet, then on the left, lower each time.
            let slot = *n_here;
            *n_here += 1;
            let a = if slot % 2 == 0 { -1.45 + 0.3 * (slot / 2) as f32 } else { -1.7 - 0.3 * (slot / 2) as f32 - 0.4 };
            let r = layout.orbit_radius(p) * 1.05;
            let pos = base + Vec3::new(a.cos() * r, layout.unit * 0.25, a.sin() * r);
            if let Some(&root) = heads.0.get(&key) {
                if let Ok(mut t) = roots.get_mut(root) {
                    t.translation = pos;
                    t.rotation = Quat::IDENTITY;
                }
                continue;
            }
            let Some(icon) = catalog.unit_icons.get(u as usize).cloned().flatten() else { continue };
            let Some(tex) = textures.as_mut().and_then(|t| t.get(&mut images, &icon)) else { continue };
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
                    .with_scale(Vec3::splat(layout.unit * 0.55)),
                ChildOf(root),
            ));
            // Upright cut-out portrait standing on the disc, facing the camera.
            let h = layout.unit * 0.8;
            commands.spawn((
                Mesh3d(meshes.add(Rectangle::new(h, h))),
                MeshMaterial3d(mats.add(StandardMaterial {
                    base_color_texture: Some(tex.handle.clone()),
                    unlit: true,
                    alpha_mode: AlphaMode::Mask(0.5),
                    cull_mode: None,
                    ..default()
                })),
                Transform::from_xyz(0.0, h * 0.5, 0.0).with_rotation(cam_rot.unwrap_or(Quat::IDENTITY)),
                ChildOf(root),
                HeroModel,
            ));
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

/// Turns the portraits to face the camera.
fn face_camera(cameras: Query<&GlobalTransform, With<Camera3d>>, mut q: Query<(&mut Transform, &ChildOf), With<HeroModel>>, parents: Query<&GlobalTransform, With<HeroRoot>>) {
    let Some(cam) = cameras.iter().next() else { return };
    for (mut t, parent) in &mut q {
        let parent_rot = parents.get(parent.parent()).map_or(Quat::IDENTITY, |p| p.rotation());
        t.rotation = parent_rot.inverse() * cam.rotation();
    }
}
