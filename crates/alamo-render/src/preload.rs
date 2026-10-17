//! Asynchronous model and texture preloading.
//!
//! Loading a model reads and parses the `.alo`, decodes its textures and builds the meshes. Doing that
//! for the dozens of distinct models and effects of a battle in one frame stalls the render thread (about
//! 100 ms in release, seconds in a debug build). [`ModelPreloader::request`] moves the expensive, thread-safe
//! part (file read, `.alo` parse, texture decode) to worker threads, together with the particle systems the
//! model's proxies name (engine glows, damage fires); [`integrate_preloads`] hands the results to the caches
//! within a small per-frame time budget, after which `spawn_models` only has to build meshes and materials.
//! [`ModelPreloader::request_particles`] does the same for effect particle systems and the bolt models of
//! projectiles (effects would otherwise read and decode them on their first explosion or shot), and
//! [`ModelPreloader::request_animations`] for `.ala` clips. Poll [`ModelPreloader::is_idle`] to know when
//! everything requested is in the caches.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alamo_fs::{FileSource, GameFs};
use alamo_model::alo::ParamValue;
use alamo_model::{Animation, Model, ParticleSystem};
use bevy::prelude::*;

use crate::animation::AnimationCache;
use crate::model::{ModelCache, model_path};
use crate::particles::{ParticleCache, particle_path};
use crate::projectiles::{BoltCache, BoltGeometry, bolt_key, load_geometry};
use crate::texture::{TextureCache, image_from_bytes, read_texture_file};
use crate::xmldata::XmlDefs;

enum Job {
    Model(String),
    Particles(String),
    Bolt { model: String, scale: f32, laser_color: Option<[u8; 4]> },
    Animation(String),
}

/// What a worker read and decoded for one job.
#[derive(Default)]
struct Done {
    /// The parsed model (`None` for other jobs).
    model: Option<(String, Result<Model, String>)>,
    /// Decoded textures the model's materials, the particle emitters or the bolt layers use.
    textures: Vec<(String, Option<Image>)>,
    /// Particle systems by resolved path: the job's own, or the ones a model's proxies name (`None` = no such file).
    particles: Vec<(String, Option<ParticleSystem>)>,
    /// Bolt geometry under its cache key.
    bolt: Option<(String, Option<Arc<BoltGeometry>>)>,
    /// A parsed animation clip by path.
    animation: Option<(String, Option<Animation>)>,
}

/// Names a worker already took on (texture names, particle and animation paths, bolt keys; all lower case),
/// so that no two jobs read the same file. The caches they fill are never cleared, so neither are these.
#[derive(Default)]
struct Claims {
    textures: HashSet<String>,
    particles: HashSet<String>,
    bolts: HashSet<String>,
    animations: HashSet<String>,
}

/// Handle to the preload workers; inserted by [`AlamoRenderPlugin`](crate::AlamoRenderPlugin).
#[derive(Resource)]
pub struct ModelPreloader {
    tx: Sender<Done>,
    rx: Mutex<Receiver<Done>>,
    pending: Arc<AtomicUsize>,
    claimed: Arc<Mutex<Claims>>,
    /// Main-thread time budget per frame for [`integrate_preloads`].
    pub budget: Duration,
}

impl Default for ModelPreloader {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx: Mutex::new(rx),
            pending: Arc::new(AtomicUsize::new(0)),
            claimed: Arc::default(),
            budget: Duration::from_millis(3),
        }
    }
}

impl ModelPreloader {
    /// True when everything requested has been integrated into the caches.
    pub fn is_idle(&self) -> bool {
        self.pending.load(Ordering::Acquire) == 0
    }

    /// Number of requested jobs that are not in the caches yet.
    pub fn pending(&self) -> usize {
        self.pending.load(Ordering::Acquire)
    }

