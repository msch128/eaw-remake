//! Bevy plugin that spawns original Empire at War models with an EaW-style look.
//!
//! ```ignore
//! app.add_plugins(AlamoRenderPlugin::new(Arc::new(game_fs)));
//! commands.spawn((AlamoModel::new("W_planet_tatooine_low.alo"), Transform::default()));
//! ```
//!
//! See [`AlamoModel`] for spawning, [`spawn_sun`] for the lighting convention and
//! [`material::AlamoMaterial`] for the shader emulation.
mod lighting;
pub mod material;
pub mod model;
pub mod particles;
pub mod projectiles;
pub mod texture;
pub mod xmldata;

use std::sync::Arc;

use alamo_fs::GameFs;
use bevy::asset::embedded_asset;
use bevy::prelude::*;

pub use lighting::{SunLight, spawn_sun};
pub use model::{
    AlamoBillboard, AlamoBone, AlamoFs, AlamoModel, AlamoModelLoaded, AlamoPart, AlamoProxy,
    AlamoRenderSettings, LodSelect, ModelCache, ModelLoadFailed, ModelLoaded,
};
pub use particles::{
    AlamoParticles, ParticleInstance, ParticleSettings, ParticleStats, ParticlesLoaded,
};
pub use projectiles::{
    AutoDespawn, ImpactKind, ProjectileImpact, ProjectileVisual, ShieldFlash, ShieldHit,
    SpawnParticleEffect,
};
pub use xmldata::{ParticleDef, ProjectileDef, XmlDefs};

/// Registers the material, caches and systems.
pub struct AlamoRenderPlugin {
    fs: Arc<GameFs>,
}

impl AlamoRenderPlugin {
    /// Creates the plugin for the given game file system.
    pub fn new(fs: Arc<GameFs>) -> Self {
        Self { fs }
    }
}

impl Plugin for AlamoRenderPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "alamo.wgsl");
        app.add_plugins(MaterialPlugin::<material::AlamoMaterial>::default())
            .insert_resource(AlamoFs(self.fs.clone()))
            .init_resource::<AlamoRenderSettings>()
            .init_resource::<ModelCache>()
            .init_resource::<texture::TextureCache>()
            .add_systems(
                PostUpdate,
                (
                    model::spawn_models,
                    model::update_team_colors,
                    model::update_billboards.before(TransformSystems::Propagate),
                ),
            );
        particles::build(app);
        projectiles::build(app, &self.fs);
    }
}
