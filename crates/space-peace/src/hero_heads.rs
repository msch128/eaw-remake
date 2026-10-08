//! Hero portraits next to the fleets of the galaxy map: the hero's button portrait inside the game's
//! round hero ring (`HERO_RING_GOOD` for the player, `HERO_RING_EVIL` for others), left and right of
//! the fleet base like in the original.

use std::collections::HashMap;

use bevy::prelude::*;
use eaw_ui::textures::UiTextures;
use galaxy_play::{FleetEntities, PlayCatalog, PlayerFaction, SimState};

/// Draws the portraits.
pub struct HeroHeadsPlugin;

impl Plugin for HeroHeadsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HeroHeads>().add_systems(
            Update,
            update_hero_heads
                .after(galaxy_play::PlaySet::Visuals)
                .run_if(resource_exists::<SimState>)
                .run_if(resource_exists::<PlayCatalog>),
        );
    }
}

/// UI nodes by (fleet id, unit type).
#[derive(Resource, Default)]
struct HeroHeads(HashMap<(u32, u32), Entity>);

/// Marks a portrait root node.
#[derive(Component)]
struct HeroHead;

#[allow(clippy::too_many_arguments)]
fn update_hero_heads(
    mut commands: Commands,
    mut heads: ResMut<HeroHeads>,
    state: Res<SimState>,
    catalog: Res<PlayCatalog>,
    player: Res<PlayerFaction>,
    entities: Option<Res<FleetEntities>>,
    layout: Option<Res<galaxy_play::GalaxyLayout>>,
    ui_scale: Res<UiScale>,
    textures: Option<ResMut<UiTextures>>,
    mut images: ResMut<Assets<Image>>,
    camera: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    transforms: Query<&GlobalTransform, With<galaxy_play::FleetVisual>>,
    mut nodes: Query<(&mut Node, &mut Visibility), With<HeroHead>>,
) {
    let (Some(entities), Some(layout), Some(mut textures)) = (entities, layout, textures) else { return };
    let Some((camera, cam_tf)) = camera.iter().next() else { return };
    let us = ui_scale.0.max(0.1);
    let g = state.galaxy();
    let mut alive: Vec<(u32, u32)> = Vec::new();
    for f in g.fleets.values() {
        let Some(&e) = entities.0.get(&f.id) else { continue };
        let Ok(tf) = transforms.get(e) else { continue };
        let heroes: Vec<u32> = f
            .units
            .iter()
            .filter(|&(&u, &n)| n > 0 && state.world.units.get(u as usize).is_some_and(|d| d.hero))
            .map(|(&u, _)| u)
            .collect();
        if heroes.is_empty() {
            continue;
        }
        let base = tf.translation();
        let (Ok(p0), Ok(p1)) = (camera.world_to_viewport(cam_tf, base), camera.world_to_viewport(cam_tf, base + Vec3::X * layout.unit)) else {
            continue;
        };
        // Portrait size follows the on-screen size of a fleet base.
        let size = ((p1 - p0).length() * 2.4).clamp(10.0, 130.0);
        for (k, &u) in heroes.iter().take(4).enumerate() {
            let key = (f.id, u);
            alive.push(key);
            let side = if k % 2 == 0 { -1.0 } else { 1.0 };
            let step = (k / 2) as f32 + 1.0;
            let (x, y) = (p0.x + side * size * 1.35 * step - size * 0.5, p0.y - size * 0.9);
            if let Some(&node) = heads.0.get(&key) {
                if let Ok((mut n, mut vis)) = nodes.get_mut(node) {
                    n.left = Val::Px(x / us);
                    n.top = Val::Px(y / us);
                    n.width = Val::Px(size / us);
                    n.height = Val::Px(size / us);
                    *vis = Visibility::Inherited;
                }
                continue;
            }
            let icon = catalog.unit_icons.get(u as usize).cloned().flatten();
            let good = player.0 == Some(f.owner);
            let ring_name = if good { "HERO_RING_GOOD.TGA" } else { "HERO_RING_EVIL.TGA" };
            let (Some(ring), Some(portrait)) = (textures.get(&mut images, ring_name), icon.and_then(|n| textures.get(&mut images, &n))) else {
                continue;
            };
            let root = commands
                .spawn((
                    Node { position_type: PositionType::Absolute, left: Val::Px(x / us), top: Val::Px(y / us), width: Val::Px(size / us), height: Val::Px(size / us), ..default() },
                    ImageNode::new(ring.handle),
                    GlobalZIndex(-1),
                    HeroHead,
                    crate::galaxy_map::GalaxyEntity,
                ))
                .with_children(|p| {
                    p.spawn((
                        Node { position_type: PositionType::Absolute, left: Val::Percent(14.0), top: Val::Percent(14.0), width: Val::Percent(72.0), height: Val::Percent(72.0), ..default() },
                        ImageNode::new(portrait.handle),
                    ));
                })
                .id();
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
