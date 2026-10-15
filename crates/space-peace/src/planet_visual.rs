//! Visual representation of a planet on the galaxy map.
//!
//! [`spawn_planet_visual`] is the single swap point for planet rendering: with game files
//! available each planet is an original galactic `.alo` model (via `alamo-render`), otherwise
//! (demo data, missing model) a lit placeholder sphere. Owner and selection rings are drawn as
//! constant-pixel-width gizmos by the galaxy map from [`PlanetRadii`], which holds each planet's
//! real world radius once its model has loaded.

use alamo_render::{AlamoModel, AlamoModelLoaded, ModelLoadFailed};
use bevy::{
    mesh::{SphereKind, SphereMeshBuilder},
    prelude::*,
};

use crate::galaxy_view::GalaxyView;

/// Colour used for neutral planets.
pub const NEUTRAL: [f32; 3] = [0.55, 0.60, 0.70];

/// Converts an sRGB triple to a Bevy colour.
pub fn rgb(c: [f32; 3]) -> Color {
    Color::srgb(c[0], c[1], c[2])
}

/// World-space radius of each planet (index = planet index): an estimate from the planet
/// density until the model has loaded, then the model's real radius times its scale.
#[derive(Resource, Default, Debug)]
pub struct PlanetRadii(pub Vec<f32>);

/// Marks the root entity of a planet's visual; `.0` is the index into `GalaxyView::planets`.
#[derive(Component)]
pub struct PlanetVisual(#[allow(dead_code)] pub usize);

/// Marks the `AlamoModel` entity of planet `.0`.
#[derive(Component)]
struct PlanetModel(usize);

/// Keeps [`PlanetRadii`] in sync with the loaded models.
pub struct PlanetVisualPlugin;

impl Plugin for PlanetVisualPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlanetRadii>()
            .add_observer(on_model_failed)
            .add_systems(Update, update_radii);
    }
}

/// Shared meshes and materials used by all planets.
#[derive(Resource)]
pub struct PlanetVisualAssets {
    sphere: Handle<Mesh>,
    fallback: Handle<StandardMaterial>,
}

impl PlanetVisualAssets {
    /// Creates the shared assets.
    pub fn new(meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) -> Self {
        Self {
            sphere: meshes.add(
                SphereMeshBuilder::new(1.0, SphereKind::Uv { sectors: 48, stacks: 24 }).build(),
            ),
            fallback: materials.add(StandardMaterial {
                base_color: Color::srgb(0.5, 0.5, 0.55),
                perceptual_roughness: 0.8,
                ..default()
            }),
        }
    }
}

/// Spawns the visual of planet `index` at `position` (world space).
///
/// `radius` is the radius estimate for placeholders; `use_models` selects the original `.alo`
/// model over the placeholder sphere.
#[allow(clippy::too_many_arguments)]
pub fn spawn_planet_visual(
    commands: &mut Commands,
    assets: &PlanetVisualAssets,
    materials: &mut Assets<StandardMaterial>,
    view: &GalaxyView,
    index: usize,
    position: Vec3,
    radius: f32,
    use_models: bool,
) -> Entity {
    let planet = &view.planets[index];
    let owner = planet.owner.and_then(|o| view.factions.get(o));
    let owner_c = rgb(owner.map_or(NEUTRAL, |f| f.color));

    let root = commands
        .spawn((
            Transform::from_translation(position),
            Visibility::default(),
            PlanetVisual(index),
        ))
        .id();

    match (&planet.model, use_models) {
        (Some(model), true) => {
            commands.spawn((
                AlamoModel::new(model.clone()).with_team_color(owner_c),
                Transform::from_scale(Vec3::splat(planet.model_scale * PLANET_VISUAL_BOOST * sector_boost(model))),
                PlanetModel(index),
                ChildOf(root),
            ));
        }
        _ => {
            // Placeholder sphere tinted by surface and owner colour.
            let owner_l = owner_c.to_linear();
            let s = planet.surface_color;
            let tint = if planet.owner.is_some() { 0.28 } else { 0.0 };
            let base = Color::linear_rgb(
                s[0].powf(2.2) * (1.0 - tint) + owner_l.red * tint,
                s[1].powf(2.2) * (1.0 - tint) + owner_l.green * tint,
                s[2].powf(2.2) * (1.0 - tint) + owner_l.blue * tint,
            );
            let material = materials.add(StandardMaterial {
                base_color: base,
                perceptual_roughness: 0.75,
                ..default()
            });
            commands.spawn((
                Mesh3d(assets.sphere.clone()),
                MeshMaterial3d(material),
                Transform::from_scale(Vec3::splat(radius)),
                ChildOf(root),
            ));
        }
    }
    root
}

