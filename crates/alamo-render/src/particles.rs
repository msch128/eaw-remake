//! Particle systems: CPU simulation, one dynamic quad mesh per (texture, blend mode) batch.
//!
//! Every [`AlamoParticles`] entity owns a set of emitter runtimes simulated in parallel. After the
//! simulation each live particle is a camera-facing quad; all quads of the same texture and blend
//! mode (from every instance in the scene) are written into a single dynamic mesh, so the number
//! of draw calls is the number of distinct texture/blend pairs, independent of the particle count.
//! Alpha-blended batches are sorted back to front, additive batches are not.
//!
//! Emitter-local space is **Y-up** (inferred from engine exhausts and damage fires, which only
//! line up with their proxy bones if particle Y maps to the bone's Z axis). Systems attached to
//! model proxies map it onto the bone frame (`(x, y, z) -> (x, -z, y)`) and inherit the model's
//! Z-up to Y-up conversion; standalone systems use their Bevy transform directly (see
//! [`AlamoParticles::in_model_space`]).

use std::collections::HashMap;
use std::sync::Arc;

use alamo_fs::FileSource;
use alamo_model::ParticleSystem;
use alamo_model::particle::{BlendMode, Emitter};
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::Affine3A;
use bevy::mesh::PrimitiveTopology;
use bevy::pbr::{Material, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;

use crate::model::{AlamoFs, AlamoProxy};
use crate::texture::TextureCache;

/// Path of the embedded particle shader.
pub const PARTICLE_SHADER_PATH: &str = "embedded://alamo_render/particle.wgsl";

/// Global particle options.
#[derive(Resource, Debug, Clone)]
pub struct ParticleSettings {
    /// Name prefixes of proxy particle systems that start enabled (see [`starts_enabled`]).
    pub always_on_prefixes: Vec<String>,
    /// Spawn particle systems for model proxies that name an existing particle file.
    pub auto_attach: bool,
    /// Multiplier on spawn rates (quality knob).
    pub density: f32,
    /// Upper bound for the total number of live particles; spawning stops beyond it.
    pub max_total: usize,
    /// Brightness multiplier for projectile bolts (the `Color` of bolt models is 0.5 and meant for
    /// an HDR pipeline with bloom).
    pub bolt_gain: f32,
    /// Scale additive particles by their alpha track. Off by default: additive emitters fade through\n    /// their color tracks and a third of them have a low alpha peak that would make them nearly invisible.
    pub additive_alpha: bool,
    /// Fixed simulation step in seconds; `None` uses the frame time (clamped to 0.1 s).
    pub fixed_step: Option<f32>,
}

impl Default for ParticleSettings {
    fn default() -> Self {
        Self {
            auto_attach: true,
            always_on_prefixes: vec!["pe_".into(), "pte_".into()],
            density: 1.0,
            max_total: 200_000,
            additive_alpha: false,
            bolt_gain: 4.0,
            fixed_step: None,
        }
    }
}

/// Statistics of the last frame, for diagnostics and tests.
#[derive(Resource, Debug, Default, Clone)]
pub struct ParticleStats {
    /// Live particles.
    pub live: usize,
    /// Draw batches with at least one particle.
    pub batches: usize,
    /// Systems simulated.
    pub systems: usize,
    /// Milliseconds spent in simulation plus mesh building.
    pub cpu_ms: f32,
}

/// Plays a particle system (`Art/Models/<path>`). Insert it on an entity with a transform;
/// its [`Transform`] is a normal Bevy transform unless [`in_model_space`](Self::in_model_space)
/// is set.
#[derive(Component, Debug, Clone)]
#[require(Transform, Visibility)]
pub struct AlamoParticles {
    /// File name or full path of the particle `.alo`.
    pub path: String,
    /// New particles are only emitted while enabled; existing ones finish their life.
    /// Switching from false to true restarts the emitters.
    pub enabled: bool,
    /// The entity is already below an Alamo-space node (a model proxy), so no Z-up conversion
    /// is applied.
    pub in_model_space: bool,
    /// Brightness and size multiplier (1 = as authored); engine exhausts follow [`EngineThrottle`].
    pub intensity: f32,
}

impl AlamoParticles {
    /// An enabled standalone particle system.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            enabled: true,
            in_model_space: false,
            intensity: 1.0,
        }
    }

    /// Starts disabled.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// Triggered on the entity once its particle system has been loaded.
#[derive(EntityEvent, Debug)]
pub struct ParticlesLoaded {
    /// The [`AlamoParticles`] entity.
    pub entity: Entity,
}

/// Particle quad material: texture times vertex color; additive or alpha blended.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct ParticleMaterial {
    /// Sprite atlas.
    #[texture(0)]
    #[sampler(1)]
    pub texture: Option<Handle<Image>>,
    /// 0 alpha blending, 1 additive scaled by alpha, 2 additive ignoring alpha.
    #[uniform(2)]
    pub additive: u32,
    /// Header `[live index count]` followed by the triangle indices of the batch (see `update_particle_batches`).
    #[storage(3, read_only)]
    pub indices: Handle<ShaderBuffer>,
    /// Three `vec4` per vertex: position, uv, color. World space, written every frame in place.
    #[storage(4, read_only)]
    pub verts: Handle<ShaderBuffer>,
}

impl Material for ParticleMaterial {
    fn vertex_shader() -> ShaderRef {
        PARTICLE_SHADER_PATH.into()
    }

