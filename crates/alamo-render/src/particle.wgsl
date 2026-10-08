// Particle quads: sprite atlas texture times vertex color.
//
// Textures are plain unorm data and shading happens in gamma space like in the original engine;
// the result is converted to linear at the end (see alamo.wgsl). `AlphaMode::Add` is
// premultiplied blending, so additive batches output alpha 0.

#import bevy_pbr::{forward_io::VertexOutput, mesh_bindings::mesh, mesh_view_bindings::view}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> additive: u32;

// Vertex stage: the batch geometry lives in two storage buffers that are rewritten in place every
// frame (no mesh asset churn). The draw mesh only provides `vertex_index`; indices at or beyond the
// live count produce degenerate (clipped) triangles.
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var<storage, read> quad_indices: array<u32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var<storage, read> quad_verts: array<vec4<f32>>;

struct ParticleVertexInput {
    @builtin(instance_index) instance_index: u32,
    @builtin(vertex_index) vi: u32,
}

@vertex
fn vertex(in: ParticleVertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.world_normal = vec3<f32>(0.0, 0.0, 1.0);
#ifdef VERTEX_UVS_A
    out.uv = vec2<f32>(0.0);
#endif
#ifdef VERTEX_COLORS
    out.color = vec4<f32>(0.0);
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = 0u;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = 0;
#endif
    // `vertex_index` of a non-indexed draw includes the first vertex of the mesh inside the shared
    // mesh slab; subtract it to get the position inside the dummy draw mesh.
    let local_vi = in.vi - mesh[in.instance_index].first_vertex_index;
    if local_vi >= quad_indices[0] {
        out.position = vec4<f32>(0.0);
        out.world_position = vec4<f32>(0.0);
        return out;
    }
    let ix = quad_indices[1u + local_vi];
    let p = quad_verts[ix * 3u];
    let t = quad_verts[ix * 3u + 1u];
    let c = quad_verts[ix * 3u + 2u];
    out.world_position = vec4<f32>(p.xyz, 1.0);
    out.position = view.clip_from_world * out.world_position;
#ifdef VERTEX_UVS_A
    out.uv = t.xy;
#endif
#ifdef VERTEX_COLORS
    out.color = c;
#endif
    return out;
}


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