    /// Starts loading `paths` (model names as in the XML, with or without `.ALO`) on worker threads.
    /// Models that are already cached are skipped.
    pub fn request(&self, fs: &Arc<GameFs>, cache: &ModelCache, paths: impl IntoIterator<Item = String>) {
        let mut seen = HashSet::new();
        let jobs: Vec<Job> = paths
            .into_iter()
            .map(|p| model_path(&p))
            .filter(|p| !cache.contains(p) && seen.insert(p.to_ascii_lowercase()))
            .map(Job::Model)
            .collect();
        self.spawn(fs, jobs);
    }

    /// Preloads the particle objects `names` (`<Particle>` names of the XML, e.g. death and impact effects)
    /// and, for the projectiles `projectiles`, their bolt models and the particle objects they spawn.
    pub fn request_particles(
        &self,
        fs: &Arc<GameFs>,
        defs: &XmlDefs,
        names: impl IntoIterator<Item = String>,
        projectiles: impl IntoIterator<Item = String>,
    ) {
        let mut all: Vec<String> = names.into_iter().collect();
        let mut jobs = Vec::new();
        for p in projectiles {
            if let Some(d) = defs.projectile(&p) {
                all.extend(
                    [&d.object_detonation, &d.armor_detonation, &d.lifetime_detonation, &d.shield_absorb]
                        .into_iter()
                        .flatten()
                        .cloned(),
                );
                if let Some(model) = &d.model {
                    jobs.push(Job::Bolt { model: model.clone(), scale: d.scale, laser_color: d.laser_color });
                }
            }
        }
        let mut seen = HashSet::new();
        jobs.extend(
            all.iter()
                .filter_map(|n| defs.particle(n))
                .map(|d| particle_path(&d.model))
                .filter(|p| seen.insert(p.to_ascii_lowercase()))
                .map(Job::Particles),
        );
        self.spawn(fs, jobs);
    }

    /// Preloads the `.ala` animation clips at `paths`.
    pub fn request_animations(&self, fs: &Arc<GameFs>, paths: impl IntoIterator<Item = String>) {
        let mut seen = HashSet::new();
        let jobs: Vec<Job> = paths.into_iter().filter(|p| seen.insert(p.to_ascii_lowercase())).map(Job::Animation).collect();
        self.spawn(fs, jobs);
    }

    fn spawn(&self, fs: &Arc<GameFs>, jobs: Vec<Job>) {
        if jobs.is_empty() {
            return;
        }
        self.pending.fetch_add(jobs.len(), Ordering::AcqRel);
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(1, 4));
        let queue = Arc::new(Mutex::new(jobs));
        for _ in 0..threads {
            let (fs, queue, tx, claimed) = (fs.clone(), queue.clone(), self.tx.clone(), self.claimed.clone());
            std::thread::Builder::new()
                .name("alamo-preload".into())
                .spawn(move || {
                    while let Some(job) = queue.lock().unwrap().pop() {
                        let _ = tx.send(run(&fs, &claimed, job));
                    }
                })
                .expect("spawn preload thread");
        }
    }
}

/// Claims `key` (lower-cased) in the set `pick` selects; false if a worker already has it.
fn claim(claimed: &Mutex<Claims>, pick: fn(&mut Claims) -> &mut HashSet<String>, key: &str) -> bool {
    pick(&mut claimed.lock().unwrap()).insert(key.to_ascii_lowercase())
}

fn decode_new(fs: &GameFs, claimed: &Mutex<Claims>, names: Vec<String>) -> Vec<(String, Option<Image>)> {
    let mut out = Vec::new();
    for name in names {
        if name.is_empty() || !claim(claimed, |c| &mut c.textures, &name) {
            continue;
        }
        let img = match read_texture_file(fs, &name) {
            // A data gap (the file exists in no layer): logged once, not an error of ours.
            None => {
                info!("texture {name}: absent in data, using the neutral fallback");
                None
            }
            Some(d) => match image_from_bytes(&d) {
                Ok(i) => Some(i),
                Err(e) => {
                    warn!("texture {name}: undecodable ({e}), using the neutral fallback");
                    None
                }
            },
        };
        out.push((name, img));
    }
    out
}