    fn fragment_shader() -> ShaderRef {
        PARTICLE_SHADER_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        if self.additive != 0 {
            AlphaMode::Add
        } else {
            AlphaMode::Blend
        }
    }

    fn enable_shadows() -> bool {
        false
    }

    fn enable_prepass() -> bool {
        false
    }
}

#[derive(Clone, Copy)]
struct Particle {
    pos: Vec3,
    vel: Vec3,
    age: f32,
    life: f32,
    frame: u32,
    rot: f32,
    spin: f32,
}

/// One camera-facing quad ready to be written into a batch mesh.
#[derive(Clone, Copy)]
struct Quad {
    center: Vec3,
    size: f32,
    rot: f32,
    color: [f32; 4],
    cell: u32,
    grid: u32,
}

struct EmitterRuntime {
    particles: Vec<Particle>,
    time: f32,
    spawn_acc: f32,
    burst_spawned: u32,
    cycles_done: u32,
    rng: u64,
    quads: Vec<Quad>,
}

impl EmitterRuntime {
    fn new(seed: u64) -> Self {
        Self {
            particles: Vec::new(),
            time: 0.0,
            spawn_acc: 0.0,
            burst_spawned: 0,
            cycles_done: 0,
            rng: seed | 1,
            quads: Vec::new(),
        }
    }

    fn restart(&mut self) {
        self.time = 0.0;
        self.spawn_acc = 0.0;
        self.burst_spawned = 0;
        self.cycles_done = 0;
    }

    fn rand(&mut self) -> f32 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let v = self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D);
        ((v >> 40) as f32) / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.rand()
    }
}

/// Runtime state of a spawned particle system.
#[derive(Component)]
pub struct ParticleInstance {
    system: Arc<ParticleSystem>,
    emitters: Vec<EmitterRuntime>,
    was_enabled: bool,
}

impl ParticleInstance {
    /// Number of live particles.
    pub fn live(&self) -> usize {
        self.emitters.iter().map(|e| e.particles.len()).sum()
    }

    /// The parsed system.
    pub fn system(&self) -> &ParticleSystem {
        &self.system
    }
}

/// Cache of parsed particle systems.
#[derive(Resource, Default)]
pub struct ParticleCache {
    systems: HashMap<String, Option<Arc<ParticleSystem>>>,
}

/// Resolves a particle name to a game path (adds the directory and `.alo` when missing).
pub fn particle_path(name: &str) -> String {
    let mut n = name.to_string();
    if !n.to_ascii_lowercase().ends_with(".alo") {
        n.push_str(".alo");
    }
    if n.contains('/') || n.contains('\\') {
        n
    } else {
        format!("Art/Models/{n}")
    }
}

/// True if a proxy particle system should run from the start: engine exhausts (`pe_`, `pte_`).
/// Everything else (damage fire `pf_`/`pi_`, impacts and explosions `p_`, projectiles `pd_`/`pp_`)
/// is driven by game events, so [`attach_proxy_particles`] spawns it disabled; the game enables it
/// through [`AlamoParticles::enabled`].
pub fn starts_enabled(name: &str, prefixes: &[String]) -> bool {
    let n = name.to_ascii_lowercase();
    prefixes.iter().any(|p| n.starts_with(p.as_str()))
}

impl ParticleCache {
    fn get(&mut self, fs: &AlamoFs, path: &str) -> Option<Arc<ParticleSystem>> {
        let key = path.to_ascii_lowercase();
        self.systems
            .entry(key)
            .or_insert_with(|| {
                let data = fs.0.read(path).ok()?;
                match ParticleSystem::parse(&data) {
                    Ok(s) => Some(Arc::new(s)),
                    Err(e) => {
                        warn!("particle system {path}: {e}");
                        None
                    }
                }
            })
            .clone()
    }

    /// True if `path` exists and parses.
    pub fn exists(&mut self, fs: &AlamoFs, path: &str) -> bool {
        self.get(fs, path).is_some()
    }
}

/// Creates the runtime for newly added [`AlamoParticles`].
pub fn load_particles(
    mut commands: Commands,
    fs: Res<AlamoFs>,
    mut cache: ResMut<ParticleCache>,
    new: Query<(Entity, &AlamoParticles), Without<ParticleInstance>>,
) {
    for (entity, spec) in &new {
        let path = particle_path(&spec.path);
        let Some(system) = cache.get(&fs, &path) else {
            error!("cannot load particle system {path}");
            commands.entity(entity).insert(ParticleInstance {
                system: Arc::new(ParticleSystem::default()),
                emitters: Vec::new(),
                was_enabled: false,
            });
            continue;
        };
        let emitters = (0..system.emitters.len())
            .map(|i| {
                EmitterRuntime::new(
                    (entity.to_bits() ^ ((i as u64 + 1) << 32)).wrapping_mul(0x9E37_79B9_7F4A_7C15),
                )
            })
            .collect();
        commands.entity(entity).insert(ParticleInstance {
            system,
            emitters,
            was_enabled: false,
        });
        commands.trigger(ParticlesLoaded { entity });
    }
}

type ProxyFilterItem = (Entity, &'static AlamoProxy, &'static Visibility);
type ProxyFilter = (Added<AlamoProxy>, Without<AlamoParticles>);

