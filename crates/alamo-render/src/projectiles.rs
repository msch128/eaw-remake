//! Projectile visuals and impact effects.
//!
//! The original engine draws laser bolts itself: a `<Projectile>` in the XML names a small bolt
//! model (`w_turbolaser_g.alo`: crossed additive planes, head at +Y in model space, tail trailing
//! behind it) and a `Scale_Factor`. [`ProjectileVisual`] entities are rendered as that geometry,
//! transformed on the CPU into the shared per-texture batch meshes of the particle renderer, so a
//! thousand bolts cost the same number of draw calls as one.
//!
//! Impacts are events: [`ProjectileImpact`] spawns the particle object the projectile definition
//! names for that kind of hit, [`SpawnParticleEffect`] spawns a particle object by name (hardpoint
//! explosions) and [`ShieldHit`] flashes the shield mesh of a model locally.

use std::collections::HashMap;
use std::sync::Arc;

use alamo_fs::FileSource;
use alamo_model::Model;
use bevy::prelude::*;

use crate::material::{AlamoMaterial, FLAG_SHIELD_HITS};
use crate::model::{AlamoFs, AlamoModelLoaded, AlamoPart};
use crate::particles::{AlamoParticles, ExtraBatchGeometry, MeshBuf, ParticleSettings};
use crate::xmldata::{FRAMES_PER_SECOND, XmlDefs};

/// Draws a projectile bolt described by the `<Projectile>` named `name`.
///
/// The bolt flies along the entity's forward direction (`-Z`). With `simulate` the plugin also
/// moves it at the XML speed and removes it at the maximum flight distance (triggering a
/// [`ImpactKind::Lifetime`] impact); game code that owns the motion leaves it `false`.
#[derive(Component, Debug, Clone)]
#[require(Transform, Visibility)]
pub struct ProjectileVisual {
    /// Projectile object name, e.g. `Proj_Ship_Turbolaser_Green`.
    pub name: String,
    /// Let the plugin move the bolt.
    pub simulate: bool,
    /// Extra color multiplier (white = unchanged).
    pub tint: Color,
}

impl ProjectileVisual {
    /// A bolt that is moved by the caller.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            simulate: false,
            tint: Color::WHITE,
        }
    }

    /// A bolt the plugin flies along its forward axis.
    pub fn simulated(mut self) -> Self {
        self.simulate = true;
        self
    }
}

/// One textured layer of a bolt model, already in Bevy space and scaled.
struct BoltLayer {
    texture: String,
    additive: bool,
    color: Vec4,
    pos: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    idx: Vec<u32>,
}

/// Geometry of a bolt model.
pub struct BoltGeometry {
    layers: Vec<BoltLayer>,
}

#[derive(Component)]
pub(crate) struct BoltInstance {
    geometry: Arc<BoltGeometry>,
    tint: Vec4,
    speed: f32,
    max_distance: f32,
    travelled: f32,
    simulate: bool,
    name: String,
}

impl BoltInstance {
    fn empty(name: &str) -> Self {
        Self {
            geometry: Arc::new(BoltGeometry { layers: Vec::new() }),
            tint: Vec4::ONE,
            speed: 0.0,
            max_distance: 0.0,
            travelled: 0.0,
            simulate: false,
            name: name.to_string(),
        }
    }
}

/// Cache of bolt geometry per model.
#[derive(Resource, Default)]
pub struct BoltCache {
    geometry: HashMap<String, Option<Arc<BoltGeometry>>>,
}