/// Records the real radius of planets whose model just finished loading.
fn update_radii(
    mut radii: ResMut<PlanetRadii>,
    loaded: Query<(&PlanetModel, &AlamoModelLoaded, &Transform), Added<AlamoModelLoaded>>,
) {
    for (planet, model, tf) in &loaded {
        if let Some(r) = radii.0.get_mut(planet.0) {
            let model_r = model.radius * tf.scale.x;
            // Planet models should have a physical node radius around 2.0 .. 5.0.
            // Giant decorative meshes (like nebulae or cosmic clouds) must not inflate the planet's pick/territory radius.
            *r = model_r.clamp(1.0, 5.0);
            debug!("planet {}: model radius {:.1} x scale {:.2} = {:.1} (clamped: {:.1})", planet.0, model.radius, tf.scale.x, model_r, *r);
        }
    }
}

/// A planet model that cannot be loaded is replaced by a grey sphere so the planet stays visible.
fn on_model_failed(
    ev: On<ModelLoadFailed>,
    models: Query<(&PlanetModel, &Transform)>,
    radii: Res<PlanetRadii>,
    assets: Res<PlanetVisualAssets>,
    mut commands: Commands,
) {
    warn!("planet model failed to load: {}", ev.error);
    if let Ok((planet, tf)) = models.get(ev.entity) {
        let r = radii.0.get(planet.0).copied().unwrap_or(10.0) / tf.scale.x.max(1e-4);
        commands.spawn((
            Mesh3d(assets.sphere.clone()),
            MeshMaterial3d(assets.fallback.clone()),
            Transform::from_scale(Vec3::splat(r)),
            ChildOf(ev.entity),
        ));
    }
}

/// Planet globe scale on top of the model scale. 1.7 made the globes far too big next to labels, ring and lanes; the Remake frames
/// (yt CS9VdxglewY f0018/f0019: globe about 14 px wide on a 1024 px frame, label caps about 5 px) put the globe at roughly
/// two to three label cap heights. 1.15 is measured from those frames (an estimate, the engine value is unknown).
const PLANET_VISUAL_BOOST: f32 = 1.15;

/// Extra scale of the sector clouds (`w_galaxy_mini`, `w_nebula*`): next to the Remake frames (CS9 f0016) ours were about 1.5 times too
/// small compared with the planet globes next to them (an estimate measured from those frames).
const SECTOR_VISUAL_BOOST: f32 = 1.5;

fn sector_boost(model: &str) -> f32 {
    let m = model.to_ascii_lowercase();
    if m.starts_with("w_galaxy_mini") || m.starts_with("w_nebula") { SECTOR_VISUAL_BOOST } else { 1.0 }
}

#[cfg(test)]
mod sector_tests {
    use super::*;

    #[test]
    fn only_the_sector_clouds_grow() {
        assert_eq!(sector_boost("W_Galaxy_Mini.alo"), SECTOR_VISUAL_BOOST);
        assert_eq!(sector_boost("w_nebula04_Blue.alo"), SECTOR_VISUAL_BOOST);
        assert_eq!(sector_boost("W_Planet_Terran.alo"), 1.0);
    }
}