/// Starts particle systems on model proxies whose name resolves to a particle file.
pub fn attach_proxy_particles(
    mut commands: Commands,
    settings: Res<ParticleSettings>,
    fs: Res<AlamoFs>,
    mut cache: ResMut<ParticleCache>,
    proxies: Query<ProxyFilterItem, ProxyFilter>,
) {
    if !settings.auto_attach {
        return;
    }
    for (entity, proxy, vis) in &proxies {
        let path = particle_path(&proxy.name);
        if !cache.exists(&fs, &path) {
            continue;
        }
        let enabled =
            *vis != Visibility::Hidden && starts_enabled(&proxy.name, &settings.always_on_prefixes);
        commands.entity(entity).insert(AlamoParticles {
            path: proxy.name.clone(),
            enabled,
            in_model_space: true,
            intensity: 1.0,
        });
    }
}

/// Camera data used to orient quads.
#[derive(Resource, Default, Clone, Copy)]
pub struct ParticleView {
    pub(crate) rotation: Quat,
    pub(crate) position: Vec3,
    /// Screen pixels covered by one world unit at distance 1 (perspective cameras).
    pub(crate) px_per_unit: f32,
}

/// Captures the active camera.
pub fn update_particle_view(
    mut view: ResMut<ParticleView>,
    cameras: Query<(&Camera, &GlobalTransform, &Projection), With<Camera3d>>,
) {
    if let Some((cam, gt, proj)) = cameras.iter().find(|(c, _, _)| c.is_active) {
        view.rotation = gt.rotation();
        view.position = gt.translation();
        view.px_per_unit = match (proj, cam.physical_target_size()) {
            (Projection::Perspective(p), Some(size)) => size.y as f32 / (2.0 * (p.fov * 0.5).tan()),
            _ => 0.0,
        };
    }
}
/// Particle space is Y-up, Alamo model space Z-up: `(x, y, z) -> (x, -z, y)`.
fn particle_to_alamo() -> Quat {
    Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)
}

fn sample_box(rt: &mut EmitterRuntime, min: [f32; 3], max: [f32; 3]) -> Vec3 {
    Vec3::new(
        rt.range(min[0].min(max[0]), min[0].max(max[0])),
        rt.range(min[1].min(max[1]), min[1].max(max[1])),
        rt.range(min[2].min(max[2]), min[2].max(max[2])),
    )
}

/// Light emitters (type 4) whose size is a light radius of hundreds of units or more are dynamic
/// lights and are not drawn as sprites (drawn as sprites they wash out the screen); smaller ones
/// (`light_explosion_*`, muzzle halos, 13..160 units) are corona sprites drawn additively. Inferred.
const MAX_CORONA_SIZE: f32 = 3000.0;

fn is_light_volume(e: &Emitter) -> bool {
    matches!(e.blend, BlendMode::Light)
        && e.size
            .keys
            .iter()
            .map(|k| k.1)
            .fold(e.size.start.max(e.size.end), f32::max)
            > MAX_CORONA_SIZE
}

/// Random point of an emitter position shape. Kind 3 is an ellipsoid volume and kind 4 an
/// ellipsoid surface, both with the semi-axes in shape params 6..8 (they equal the min/max extents
/// without the 0.8% padding; Inferred from `debre` 32.4 vs 32.67 and the `Ring` emitter name).
/// Other kinds (0 point, 1 line, 2) use the min/max box.
fn sample_position(rt: &mut EmitterRuntime, s: &alamo_model::particle::Shape) -> Vec3 {
    if matches!(s.kind, 3 | 4) {
        let axes = [s.params[6], s.params[7], s.params[8]];
        let half = |i: usize| {
            if axes[i].abs() > 1e-4 {
                axes[i].abs()
            } else {
                (s.max[i] - s.min[i]).abs() * 0.5
            }
        };
        let r = Vec3::new(half(0), half(1), half(2));
        if r.max_element() > 0.0 {
            let dir = loop {
                let d = Vec3::new(
                    rt.range(-1.0, 1.0),
                    rt.range(-1.0, 1.0),
                    rt.range(-1.0, 1.0),
                );
                let l = d.length();
                if l > 1e-3 && l <= 1.0 {
                    break if s.kind == 4 { d / l } else { d };
                }
            };
            return dir * r;
        }
    }
    sample_box(rt, s.min, s.max)
}

fn spawn_particle(e: &Emitter, rt: &mut EmitterRuntime, to_world: &Affine3A, scale: f32) {
    let local_pos = sample_position(rt, &e.position);
    let mut local_vel = sample_box(rt, e.velocity.min, e.velocity.max);
    // `speed` (property 0x09) is a radial speed away from the emitter origin; negative values pull the
    // particles inward (`inwardsucking` has -350, explosion fire +135). Inferred from names and magnitudes.
    if e.speed != 0.0 {
        let dir = if local_pos.length() > 1e-3 {
            local_pos.normalize()
        } else {
            let d = Vec3::new(rt.range(-1.0, 1.0), rt.range(-1.0, 1.0), rt.range(-1.0, 1.0));
            if d.length() > 1e-3 { d.normalize() } else { Vec3::Y }
        };
        local_vel += dir * e.speed;
    }
    let (pos, vel) = if e.world_space {
        (
            to_world.transform_point3(local_pos),
            to_world.transform_vector3(local_vel),
        )
    } else {
        (local_pos, local_vel)
    };
    let rot = if matches!(e.blend, BlendMode::Alpha) {
        rt.range(0.0, std::f32::consts::TAU)
    } else {
        0.0
    };
    let sign = if rt.rand() < 0.5 { -1.0 } else { 1.0 };
    let spin = e.spin.at(rt.rand()).to_radians() * sign;
    let frame = 0;
    let _ = scale;
    rt.particles.push(Particle {
        pos,
        vel,
        age: 0.0,
        life: e.lifetime,
        frame,
        rot,
        spin,
    });
}