fn load_geometry(
    fs: &AlamoFs,
    model: &str,
    scale: f32,
    laser_color: Option<[u8; 4]>,
) -> Option<Arc<BoltGeometry>> {
    let path = crate::model::model_path(model);
    let data = fs.0.read(&path).ok()?;
    let m = Model::parse(&data).ok()?;
    // Alamo (Z-up) to Bevy (Y-up): (x, y, z) -> (x, z, -y), then the bolt scale.
    let conv = |p: [f32; 3]| [p[0] * scale, p[2] * scale, -p[1] * scale];
    let mut layers = Vec::new();
    for (mi, mesh) in m.meshes.iter().enumerate() {
        if mesh.hidden || mesh.is_shadow_volume() {
            continue;
        }
        let w = m.world_transform(m.bone_of_object(mi));
        for sub in &mesh.sub_meshes {
            let shader = sub.material.shader.to_ascii_lowercase();
            if shader.contains("collision") {
                continue;
            }
            let Some(tex) = sub.material.texture("BaseTexture") else {
                continue;
            };
            let mut color = sub.material.float4("Color").map_or(Vec4::ONE, Vec4::from);
            color.w = 1.0;
            if let Some(c) = laser_color {
                color *= Vec4::new(c[0] as f32, c[1] as f32, c[2] as f32, 255.0) / 255.0;
            }
            let pos = sub
                .vertices
                .iter()
                .map(|v| {
                    let p = v.position;
                    conv([
                        w[0][0] * p[0] + w[0][1] * p[1] + w[0][2] * p[2] + w[0][3],
                        w[1][0] * p[0] + w[1][1] * p[1] + w[1][2] * p[2] + w[1][3],
                        w[2][0] * p[0] + w[2][1] * p[1] + w[2][2] * p[2] + w[2][3],
                    ])
                })
                .collect();
            layers.push(BoltLayer {
                texture: tex.to_ascii_lowercase(),
                additive: shader.contains("additive"),
                color,
                pos,
                uv: sub.vertices.iter().map(|v| v.uv[0]).collect(),
                idx: sub.indices.iter().map(|&i| i as u32).collect(),
            });
        }
    }
    (!layers.is_empty()).then(|| Arc::new(BoltGeometry { layers }))
}

/// Resolves new [`ProjectileVisual`] entities.
pub(crate) fn load_bolts(
    mut commands: Commands,
    fs: Res<AlamoFs>,
    defs: Res<XmlDefs>,
    mut cache: ResMut<BoltCache>,
    new: Query<(Entity, &ProjectileVisual), Without<BoltInstance>>,
) {
    for (entity, spec) in &new {
        let Some(def) = defs.projectile(&spec.name) else {
            warn!("unknown projectile {}", spec.name);
            commands
                .entity(entity)
                .insert(BoltInstance::empty(&spec.name));
            continue;
        };
        let Some(model) = def.model.clone() else {
            // Engine-drawn bolt without a model; not rendered yet.
            commands
                .entity(entity)
                .insert(BoltInstance::empty(&spec.name));
            continue;
        };
        let key = format!("{}@{}", model.to_ascii_lowercase(), def.scale);
        let geometry = cache
            .geometry
            .entry(key)
            .or_insert_with(|| load_geometry(&fs, &model, def.scale, def.laser_color))
            .clone();
        let Some(geometry) = geometry else {
            warn!("projectile model {model} could not be loaded");
            commands
                .entity(entity)
                .insert(BoltInstance::empty(&spec.name));
            continue;
        };
        let tint = spec.tint.to_srgba();
        commands.entity(entity).insert(BoltInstance {
            geometry,
            tint: Vec4::new(tint.red, tint.green, tint.blue, 1.0),
            speed: def.max_speed * FRAMES_PER_SECOND,
            max_distance: def.max_distance,
            travelled: 0.0,
            simulate: spec.simulate,
            name: spec.name.clone(),
        });
    }
}

/// Moves simulated bolts and ends their flight.
pub(crate) fn fly_bolts(
    mut commands: Commands,
    time: Res<Time>,
    mut bolts: Query<(Entity, &mut BoltInstance, &mut Transform, &GlobalTransform)>,
) {
    let dt = time.delta_secs().min(0.1);
    for (e, mut b, mut t, gt) in &mut bolts {
        if !b.simulate {
            continue;
        }
        let step = b.speed * dt;
        let fwd = t.forward();
        t.translation += fwd * step;
        b.travelled += step;
        if b.max_distance > 0.0 && b.travelled >= b.max_distance {
            commands.trigger(ProjectileImpact {
                projectile: b.name.clone(),
                kind: ImpactKind::Lifetime,
                position: gt.translation(),
                normal: -*fwd,
            });
            commands.entity(e).despawn();
        }
    }
}

