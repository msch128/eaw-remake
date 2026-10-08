// Particle quads: sprite atlas texture times vertex color.
//
// Textures are plain unorm data and shading happens in gamma space like in the original engine;
// the result is converted to linear at the end (see alamo.wgsl). `AlphaMode::Add` is
// premultiplied blending, so additive batches output alpha 0.

#import bevy_pbr::forward_io::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> additive: u32;

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, in.uv);
#ifdef VERTEX_COLORS
    let c = in.color;
#else
    let c = vec4<f32>(1.0);
#endif
    let rgb = t.rgb * c.rgb;
    if additive == 1u {
        return vec4<f32>(to_linear(rgb * c.a * t.a), 0.0);
    }
    if additive == 2u {
        return vec4<f32>(to_linear(rgb * t.a), 0.0);
    }
    return vec4<f32>(to_linear(rgb), t.a * c.a);
}
