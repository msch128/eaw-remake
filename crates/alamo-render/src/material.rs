//! The single custom material that emulates the Empire at War mesh shaders.
//!
//! Every EaW shader this crate knows is mapped to a [`ShaderKind`]; the shader flavour and its
//! parameters are plain uniform data, so all models share one pipeline per blend mode.

use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState,
    RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

/// Path of the embedded WGSL source.
pub const SHADER_PATH: &str = "embedded://alamo_render/alamo.wgsl";

/// Rendering flavour of a material; see `alamo.wgsl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ShaderKind {
    /// Opaque lit mesh (`MeshGloss`, `MeshBumpColorize`, ...).
    Lit = 0,
    /// Galactic planet (`Planet.fx`).
    Planet = 1,
    /// Additive glow (`MeshAdditive`).
    Additive = 2,
    /// Lit mesh with alpha blending (`MeshAlpha`).
    Alpha = 3,
    /// Shield bubble (`MeshShield`).
    Shield = 4,
    /// Flat color.
    Solid = 5,
}

/// Normal map present.
pub const FLAG_NORMAL_MAP: u32 = 1;
/// `BaseTexture` alpha masks where the team color applies.
pub const FLAG_COLORIZE: u32 = 2;
/// Specular mask comes from the base texture alpha (otherwise from the normal map alpha).
pub const FLAG_SPEC_FROM_BASE_ALPHA: u32 = 4;
/// Multiply by vertex color.
pub const FLAG_VERTEX_COLOR: u32 = 8;
/// Alpha is taken from the base texture.
pub const FLAG_ALPHA_FROM_BASE: u32 = 16;
/// Invert the green channel of normal maps.
pub const FLAG_FLIP_GREEN: u32 = 32;
/// Receive directional shadow maps.
pub const FLAG_SHADOWS: u32 = 128;
/// Shield material that is only visible where it was hit (see `hits`).
pub const FLAG_SHIELD_HITS: u32 = 64;
/// Sky model: drawn at the far plane depth, behind everything, whatever its radius.
pub const FLAG_BACKGROUND: u32 = 256;
/// Additive glow that belongs to the backdrop (the galaxy disc art): drawn in the alpha-mask pass,
/// after the opaque surfaces and before the transmissive ones, still adding and without depth
/// writes. Transmissive materials then find it in Bevy's view transmission texture (the galaxy
/// lanes blend over it in gamma space). Only for [`ShaderKind::Additive`].
pub const FLAG_UNDERLAY: u32 = 512;

/// Uniform block of [`AlamoMaterial`]; mirrors `AlamoParams` in the shader.
#[derive(Debug, Clone, Copy, ShaderType)]
pub struct AlamoParams {
    /// `Diffuse` (or `Color` for additive shaders).
    pub diffuse: Vec4,
    /// `Emissive`.
    pub emissive: Vec4,
    /// `Specular`.
    pub specular: Vec4,
    /// Team color for `*Colorize` shaders.
    pub colorization: Vec4,
    /// Planet atmosphere color; `w` scales the effect.
    pub atmosphere: Vec4,
    /// Planet city light color.
    pub city_color: Vec4,
    /// `xy`: constant UV offset, `zw`: UV scroll per second.
    pub uv_offset: Vec4,
    /// [`ShaderKind`] as integer.
    pub kind: u32,
    /// `FLAG_*` bits.
    pub flags: u32,
    /// Specular exponent.
    pub shininess: f32,
    /// Atmosphere rim exponent.
    pub atmosphere_power: f32,
    /// Cloud scroll rate in UV units per second.
    pub cloud_scroll_rate: f32,
    /// Planet: local radius of a flat backdrop disc, 0 for real spheres (mirrors the shader).
    pub disc_radius: f32,
    /// Padding.
    pub pad1: f32,
    /// Padding.
    pub pad2: f32,
    /// Shield hit flash: `x` radius, `y` duration (s), `z` intensity.
    pub hit_params: Vec4,
    /// Shield hits: `xyz` world position, `w` start time (`globals.time`), very negative if unused.
    pub hits: [Vec4; 8],
}

impl Default for AlamoParams {
    fn default() -> Self {
        Self {
            diffuse: Vec4::ONE,
            emissive: Vec4::ZERO,
            specular: Vec4::ZERO,
            colorization: Vec4::ONE,
            atmosphere: Vec4::ZERO,
            city_color: Vec4::ZERO,
            uv_offset: Vec4::ZERO,
            kind: ShaderKind::Lit as u32,
            flags: 0,
            shininess: 32.0,
            atmosphere_power: 4.0,
            cloud_scroll_rate: 0.0,
            disc_radius: 0.0,
            pad1: 0.0,
            pad2: 0.0,
            hit_params: Vec4::new(40.0, 1.0, 1.0, 0.0),
            hits: [Vec4::new(0.0, 0.0, 0.0, -1.0e6); 8],
        }
    }
}