/// Writes all bolts into the shared batch geometry.
pub(crate) fn collect_bolts(
    settings: Res<ParticleSettings>,
    mut extra: ResMut<ExtraBatchGeometry>,
    bolts: Query<(&BoltInstance, &GlobalTransform, &InheritedVisibility)>,
) {
    let mode_for = |additive: bool| match (additive, settings.additive_alpha) {
        (false, _) => 0,
        (true, true) => 1,
        (true, false) => 2,
    };
    for (b, gt, vis) in &bolts {
        if !vis.get() {
            continue;
        }
        let m = gt.affine();
        for layer in &b.geometry.layers {
            let buf: &mut MeshBuf = extra
                .buffers
                .entry((layer.texture.clone(), mode_for(layer.additive)))
                .or_default();
            let base = buf.pos.len() as u32;
            let mut c = layer.color * b.tint;
            if layer.additive {
                c = (c.truncate() * settings.bolt_gain).extend(1.0);
            }
            let color = c.to_array();
            for (p, uv) in layer.pos.iter().zip(&layer.uv) {
                buf.pos.push(m.transform_point3(Vec3::from(*p)).to_array());
                buf.uv.push(*uv);
                buf.col.push(color);
            }
            buf.idx.extend(layer.idx.iter().map(|i| i + base));
        }
    }
}

/// Kind of a projectile impact; selects the particle the XML names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImpactKind {
    /// Hit on a hull (`Projectile_Object_Detonation_Particle`).
    Hull,
    /// Hit on armor (`Projectile_Object_Armor_Reduced_Detonation_Particle`).
    Armor,
    /// End of flight (`Projectile_Lifetime_Detonation_Particle`).
    Lifetime,
    /// Hit on the ground (`Projectile_Ground_Detonation_Particle`).
    Ground,
    /// Absorbed by a shield (`Projectile_Absorbed_By_Shields_Particle`).
    Shield,
}

/// Spawns the impact particle of a projectile type at a world position.
#[derive(Event, Debug, Clone)]
pub struct ProjectileImpact {
    /// Projectile object name.
    pub projectile: String,
    /// Kind of impact.
    pub kind: ImpactKind,
    /// World position.
    pub position: Vec3,
    /// Surface normal (particle `+Y` is aligned with it).
    pub normal: Vec3,
}

/// Spawns a `<Particle>` object (by XML name, e.g. a hardpoint death explosion).
#[derive(Event, Debug, Clone)]
pub struct SpawnParticleEffect {
    /// Particle object name.
    pub particle: String,
    /// World position.
    pub position: Vec3,
    /// Direction of the effect's up axis.
    pub normal: Vec3,
}

/// Removes the entity when the timer ends (impact effects).
#[derive(Component, Debug)]
pub struct AutoDespawn(pub Timer);

fn spawn_effect(
    commands: &mut Commands,
    defs: &XmlDefs,
    particle: &str,
    position: Vec3,
    normal: Vec3,
) {
    let Some(def) = defs.particle(particle) else {
        warn!("unknown particle object {particle}");
        return;
    };
    let up = if normal.length_squared() > 1e-6 {
        normal.normalize()
    } else {
        Vec3::Y
    };
    commands.spawn((
        Name::new(format!("fx {particle}")),
        AlamoParticles::new(def.model.clone()),
        Transform {
            translation: position,
            rotation: Quat::from_rotation_arc(Vec3::Y, up),
            scale: Vec3::splat(def.scale),
        },
        AutoDespawn(Timer::from_seconds(def.lifetime_secs(), TimerMode::Once)),
    ));
}

/// Observer: [`ProjectileImpact`].
pub fn on_projectile_impact(ev: On<ProjectileImpact>, mut commands: Commands, defs: Res<XmlDefs>) {
    let Some(p) = defs.projectile(&ev.projectile) else {
        return;
    };
    let name = match ev.kind {
        ImpactKind::Hull => &p.object_detonation,
        ImpactKind::Armor => {
            if p.armor_detonation.is_some() {
                &p.armor_detonation
            } else {
                &p.object_detonation
            }
        }
        ImpactKind::Lifetime => &p.lifetime_detonation,
        ImpactKind::Ground => &p.ground_detonation,
        ImpactKind::Shield => &p.shield_absorb,
    };
    if let Some(name) = name {
        spawn_effect(&mut commands, &defs, name, ev.position, ev.normal);
    }
}

/// Observer: [`SpawnParticleEffect`].
pub fn on_spawn_effect(ev: On<SpawnParticleEffect>, mut commands: Commands, defs: Res<XmlDefs>) {
    spawn_effect(&mut commands, &defs, &ev.particle, ev.position, ev.normal);
}

/// Despawns finished effects.
pub fn auto_despawn(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut AutoDespawn)>,
) {
    for (e, mut t) in &mut q {
        if t.0.tick(time.delta()).is_finished() {
            commands.entity(e).despawn();
        }
    }
}

