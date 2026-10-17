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
use battle_sim::{Event, HitKind, HpKind, ProjId, TargetClass};
use bevy::gizmos::config::GizmoConfigStore;
use bevy::post_process::bloom::Bloom;
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
    _commands: Commands,
    _rt: Res<BattleRuntime>,
    _meshes: ResMut<Assets<Mesh>>,
    _materials: ResMut<Assets<StandardMaterial>>,
) {
    // Artificial 3D sphere meshes removed; lasers shoot authentic projectile bolts.
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
                let shield_alive = rt.cur.as_ref().and_then(|c| c.ship(*ship)).is_some_and(|s| s.shield > 0.0 && rt.catalog.ship(s.def).shield > 0.0);
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
            // Only a ship that has shields in the data and still holds some shows the bubble.
            if rt.cur.as_ref().and_then(|c| c.ship(*ship)).is_some_and(|s| s.shield > 0.0 && rt.catalog.ship(s.def).shield > 0.0) {
                commands.trigger(ShieldHit {
                    model: entity,
                    position: at,
                    intensity: 3.5,
                });
            }
        }
    }
}

/// Spawns a fading additive beam (outer glow plus white core) between two world points, as the special weapons and the Death Star do.
pub fn spawn_beam(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    from: Vec3,
    to: Vec3,
    outer: Color,
    radius: f32,
    seconds: f32,
) {
    let dir = to - from;
    let length = dir.length();
    if length < 1.0 {
        return;
    }
    let rot = Transform::IDENTITY.looking_to(dir / length, Vec3::Y).rotation * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let mid = (from + to) * 0.5;
    for (name, r, color) in [("beam_outer", radius, outer), ("beam_core", radius * 0.33, Color::srgba(1.0, 1.0, 1.0, 0.95))] {
        commands.spawn((
            Name::new(name),
            BattleEntity,
            Mesh3d(meshes.add(Cylinder::new(r, length))),
            MeshMaterial3d(materials.add(StandardMaterial { base_color: color, unlit: true, alpha_mode: AlphaMode::Add, ..default() })),
            Transform::from_translation(mid).with_rotation(rot),
            SpecialBeam { timer: seconds, max_time: seconds, initial_color: color },
        ));
    }
}

/// Updates beam alpha over time and despawns expired beams; a textured beam also steps through its texture frames.
pub fn update_special_beams(
    mut commands: Commands,
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut query: Query<(Entity, &mut SpecialBeam, &MeshMaterial3d<StandardMaterial>, Option<&BeamFrames>)>,
) {
    let dt = time.delta_secs();
    for (entity, mut beam, mat_handle, frames) in &mut query {
        beam.timer -= dt;
        if beam.timer <= 0.0 {
            commands.entity(entity).despawn();
        } else if let Some(mut mat) = materials.get_mut(&mat_handle.0) {
            let alpha = (beam.timer / beam.max_time).clamp(0.0, 1.0);
            let mut c = beam.initial_color.to_srgba();
            c.alpha *= alpha;
            mat.base_color = Color::Srgba(c);
            if let Some(f) = frames {
                mat.uv_transform = beam_frame_uv((beam.max_time - beam.timer) * BEAM_FRAMES_PER_SECOND, f.0);
            }
        }
    }
}

/// The frames of a textured beam ([`spawn_textured_beam`]): `Num_Anim_Frames` of the ability.
#[derive(Component, Debug)]
pub struct BeamFrames(pub u32);

/// Frame rate of an animated beam texture (estimate: the data gives only the number of frames).
const BEAM_FRAMES_PER_SECOND: f32 = 15.0;

