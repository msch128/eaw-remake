// Fragment shader emulating the Empire at War mesh shaders (Mesh*/RSkin*/Planet .fx family).
//
// The original engine shades in gamma space: textures are uploaded as plain unorm data, all math
// below works on the stored values and only the final color is converted to linear
// (`pow(c, 2.2)`) so that the sRGB framebuffer encode gives the intended gamma-space result.
//
// The sun is the first Bevy `DirectionalLight`; its color is multiplied by `view.exposure` so that
// `AlamoLighting`/`spawn_sun` can use plain 0..1 intensities regardless of camera exposure.

#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings::{view, lights, globals},
    shadows,
}

const KIND_LIT: u32 = 0u;
const KIND_PLANET: u32 = 1u;
const KIND_ADDITIVE: u32 = 2u;
const KIND_ALPHA: u32 = 3u;
const KIND_SHIELD: u32 = 4u;
const KIND_SOLID: u32 = 5u;

const FLAG_NORMAL_MAP: u32 = 1u;
const FLAG_COLORIZE: u32 = 2u;
const FLAG_SPEC_FROM_BASE_ALPHA: u32 = 4u;
const FLAG_VERTEX_COLOR: u32 = 8u;
const FLAG_ALPHA_FROM_BASE: u32 = 16u;
const FLAG_FLIP_GREEN: u32 = 32u;
const FLAG_SHIELD_HITS: u32 = 64u;
const FLAG_SHADOWS: u32 = 128u;

struct AlamoParams {
    diffuse: vec4<f32>,
    emissive: vec4<f32>,
    specular: vec4<f32>,
    colorization: vec4<f32>,
    atmosphere: vec4<f32>,
    city_color: vec4<f32>,
    // xy: constant UV offset, zw: UV scroll rate per second
    uv_offset: vec4<f32>,
    kind: u32,
    flags: u32,
    shininess: f32,
    atmosphere_power: f32,
    cloud_scroll_rate: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
    hit_params: vec4<f32>,
    hits: array<vec4<f32>, 8>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> mat: AlamoParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var normal_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var normal_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var cloud_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var cloud_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var cloud_normal_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var cloud_normal_samp: sampler;

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
}

fn has(flag: u32) -> bool {
    return (mat.flags & flag) != 0u;
}

// Tangent-space normal from a normal map sample; `tbn` columns are tangent, bitangent, normal.
fn perturb(tbn: mat3x3<f32>, sample_rgb: vec3<f32>) -> vec3<f32> {
    let t = sample_rgb * 2.0 - 1.0;
    let g = select(t.y, -t.y, has(FLAG_FLIP_GREEN));
    return normalize(tbn * vec3<f32>(t.x, g, t.z));
}

fn sun_dir() -> vec3<f32> {
    if lights.n_directional_lights > 0u {
        return normalize(lights.directional_lights[0].direction_to_light);
    }
    return normalize(vec3<f32>(0.4, 0.8, 0.4));
}

fn sun_color() -> vec3<f32> {
    if lights.n_directional_lights > 0u {
        return lights.directional_lights[0].color.rgb * view.exposure;
    }
    return vec3<f32>(1.0);
}

fn ambient_color() -> vec3<f32> {
    return lights.ambient_color.rgb * view.exposure;
}

// Diffuse contribution of the fill lights: the directional lights after the sun (the map's second and third
// light). They cast no shadows and have no specular.
fn fill_light(n: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    let count = min(lights.n_directional_lights, 3u);
    for (var i = 1u; i < count; i = i + 1u) {
        let l = lights.directional_lights[i];
        sum += l.color.rgb * view.exposure * saturate(dot(n, normalize(l.direction_to_light)));
    }
    return sum;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let uv = in.uv + mat.uv_offset.xy + mat.uv_offset.zw * globals.time;
    let base = textureSample(base_tex, base_samp, uv);
    let V = normalize(view.world_position - in.world_position.xyz);
    var N = normalize(in.world_normal);
    if !front {
        N = -N;
    }
#ifdef VERTEX_TANGENTS
    let T = normalize(in.world_tangent.xyz - N * dot(in.world_tangent.xyz, N));
    let B = cross(N, T) * in.world_tangent.w;
    let tbn = mat3x3<f32>(T, B, N);
#else
    let tbn = mat3x3<f32>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), N);
#endif
    let L = sun_dir();
    let sun = sun_color();
    let amb = ambient_color();

    var vcol = vec4<f32>(1.0);
#ifdef VERTEX_COLORS
    if has(FLAG_VERTEX_COLOR) {
        vcol = in.color;
    }