/// Flashes the shield of a model where a hit landed. `model` is the entity carrying
/// [`AlamoModel`](crate::AlamoModel); the model must have a hidden `MeshShield` mesh.
#[derive(Event, Debug, Clone)]
pub struct ShieldHit {
    /// Model root entity.
    pub model: Entity,
    /// World-space hit position.
    pub position: Vec3,
    /// Brightness multiplier (1 = normal).
    pub intensity: f32,
}

/// Per-model shield state (own copies of the shield materials).
#[derive(Component, Debug, Default)]
pub struct ShieldFlash {
    parts: Vec<Entity>,
    next: usize,
    last_hit: f32,
}

const SHIELD_FLASH_SECS: f32 = 0.35;

/// Observer: [`ShieldHit`].
pub fn on_shield_hit(
    ev: On<ShieldHit>,
    mut commands: Commands,
    time: Res<Time>,
    mut materials: ResMut<Assets<AlamoMaterial>>,
    loaded: Query<&AlamoModelLoaded>,
    mut flashes: Query<&mut ShieldFlash>,
    mut parts: Query<(
        &AlamoPart,
        &mut MeshMaterial3d<AlamoMaterial>,
        &mut Visibility,
    )>,
) {
    let Ok(l) = loaded.get(ev.model) else {
        return;
    };
    let now = time.elapsed_secs_wrapped();
    let existing = flashes
        .get(ev.model)
        .ok()
        .map(|s| (s.next, s.parts.clone()));
    let first = existing.is_none();
    let (slot, shield_parts) = match existing {
        Some(x) => x,
        None => {
            let shield_parts: Vec<Entity> = l
                .parts
                .iter()
                .copied()
                .filter(|&e| parts.get(e).is_ok_and(|(p, _, _)| p.shield))
                .collect();
            // Give this instance its own copy of every shield material.
            for &e in &shield_parts {
                if let Ok((_, mut mat, _)) = parts.get_mut(e)
                    && let Some(src) = materials.get(&mat.0).cloned()
                {
                    let mut own = src;
                    own.params.flags |= FLAG_SHIELD_HITS;
                    own.params.hit_params = Vec4::new(l.radius * 0.14, SHIELD_FLASH_SECS, 1.0, 0.0);
                    mat.0 = materials.add(own);
                }
            }
            (0, shield_parts)
        }
    };
    for &e in &shield_parts {
        let Ok((_, mat, mut vis)) = parts.get_mut(e) else {
            continue;
        };
        if let Some(mut m) = materials.get_mut(&mat.0) {
            m.params.hits[slot % 8] = ev.position.extend(now);
            m.params.hit_params.z = ev.intensity;
        }
        *vis = Visibility::Visible;
    }
    if first {
        commands.entity(ev.model).insert(ShieldFlash {
            parts: shield_parts,
            next: 1,
            last_hit: now,
        });
    } else if let Ok(mut s) = flashes.get_mut(ev.model) {
        s.next = slot + 1;
        s.last_hit = now;
    }
}

/// Hides shield meshes again once their flashes have faded.
pub fn update_shields(
    time: Res<Time>,
    flashes: Query<&ShieldFlash>,
    mut vis: Query<&mut Visibility>,
) {
    let now = time.elapsed_secs_wrapped();
    for f in &flashes {
        if now - f.last_hit > SHIELD_FLASH_SECS {
            for &p in &f.parts {
                if let Ok(mut v) = vis.get_mut(p)
                    && *v != Visibility::Hidden
                {
                    *v = Visibility::Hidden;
                }
            }
        }
    }
}

/// Registers projectile resources, systems and observers.
pub fn build(app: &mut App, fs: &Arc<alamo_fs::GameFs>) {
    app.insert_resource(XmlDefs::load(fs))
        .init_resource::<BoltCache>()
        .add_observer(on_projectile_impact)
        .add_observer(on_spawn_effect)
        .add_observer(on_shield_hit)
        .add_systems(
            PostUpdate,
            (
                load_bolts,
                fly_bolts,
                collect_bolts,
                auto_despawn,
                update_shields,
            )
                .chain()
                .after(bevy::transform::TransformSystems::Propagate)
                .before(crate::particles::update_particle_batches),
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shield_flash_duration_is_short() {
        assert!(SHIELD_FLASH_SECS <= 0.5, "shield flash must be short and crisp");
    }
}
