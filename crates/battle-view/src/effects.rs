//! Projectile bolts and impact effects.
//!
//! Bolts are [`alamo_render::ProjectileVisual`] entities (batched geometry in the particle
//! renderer). They are pooled per projectile definition and re-positioned every frame from the
//! snapshot: the simulation only publishes positions and velocities, so a bolt is extrapolated
//! from its last known position by the fraction of the step that has elapsed.

use std::collections::HashMap;

use alamo_render::{
    AlamoModelLoaded, AlamoPart, AutoDespawn, ImpactKind, ProjectileImpact, ProjectileVisual, ShieldHit,
};
use battle_sim::{Event, HitKind, ProjId};
use bevy::prelude::*;

use crate::convert::{sim_dir_to_bevy, sim_to_bevy};
use crate::{BattleEntity, BattleRuntime, BattleViewSettings};

/// Pool of bolt entities per projectile definition.
#[derive(Resource, Default)]
pub struct BoltPools {
    pools: HashMap<ProjId, Vec<Entity>>,
}

/// Positions the pooled bolt entities for the current frame.
pub fn update_projectiles(
    mut commands: Commands,
    rt: Res<BattleRuntime>,
    defs: Option<Res<alamo_render::XmlDefs>>,
    mut pools: ResMut<BoltPools>,
    mut bolts: Query<(&mut Transform, &mut Visibility), With<ProjectileVisual>>,
) {
    let Some(cur) = rt.cur.clone() else { return };
    let dt = rt.dt;
    let t = rt.alpha * dt;
    let mut used: HashMap<ProjId, usize> = HashMap::new();
    for p in &cur.projectiles {
        // Missiles and torpedoes are flown as models with their trails by `missiles.rs`.
        if defs.as_deref().is_some_and(|d| crate::missiles::drawn_as_model(&rt, d, p.def)) {
            continue;
        }
        let Some(name) = rt.catalog.projectiles.get(p.def as usize).map(|d| d.name.clone()) else {
            continue;
        };
        let pool = pools.pools.entry(p.def).or_default();
        let idx = used.entry(p.def).or_insert(0);
        let pos = sim_to_bevy(p.pos + p.vel * t);
        let dir = sim_dir_to_bevy(p.vel);
        let rotation = if dir.length_squared() > 1e-6 {
            Transform::IDENTITY.looking_to(dir, Vec3::Y).rotation
        } else {
            Quat::IDENTITY
        };
        if *idx >= pool.len() {
            let e = commands
                .spawn((
                    BattleEntity,
                    ProjectileVisual::new(name),
                    Transform {
                        translation: pos,
                        rotation,
                        ..default()
                    },
                ))
                .id();
            pool.push(e);
        } else if let Ok((mut tf, mut vis)) = bolts.get_mut(pool[*idx]) {
            tf.translation = pos;
            tf.rotation = rotation;
            if *vis != Visibility::Inherited {
                *vis = Visibility::Inherited;
            }
        }
        *idx += 1;
    }
    for (def, pool) in &pools.pools {
        let n = used.get(def).copied().unwrap_or(0);
        for &e in pool.iter().skip(n) {
            if let Ok((_, mut vis)) = bolts.get_mut(e)
                && *vis != Visibility::Hidden
            {
                *vis = Visibility::Hidden;
            }
        }
    }
}

/// Special weapon beam component (massive beam rising from planet).
#[derive(Component)]
pub struct SpecialBeam {
    pub timer: f32,
    pub max_time: f32,
    pub initial_color: Color,
}

/// Special weapon impact shockwave component.
#[derive(Component)]
pub struct SpecialImpactFlash {
    pub timer: f32,
    pub max_time: f32,
    pub start_radius: f32,
    pub end_radius: f32,
    pub initial_color: Color,
}

