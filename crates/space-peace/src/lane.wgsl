// Galaxy lane ribbons, added in display (sRGB) values like the original engine, which blends in gamma space. Bevy blends in linear
// light, where a lane over the pink core nebula all but vanishes. The lane reads what lies under it (Bevy's view transmission
// texture: the opaque pass plus the galaxy disc art, which draws first with `FLAG_UNDERLAY`) and writes linear(srgb(under) + lane).

#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings::{view_transmission_texture, view_transmission_sampler},
}

struct LaneParams {
    // xy: UV offset (the glints move along v).
    scroll: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> lane: LaneParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var lane_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var lane_samp: sampler;

// The exact sRGB curves, the same the framebuffer encode uses, so a lane over black shows its vertex colour on screen.
fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3<f32>(0.0031308));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return select(pow((x + 0.055) / 1.055, vec3<f32>(2.4)), x / 12.92, x <= vec3<f32>(0.04045));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Band and glint profile (`lane_texture`) times the vertex colour: what the lane adds, in display values.
    let share = textureSample(lane_tex, lane_samp, in.uv + lane.scroll.xy).a;
    var amount = vec3<f32>(0.0);
#ifdef VERTEX_COLORS
    amount = in.color.rgb * share;
#endif
    let uv = in.position.xy / vec2<f32>(textureDimensions(view_transmission_texture));
    let under = textureSampleLevel(view_transmission_texture, view_transmission_sampler, uv, 0.0).rgb;
    return vec4<f32>(to_linear(to_srgb(under) + amount), 1.0);
}