/// Per-instance parameters of one simulation step.
struct StepCtx<'a> {
    dt: f32,
    enabled: bool,
    to_world: &'a Affine3A,
    scale: f32,
    density: f32,
}

fn step_emitter(e: &Emitter, rt: &mut EmitterRuntime, cx: &StepCtx, budget: &mut usize) {
    let StepCtx {
        dt,
        enabled,
        to_world,
        scale,
        density,
    } = *cx;
    // Age and move.
    // Property 0x0b: gravity along the emitter's up axis (values 0 .. -145, a few +34; Inferred, most emitters 0).
    let acc = Vec3::from(e.acceleration) + Vec3::new(0.0, e.gravity, 0.0);
    for p in &mut rt.particles {
        p.age += dt;
        p.vel += acc * dt;
        p.pos += p.vel * dt;
        p.rot += p.spin * dt;
    }
    rt.particles.retain(|p| p.age < p.life);

    if !enabled || is_light_volume(e) || matches!(e.blend, BlendMode::Mesh) {
        return;
    }
    rt.time += dt;
    if rt.time < e.initial_delay {
        return;
    }
    let cap = if e.max_particles > 0 {
        e.max_particles as usize
    } else {
        usize::MAX
    };
    let forever = e.loop_count == 0 || e.loop_count == u32::MAX;
    if e.loop_count == 0 {
        // Continuous.
        let rate = if e.spawn_rate > 0.0 {
            e.spawn_rate
        } else if e.spawn_interval > 0.0 {
            1.0 / e.spawn_interval
        } else {
            0.0
        } * density;
        rt.spawn_acc += rate * dt;
        while rt.spawn_acc >= 1.0 {
            rt.spawn_acc -= 1.0;
            if rt.particles.len() < cap && *budget > 0 {
                spawn_particle(e, rt, to_world, scale);
                *budget -= 1;
            }
        }
    } else {
        // Burst: `cap` particles spaced `interval` apart, repeated `loop_count` times.
        let count = e.max_particles.max(1);
        let spacing = if e.spawn_interval > 0.0 {
            e.spawn_interval.min(e.lifetime / count as f32)
        } else {
            0.0
        };
        if forever || rt.cycles_done < e.loop_count {
            let t = rt.time - e.initial_delay;
            let due = if spacing <= 0.0 {
                count
            } else {
                ((t / spacing) as u32 + 1).min(count)
            };
            while rt.burst_spawned < due {
                if *budget > 0 {
                    spawn_particle(e, rt, to_world, scale);
                    *budget -= 1;
                }
                rt.burst_spawned += 1;
            }
            if rt.burst_spawned >= count && t > e.lifetime {
                rt.cycles_done += 1;
                rt.burst_spawned = 0;
                rt.time = e.initial_delay;
            }
        }
    }
}

fn build_quads(
    e: &Emitter,
    rt: &mut EmitterRuntime,
    to_world: &Affine3A,
    scale: f32,
    intensity: f32,
    cull: Option<(Vec3, f32)>,
) {
    let color_gain = if intensity < 1.0 {
        0.25 + 0.75 * intensity.max(0.0)
    } else {
        intensity
    };
    let size_gain = if intensity < 1.0 {
        0.7 + 0.3 * intensity.max(0.0)
    } else {
        1.0
    };
    rt.quads.clear();
    let grid = e.atlas_grid.max(1);
    let cells = grid * grid;
    for p in &rt.particles {
        let t = (p.age / p.life).clamp(0.0, 1.0);
        let a = e.color[3].at(t);
        let (r, g, b) = (
            e.color[0].at(t) * color_gain,
            e.color[1].at(t) * color_gain,
            e.color[2].at(t) * color_gain,
        );
        let size = e.size.at(t) * scale * size_gain;
        if a <= 0.001 || size <= 0.0 {
            continue;
        }
        let center = if e.world_space {
            p.pos
        } else {
            to_world.transform_point3(p.pos)
        };
        let _ = p.frame;
        // Frame tracks use mode 2: only the start value is the sprite; the end value is a
        // placeholder (usually 2 or 0).
        if let Some((eye, px_per_unit)) = cull
            && size * px_per_unit < (center - eye).length().max(1e-3)
        {
            continue;
        }
        // End values 0 and 2 are the editor default (4882 + 2430 of 8200 emitters); other end values
        // (16, 49, 64, ... = sheet sizes) are real flipbook animations over the particle life (Inferred).
        let frame = if e.frame.end > 2.0 && e.frame.end > e.frame.start {
            e.frame.at(t)
        } else {
            e.frame.start
        };
        let cell = (frame.max(0.0) as u32).min(cells - 1);
        rt.quads.push(Quad {
            center,
            size,
            rot: p.rot,
            color: [r, g, b, a],
            cell,
            grid,
        });
    }
}