/// Reads and parses the particle system at `path` unless a worker has it already: records it (`None` if there is
/// no such file) and queues its sprite textures.
fn read_particles(fs: &GameFs, claimed: &Mutex<Claims>, path: String, done: &mut Done, textures: &mut Vec<String>) {
    if !claim(claimed, |c| &mut c.particles, &path) {
        return;
    }
    let system = fs.read(&path).ok().and_then(|d| match ParticleSystem::parse(&d) {
        Ok(s) => Some(s),
        Err(e) => {
            warn!("particle system {path}: {e}");
            None
        }
    });
    for e in system.iter().flat_map(|s| &s.emitters) {
        textures.push(e.texture.clone());
        textures.extend(e.texture2.clone());
    }
    done.particles.push((path, system));
}

fn run(fs: &GameFs, claimed: &Mutex<Claims>, job: Job) -> Done {
    let mut done = Done::default();
    let mut textures = Vec::new();
    match job {
        Job::Model(key) => match fs.read(&key).map(|d| Model::parse(&d)) {
            Ok(Ok(m)) => {
                for mesh in &m.meshes {
                    for sub in &mesh.sub_meshes {
                        for (_, v) in &sub.material.params {
                            if let ParamValue::Texture(t) = v {
                                textures.push(t.clone());
                            }
                        }
                    }
                }
                // Proxies naming a particle file get it attached when the model spawns (`attach_proxy_particles`).
                for p in &m.proxies {
                    read_particles(fs, claimed, particle_path(&p.name), &mut done, &mut textures);
                }
                done.model = Some((key, Ok(m)));
            }
            // Some projectiles name a particle system as their model (`pp_plasma_bomb`): it is preloaded as one.
            Ok(Err(alamo_model::Error::NotAModel(_))) => read_particles(fs, claimed, key, &mut done, &mut textures),
            Ok(Err(e)) => done.model = Some((key.clone(), Err(format!("cannot parse {key}: {e}")))),
            Err(e) => done.model = Some((key.clone(), Err(format!("cannot read {key}: {e}")))),
        },
        Job::Particles(path) => read_particles(fs, claimed, path, &mut done, &mut textures),
        Job::Bolt { model, scale, laser_color } => {
            let key = bolt_key(&model, scale);
            if claim(claimed, |c| &mut c.bolts, &key) {
                let geometry = load_geometry(fs, &model, scale, laser_color);
                textures.extend(geometry.iter().flat_map(|g| g.textures().map(str::to_string)));
                done.bolt = Some((key, geometry));
            }
        }
        Job::Animation(path) => {
            if claim(claimed, |c| &mut c.animations, &path) {
                let clip = fs.read(&path).ok().and_then(|d| match Animation::parse(&d) {
                    Ok(a) => Some(a),
                    Err(e) => {
                        warn!("animation {path}: {e}");
                        None
                    }
                });
                done.animation = Some((path, clip));
            }
        }
    }
    done.textures = decode_new(fs, claimed, textures);
    done
}

