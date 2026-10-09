//! Projectile bolts and impact effects.
//!
//! Bolts are [`alamo_render::ProjectileVisual`] entities (batched geometry in the particle
//! renderer). They are pooled per projectile definition and re-positioned every frame from the
//! snapshot: the simulation only publishes positions and velocities, so a bolt is extrapolated
//! from its last known position by the fraction of the step that has elapsed.

use std::collections::HashMap;

use alamo_render::{
    AlamoModelLoaded, AlamoPart, AutoDespawn, ImpactKind, ProjectileImpact, ProjectileVisual, ShieldFlash, ShieldHit,
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
    mut pools: ResMut<BoltPools>,
    mut bolts: Query<(&mut Transform, &mut Visibility), With<ProjectileVisual>>,
) {
    let Some(cur) = rt.cur.clone() else { return };
    let dt = rt.dt;
    let t = rt.alpha * dt;
    let mut used: HashMap<ProjId, usize> = HashMap::new();
    for p in &cur.projectiles {
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

/// Spawns impact particles and shield flashes for the hit events of newly reached steps.
pub fn impact_effects(
    mut commands: Commands,
    rt: Res<BattleRuntime>,
    settings: Res<BattleViewSettings>,
    live_fx: Query<(), With<AutoDespawn>>,
) {
    let mut budget = settings
        .max_effects
        .saturating_sub(live_fx.iter().count())
        .min(settings.max_impacts_per_frame);
    for ev in &rt.fresh_events {
        let Event::Hit {
            ship,
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
                // The shield flash is cheap (a material parameter); the particle is budgeted.
                // A shield that is down (or never existed) shows no bubble, only the hull impact.
                if rt.cur.as_ref().and_then(|c| c.ship(*ship)).is_none_or(|s| s.shield > 0.0) {
                    commands.trigger(ShieldHit {
                        model: entity,
                        position: at,
                        intensity: 1.0,
                    });
                }
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
            HitKind::Hull | HitKind::Hardpoint(_) => {
                if budget > 0 {
                    budget -= 1;
                    commands.trigger(ProjectileImpact {
                        projectile: def.name.clone(),
                        kind: ImpactKind::Hull,
                        position: at,
                        normal,
                    });
                }
            }
        }
    }
}

/// Hides the shield meshes of ships whose shield is down, so a flash that was still fading when the shield
/// broke does not outlive it.
pub fn hide_downed_shields(
    rt: Res<BattleRuntime>,
    flashing: Query<&AlamoModelLoaded, With<ShieldFlash>>,
    parts: Query<&AlamoPart>,
    mut vis: Query<&mut Visibility>,
) {
    let Some(cur) = rt.cur.as_ref() else { return };
    for (id, &entity) in &rt.entities {
        let Ok(loaded) = flashing.get(entity) else { continue };
        if cur.ship(*id).is_none_or(|s| s.shield > 0.0) {
            continue;
        }
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