/// Advances all particle systems.
#[allow(clippy::too_many_arguments)]
pub fn simulate_particles(
    time: Res<Time>,
    view: Res<ParticleView>,
    lod: Res<crate::lod::LodSettings>,
    settings: Res<ParticleSettings>,
    mut stats: ResMut<ParticleStats>,
    mut systems: Query<(
        &AlamoParticles,
        &GlobalTransform,
        Option<&InheritedVisibility>,
        &mut ParticleInstance,
    )>,
) {
    let started = std::time::Instant::now();
    let cull = (lod.enabled && view.px_per_unit > 0.0).then(|| {
        (
            view.position,
            view.px_per_unit / lod.particle_min_px.max(0.01),
        )
    });
    let dt = settings
        .fixed_step
        .unwrap_or_else(|| time.delta_secs().min(0.1));
    let mut live_total: usize = systems.iter().map(|(_, _, _, i)| i.live()).sum();
    let budget_total = settings.max_total.saturating_sub(live_total);
    let mut budget = budget_total;
    // Spawn budget is shared sequentially, the heavy per-emitter work is cheap enough serially;
    // `par_iter` splits it per instance with an even budget share.
    let n = systems.iter().count().max(1);
    let share = budget_total / n + 1;
    let _ = &mut budget;
    systems
        .par_iter_mut()
        .for_each(|(spec, gt, vis, mut inst)| {
            let inst = &mut *inst;
            let visible = vis.is_none_or(|v| v.get());
            let enabled = spec.enabled && visible;
            if enabled && !inst.was_enabled {
                for rt in &mut inst.emitters {
                    rt.restart();
                }
            }
            inst.was_enabled = enabled;
            let mut to_world = gt.affine();
            let mut scale = to_world.matrix3.x_axis.length();
            if spec.in_model_space {
                // Particle space is Y-up; below a model it is mapped onto the bone's Z-up frame.
                to_world *= Affine3A::from_quat(particle_to_alamo());
            }
            scale = scale.max(1e-6);
            let mut local_budget = share;
            let sys = inst.system.clone();
            for (e, rt) in sys.emitters.iter().zip(inst.emitters.iter_mut()) {
                step_emitter(
                    e,
                    rt,
                    &StepCtx {
                        dt,
                        enabled,
                        to_world: &to_world,
                        scale,
                        density: settings.density,
                    },
                    &mut local_budget,
                );
                build_quads(e, rt, &to_world, scale, spec.intensity, cull);
            }
        });
    live_total = systems.iter().map(|(_, _, _, i)| i.live()).sum();
    stats.live = live_total;
    stats.systems = systems.iter().count();
    stats.cpu_ms = started.elapsed().as_secs_f32() * 1000.0;
}

/// Heat-haze emitters distort the background in the original engine; they are not drawn.
fn is_distortion(e: &Emitter) -> bool {
    e.texture.to_ascii_lowercase().contains("heat")
}

type BatchKey = (String, u32);

struct Batch {
    entity: Entity,
    mesh: Handle<Mesh>,
    indices: Handle<ShaderBuffer>,
    verts: Handle<ShaderBuffer>,
    material: Handle<ParticleMaterial>,
    /// False when the sprite texture could not be loaded: such a batch is never drawn (a `None` texture
    /// would bind Bevy's white fallback image and show solid squares). A texture that is merely not
    /// uploaded yet needs no check: the material stays unprepared (`RetryNextUpdate`) and is not drawn.
    texture_ok: bool,
    /// Capacity in indices (also the vertex count of the dummy draw mesh) and in vertices.
    cap_idx: usize,
    cap_vert: usize,
}

const INITIAL_IDX_CAPACITY: usize = 6 * 1024;

/// Vertex data of extra geometry (projectile bolts) that is merged into the batch of the same
/// texture and blend mode every frame.
#[derive(Default)]
pub struct MeshBuf {
    /// Positions (world space).
    pub pos: Vec<[f32; 3]>,
    /// UVs.
    pub uv: Vec<[f32; 2]>,
    /// Vertex colors.
    pub col: Vec<[f32; 4]>,
    /// Triangle indices into this buffer.
    pub idx: Vec<u32>,
}

/// Extra batch geometry for the current frame, keyed by lowercase texture name and blend mode
/// (0 alpha, 1 additive with alpha, 2 additive). Cleared and refilled by the projectile systems.
#[derive(Resource, Default)]
pub struct ExtraBatchGeometry {
    pub(crate) buffers: HashMap<(String, u32), MeshBuf>,
}

/// Dynamic quad meshes, one per (texture, additive) pair.
#[derive(Resource, Default)]
pub struct ParticleBatches {
    batches: HashMap<BatchKey, Batch>,
}

