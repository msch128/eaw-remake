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
            (update_hero_heads, face_camera, hide_in_planet_view)
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
    filters: Res<crate::galaxy_map::MapFilters>,
    fog: Res<galaxy_play::Fog>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    textures: Option<ResMut<eaw_ui::textures::UiTextures>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut roots: Query<&mut Transform, With<HeroRoot>>,
    fleet_entities: Res<galaxy_play::FleetEntities>,
    fleet_tf: Query<&Transform, (With<galaxy_play::FleetVisual>, Without<HeroRoot>)>,
) {
    let mut textures = textures;
    let cam_rot = cameras.iter().next().map(|c| c.rotation());
    let g = state.galaxy();
    let fake = std::env::var_os("SP_HERO_FAKE").is_some();
    let mut alive: Vec<(u32, u32)> = Vec::new();
    let cam_right = cam_rot.map_or(Vec3::X, |r| r * Vec3::X);
    for f in g.fleets.values().filter(|f| fog.fleet(f.id) == galaxy_sim::Sight::Detailed) {
        let Some(p) = f.planet() else { continue };
        // Heroes stand left and right of the fleet itself (its slot above the planet).
        let base = fleet_entities.0.get(&f.id).and_then(|&e| fleet_tf.get(e).ok()).map_or(layout.positions[p as usize], |t| t.translation);
        let heroes: Vec<u32> = f
            .units
            .iter()
            .filter(|&(&u, &n)| n > 0 && (fake || state.world.units.get(u as usize).is_some_and(|d| d.hero)))
            .map(|(&u, _)| u)
            .take(4)
            .collect();
        for (k, &u) in heroes.iter().enumerate() {
            let key = (f.id, u);
            alive.push(key);
            let side = if k % 2 == 0 { -1.0 } else { 1.0 };
            let off = layout.unit * (0.62 + 0.5 * (k / 2) as f32);
            let pos = base + cam_right * side * off + Vec3::Y * layout.unit * if filters.heroes { 0.0 } else { -0.15 };
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
                Mesh3d(meshes.add(Annulus::new(0.01, 0.92).mesh().resolution(48))),
                MeshMaterial3d(mats.add(StandardMaterial {
                    base_color: Color::srgb(0.0, 0.035, 0.01),
                    unlit: true,
                    depth_bias: 401.0,
                    alpha_mode: AlphaMode::Opaque,
                    cull_mode: None,
                    ..default()
                })),
                Transform::from_xyz(0.0, -layout.unit * 0.12, 0.0)
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                    .with_scale(Vec3::splat(layout.unit * 0.3)),
                ChildOf(root),
            ));
            // Bright rim around the dark disc.
            commands.spawn((
                Mesh3d(meshes.add(Annulus::new(0.9, 1.0).mesh().resolution(48))),
                MeshMaterial3d(mats.add(StandardMaterial { base_color: col, emissive: lin * 2.0, unlit: true, depth_bias: 402.0, cull_mode: None, ..default() })),
                Transform::from_xyz(0.0, -layout.unit * 0.11, 0.0)
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                    .with_scale(Vec3::splat(layout.unit * 0.3)),
                ChildOf(root),
            ));
            // Upright cut-out portrait standing on the disc, facing the camera.
            let h = layout.unit * 0.5;
            commands.spawn((
                Mesh3d(meshes.add(Rectangle::new(h, h))),
                MeshMaterial3d(mats.add(StandardMaterial {
                    base_color_texture: Some(tex.handle.clone()),
                    unlit: true,
                    alpha_mode: AlphaMode::Mask(0.5),
                    cull_mode: None,
                    ..default()
                })),
                Transform::from_xyz(0.0, h * 0.42, 0.0).with_rotation(cam_rot.unwrap_or(Quat::IDENTITY)),
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

/// Hides the hero portraits while the planet view is open.
fn hide_in_planet_view(state: Res<crate::planet_view::PlanetViewState>, mut q: Query<&mut Visibility, With<HeroRoot>>) {
    let want = if state.active || state.blend > 0.0 { Visibility::Hidden } else { Visibility::Inherited };
    for mut v in &mut q {
        if *v != want {
            *v = want;
        }
    }
}

