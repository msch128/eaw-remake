//! Lighting convention: the sun is the first Bevy `DirectionalLight`, the fill lights follow it, the ambient is Bevy's
//! `GlobalAmbientLight`.
//!
//! # Map lighting (the original's mesh shader model)
//!
//! A sun entity with [`MapLighting`] (a space map's scene) switches the lit meshes to the model of the shipped mesh shaders.
//! `Art/Shaders/MeshBumpColorize.fxo` (read with `fx_disasm`, its constant table names the registers) computes in ps_2_0
//!
//! ```text
//! albedo = lerp(base, base * Colorization, base.a)
//! v0     = saturate(Diffuse * m_lightScale * m_sphFill(n) + Emissive)       (vs_1_1 colour output oD0, clamped)
//! rgb    = 2 * albedo * (Diffuse * m_light0Diffuse * m_lightScale * saturate(n.L) + v0)
//!        + pow(saturate(n.H), 16) * Specular * m_light0Specular * NormalTexture.a
//! ```
//!
//! `MeshGloss.fxo` is the same with the sun inside the harmonics (`m_sphAll`) and the doubling as `mul r0.xyz_x2, t0, v0`. No mesh
//! shader reads `m_lightAmbient` (only `Planet.fxo` does), so the map ambient is part of `m_sphFill`. What the engine puts into
//! these constants is not in the shader files; the estimates ([`map_light_color`], [`sh_clamped_cosine`]):
//! * `m_light0Diffuse` and the fill lights are the map light's colour times its intensity (`.ted` lighting mini-chunks 0..2 and
//!   5..7). Maps switch lights off through the intensity alone: Taris keeps white fill colours at intensity 0.
//! * `m_sphFill` is the ambient plus each fill light projected into second-order spherical harmonics (the clamped cosine).
//! * `m_lightScale` is 1 (the effect file's default value `1, 1, 1, 1`).
//! * `m_light0Specular` is the map's sun specular colour (mini-chunk 3), not scaled by the intensity.
//!
//! The renderer folds the doubling into the light colours ("effective" units: 2 x colour x intensity), which the planet shader
//! emulation was tuned with. [`shade_map_lit`] is the same model on the CPU, for the census of the maps' looks.

use bevy::asset::uuid_handle;
use bevy::camera::Exposure;
use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use bevy::render::storage::ShaderBuffer;

/// The storage buffer every `AlamoMaterial` binds for the scene's lighting model.
pub const LIGHTING_BUFFER: Handle<ShaderBuffer> = uuid_handle!("6c1f3f8e-2b9d-4c55-9a0e-6c4d2f1b7a31");

/// Put on the sun of a space map's scene: the lit meshes then follow the original's mesh shader model (module docs) with this sun
/// specular colour. Without one (galaxy map, tools) they keep the galaxy look.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct MapLighting {
    /// The map's sun specular colour (`m_light0Specular`).
    pub sun_specular: Vec3,
}

/// Mirrors `AlamoLighting` in `alamo.wgsl`.
#[derive(Debug, Clone, Copy, PartialEq, ShaderType)]
struct GpuLighting {
    sun_specular: Vec4,
    mode: Vec4,
}

impl GpuLighting {
    fn of(map: Option<&MapLighting>) -> Self {
        match map {
            Some(m) => Self { sun_specular: m.sun_specular.extend(1.0), mode: Vec4::X },
            None => Self { sun_specular: Vec4::ONE, mode: Vec4::ZERO },
        }
    }

    fn buffer(self) -> ShaderBuffer {
        let mut b = ShaderBuffer::default();
        b.set_data(self);
        b
    }
}

pub(crate) fn build(app: &mut App) {
    // Headless apps without the render plugins have no buffers to fill.
    if let Some(mut buffers) = app.world_mut().get_resource_mut::<Assets<ShaderBuffer>>() {
        let _ = buffers.insert(LIGHTING_BUFFER.id(), GpuLighting::of(None).buffer());
    }
    app.add_systems(PostUpdate, sync_lighting);
}

/// Writes the lighting model of the scene's map sun (or the galaxy look when there is none) into [`LIGHTING_BUFFER`].
fn sync_lighting(maps: Query<&MapLighting>, buffers: Option<ResMut<Assets<ShaderBuffer>>>, mut last: Local<Option<GpuLighting>>) {
    let Some(mut buffers) = buffers else { return };
    let now = GpuLighting::of(maps.iter().next());
    if *last != Some(now) {
        let _ = buffers.insert(LIGHTING_BUFFER.id(), now.buffer());
        *last = Some(now);
    }
}

/// A `.ted` light as the renderer takes it: colour times intensity, doubled (the mesh shaders' `2 * albedo * light`).
pub fn map_light_color(color: Vec3, intensity: f32) -> Vec3 {
    color * 2.0 * intensity.max(0.0)
}

/// Second-order spherical harmonics projection of a clamped cosine (`max(c, 0)` for `c = n.l`): `1/4 + c/2 + 5/16 P2(c)`. The fill
/// lights reach the mesh shaders through `m_sphFill`, a second-order harmonics set; this is what one light contributes.
pub fn sh_clamped_cosine(c: f32) -> f32 {
    3.0 / 32.0 + 0.5 * c + 15.0 / 32.0 * c * c
}

/// One map light as the shaders see it: the direction towards the light and its effective colour ([`map_light_color`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectiveLight {
    /// Unit vector towards the light.
    pub to_light: Vec3,
    /// Effective colour.
    pub color: Vec3,
}