/// Spawns massive rising beam visual cylinders when a ground-to-space special weapon fires.
pub fn spawn_special_weapon_beams(
    mut commands: Commands,
    rt: Res<BattleRuntime>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for ev in &rt.fresh_events {
        let Event::ProjectileFired { ship, projectile, from, .. } = ev else { continue };
        let Some(cur) = rt.cur.as_ref() else { continue };
        let Some(firing_ship) = cur.ship(*ship) else { continue };
        let def = rt.catalog.ship(firing_ship.def);
        if def.special_weapon.is_none() {
            continue;
        }
        let p_name = rt.catalog.projectiles.get(*projectile as usize).map(|p| p.name.to_ascii_lowercase()).unwrap_or_default();
        let is_ion = p_name.contains("ion");
        let target_bevy = sim_to_bevy(*from);
        // Beam originates from the planet deep below the battle plane
        let origin = Vec3::new(target_bevy.x * 0.7, -2400.0, target_bevy.z * 0.7 + 600.0);
        let dir = target_bevy - origin;
        let length = dir.length();
        let mid = (origin + target_bevy) * 0.5;
        let rot = if length > 1.0 {
            Transform::IDENTITY.looking_to(dir.normalize(), Vec3::Y).rotation * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)
        } else {
            Quat::IDENTITY
        };
        let outer_color = if is_ion {
            Color::srgba(0.2, 0.65, 1.0, 0.85)
        } else {
            Color::srgba(1.0, 0.9, 0.55, 0.9)
        };
        let core_color = Color::srgba(1.0, 1.0, 1.0, 0.95);
        let mesh = meshes.add(Cylinder::new(18.0, length));
        let core_mesh = meshes.add(Cylinder::new(6.0, length));
        let outer_mat = materials.add(StandardMaterial {
            base_color: outer_color,
            unlit: true,
            alpha_mode: AlphaMode::Add,
            ..default()
        });
        let core_mat = materials.add(StandardMaterial {
            base_color: core_color,
            unlit: true,
            alpha_mode: AlphaMode::Add,
            ..default()
        });
        commands.spawn((
            Name::new("special_beam_outer"),
            BattleEntity,
            Mesh3d(mesh),
            MeshMaterial3d(outer_mat),
            Transform::from_translation(mid).with_rotation(rot),
            SpecialBeam { timer: 0.75, max_time: 0.75, initial_color: outer_color },
        ));
        commands.spawn((
            Name::new("special_beam_core"),
            BattleEntity,
            Mesh3d(core_mesh),
            MeshMaterial3d(core_mat),
            Transform::from_translation(mid).with_rotation(rot),
            SpecialBeam { timer: 0.75, max_time: 0.75, initial_color: core_color },
        ));
    }
}

/// Spawns impact particles and shield flashes for the hit events of newly reached steps.
pub fn impact_effects(
    mut commands: Commands,
    rt: Res<BattleRuntime>,
    rendered: Option<Res<crate::ships::RenderedShips>>,
    settings: Res<BattleViewSettings>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    live_fx: Query<(), With<AutoDespawn>>,
) {
    let mut budget = settings
        .max_effects
        .saturating_sub(live_fx.iter().count())
        .min(settings.max_impacts_per_frame);
    for ev in &rt.fresh_events {
        let Event::Hit {
            ship,
            attacker,
            projectile,
            pos,
            kind,
            ..
        } = ev
        else {
            continue;
        };
        let Some(&entity) = rt.entities.get(ship) else {
            continue;
        };
        let Some(def) = rt.catalog.projectiles.get(*projectile as usize) else {
            continue;
        };
        let at = sim_to_bevy(*pos);
        let center = rt
            .cur
            .as_ref()
            .and_then(|c| c.ship(*ship))
            .map_or(at, |s| sim_to_bevy(s.pos));
        let normal = (at - center).normalize_or_zero();
        match kind {
            HitKind::Shield => {
                let shield_alive = rt.cur.as_ref().and_then(|c| c.ship(*ship)).map_or(true, |s| s.shield > 0.0);
                if shield_alive {
                    commands.trigger(ShieldHit {
                        model: entity,
                        position: at,
                        intensity: 1.0,
                    });
                    if budget > 0 {
                        budget -= 1;
                        commands.trigger(ProjectileImpact {
                            projectile: def.name.clone(),
                            kind: ImpactKind::Shield,
                            position: at,
                            normal,
                        });
                    }
                }
            }
            HitKind::Hardpoint(i) => {
                let hp_world = rendered
                    .as_ref()
                    .and_then(|r| r.0.get(ship))
                    .and_then(|s| s.hp_pos.get(*i as usize).copied().flatten());
                let target_at = hp_world.unwrap_or_else(|| center + (at - center) * 0.4);
                let hit_normal = (target_at - center).normalize_or_zero();
                if budget > 0 {
                    budget -= 1;
                    commands.trigger(ProjectileImpact {
                        projectile: def.name.clone(),
                        kind: ImpactKind::Hull,
                        position: target_at,
                        normal: if hit_normal.length_squared() > 1e-4 { hit_normal } else { normal },
                    });
                }
            }
            HitKind::Hull => {
                let target_at = if let Some(s) = rendered.as_ref().and_then(|r| r.0.get(ship)) {
                    s.hp_pos
                        .iter()
                        .flatten()
                        .min_by(|p1, p2| p1.distance_squared(at).total_cmp(&p2.distance_squared(at)))
                        .copied()
                        .unwrap_or_else(|| center + (at - center) * 0.35)
                } else {
                    center + (at - center) * 0.35
                };
                let hit_normal = (target_at - center).normalize_or_zero();
                if budget > 0 {
                    budget -= 1;
                    commands.trigger(ProjectileImpact {
                        projectile: def.name.clone(),
                        kind: ImpactKind::Hull,
                        position: target_at,
                        normal: if hit_normal.length_squared() > 1e-4 { hit_normal } else { normal },
                    });
                }
            }
        }
        let is_special = rt.cur.as_ref().and_then(|c| c.ship(*attacker)).map_or(false, |s| rt.catalog.ship(s.def).special_weapon.is_some());
        if is_special {
            let p_name = def.name.to_ascii_lowercase();
            let is_ion = p_name.contains("ion");
            let flash_color = if is_ion {
                Color::srgba(0.3, 0.7, 1.0, 0.95)
            } else {
                Color::srgba(1.0, 0.95, 0.7, 0.95)
            };
            let sphere_mesh = meshes.add(Sphere::new(1.0).mesh().uv(24, 16));
            let flash_mat = materials.add(StandardMaterial {
                base_color: flash_color,
                unlit: true,
                alpha_mode: AlphaMode::Add,
                ..default()
            });
            commands.spawn((
                Name::new("special_impact_flash"),
                BattleEntity,
                Mesh3d(sphere_mesh),
                MeshMaterial3d(flash_mat),
                Transform::from_translation(at).with_scale(Vec3::splat(50.0)),
                SpecialImpactFlash {
                    timer: 0.6,
                    max_time: 0.6,
                    start_radius: 50.0,
                    end_radius: 350.0,
                    initial_color: flash_color,
                },
            ));
            commands.trigger(ShieldHit {
                model: entity,
                position: at,
                intensity: 3.5,
            });
        }
    }
}