/// Texture transform showing frame `t` (counting up, wrapping) of a beam texture with `frames` frames side by side along u (the
/// beam's length). Estimate: the Remake's `tractor_beam01.tga` is one continuous horizontal beam; four 128 px columns of it differ
/// only in the crackle of the white core, which reads as frames side by side.
pub fn beam_frame_uv(t: f32, frames: u32) -> bevy::math::Affine2 {
    let n = frames.max(1);
    let frame = (t.max(0.0) as u32) % n;
    bevy::math::Affine2::from_scale_angle_translation(Vec2::new(1.0 / n as f32, 1.0), 0.0, Vec2::new(frame as f32 / n as f32, 0.0))
}

/// Beam textures (`Beam_Texture_Name`) decoded once per battle, by lower-case name; `None` = missing in the data.
#[derive(Resource, Default)]
pub struct BeamArt(HashMap<String, Option<Handle<Image>>>);

impl BeamArt {
    /// The texture `name` with its alpha set to opaque: the beam is drawn additively, where Bevy scales the colour by the alpha, and
    /// the alpha channel of the Remake's `tractor_beam01.tga` holds an unrelated vertical streak (the green beam is in the colour).
    pub fn get_or_load(&mut self, fs: &alamo_fs::GameFs, images: &mut Assets<Image>, name: &str) -> Option<Handle<Image>> {
        self.0
            .entry(name.to_ascii_lowercase())
            .or_insert_with(|| {
                let bytes = alamo_render::texture::read_texture_file(fs, name)?;
                let mut rgba = alamo_model::image::decode_texture(&bytes).map_err(|e| warn!("beam texture {name}: {e}")).ok()?;
                for px in rgba.pixels.as_chunks_mut::<4>().0 {
                    px[3] = 255;
                }
                Some(images.add(alamo_render::texture::rgba_with_mips(&rgba)))
            })
            .clone()
    }
}

/// Spawns a beam drawn with a texture (`Beam_Texture_Name`): two crossed quads `width` wide from `from` to `to`, the texture's beam
/// along the length, additive, tinted by `tint` (`Line_Color`) and fading over `seconds`, stepping through `frames` texture frames.
/// One quad lies across the battle plane and one stands upright, so the beam reads from the battle camera's high angle and from low
/// ones. Returns the beam entity (at `from`).
#[allow(clippy::too_many_arguments)]
pub fn spawn_textured_beam(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    texture: Handle<Image>,
    from: Vec3,
    to: Vec3,
    width: f32,
    tint: Color,
    frames: u32,
    seconds: f32,
) -> Option<Entity> {
    let dir = to - from;
    let length = dir.length();
    if length < 1.0 {
        return None;
    }
    let material = materials.add(StandardMaterial {
        base_color: tint,
        base_color_texture: Some(texture),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        double_sided: true,
        cull_mode: None,
        uv_transform: beam_frame_uv(0.0, frames),
        ..default()
    });
    let entity = commands
        .spawn((
            Name::new("textured beam"),
            BattleEntity,
            Mesh3d(meshes.add(crossed_beam_mesh(length, width))),
            MeshMaterial3d(material),
            Transform::from_translation(from).with_rotation(beam_rotation(dir / length)),
            SpecialBeam { timer: seconds, max_time: seconds, initial_color: tint },
            BeamFrames(frames),
        ))
        .id();
    Some(entity)
}

/// Rotation that turns local +Y onto `dir` with local +X level (across the beam on the battle plane).
fn beam_rotation(dir: Vec3) -> Quat {
    let side = dir.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
    Quat::from_mat3(&Mat3::from_cols(side, dir, side.cross(dir)))
}