#endif

    var rgb = vec3<f32>(0.0);
    var alpha = 1.0;

    if mat.kind == KIND_ADDITIVE {
        // Bevy's `AlphaMode::Add` is premultiplied blending: alpha 0 means pure addition.
        rgb = base.rgb * mat.diffuse.rgb * vcol.rgb;
        return vec4<f32>(to_linear(rgb), 0.0);
    }

    if mat.kind == KIND_SOLID {
        return vec4<f32>(to_linear(mat.diffuse.rgb), mat.diffuse.a);
    }

    if mat.kind == KIND_SHIELD {
        let rim = pow(1.0 - saturate(abs(dot(N, V))), 2.0);
        if has(FLAG_SHIELD_HITS) {
            // Visible only where the bubble was hit: a glow around each hit point that fades out.
            let radius = max(mat.hit_params.x, 1.0);
            let duration = max(mat.hit_params.y, 0.05);
            var glow = 0.0;
            for (var i = 0; i < 8; i = i + 1) {
                let h = mat.hits[i];
                let age = globals.time - h.w;
                if age >= 0.0 && age < duration {
                    let d = distance(in.world_position.xyz, h.xyz) / radius;
                    let fade = 1.0 - age / duration;
                    glow += exp(-d * d * 2.0) * fade * fade;
                }
            }
            let scale = max(mat.atmosphere.y, 1.0);
            let tex = textureSample(base_tex, base_samp, in.uv * scale + vec2<f32>(globals.time * 0.1, 0.0)).rgb;
            let hex = mix(vec3<f32>(1.0), tex, 0.8);
            let c = mat.diffuse.rgb * hex * glow * mat.hit_params.z * (0.6 + 0.8 * rim);
            return vec4<f32>(to_linear(c), 0.0);
        }
        rgb = base.rgb * mat.diffuse.rgb * (0.15 + rim);
        return vec4<f32>(to_linear(rgb), 0.0);
    }

    if mat.kind == KIND_PLANET {
        var n = N;
        var city_mask = 0.0;
        var spec_mask = base.a;
        if has(FLAG_NORMAL_MAP) {
            let nm = textureSample(normal_tex, normal_samp, uv);
            n = perturb(tbn, nm.rgb);
            city_mask = nm.a;
        }
        // Clouds scroll horizontally over the surface.
        let cuv = in.uv * vec2<f32>(1.0, 1.0) + vec2<f32>(globals.time * mat.cloud_scroll_rate, 0.0);
        let cloud = textureSample(cloud_tex, cloud_samp, cuv);
        let cn = textureSample(cloud_normal_tex, cloud_normal_samp, cuv);
        let cloud_amount = saturate(cloud.a * select(cloud.r, 1.0, false));
        let ncloud = perturb(tbn, cn.rgb);

        let wrap = 0.0;
        let ndl = saturate(dot(n, L));
        let cdl = saturate(dot(ncloud, L));
        let surf = base.rgb * mat.diffuse.rgb * (amb + sun * ndl + fill_light(n));
        let H = normalize(L + V);
        let spec = mat.specular.rgb * spec_mask * pow(saturate(dot(n, H)), max(mat.shininess, 8.0)) * sun * step(0.0, dot(N, L));
        var col = surf + spec;
        // Cloud layer, shaded with its own normal map.
        let cloud_lit = vec3<f32>(1.0) * (amb + sun * cdl);
        col = mix(col, cloud_lit, cloud_amount * 0.9);
        // City lights on the night side.
        let night = smoothstep(0.08, -0.12, dot(N, L));
        col = col + mat.city_color.rgb * city_mask * night * (1.0 - cloud_amount * 0.8);
        // Atmosphere: lit rim.
        let ndv = saturate(dot(N, V));
        let rim = pow(1.0 - ndv, max(mat.atmosphere_power, 0.5));
        let day = smoothstep(-0.25, 0.4, dot(N, L));
        col = col + mat.atmosphere.rgb * rim * day * mat.atmosphere.a;
        col = col + mat.emissive.rgb;
        return vec4<f32>(to_linear(col), 1.0);
    }

    // KIND_LIT / KIND_ALPHA: bump or plain mapping, diffuse + specular from the sun.
    var n = N;
    var spec_mask = base.a;
    if has(FLAG_NORMAL_MAP) {
        let nm = textureSample(normal_tex, normal_samp, uv);
        n = perturb(tbn, nm.rgb);
        if !has(FLAG_SPEC_FROM_BASE_ALPHA) {
            spec_mask = nm.a;
        }
    }
    var albedo = base.rgb * mat.diffuse.rgb * vcol.rgb;
    if has(FLAG_COLORIZE) {
        albedo = mix(albedo, albedo * mat.colorization.rgb, base.a);
        if !has(FLAG_NORMAL_MAP) {
            spec_mask = 0.0;
        }
    }
    var shadow = 1.0;
    if has(FLAG_SHADOWS) && lights.n_directional_lights > 0u {
        let view_z = dot(vec4<f32>(
            view.view_from_world[0].z,
            view.view_from_world[1].z,
            view.view_from_world[2].z,
            view.view_from_world[3].z,
        ), in.world_position);
        shadow = shadows::fetch_directional_shadow(0u, in.world_position, N, view_z, in.position.xy);
    }
    let ndl = saturate(dot(n, L)) * shadow;
    let H = normalize(L + V);
    let spec = mat.specular.rgb * spec_mask * pow(saturate(dot(n, H)), max(mat.shininess, 1.0)) * sun * step(0.0, dot(N, L)) * shadow;
    // Emissive is the self-illumination of the base texture.
    rgb = albedo * (amb + sun * ndl + fill_light(n)) + spec + mat.emissive.rgb * base.rgb;
    if mat.kind == KIND_ALPHA {
        alpha = mat.diffuse.a;
        if has(FLAG_ALPHA_FROM_BASE) {
            alpha = alpha * base.a;
        }
        if mat.diffuse.a == 0.0 {
            alpha = base.a;
        }
    }
    return vec4<f32>(to_linear(rgb), alpha);
}