/// The diffuse part of a lit mesh under map lighting, in display (gamma) values before the framebuffer clamp: the CPU twin of the
/// `map_lighting()` branch of `alamo.wgsl` without specular and shadows. `surface` is the texture colour, `diffuse` the material's
/// `Diffuse`, `normal` a unit normal, `ambient` the map ambient (not doubled).
pub fn shade_map_lit(surface: Vec3, diffuse: Vec3, normal: Vec3, sun: EffectiveLight, fills: &[EffectiveLight], ambient: Vec3) -> Vec3 {
    let sh = fills.iter().fold(ambient, |s, f| s + 0.5 * f.color * sh_clamped_cosine(normal.dot(f.to_light)));
    let v0 = (diffuse * sh).clamp(Vec3::ZERO, Vec3::ONE);
    (surface * diffuse * sun.color * normal.dot(sun.to_light).max(0.0) + surface * 2.0 * v0).min(Vec3::ONE)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    const GREY: Vec3 = Vec3::splat(0.5);

    fn sun(color: Vec3) -> EffectiveLight {
        EffectiveLight { to_light: Vec3::Y, color }
    }

    #[test]
    fn a_map_light_is_colour_times_intensity_doubled() {
        // The mesh shaders double `albedo * light`: intensity 0.5 lights a surface with its own colour.
        assert_eq!(map_light_color(Vec3::new(1.0, 0.5, 0.25), 0.5), Vec3::new(1.0, 0.5, 0.25));
        assert_eq!(map_light_color(Vec3::ONE, 0.75), Vec3::splat(1.5));
        // Taris keeps white fill colours at intensity 0: the light is off.
        assert_eq!(map_light_color(Vec3::ONE, 0.0), Vec3::ZERO);
    }

    #[test]
    fn a_fill_light_reaches_round_the_hull_as_second_order_harmonics() {
        assert!((sh_clamped_cosine(1.0) - 1.0625).abs() < 1e-6, "facing the light");
        assert!((sh_clamped_cosine(0.0) - 3.0 / 32.0).abs() < 1e-6, "side on");
        assert!((sh_clamped_cosine(-1.0) - 1.0 / 16.0).abs() < 1e-6, "facing away");
    }

    #[test]
    fn the_sun_at_intensity_one_half_lights_a_hull_with_its_own_colour() {
        let lit = shade_map_lit(GREY, Vec3::ONE, Vec3::Y, sun(Vec3::ONE), &[], Vec3::ZERO);
        assert!((lit - GREY).abs().max_element() < 1e-6, "{lit}");
        // Turned away: only the ambient, doubled like every light.
        let dark = shade_map_lit(GREY, Vec3::ONE, Vec3::NEG_Y, sun(Vec3::ONE), &[], Vec3::splat(0.1));
        assert!((dark - Vec3::splat(0.1)).abs().max_element() < 1e-6, "{dark}");
    }

    #[test]
    fn a_tinted_sun_tints_the_hull_and_keeps_its_texture() {
        // Taris: a pink sun, no fill, a red ambient. Two texels of different brightness stay apart in every channel: the map
        // light multiplies the hull colour, it does not replace it with one flat colour.
        let pink = sun(Vec3::new(1.0, 0.5, 0.5));
        let amb = Vec3::new(0.08, 0.0, 0.0);
        let dark = shade_map_lit(Vec3::splat(0.35), Vec3::ONE, Vec3::Y, pink, &[], amb);
        let bright = shade_map_lit(Vec3::splat(0.7), Vec3::ONE, Vec3::Y, pink, &[], amb);
        assert!(bright.cmpgt(dark).all() && bright.x < 1.0, "{dark} {bright}");
        assert!((bright.y / bright.x - 0.5 / 1.16).abs() < 1e-3, "the hue is the light's: {bright}");
    }

    #[test]
    fn the_fill_term_saturates_on_its_own_and_the_sum_at_one() {
        // v0 is a colour register (0..1): a huge ambient adds at most 2 x albedo.
        let v0 = shade_map_lit(Vec3::splat(0.3), Vec3::ONE, Vec3::Y, sun(Vec3::ZERO), &[], Vec3::splat(5.0));
        assert!((v0 - Vec3::splat(0.6)).abs().max_element() < 1e-6, "{v0}");
        let white = shade_map_lit(GREY, Vec3::ONE, Vec3::Y, sun(Vec3::splat(3.0)), &[], Vec3::ZERO);
        assert_eq!(white, Vec3::ONE);
    }

    #[test]
    fn the_shader_has_the_same_model() {
        let wgsl = include_str!("alamo.wgsl");
        for needle in [
            "const MAP_SPECULAR_POWER: f32 = 16.0;",
            "3.0 / 32.0 + 0.5 * c + 15.0 / 32.0 * c * c",
            "0.5 * l.color.rgb * view.exposure",
            "saturate3(mat.diffuse.rgb * sh_fill(n) + mat.emissive.rgb)",
            "min(albedo * sun * ndl + surface * 2.0 * v0, vec3<f32>(1.0))",
            "lighting.sun_specular.rgb",
        ] {
            assert!(wgsl.contains(needle), "alamo.wgsl lost `{needle}`");
        }
    }

    #[test]
    fn a_map_sun_switches_the_shaders_to_the_map_model() {
        assert_eq!(GpuLighting::of(None).mode, Vec4::ZERO);
        let m = GpuLighting::of(Some(&MapLighting { sun_specular: Vec3::new(1.0, 0.5, 0.25) }));
        assert_eq!((m.mode.x, m.sun_specular.truncate()), (1.0, Vec3::new(1.0, 0.5, 0.25)));
    }
}
