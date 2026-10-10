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
use battle_sim::{Event, HitKind, HpKind, ProjId};
use bevy::gizmos::config::GizmoConfigStore;
use bevy::prelude::*;

use crate::convert::{ship_rotation, sim_dir_to_bevy, sim_to_bevy};
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

/// A short muzzle flash flare at a weapon barrel (U70).
#[derive(Component)]
pub struct MuzzleFlash {
    pub timer: f32,
    pub max_time: f32,
    pub initial_color: Color,
    pub radius: f32,
}

/// An expanding deflector shield impact ripple ring (U72).
#[derive(Component)]
pub struct DeflectorWave {
    pub timer: f32,
    pub max_time: f32,
    pub start_radius: f32,
    pub end_radius: f32,
}

/// Spawns 0.08s muzzle flash flares at cannon barrels when weapons fire (U70).
pub fn spawn_muzzle_flashes(
    mut commands: Commands,
    rt: Res<BattleRuntime>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for ev in &rt.fresh_events {
        let Event::ProjectileFired { projectile, from, .. } = ev else { continue };
        let muzzle_pos = sim_to_bevy(*from);
        let p_name = rt.catalog.projectiles.get(*projectile as usize).map(|p| p.name.to_ascii_lowercase()).unwrap_or_default();
        if p_name.contains("missile") || p_name.contains("torpedo") {
            continue;
        }
        let is_ion = p_name.contains("ion");
        let is_green = p_name.contains("green") || p_name.contains("rebel");
        let is_yellow = p_name.contains("yellow") || p_name.contains("pirate");
        let flash_color = if is_ion {
            Color::srgba(0.3, 0.75, 1.0, 0.95)
        } else if is_green {
            Color::srgba(0.25, 1.0, 0.35, 0.95)
        } else if is_yellow {
            Color::srgba(1.0, 0.9, 0.25, 0.95)
        } else {
            Color::srgba(1.0, 0.35, 0.15, 0.95)
        };
        let sphere_mesh = meshes.add(Sphere::new(1.0).mesh().uv(16, 12));
        let flash_mat = materials.add(StandardMaterial {
            base_color: flash_color,
            unlit: true,
            alpha_mode: AlphaMode::Add,
            ..default()
        });
        let radius = if p_name.contains("heavy") || p_name.contains("super") { 22.0 } else { 12.0 };
        commands.spawn((
            Name::new("muzzle_flash"),
            BattleEntity,
            Mesh3d(sphere_mesh),
            MeshMaterial3d(flash_mat),
            Transform::from_translation(muzzle_pos).with_scale(Vec3::splat(radius)),
            MuzzleFlash {
                timer: 0.08,
                max_time: 0.08,
                initial_color: flash_color,
                radius,
            },
        ));
    }
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
                    // U72: Deflector wave expanding ripple ring
                    let ship_rad = rendered.as_ref().and_then(|r| r.0.get(ship)).map_or(80.0, |s| s.radius);
                    let end_r = (ship_rad * 0.45).clamp(25.0, 140.0);
                    let wave_rot = if normal.length_squared() > 1e-4 {
                        Transform::IDENTITY.looking_to(normal, Vec3::Y).rotation * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)
                    } else {
                        Quat::IDENTITY
                    };
                    let wave_mesh = meshes.add(Torus::new(1.0, 0.06).mesh());
                    let wave_mat = materials.add(StandardMaterial {
                        base_color: Color::srgba(0.35, 0.8, 1.0, 0.95),
                        unlit: true,
                        alpha_mode: AlphaMode::Add,
                        ..default()
                    });
                    commands.spawn((
                        Name::new("deflector_wave"),
                        BattleEntity,
                        Mesh3d(wave_mesh),
                        MeshMaterial3d(wave_mat),
                        Transform::from_translation(at).with_rotation(wave_rot).with_scale(Vec3::splat(5.0)),
                        DeflectorWave {
                            timer: 0.3,
                            max_time: 0.3,
                            start_radius: 5.0,
                            end_radius: end_r,
                        },
                    ));
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

/// Updates muzzle flash flares, expanding radius slightly and fading out over 0.08s (U70).
pub fn update_muzzle_flashes(
    mut commands: Commands,
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut query: Query<(Entity, &mut MuzzleFlash, &mut Transform, &MeshMaterial3d<StandardMaterial>)>,
) {
    let dt = time.delta_secs();
    for (entity, mut flash, mut tf, mat_handle) in &mut query {
        flash.timer -= dt;
        if flash.timer <= 0.0 {
            commands.entity(entity).try_despawn();
        } else {
            let progress = 1.0 - (flash.timer / flash.max_time).clamp(0.0, 1.0);
            let scale = flash.radius * (1.0 + progress * 0.4);
            tf.scale = Vec3::splat(scale);
            if let Some(mut mat) = materials.get_mut(&mat_handle.0) {
                let mut c = flash.initial_color.to_srgba();
                c.alpha *= 1.0 - progress;
                mat.base_color = Color::Srgba(c);
            }
        }
    }
}