/// Two crossed quads along local +Y from 0 to `length`, `width` wide: u runs along the beam, v across it.
fn crossed_beam_mesh(length: f32, width: f32) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let h = width * 0.5;
    let positions = vec![
        [-h, 0.0, 0.0],
        [h, 0.0, 0.0],
        [h, length, 0.0],
        [-h, length, 0.0],
        [0.0, 0.0, -h],
        [0.0, 0.0, h],
        [0.0, length, h],
        [0.0, length, -h],
    ];
    let normals = vec![[0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
    let uvs = vec![[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]))
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
    cfg.line.width = 2.2;
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
                if !is_alive {
                    // Destroyed engine emits dark smoke billowing backward and upward
                    let smoke_start = Color::srgba(0.18, 0.18, 0.18, 0.85);
                    let smoke_end = Color::srgba(0.05, 0.05, 0.05, 0.0);
                    let smoke_dir = -fwd * 0.7 + Vec3::Y * 0.5;
                    gizmos.line_gradient(pos, pos + smoke_dir * 50.0, smoke_start, smoke_end);
                }
            }
        } else if !def.is_static && (def.class == TargetClass::Fighter || def.class == TargetClass::Bomber) {
            // Fighters / bombers without hardpoints: short engine trails
            let rear_center = ship.pos - fwd * (ship.radius * 0.65);
            let trail_len = ((ship.radius * 1.5 + 8.0) * speed_ratio).min(20.0);
            let wing_span = (ship.radius * 0.25).max(1.0);
            let right = rot * Vec3::X;
            let left_eng = rear_center - right * wing_span;
            let right_eng = rear_center + right * wing_span;
            gizmos.line_gradient(left_eng, left_eng - fwd * trail_len, core_color, tail_color);
            gizmos.line_gradient(right_eng, right_eng - fwd * trail_len, core_color, tail_color);
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

/// An expanding radar wave ring produced by a tactical sensor ping or reveal ability.
#[derive(Component, Debug, Clone)]
pub struct TacticalSensorPing {
    /// Center on the tactical 3D plane (y = 0).
    pub pos: Vec3,
    /// Maximum wave radius.
    pub max_radius: f32,
    /// Total expansion and fade duration in seconds.
    pub duration: f32,
    /// Elapsed seconds.
    pub elapsed: f32,
    /// Base tint color.
    pub color: Color,
}

/// Updates and draws expanding radar rings for active tactical sensor pings.
pub fn draw_sensor_ping_waves(
    time: Res<Time>,
    mut pings: Query<(Entity, &mut TacticalSensorPing)>,
    mut commands: Commands,
    mut gizmos: Gizmos,
) {
    for (entity, mut ping) in &mut pings {
        ping.elapsed += time.delta_secs();
        if ping.elapsed >= ping.duration {
            commands.entity(entity).try_despawn();
            continue;
        }
        let t = (ping.elapsed / ping.duration).clamp(0.0, 1.0);
        let radius = ping.max_radius * (1.0 - (1.0 - t).powi(2));
        let alpha = (1.0 - t).powi(2) * 0.85;
        let color = ping.color.with_alpha(alpha);
        let inner_color = ping.color.with_alpha(alpha * 0.45);

        // Draw primary expanding radar ring
        gizmos.circle(
            Isometry3d::new(ping.pos, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
            radius,
            color,
        ).resolution(64);

        // Trailing soft inner pulse ring
        if radius > 15.0 {
            gizmos.circle(
                Isometry3d::new(ping.pos, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                radius * 0.94,
                inner_color,
            ).resolution(64);
        }
    }
}

/// Spawns sensor ping radar effects when reveal abilities are activated.
pub fn trigger_reveal_ping_effects(
    rt: Res<BattleRuntime>,
    mut commands: Commands,
    mut seen_tick: Local<u64>,
) {
    let Some(cur) = rt.cur.as_ref() else { return };
    if cur.tick <= *seen_tick {
        return;
    }
    *seen_tick = cur.tick;
    for ev in &cur.events {
        if let battle_sim::Event::AbilityActivated { ship, ability } = ev {
            if let Some(s) = cur.ship(*ship) {
                let def = &rt.catalog.ship(s.def);
                if let Some(a) = def.abilities.get(*ability as usize) {
                    if a.reveal_radius > 0.0 || a.kind.eq_ignore_ascii_case("FOW_REVEAL_PING") {
                        let sim_pos = s.pos;
                        let world_pos = crate::convert::sim_to_bevy(sim_pos);
                        commands.spawn((
                            crate::BattleEntity,
                            TacticalSensorPing {
                                pos: world_pos,
                                max_radius: a.reveal_radius.max(800.0),
                                duration: 2.5,
                                elapsed: 0.0,
                                color: Color::srgb(0.25, 0.85, 1.0),
                            },
                        ));
                    }
                }
            }
        }
    }
}

/// Gravity well of an interdictor: two slow pulsing rings on the battle plane around the ship, drawn only while its `INTERDICT` ability is
/// switched on (user rule). A ship that has `Is_Interdictor` in the data blocks the retreat all the time (sim) but shows no rings until the
/// toggle is on. The data names no particle and no radius for the field (estimate: the ring
/// is six hull radii wide and only marks the ship), the effect is gizmo lines like the sensor ping.
pub fn draw_gravity_wells(
    time: Res<Time>,
    rt: Res<BattleRuntime>,
    settings: Res<BattleViewSettings>,
    mut gizmos: Gizmos,
) {
    let Some(cur) = rt.cur.as_ref() else { return };
    let phase = time.elapsed_secs() * 0.8;
    for s in &cur.ships {
        if s.seen_by & (1 << settings.player_side.min(7)) == 0 {
            continue;
        }
        let def = rt.catalog.ship(s.def);
        let on = def.abilities.iter().zip(&s.abilities).any(|(a, &(active, _))| a.interdict && active > 0.0);
        if !on {
            continue;
        }
        let centre = crate::convert::sim_to_bevy(s.pos);
        let iso = Isometry3d::new(centre, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2));
        let base = def.radius.max(60.0) * 6.0;
        for k in 0..2 {
            let t = (phase + k as f32 * 0.5).fract();
            // Rings move in from the rim and fade as they reach the ship, the pull of a gravity well.
            let radius = base * (1.0 - 0.6 * t);
            let alpha = 0.55 * (t * std::f32::consts::PI).sin();
            gizmos.circle(iso, radius, Color::srgba(0.55, 0.35, 1.0, alpha)).resolution(64);
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

    #[test]
    fn test_sensor_ping_expansion_and_fade() {
        let ping = TacticalSensorPing {
            pos: Vec3::ZERO,
            max_radius: 1200.0,
            duration: 2.5,
            elapsed: 0.0,
            color: Color::srgb(0.25, 0.85, 1.0),
        };
        assert_eq!(ping.elapsed, 0.0);
        let t_half = 1.25 / ping.duration;
        let r_half = ping.max_radius * (1.0 - (1.0 - t_half).powi(2));
        assert!(r_half > 0.6 * ping.max_radius, "ease-out expands quickly initially");
        let alpha_half = (1.0 - t_half).powi(2) * 0.85;
        assert!(alpha_half < 0.85, "alpha fades over time");
    }

    #[test]
    fn a_beam_steps_through_its_texture_frames_side_by_side() {
        // Four frames: each shows a quarter of the texture's width, frame 1 starts a quarter in, frame 4 wraps to 0.
        let uv = |t: f32| {
            let a = beam_frame_uv(t, 4);
            (a.matrix2.x_axis.x, a.translation.x)
        };
        assert_eq!(uv(0.0), (0.25, 0.0));
        assert_eq!(uv(1.5), (0.25, 0.25));
        assert_eq!(uv(3.0), (0.25, 0.75));
        assert_eq!(uv(4.2), (0.25, 0.0));
        // A texture without frames is shown whole.
        assert_eq!((beam_frame_uv(7.0, 1).matrix2.x_axis.x, beam_frame_uv(7.0, 1).translation.x), (1.0, 0.0));
    }
}


/// Bloom flash of big explosions (capital / station death): `(position, strength)` entries wait here until
/// [`bloom_pulse`] folds them into the camera's bloom.
#[derive(Resource, Default)]
pub struct BloomPulses {
    queued: Vec<(Vec3, f32)>,
    /// Where the last big explosion happened (offscreen captures aim the camera there).
    pub last_big: Option<Vec3>,
    strength: f32,
    base: Option<f32>,
}

impl BloomPulses {
    /// Queues a flash at a world position (1.0 = a capital ship's final explosion right in front of the camera).
    pub fn flash(&mut self, at: Vec3, strength: f32) {
        if strength >= 0.5 {
            self.last_big = Some(at);
        }
        self.queued.push((at, strength));
    }
}

/// Distance from the camera beyond which an explosion adds no flash.
const BLOOM_PULSE_RANGE: f32 = 5000.0;
/// Extra bloom intensity at full strength (the camera's resting intensity is estimated at 0.07). Small: the UI is drawn into the same HDR camera and would wash out too.
const BLOOM_PULSE_BOOST: f32 = 0.12;
/// Strength lost per second: a flash lasts about 0.7 s.
const BLOOM_PULSE_DECAY: f32 = 1.4;


/// Full-screen white layer of the explosion flash.
#[derive(Component)]
pub struct FlashOverlay;

/// Peak opacity of the white flash layer.
const FLASH_OVERLAY_ALPHA: f32 = 0.8;

/// Raises the camera bloom and a white layer for a moment when something big blows up (the Remake whites out the screen, n93 f0006).
pub fn bloom_pulse(
    mut commands: Commands,
    time: Res<Time>,
    mut pulses: ResMut<BloomPulses>,
    ui: Option<Res<eaw_ui::EawUiSettings>>,
    mut overlay: Query<(&mut BackgroundColor, &mut Node), With<FlashOverlay>>,
    mut cam: Query<(&Transform, Option<&mut Bloom>), With<crate::camera::BattleCamera>>,
) {
    let Ok((tf, bloom)) = cam.single_mut() else { return };
    let queued = std::mem::take(&mut pulses.queued);
    for (at, s) in queued {
        let near = (1.0 - tf.translation.distance(at) / BLOOM_PULSE_RANGE).clamp(0.0, 1.0);
        pulses.strength = pulses.strength.max(s * near);
    }
    pulses.strength = (pulses.strength - BLOOM_PULSE_DECAY * time.delta_secs()).max(0.0);
    // Test hook: `SP_FLASH_HOLD=<0..1>` keeps the flash at that strength so an offscreen capture can judge its look.
    static HOLD: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    if let Some(hold) = *HOLD.get_or_init(|| std::env::var("SP_FLASH_HOLD").ok().and_then(|v| v.parse::<f32>().ok())) {
        pulses.strength = hold.clamp(0.0, 1.0);
    }
    if let Some(mut bloom) = bloom {
        let base = *pulses.base.get_or_insert(bloom.intensity);
        let want = base + BLOOM_PULSE_BOOST * pulses.strength;
        if (bloom.intensity - want).abs() > 1e-4 {
            bloom.intensity = want;
        }
    }
    let alpha = pulses.strength.powi(2) * FLASH_OVERLAY_ALPHA;
    if let Ok((mut bg, mut node)) = overlay.single_mut() {
        node.display = if alpha > 0.005 { Display::Flex } else { Display::None };
        bg.0 = Color::srgba(1.0, 0.99, 0.95, alpha);
    } else if alpha > 0.005 {
        let mut e = commands.spawn((
            BattleEntity,
            FlashOverlay,
            Name::new("explosion flash"),
            Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
            BackgroundColor(Color::srgba(1.0, 0.99, 0.95, alpha)),
            // Under the tactical bar and the HUD (they sit at z >= 0).
            GlobalZIndex(-1000),
            bevy::ui::FocusPolicy::Pass,
        ));
        if let Some(c) = ui.and_then(|u| u.camera) {
            e.insert(UiTargetCamera(c));
        }
    }
}