/// Collects all quads into the batch meshes.
#[allow(clippy::too_many_arguments)]
pub fn update_particle_batches(
    mut commands: Commands,
    fs: Res<AlamoFs>,
    settings: Res<ParticleSettings>,
    view: Res<ParticleView>,
    mut textures: ResMut<TextureCache>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ParticleMaterial>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut batches: ResMut<ParticleBatches>,
    mut stats: ResMut<ParticleStats>,
    mut extra: ResMut<ExtraBatchGeometry>,
    systems: Query<&ParticleInstance>,
    mut visibility: Query<&mut Visibility>,
) {
    let started = std::time::Instant::now();
    let mut gathered: HashMap<BatchKey, Vec<Quad>> = HashMap::new();
    let mut extra_bufs = std::mem::take(&mut extra.buffers);
    for k in extra_bufs.keys() {
        gathered.entry(k.clone()).or_default();
    }
    for inst in &systems {
        for (e, rt) in inst.system.emitters.iter().zip(&inst.emitters) {
            if rt.quads.is_empty() {
                continue;
            }
            if is_distortion(e) || is_light_volume(e) || matches!(e.blend, BlendMode::Mesh) {
                continue;
            }
            let mode = match e.blend {
                BlendMode::Alpha => 0,
                BlendMode::Light => 1,
                _ if settings.additive_alpha => 1,
                _ => 2,
            };
            gathered
                .entry((e.texture.to_ascii_lowercase(), mode))
                .or_default()
                .extend_from_slice(&rt.quads);
        }
    }
    let right = view.rotation * Vec3::X;
    let up = view.rotation * Vec3::Y;
    let mut active = 0;
    for (key, batch) in &mut batches.batches {
        if !gathered.contains_key(key)
            && let Ok(mut v) = visibility.get_mut(batch.entity)
        {
            *v = Visibility::Hidden;
        }
    }
    for (key, mut quads) in gathered {
        let bolts = extra_bufs.remove(&key);
        if !batches.batches.contains_key(&key) {
            let texture = textures.get(&fs.0, &mut images, &key.0);
            let texture_ok = texture.is_some();
            let cap_idx = INITIAL_IDX_CAPACITY;
            let cap_vert = cap_idx / 6 * 4;
            let indices = buffers.add(ShaderBuffer::new(&vec![0u8; (cap_idx + 1) * 4], RenderAssetUsages::default()));
            let verts = buffers.add(ShaderBuffer::new(&vec![0u8; cap_vert * 48], RenderAssetUsages::default()));
            let material = materials.add(ParticleMaterial {
                texture,
                additive: key.1,
                indices: indices.clone(),
                verts: verts.clone(),
            });
            let mesh = meshes.add(draw_mesh(cap_idx));
            let entity = commands
                .spawn((
                    Name::new(format!("particles {}", key.0)),
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::IDENTITY,
                    NoFrustumCulling,
                ))
                .id();
            batches.batches.insert(
                key.clone(),
                Batch { entity, mesh, indices, verts, material, texture_ok, cap_idx, cap_vert },
            );
        }
        let batch = batches.batches.get_mut(&key).expect("batch exists");
        if key.1 == 0 {
            // Alpha blended: far to near.
            quads.sort_by(|a, b| {
                let da = (a.center - view.position).length_squared();
                let db = (b.center - view.position).length_squared();
                db.total_cmp(&da)
            });
        }
        let n = quads.len();
        active += 1;
        let mut pos = Vec::with_capacity(n * 4);
        let mut uv = Vec::with_capacity(n * 4);
        let mut col = Vec::with_capacity(n * 4);
        let mut idx = Vec::with_capacity(n * 6);
        for (i, q) in quads.iter().enumerate() {
            let h = q.size * 0.5;
            let (s, c) = q.rot.sin_cos();
            let ax = (right * c + up * s) * h;
            let ay = (up * c - right * s) * h;
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                pos.push((q.center + ax * dx + ay * dy).to_array());
            }
            let cell = q.cell % (q.grid * q.grid);
            let g = q.grid as f32;
            let (cx, cy) = ((cell % q.grid) as f32, (cell / q.grid) as f32);
            let (u0, v0, u1, v1) = (cx / g, cy / g, (cx + 1.0) / g, (cy + 1.0) / g);
            uv.extend([[u0, v1], [u1, v1], [u0, v0], [u1, v0]]);
            col.extend([q.color; 4]);
            let b = (i * 4) as u32;
            idx.extend([b, b + 1, b + 2, b + 2, b + 1, b + 3]);
        }
        if let Some(b) = bolts {
            let base = pos.len() as u32;
            pos.extend(b.pos);
            uv.extend(b.uv);
            col.extend(b.col);
            idx.extend(b.idx.iter().map(|i| i + base));
        }
        if pos.is_empty() {
            if let Ok(mut v) = visibility.get_mut(batch.entity) {
                *v = Visibility::Hidden;
            }
            continue;
        }
        // Grow geometrically (never shrink); a larger draw mesh and new buffers are rare events.
        if idx.len() > batch.cap_idx || pos.len() > batch.cap_vert {
            while batch.cap_idx < idx.len() {
                batch.cap_idx *= 2;
            }
            while batch.cap_vert < pos.len() {
                batch.cap_vert *= 2;
            }
            if let Some(mut m) = meshes.get_mut(&batch.mesh) {
                *m = draw_mesh(batch.cap_idx);
            }
            // New buffer assets of the new size; assigning them marks the material changed so its bind
            // group is rebuilt around them.
            batch.indices = buffers.add(ShaderBuffer::new(
                &vec![0u8; (batch.cap_idx + 1) * 4],
                RenderAssetUsages::default(),
            ));
            batch.verts = buffers.add(ShaderBuffer::new(
                &vec![0u8; batch.cap_vert * 48],
                RenderAssetUsages::default(),
            ));
            if let Some(mut m) = materials.get_mut(&batch.material) {
                m.indices = batch.indices.clone();
                m.verts = batch.verts.clone();
            }
        }
        // Rewrite both storage buffers in place (same size: Bevy uses `write_buffer`, no allocation).
        let mut ib = Vec::with_capacity((batch.cap_idx + 1) * 4);
        ib.extend_from_slice(&(idx.len() as u32).to_le_bytes());
        for i in &idx {
            ib.extend_from_slice(&i.to_le_bytes());
        }
        ib.resize((batch.cap_idx + 1) * 4, 0);
        let mut vb = Vec::with_capacity(batch.cap_vert * 48);
        for i in 0..pos.len() {
            for f in pos[i].iter().chain(&[0.0]).chain(&uv[i]).chain(&[0.0, 0.0]).chain(&col[i]) {
                vb.extend_from_slice(&f.to_le_bytes());
            }
        }
        vb.resize(batch.cap_vert * 48, 0);
        if let Some(mut b) = buffers.get_mut(&batch.indices) {
            b.data = Some(ib);
            b.buffer_description.size = ((batch.cap_idx + 1) * 4) as u64;
        }
        if let Some(mut b) = buffers.get_mut(&batch.verts) {
            b.data = Some(vb);
            b.buffer_description.size = (batch.cap_vert * 48) as u64;
        }
        if let Ok(mut v) = visibility.get_mut(batch.entity) {
            *v = if batch.texture_ok { Visibility::Visible } else { Visibility::Hidden };
        }
    }
    stats.batches = active;
    stats.cpu_ms += started.elapsed().as_secs_f32() * 1000.0;
}

