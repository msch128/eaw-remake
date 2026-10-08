//! Asynchronous model and texture preloading.
//!
//! Loading a model reads and parses the `.alo`, decodes its textures and builds the meshes. Doing that
//! for the dozens of distinct models and effects of a battle in one frame stalls the render thread (about
//! 100 ms in release, seconds in a debug build). [`ModelPreloader::request`] moves the expensive, thread-safe
//! part (file read, `.alo` parse, texture decode) to worker threads; [`integrate_preloads`] hands the results
//! to the caches within a small per-frame time budget, after which `spawn_models` only has to build meshes
//! and materials. [`ModelPreloader::request_particles`] does the same for the sprite textures of particle
//! systems (effects would otherwise decode them on their first explosion). Poll
//! [`ModelPreloader::is_idle`] to know when everything requested is in the caches.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alamo_fs::{FileSource, GameFs};
use alamo_model::alo::ParamValue;
use alamo_model::{Model, ParticleSystem};
use bevy::prelude::*;

use crate::model::{ModelCache, model_path};
use crate::texture::{TextureCache, image_from_bytes, read_texture_file};
use crate::xmldata::XmlDefs;

enum Job {
    Model(String),
    Particles(String),
}

/// A parsed model and the decoded textures its materials or emitters reference.
struct Done {
    key: String,
    /// `None` for particle systems (only their textures are preloaded).
    model: Option<Result<Model, String>>,
    textures: Vec<(String, Option<Image>)>,
}

/// Handle to the preload workers; inserted by [`AlamoRenderPlugin`](crate::AlamoRenderPlugin).
#[derive(Resource)]
pub struct ModelPreloader {
    tx: Sender<Done>,
    rx: Mutex<Receiver<Done>>,
    pending: Arc<AtomicUsize>,
    /// Texture names already claimed by a worker.
    claimed: Arc<Mutex<HashSet<String>>>,
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

    /// Preloads the sprite textures of the particle objects `names` (`<Particle>` names of the XML, e.g. death
    /// and impact effects) and of the particle objects that the projectiles `projectiles` spawn.
    pub fn request_particles(
        &self,
        fs: &Arc<GameFs>,
        defs: &XmlDefs,
        names: impl IntoIterator<Item = String>,
        projectiles: impl IntoIterator<Item = String>,
    ) {
        let mut all: Vec<String> = names.into_iter().collect();
        for p in projectiles {
            if let Some(d) = defs.projectile(&p) {
                all.extend(
                    [&d.object_detonation, &d.armor_detonation, &d.lifetime_detonation, &d.shield_absorb]
                        .into_iter()
                        .flatten()
                        .cloned(),
                );
            }
        }
        let mut seen = HashSet::new();
        let jobs: Vec<Job> = all
            .iter()
            .filter_map(|n| defs.particle(n))
            .map(|d| model_path(&d.model))
            .filter(|p| seen.insert(p.to_ascii_lowercase()))
            .map(Job::Particles)
            .collect();
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

fn decode_new(fs: &GameFs, claimed: &Mutex<HashSet<String>>, names: Vec<String>) -> Vec<(String, Option<Image>)> {
    let mut out = Vec::new();
    for name in names {
        if name.is_empty() || !claimed.lock().unwrap().insert(name.to_ascii_lowercase()) {
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

fn run(fs: &GameFs, claimed: &Mutex<HashSet<String>>, job: Job) -> Done {
    match job {
        Job::Model(key) => {
            let model = fs
                .read(&key)
                .map_err(|e| format!("cannot read {key}: {e}"))
                .and_then(|d| Model::parse(&d).map_err(|e| format!("cannot parse {key}: {e}")));
            let mut names = Vec::new();
            if let Ok(m) = &model {
                for mesh in &m.meshes {
                    for sub in &mesh.sub_meshes {
                        for (_, v) in &sub.material.params {
                            if let ParamValue::Texture(t) = v {
                                names.push(t.clone());
                            }
                        }
                    }
                }
            }
            Done { key, model: Some(model), textures: decode_new(fs, claimed, names) }
        }
        Job::Particles(key) => {
            let mut names = Vec::new();
            if let Ok(d) = fs.read(&key)
                && let Ok(sys) = ParticleSystem::parse(&d)
            {
                for e in &sys.emitters {
                    names.push(e.texture.clone());
                    names.extend(e.texture2.clone());
                }
            }
            Done { key, model: None, textures: decode_new(fs, claimed, names) }
        }
    }
}

/// Moves finished preloads into [`ModelCache`] and [`TextureCache`] (at most
/// [`ModelPreloader::budget`] of main-thread time per frame).
pub fn integrate_preloads(
    preloader: Res<ModelPreloader>,
    mut cache: ResMut<ModelCache>,
    mut textures: ResMut<TextureCache>,
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
        match done.model {
            Some(Ok(m)) => cache.insert_preparsed(&done.key, m),
            Some(Err(e)) => warn!("preload: {e}"),
            None => {}
        }
        preloader.pending.fetch_sub(1, Ordering::AcqRel);
        if started.elapsed() >= preloader.budget {
            break;
        }
    }
}
