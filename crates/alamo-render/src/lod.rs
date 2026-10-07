//! Distance level of detail: whole models that cover less than a few pixels are not drawn, and
//! particles below a pixel are not built.
//!
//! The original engine also switches between `_ALTn` low-detail meshes. In this data set only
//! planets and a few props have them (no ship, fighter or hardpoint model does), so the ALT
//! choice stays a spawn-time option ([`LodSelect`](crate::LodSelect)); the culling here is what
//! helps ship battles.

use bevy::prelude::*;

use crate::model::AlamoModelLoaded;
use crate::particles::ParticleView;

/// Level-of-detail options.
#[derive(Resource, Debug, Clone)]
pub struct LodSettings {
    /// Master switch.
    pub enabled: bool,
    /// Models whose bounding sphere is smaller than this many pixels in diameter are hidden.
    pub min_model_px: f32,
    /// Particle quads smaller than this many pixels are skipped.
    pub particle_min_px: f32,
}

impl Default for LodSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            min_model_px: 2.0,
            particle_min_px: 1.0,
        }
    }
}

/// Hides the `alamo_axis` node of models that are too small on screen.
pub fn cull_small_models(
    lod: Res<LodSettings>,
    view: Res<ParticleView>,
    models: Query<(&AlamoModelLoaded, &GlobalTransform)>,
    mut axes: Query<&mut Visibility>,
) {
    if view.px_per_unit <= 0.0 {
        return;
    }
    for (loaded, gt) in &models {
        let Ok(mut vis) = axes.get_mut(loaded.axis) else {
            continue;
        };
        let want = if lod.enabled {
            let world_center = gt.transform_point(loaded.center);
            let dist = (world_center - view.position).length().max(1e-3);
            let radius = loaded.radius * gt.scale().max_element();
            let px = 2.0 * radius * view.px_per_unit / dist;
            if px < lod.min_model_px {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            }
        } else {
            Visibility::Inherited
        };
        if *vis != want {
            *vis = want;
        }
    }
}
