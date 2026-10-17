//! Bevy plugin that spawns original Empire at War models with an EaW-style look.
//!
//! ```ignore
//! app.add_plugins(AlamoRenderPlugin::new(Arc::new(game_fs)));
//! commands.spawn((AlamoModel::new("W_planet_tatooine_low.alo"), Transform::default()));
//! ```
//!
//! See [`AlamoModel`] for spawning, [`spawn_sun`] for the lighting convention and
//! [`material::AlamoMaterial`] for the shader emulation.
pub mod animation;
pub mod frame_trace;
pub mod hardpoints;
mod lighting;
pub mod lod;
pub mod material;
pub mod model;
pub mod particles;
pub mod preload;
pub mod projectiles;
pub mod sequences;
pub mod texture;
pub mod xmldata;

use std::sync::Arc;

use alamo_fs::GameFs;
use bevy::asset::embedded_asset;
use bevy::prelude::*;

pub use animation::{AlamoAnimation, AnimationIndex, AnimationSettings, TurretAim};
pub use hardpoints::{AimTurrets, HardpointDestroyed};
pub use lighting::{SunLight, spawn_sun, spawn_sun_shadowed};
pub use lod::LodSettings;
pub use model::{
    AlamoBillboard, AlamoBone, AlamoFs, AlamoHardpoint, AlamoModel, AlamoModelLoaded, AlamoPart,
    AlamoProxy, AlamoRenderSettings, LodSelect, ModelCache, ModelLoadFailed, ModelLoaded,
};
pub use particles::{
    AlamoParticles, EngineThrottle, ParticleInstance, ParticleSettings, ParticleStats,
    ParticlesLoaded,
};
pub use projectiles::{
    AutoDespawn, ImpactKind, ProjectileImpact, ProjectileVisual, ShieldFlash, ShieldHit,
    SpawnParticleEffect,
};
pub use preload::ModelPreloader;
pub use sequences::{DeathSequence, HyperspaceArrival, HyperspaceArrived, PlayDeath};
pub use xmldata::{HardpointDef, ParticleDef, ProjectileDef, UnitDef, XmlDefs};

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
            .init_resource::<preload::ModelPreloader>()
            .add_systems(
                PostUpdate,
                (
                    preload::integrate_preloads.before(model::spawn_models),
                    model::spawn_models,
                    model::update_team_colors,
                    model::update_billboards.before(TransformSystems::Propagate),
                ),
            );
        particles::build(app);
        app.add_systems(
            PostUpdate,
            lod::cull_small_models.after(particles::update_particle_view),
        );
        projectiles::build(app, &self.fs);
        animation::build(app, &self.fs);
        sequences::build(app);
        hardpoints::build(app);
    }
}
