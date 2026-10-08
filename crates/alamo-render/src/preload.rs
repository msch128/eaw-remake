//! Asynchronous model and texture preloading.
//!
//! Loading a model reads and parses the `.alo`, decodes its textures and builds the meshes. Doing that
//! for the dozens of distinct models of a battle in one frame stalls the render thread (about 100 ms in
//! release, seconds in a debug build). [`ModelPreloader::request`] moves the expensive, thread-safe part
//! (file read, `.alo` parse, texture decode) to worker threads; [`integrate_preloads`] hands the results to
//! the caches within a small per-frame time budget, after which `spawn_models` only has to build meshes and
//! materials. Poll [`ModelPreloader::is_idle`] to know when everything requested is in the caches.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alamo_fs::GameFs;
use alamo_model::Model;
use alamo_model::alo::ParamValue;
use bevy::prelude::*;

use crate::model::{ModelCache, model_path};
use crate::texture::{TextureCache, image_from_bytes, read_texture_file};

/// A parsed model and the decoded textures its materials reference.
struct Done {
    key: String,
    model: Result<Model, String>,
    textures: Vec<(String, Option<Image>)>,
}

/// Handle to the preload workers; insert by [`AlamoRenderPlugin`](crate::AlamoRenderPlugin).
#[derive(Resource)]
pub struct ModelPreloader {
    tx: Sender<Done>,
    rx: Mutex<Receiver<Done>>,
    pending: Arc<AtomicUsize>,
    /// Texture names already claimed by a worker or present in the cache.
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
    /// True when every requested model has been integrated into the caches.
    pub fn is_idle(&self) -> bool {
        self.pending.load(Ordering::Acquire) == 0
    }

    /// Number of requested models that are not in the caches yet.
    pub fn pending(&self) -> usize {
        self.pending.load(Ordering::Acquire)
    }

    /// Starts loading `paths` (model names as in the XML, with or without `.ALO`) on worker threads.
    /// Models that are already cached or requested before are cheap to request again.
    pub fn request(&self, fs: &Arc<GameFs>, cache: &ModelCache, paths: impl IntoIterator<Item = String>) {
        let mut seen = HashSet::new();
        let jobs: Vec<String> = paths
            .into_iter()
            .map(|p| model_path(&p))
            .filter(|p| !cache.contains(p) && seen.insert(p.to_ascii_lowercase()))
            .collect();
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
                    while let Some(key) = queue.lock().unwrap().pop() {
                        let _ = tx.send(load(&fs, &claimed, key));
                    }
                })
                .expect("spawn preload thread");
        }
    }
}

fn load(fs: &GameFs, claimed: &Mutex<HashSet<String>>, key: String) -> Done {
    use alamo_fs::FileSource;
    let model = fs
        .read(&key)
        .map_err(|e| format!("cannot read {key}: {e}"))
        .and_then(|d| Model::parse(&d).map_err(|e| format!("cannot parse {key}: {e}")));
    let mut textures = Vec::new();
    if let Ok(m) = &model {
        let mut names: Vec<&str> = Vec::new();
        for mesh in &m.meshes {
            for sub in &mesh.sub_meshes {
                for (_, v) in &sub.material.params {
                    if let ParamValue::Texture(t) = v {
                        names.push(t);
                    }
                }
            }
        }
        for name in names {
            if !claimed.lock().unwrap().insert(name.to_ascii_lowercase()) {
                continue;
            }
            let img = read_texture_file(fs, name).and_then(|d| image_from_bytes(&d).ok());
            textures.push((name.to_string(), img));
        }
    }
    Done { key, model, textures }
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
            Ok(m) => cache.insert_preparsed(&done.key, m),
            Err(e) => warn!("preload: {e}"),
        }
        preloader.pending.fetch_sub(1, Ordering::AcqRel);
        if started.elapsed() >= preloader.budget {
            break;
        }
    }
}