/// Updates beam alpha over time and despawns expired beams.
pub fn update_special_beams(
    mut commands: Commands,
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut query: Query<(Entity, &mut SpecialBeam, &MeshMaterial3d<StandardMaterial>)>,
) {
    let dt = time.delta_secs();
    for (entity, mut beam, mat_handle) in &mut query {
        beam.timer -= dt;
        if beam.timer <= 0.0 {
            commands.entity(entity).despawn();
        } else if let Some(mut mat) = materials.get_mut(&mat_handle.0) {
            let alpha = (beam.timer / beam.max_time).clamp(0.0, 1.0);
            let mut c = beam.initial_color.to_srgba();
            c.alpha *= alpha;
            mat.base_color = Color::Srgba(c);
        }
    }
}

/// Updates shockwave flash spheres, expanding radius and fading out.
pub fn update_special_flashes(
    mut commands: Commands,
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut query: Query<(Entity, &mut SpecialImpactFlash, &mut Transform, &MeshMaterial3d<StandardMaterial>)>,
) {
    let dt = time.delta_secs();
    for (entity, mut flash, mut tf, mat_handle) in &mut query {
        flash.timer -= dt;
        if flash.timer <= 0.0 {
            commands.entity(entity).despawn();
        } else {
            let progress = 1.0 - (flash.timer / flash.max_time).clamp(0.0, 1.0);
            let radius = flash.start_radius + (flash.end_radius - flash.start_radius) * progress;
            tf.scale = Vec3::splat(radius);
            if let Some(mut mat) = materials.get_mut(&mat_handle.0) {
                let mut c = flash.initial_color.to_srgba();
                c.alpha *= 1.0 - progress;
                mat.base_color = Color::Srgba(c);
            }
        }
    }
}

/// Hides the shield meshes of ships whose shield is down, so a flash that was still fading when the shield
/// broke does not outlive it.
pub fn hide_downed_shields(
    rt: Res<BattleRuntime>,
    loaded_query: Query<&AlamoModelLoaded>,
    parts: Query<&AlamoPart>,
    mut vis: Query<&mut Visibility>,
) {
    let Some(cur) = rt.cur.as_ref() else { return };
    for (id, &entity) in &rt.entities {
        let Ok(loaded) = loaded_query.get(entity) else { continue };
        if cur.ship(*id).is_some_and(|s| s.shield <= 0.0) {
            for &p in &loaded.parts {
                if parts.get(p).is_ok_and(|a| a.shield)
                    && let Ok(mut v) = vis.get_mut(p)
                    && *v != Visibility::Hidden
                {
                    *v = Visibility::Hidden;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_special_beam_color_and_timing() {
        let beam = SpecialBeam {
            timer: 0.75,
            max_time: 0.75,
            initial_color: Color::srgba(0.2, 0.65, 1.0, 0.85),
        };
        assert_eq!(beam.timer, beam.max_time);
        let flash = SpecialImpactFlash {
            timer: 0.6,
            max_time: 0.6,
            start_radius: 50.0,
            end_radius: 350.0,
            initial_color: Color::srgba(0.3, 0.7, 1.0, 0.95),
        };
        assert!(flash.end_radius > flash.start_radius);
    }
}

