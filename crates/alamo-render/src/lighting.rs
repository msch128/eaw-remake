//! Lighting convention: the sun is the first Bevy `DirectionalLight`.

use bevy::camera::Exposure;
use bevy::prelude::*;

/// Marker for the sun entity. The Alamo shaders light everything (including the planet
/// terminator, night-side city lights and the atmosphere rim) from the first `DirectionalLight`
/// in the scene, so spawn exactly one and point it with its transform.
#[derive(Component, Debug, Default)]
pub struct SunLight;

/// Spawns the sun, shining along `direction` (the direction the light travels, like
/// `Transform::looking_to`).
///
/// `intensity` is the unit-less strength the shaders see (1.0 = full white). It is converted
/// to illuminance for the default camera exposure (`Exposure::default()`); cameras with another
/// exposure scale the result accordingly, which the shaders compensate automatically.
pub fn spawn_sun(commands: &mut Commands, direction: Vec3, intensity: f32, color: Color) -> Entity {
    let illuminance = intensity / Exposure::default().exposure();
    commands
        .spawn((
            Name::new("sun"),
            SunLight,
            DirectionalLight {
                color,
                illuminance,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::IDENTITY.looking_to(direction, Vec3::Y),
        ))
        .id()
}

/// Like [`spawn_sun`] but the sun casts cascaded shadow maps over `max_distance` world units
/// (ships shadow each other and themselves). Pair with
/// `AlamoRenderSettings { shadows: true, .. }`; every shadow-casting mesh is drawn once more per
/// cascade, so keep `cascades` small (2 is a good start) in big battles.
pub fn spawn_sun_shadowed(
    commands: &mut Commands,
    direction: Vec3,
    intensity: f32,
    color: Color,
    max_distance: f32,
    cascades: usize,
) -> Entity {
    let illuminance = intensity / Exposure::default().exposure();
    commands
        .spawn((
            Name::new("sun"),
            SunLight,
            DirectionalLight {
                color,
                illuminance,
                shadow_maps_enabled: true,
                ..default()
            },
            bevy::light::CascadeShadowConfigBuilder {
                num_cascades: cascades.max(1),
                minimum_distance: 1.0,
                maximum_distance: max_distance,
                first_cascade_far_bound: max_distance / (cascades.max(1) as f32 * 3.0),
                ..default()
            }
            .build(),
            Transform::IDENTITY.looking_to(direction, Vec3::Y),
        ))
        .id()
}