/// Draw mesh of a batch: `cap_idx` dummy vertices. The vertex shader ignores their data and reads the
/// batch's storage buffers instead, so this mesh only changes when the capacity grows.
fn draw_mesh(cap_idx: usize) -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; cap_idx]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; cap_idx]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32; 4]; cap_idx]);
    mesh
}

/// Engine throttle of a ship (0 = idle, 1 = full thrust), set by game code from its speed.
/// The `pe_*` engine particle systems of the model scale their brightness and size with it and
/// switch off at zero.
#[derive(Component, Debug, Clone, Copy)]
pub struct EngineThrottle(pub f32);

/// Applies [`EngineThrottle`] to the engine proxies of a model.
pub fn apply_engine_throttle(
    ships: Query<(&EngineThrottle, &crate::model::AlamoModelLoaded)>,
    mut proxies: Query<&mut AlamoParticles>,
) {
    for (t, loaded) in &ships {
        let v = t.0.clamp(0.0, 1.0);
        for &e in &loaded.proxies {
            let Ok(mut p) = proxies.get_mut(e) else {
                continue;
            };
            let name = p.path.to_ascii_lowercase();
            if name.starts_with("pe_") || name.starts_with("pte_") {
                p.intensity = v;
                p.enabled = v > 0.02;
            }
        }
    }
}