/// Pipeline specialization data derived from the material.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AlamoKey {
    kind: ShaderKind,
    background: bool,
    underlay: bool,
}

impl From<&AlamoMaterial> for AlamoKey {
    fn from(m: &AlamoMaterial) -> Self {
        AlamoKey {
            kind: m.shader_kind(),
            background: m.params.flags & FLAG_BACKGROUND != 0,
            underlay: m.is_underlay(),
        }
    }
}

/// Material emulating EaW's mesh shaders.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(AlamoKey)]
pub struct AlamoMaterial {
    /// Uniform parameters.
    #[uniform(0)]
    pub params: AlamoParams,
    /// `BaseTexture`.
    #[texture(1)]
    #[sampler(2)]
    pub base: Option<Handle<Image>>,
    /// `NormalTexture`.
    #[texture(3)]
    #[sampler(4)]
    pub normal: Option<Handle<Image>>,
    /// `CloudTexture` (planets).
    #[texture(5)]
    #[sampler(6)]
    pub cloud: Option<Handle<Image>>,
    /// `CloudNormalTexture` (planets).
    #[texture(7)]
    #[sampler(8)]
    pub cloud_normal: Option<Handle<Image>>,
}

impl AlamoMaterial {
    /// The shader flavour.
    pub fn shader_kind(&self) -> ShaderKind {
        match self.params.kind {
            1 => ShaderKind::Planet,
            2 => ShaderKind::Additive,
            3 => ShaderKind::Alpha,
            4 => ShaderKind::Shield,
            5 => ShaderKind::Solid,
            _ => ShaderKind::Lit,
        }
    }

    /// An additive glow drawn with the backdrop ([`FLAG_UNDERLAY`]).
    pub fn is_underlay(&self) -> bool {
        self.params.flags & FLAG_UNDERLAY != 0 && self.shader_kind() == ShaderKind::Additive
    }
}

impl Material for AlamoMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        match self.shader_kind() {
            // The mask phase comes right after the opaque one; `specialize` turns its blend back
            // into an addition.
            ShaderKind::Additive if self.is_underlay() => AlphaMode::Mask(0.0),
            ShaderKind::Additive | ShaderKind::Shield => AlphaMode::Add,
            ShaderKind::Alpha => AlphaMode::Blend,
            _ => AlphaMode::Opaque,
        }
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Glows, grids and shields are two-sided.
        if key.bind_group_data.kind != ShaderKind::Lit
            && key.bind_group_data.kind != ShaderKind::Planet
        {
            descriptor.primitive.cull_mode = None;
        }
        if key.bind_group_data.background {
            if let Some(f) = descriptor.fragment.as_mut() {
                f.shader_defs.push("ALAMO_BACKGROUND".into());
            }
            if let Some(d) = descriptor.depth_stencil.as_mut() {
                d.depth_write_enabled = Some(false);
            }
        }
        if key.bind_group_data.underlay {
            // Same result as `AlphaMode::Add` (the shader writes alpha 0): colour added,
            // destination alpha kept.
            let target = descriptor
                .fragment
                .as_mut()
                .and_then(|f| f.targets.first_mut())
                .and_then(Option::as_mut);
            if let Some(target) = target {
                let add = |src_factor| BlendComponent {
                    src_factor,
                    dst_factor: BlendFactor::One,
                    operation: BlendOperation::Add,
                };
                target.blend = Some(BlendState {
                    color: add(BlendFactor::One),
                    alpha: add(BlendFactor::Zero),
                });
            }
            if let Some(d) = descriptor.depth_stencil.as_mut() {
                d.depth_write_enabled = Some(false);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn material(kind: ShaderKind, flags: u32) -> AlamoMaterial {
        AlamoMaterial {
            params: AlamoParams { kind: kind as u32, flags, ..default() },
            base: None,
            normal: None,
            cloud: None,
            cloud_normal: None,
        }
    }

    #[test]
    fn an_underlay_glow_draws_in_the_mask_phase_and_nothing_else_moves() {
        // Bevy draws the alpha-mask phase after the opaque one and before the transmissive one.
        let glow = material(ShaderKind::Additive, FLAG_UNDERLAY);
        assert!(glow.is_underlay() && matches!(glow.alpha_mode(), AlphaMode::Mask(_)));
        assert!(matches!(material(ShaderKind::Additive, 0).alpha_mode(), AlphaMode::Add));
        // The flag means nothing on other kinds.
        let lit = material(ShaderKind::Lit, FLAG_UNDERLAY);
        assert!(!lit.is_underlay() && matches!(lit.alpha_mode(), AlphaMode::Opaque));
        assert!(matches!(material(ShaderKind::Shield, FLAG_UNDERLAY).alpha_mode(), AlphaMode::Add));
    }
}
