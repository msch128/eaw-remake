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