/// Registers particle resources and systems.
pub fn build(app: &mut App) {
    bevy::asset::embedded_asset!(app, "particle.wgsl");
    if std::env::var_os("SP_ASSET_LOG").is_some() {
        app.add_systems(Update, log_asset_counts);
    }
    app.init_resource::<StableFonts>().add_systems(Update, (stabilize_fonts, collect_font_atlases).chain());
    app.add_plugins(MaterialPlugin::<ParticleMaterial>::default())
        .init_resource::<ParticleSettings>()
        .init_resource::<ParticleStats>()
        .init_resource::<ParticleCache>()
        .init_resource::<ParticleBatches>()
        .init_resource::<ExtraBatchGeometry>()
        .init_resource::<ParticleView>()
        .init_resource::<crate::lod::LodSettings>()
        .add_systems(
            PostUpdate,
            (
                attach_proxy_particles,
                load_particles,
                update_particle_view,
                apply_engine_throttle,
                simulate_particles,
                update_particle_batches,
            )
                .chain()
                .after(bevy::transform::TransformSystems::Propagate),
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_event_effects() {
        assert_eq!(
            particle_path("P_Smoke_Big00"),
            "Art/Models/P_Smoke_Big00.alo"
        );
        assert_eq!(particle_path("pf_small_w.alo"), "Art/Models/pf_small_w.alo");
        let pre = vec!["pe_".to_string()];
        assert!(!starts_enabled("P_Ship_Fire_Small_Explosion", &pre));
        assert!(starts_enabled("pe_imperial_star_destroyer_large", &pre));
    }

    #[test]
    fn rng_is_in_range() {
        let mut rt = EmitterRuntime::new(42);
        for _ in 0..1000 {
            let v = rt.rand();
            assert!((0.0..1.0).contains(&v));
        }
        let v = rt.range(2.0, 3.0);
        assert!((2.0..=3.0).contains(&v));
    }
}

/// Debug aid (`SP_ASSET_LOG=1`): logs asset counts every 10 s to spot leaks.
pub fn log_asset_counts(
    frames: Res<bevy::diagnostic::FrameCount>,
    images: Res<Assets<Image>>,
    meshes: Res<Assets<Mesh>>,
    std_mats: Res<Assets<StandardMaterial>>,
    part_mats: Res<Assets<ParticleMaterial>>,
    buffers: Res<Assets<ShaderBuffer>>,
    atlases: Res<bevy::text::FontAtlasSet>,
    time: Res<Time<Real>>,
    mut ft: Local<Vec<f32>>,
) {
    ft.push(time.delta_secs() * 1000.0);
    if frames.0 % 300 != 0 {
        return;
    }
    let img_bytes: usize = images.iter().map(|(_, i)| i.data.as_ref().map_or(0, |d| d.len())).sum();
    let mut hist: std::collections::BTreeMap<(u32, u32, u32), usize> = Default::default();
    for (_, i) in images.iter().filter(|(_, i)| i.data.is_some()) {
        *hist.entry((i.texture_descriptor.size.width, i.texture_descriptor.size.height, i.texture_descriptor.mip_level_count)).or_default() += 1;
    }
    let mut top: Vec<_> = hist.into_iter().collect();
    top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    let atlas_count: usize = atlases.values().map(|v| v.len()).sum();
    let mut by_size: Vec<(String, usize)> = atlases.iter().map(|(k, v)| (format!("{}/{}/{}/{:x}", k.id, k.index, f32::from_bits(k.font_size_bits), k.variations_hash), v.len())).collect();
    by_size.sort_by(|a, b| a.0.cmp(&b.0));
    info!("assetlog font atlases: keys={} atlases={} top(size,count)={:?}", atlases.len(), atlas_count, &by_size[..by_size.len().min(60)]);
    let (rss, peak) = process_memory_mb();
    ft.sort_by(|a, b| a.total_cmp(b));
    let p99 = ft.get(((ft.len() as f32 * 0.99) as usize).min(ft.len().saturating_sub(1))).copied().unwrap_or(0.0);
    let mean = ft.iter().sum::<f32>() / ft.len().max(1) as f32;
    info!("assetlog rss={rss} MB (peak {peak} MB) frame_ms mean={mean:.1} p99={p99:.1} max={:.1}", ft.last().copied().unwrap_or(0.0));
    ft.clear();
    info!("assetlog top cpu-image sizes (w,h,mips)=count: {:?}", &top[..top.len().min(4)]);
    let img_none = images.iter().filter(|(_, i)| i.data.is_none()).count();
    let img_gpu: usize = images
        .iter()
        .filter(|(_, i)| i.data.is_none())
        .map(|(_, i)| i.texture_descriptor.size.width as usize * i.texture_descriptor.size.height as usize * 4)
        .sum();
    let buf_bytes: usize = buffers.iter().map(|(_, b)| b.data.as_ref().map_or(0, |d| d.len())).sum();
    info!(
        "assetlog frame={} images={} (cpu {} MB, uploaded {} = ~{} MB) meshes={} std_mats={} particle_mats={} shader_buffers={} ({} MB)",
        frames.0,
        images.len(),
        img_bytes >> 20,
        img_none,
        img_gpu >> 20,
        meshes.len(),
        std_mats.len(),
        part_mats.len(),
        buffers.len(),
        buf_bytes >> 20
    );
}

/// Working set and peak working set of this process in MB (Windows only, 0 elsewhere).
fn process_memory_mb() -> (usize, usize) {
    #[cfg(windows)]
    {
        #[repr(C)]
        struct Counters {
            cb: u32,
            page_faults: u32,
            peak_working_set: usize,
            working_set: usize,
            rest: [usize; 6],
        }
        unsafe extern "system" {
            fn GetCurrentProcess() -> isize;
            fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, cb: u32) -> i32;
        }
        let mut c = Counters { cb: std::mem::size_of::<Counters>() as u32, page_faults: 0, peak_working_set: 0, working_set: 0, rest: [0; 6] };
        // SAFETY: plain Win32 call filling a correctly sized, repr(C) struct we own.
        if unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) } != 0 {
            return (c.working_set >> 20, c.peak_working_set >> 20);
        }
    }
    (0, 0)
}

/// Bevy keys font atlases by the font blob id, and system fonts get new blob ids whenever the font
/// source cache reloads them, so every relayout can add a fresh 512x512 atlas (1 MB of CPU data
/// each, never freed). Above a cap the atlas set is cleared and all text is re-queued, which
/// rasterizes only the glyphs in use again and lets the old atlas images drop.
pub fn collect_font_atlases(
    frames: Res<bevy::diagnostic::FrameCount>,
    mut atlases: ResMut<bevy::text::FontAtlasSet>,
    mut fonts: Query<&mut TextFont>,
) {
    const MAX_ATLASES: usize = 64;
    if frames.0 % 120 != 0 {
        return;
    }
    let count: usize = atlases.values().map(|v| v.len()).sum();
    if count <= MAX_ATLASES {
        return;
    }
    atlases.clear();
    for mut f in &mut fonts {
        f.set_changed();
    }
    info!("font atlas set had {count} atlases; cleared and re-queued all text");
}

/// System font files loaded once, so text uses a stable `Font` asset instead of a family lookup
/// (family lookups hand out a new font blob id whenever the font source cache reloads a file).
#[derive(Resource, Default)]
pub struct StableFonts(HashMap<String, Option<Handle<bevy::text::Font>>>);

/// Replaces `FontSource::Family("Bahnschrift"/"Arial"/...)` by a handle to the same file loaded
/// once from the Windows font folder. Families that cannot be read keep the family lookup.
pub fn stabilize_fonts(
    mut stable: ResMut<StableFonts>,
    mut fonts: ResMut<Assets<bevy::text::Font>>,
    mut texts: Query<&mut TextFont, Changed<TextFont>>,
) {
    for mut f in &mut texts {
        let bevy::text::FontSource::Family(name) = &f.font else { continue };
        let key = name.to_string().to_ascii_lowercase();
        let handle = stable
            .0
            .entry(key.clone())
            .or_insert_with(|| {
                let path = format!("C:/Windows/Fonts/{key}.ttf");
                let bytes = std::fs::read(path).ok()?;
                Some(fonts.add(bevy::text::Font::from_bytes(bytes)))
            })
            .clone();
        if let Some(h) = handle {
            f.font = bevy::text::FontSource::Handle(h);
        }
    }
}