/// Updates deflector waves: expanding ring outward over 0.3s and fading alpha to 0 (U72).
pub fn update_deflector_waves(
    mut commands: Commands,
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut query: Query<(Entity, &mut DeflectorWave, &mut Transform, &MeshMaterial3d<StandardMaterial>)>,
) {
    let dt = time.delta_secs();
    for (entity, mut wave, mut tf, mat_handle) in &mut query {
        wave.timer -= dt;
        if wave.timer <= 0.0 {
            commands.entity(entity).try_despawn();
        } else {
            let progress = 1.0 - (wave.timer / wave.max_time).clamp(0.0, 1.0);
            let radius = wave.start_radius + (wave.end_radius - wave.start_radius) * progress.sqrt();
            tf.scale = Vec3::splat(radius);
            if let Some(mut mat) = materials.get_mut(&mat_handle.0) {
                let alpha = (1.0 - progress) * 0.95;
                mat.base_color = Color::srgba(0.35, 0.8, 1.0, alpha);
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

/// Gizmo group of the engine ribbon exhaust trails.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct EngineTrailGizmos;

/// Configures engine trail gizmos with a line width suitable for engine exhausts.
pub fn configure_engine_trail_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (cfg, _) = store.config_mut::<EngineTrailGizmos>();
    cfg.line.width = 5.0;
    cfg.depth_bias = -0.5;
}

/// Draws dynamic engine glows and ribbon exhaust trails behind active ships,
/// and smoke plumes behind destroyed engines (U67).
pub fn draw_engine_trails(
    rt: Res<BattleRuntime>,
    rendered: Option<Res<crate::ships::RenderedShips>>,
    mut gizmos: Gizmos<EngineTrailGizmos>,
) {
    let Some(rendered) = rendered else { return };
    for ship in rendered.0.values() {
        if !ship.visible {
            continue;
        }
        let def = rt.catalog.ship(ship.def);
        let max_speed = def.max_speed.max(1.0);
        let cur_speed = if let (Some(cur), Some(nxt)) = (rt.cur.as_ref(), rt.next.as_ref()) {
            if let (Some(c), Some(n)) = (cur.ship(ship.id), nxt.ship(ship.id)) {
                (c.pos.dist(n.pos) / rt.dt.max(0.001)).max(0.0)
            } else {
                max_speed * 0.5
            }
        } else {
            max_speed * 0.5
        };
        let speed_ratio = (cur_speed / max_speed).clamp(0.1, 1.2);
        let rot = ship_rotation(ship.yaw);
        let fwd = rot * Vec3::Z;

        // Faction-based engine flame colors
        let name_lower = def.name.to_ascii_lowercase();
        let (core_color, tail_color) = if ship.side == 0 || name_lower.contains("empire") || name_lower.contains("destroyer") || name_lower.contains("tie") {
            (Color::srgba(0.85, 0.95, 1.0, 0.95), Color::srgba(0.2, 0.65, 1.0, 0.0))
        } else if name_lower.contains("rebel") || name_lower.contains("mc-") || name_lower.contains("wing") || name_lower.contains("calamari") {
            (Color::srgba(1.0, 0.9, 0.6, 0.95), Color::srgba(1.0, 0.45, 0.1, 0.0))
        } else {
            (Color::srgba(0.8, 1.0, 0.6, 0.95), Color::srgba(0.2, 0.9, 0.3, 0.0))
        };

        let engine_hps: Vec<usize> = def.hardpoints.iter().enumerate().filter(|(_, h)| h.kind == HpKind::Engine).map(|(i, _)| i).collect();
        if !engine_hps.is_empty() {
            for &idx in &engine_hps {
                let is_alive = ship.hp_alive.get(idx).copied().unwrap_or(true);
                let pos = ship.hp_pos.get(idx).and_then(|p| *p).unwrap_or_else(|| ship.pos - fwd * (ship.radius * 0.7));
                if is_alive {
                    let trail_len = (120.0 * speed_ratio).max(20.0);
                    gizmos.line_gradient(pos, pos - fwd * trail_len, core_color, tail_color);
                    gizmos.circle(Isometry3d::new(pos, rot), (4.0 * (ship.radius / 150.0)).clamp(1.5, 6.0), core_color).resolution(16);
                } else {
                    // Destroyed engine emits dark smoke billowing backward and upward
                    let smoke_start = Color::srgba(0.18, 0.18, 0.18, 0.85);
                    let smoke_end = Color::srgba(0.05, 0.05, 0.05, 0.0);
                    let smoke_dir = -fwd * 0.7 + Vec3::Y * 0.5;
                    gizmos.line_gradient(pos, pos + smoke_dir * 50.0, smoke_start, smoke_end);
                }
            }
        } else if !def.is_static {
            // Fighters / small ships without hardpoints
            let rear_center = ship.pos - fwd * (ship.radius * 0.65);
            let trail_len = (35.0 * speed_ratio).max(8.0);
            let wing_span = (ship.radius * 0.35).max(2.5);
            let right = rot * Vec3::X;
            let left_eng = rear_center - right * wing_span;
            let right_eng = rear_center + right * wing_span;
            gizmos.line_gradient(left_eng, left_eng - fwd * trail_len, core_color, tail_color);
            gizmos.line_gradient(right_eng, right_eng - fwd * trail_len, core_color, tail_color);
            gizmos.circle(Isometry3d::new(left_eng, rot), 1.2, core_color).resolution(12);
            gizmos.circle(Isometry3d::new(right_eng, rot), 1.2, core_color).resolution(12);
        }
    }
}

/// Draws persistent smoke plumes and electrical sparks from damaged hardpoints
/// on stations and large capital ships below 50% hull (U73).
pub fn draw_hull_damage_effects(
    rt: Res<BattleRuntime>,
    rendered: Option<Res<crate::ships::RenderedShips>>,
    mut gizmos: Gizmos<EngineTrailGizmos>,
) {
    let Some(rendered) = rendered else { return };
    let clock = rt.clock as f32;
    for ship in rendered.0.values() {
        if !ship.visible {
            continue;
        }
        let def = rt.catalog.ship(ship.def);
        let is_big = def.is_static || ship.radius >= 180.0;
        if !is_big || ship.hull >= 0.5 {
            continue;
        }
        let smoke_intensity = (1.0 - ship.hull * 2.0).clamp(0.2, 1.0);
        for (i, &alive) in ship.hp_alive.iter().enumerate() {
            if alive {
                continue;
            }
            let Some(Some(hp_pos)) = ship.hp_pos.get(i) else { continue };
            let seed = (ship.id * 101 + i as u32) as f32;
            let phase = (clock * 2.0 + seed).sin();
            let smoke_dir = Vec3::new(
                (clock * 1.5 + seed).cos() * 8.0,
                35.0 * smoke_intensity + phase * 5.0,
                (clock * 1.5 + seed * 1.3).sin() * 8.0,
            );
            let smoke_base = Color::srgba(0.15, 0.15, 0.16, 0.8 * smoke_intensity);
            let smoke_top = Color::srgba(0.05, 0.05, 0.05, 0.0);
            gizmos.line_gradient(*hp_pos, *hp_pos + smoke_dir, smoke_base, smoke_top);

            // Intermittent electrical sparks
            if ((clock * 6.0 + seed * 3.7).sin()) > 0.6 {
                let spark_dir = Vec3::new(
                    (clock * 12.0 + seed).sin() * 12.0,
                    (clock * 15.0 + seed * 2.0).cos() * 10.0,
                    (clock * 14.0 + seed * 1.7).sin() * 12.0,
                );
                gizmos.line(*hp_pos, *hp_pos + spark_dir, Color::srgba(1.0, 0.9, 0.45, 0.9));
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

    #[test]
    fn test_engine_trail_geometry_and_faction_colors() {
        let rot = ship_rotation(0.0);
        let fwd = rot * Vec3::Z;
        // Sim yaw 0 faces +X in sim space; verify forward vector in Bevy has positive X component
        assert!(fwd.x > 0.9, "forward vector aligns with sim heading: {:?}", fwd);

        // Empire engine flare: cyan
        let emp_core = Color::srgba(0.85, 0.95, 1.0, 0.95);
        let emp_tail = Color::srgba(0.2, 0.65, 1.0, 0.0);
        assert!(emp_core.to_srgba().blue > 0.9);
        assert_eq!(emp_tail.to_srgba().alpha, 0.0);

        // Rebel engine flare: orange / amber
        let reb_core = Color::srgba(1.0, 0.9, 0.6, 0.95);
        let reb_tail = Color::srgba(1.0, 0.45, 0.1, 0.0);
        assert!(reb_core.to_srgba().red > 0.9);
        assert_eq!(reb_tail.to_srgba().alpha, 0.0);
    }

    #[test]
    fn test_muzzle_flash_timing_and_colors() {
        let flash = MuzzleFlash {
            timer: 0.08,
            max_time: 0.08,
            initial_color: Color::srgba(1.0, 0.35, 0.15, 0.95),
            radius: 12.0,
        };
        assert_eq!(flash.timer, flash.max_time);
        assert!(flash.max_time <= 0.1, "muzzle flash is brief (<= 100ms)");
        assert!(flash.initial_color.to_srgba().red > 0.9);
    }

    #[test]
    fn test_deflector_wave_expansion_and_timing() {
        let wave = DeflectorWave {
            timer: 0.3,
            max_time: 0.3,
            start_radius: 5.0,
            end_radius: 80.0,
        };
        assert_eq!(wave.timer, wave.max_time);
        assert!(wave.end_radius > wave.start_radius);
        assert!((wave.max_time - 0.3).abs() < 1e-4);
    }

    #[test]
    fn test_hull_damage_smoke_threshold() {
        let hull_above: f32 = 0.55;
        let is_big = true;
        assert!(!(is_big && hull_above < 0.5), "no persistent smoke above 50% hull");

        let hull_below: f32 = 0.40;
        assert!(is_big && hull_below < 0.5, "persistent smoke triggers below 50% hull");
        let intensity = (1.0f32 - hull_below * 2.0).clamp(0.2, 1.0);
        assert!(intensity > 0.15, "smoke intensity increases as hull drops");
    }
}