/// Moves finished preloads into [`ModelCache`], [`TextureCache`], [`ParticleCache`], [`BoltCache`] and
/// [`AnimationCache`] (at most [`ModelPreloader::budget`] of main-thread time per frame).
#[allow(clippy::too_many_arguments)]
pub fn integrate_preloads(
    preloader: Res<ModelPreloader>,
    mut cache: ResMut<ModelCache>,
    mut textures: ResMut<TextureCache>,
    mut particles: ResMut<ParticleCache>,
    mut bolts: ResMut<BoltCache>,
    mut clips: ResMut<AnimationCache>,
    mut images: ResMut<Assets<Image>>,
) {
    if preloader.is_idle() {
        return;
    }
    let started = Instant::now();
    let rx = preloader.rx.lock().unwrap();
    while let Ok(done) = rx.try_recv() {
        for (name, img) in done.textures {
            textures.insert_decoded(&name, img, &mut images);
        }
        for (path, system) in done.particles {
            particles.insert(&path, system);
        }
        if let Some((key, geometry)) = done.bolt {
            bolts.insert(key, geometry);
        }
        if let Some((path, clip)) = done.animation {
            clips.insert(&path, clip);
        }
        match done.model {
            Some((key, Ok(m))) => cache.insert_preparsed(&key, m),
            Some((_, Err(e))) => warn!("preload: {e}"),
            None => {}
        }
        preloader.pending.fetch_sub(1, Ordering::AcqRel);
        if started.elapsed() >= preloader.budget {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AlamoFs;
    use alamo_fs::GameInstall;
    use bevy::ecs::system::RunSystemOnce;

    /// The Remake 4.0 file system, or `None` without the install or the mod (the test is skipped).
    fn remake_fs() -> Option<Arc<GameFs>> {
        let install = GameInstall::detect()?;
        install.mod_data("1770851727").is_dir().then_some(())?;
        GameFs::for_mod(&install, Some("1770851727")).ok().map(Arc::new)
    }

    fn world() -> World {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<ModelCache>();
        world.init_resource::<TextureCache>();
        world.init_resource::<ParticleCache>();
        world.init_resource::<BoltCache>();
        world.init_resource::<AnimationCache>();
        world.init_resource::<ModelPreloader>();
        world
    }

    /// Runs `integrate_preloads` until everything requested is in the caches.
    fn integrate(world: &mut World) {
        let started = Instant::now();
        while !world.resource::<ModelPreloader>().is_idle() {
            assert!(started.elapsed() < Duration::from_secs(60), "the preload did not finish");
            world.run_system_once(integrate_preloads).unwrap();
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_model_preload_brings_the_particle_systems_its_proxies_name() {
        let Some(fs) = remake_fs() else { return };
        let mut world = world();
        // The Bothan cruiser's engine glows hang on its `PE_Bothan_Cruiser_L` / `_S` proxies.
        world.resource::<ModelPreloader>().request(&fs, world.resource::<ModelCache>(), ["Neutral_Bothan_Cruiser.ALO".to_string()]);
        integrate(&mut world);
        assert!(world.resource::<ModelCache>().contains(&model_path("Neutral_Bothan_Cruiser.ALO")));
        // Asked against an empty file system, only what the preload put into the cache can be found.
        let empty = AlamoFs(Arc::new(GameFs::new(&[]).unwrap()));
        let mut particles = world.resource_mut::<ParticleCache>();
        assert!(particles.exists(&empty, &particle_path("PE_Bothan_Cruiser_L")));
        assert!(particles.exists(&empty, &particle_path("PE_Bothan_Cruiser_S")));
    }

    #[test]
    fn a_projectile_preload_builds_its_bolt() {
        let Some(fs) = remake_fs() else { return };
        let defs = XmlDefs::load(&fs);
        // A turbolaser: its bolt model is textured (`w_turbolaser.dds` took 12.5 ms on the main thread at the first shot).
        let mut lasers: Vec<&crate::xmldata::ProjectileDef> =
            defs.projectiles.values().filter(|p| p.model.as_deref().is_some_and(|m| m.to_ascii_lowercase().contains("turbolaser"))).collect();
        lasers.sort_by(|a, b| a.name.cmp(&b.name));
        let laser = lasers.first().expect("a turbolaser projectile");
        let mut world = world();
        let textures_before = world.resource::<TextureCache>().len();
        world.resource::<ModelPreloader>().request_particles(&fs, &defs, [], [laser.name.clone()]);
        integrate(&mut world);
        assert!(world.resource::<BoltCache>().built(&bolt_key(laser.model.as_deref().unwrap(), laser.scale)), "{} has no bolt", laser.name);
        assert!(world.resource::<TextureCache>().len() > textures_before, "the bolt texture is decoded with it");
    }
}
