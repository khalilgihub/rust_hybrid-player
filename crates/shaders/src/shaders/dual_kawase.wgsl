struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_kawase(@builtin(vertex_index) vid: u32) -> VertexOut {
    let x = f32((vid << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vid & 2u) * 2.0 - 1.0;
    var out: VertexOut;
    out.pos = vec4<f32>(x, -y, 0.0, 1.0);
    out.uv = vec2<f32>(x * 0.5 + 0.5, y * 0.5 + 0.5);
    return out;
}

struct KawaseUniforms {
    // xy = 1.0 / src_resolution, z = offset_scale, w = unused
    texel_offset: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u_kawase: KawaseUniforms;
@group(0) @binding(1)
var t_src: texture_2d<f32>;
@group(0) @binding(2)
var s_linear: sampler;

@fragment
fn fs_downsample(in: VertexOut) -> @location(0) vec4<f32> {
    let hp = u_kawase.texel_offset.xy * u_kawase.texel_offset.z;
    var sum = textureSampleLevel(t_src, s_linear, in.uv, 0.0) * 4.0;
    sum += textureSampleLevel(t_src, s_linear, clamp(in.uv - hp, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
    sum += textureSampleLevel(t_src, s_linear, clamp(in.uv + hp, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
    sum += textureSampleLevel(t_src, s_linear, clamp(in.uv + vec2<f32>(hp.x, -hp.y), vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
    sum += textureSampleLevel(t_src, s_linear, clamp(in.uv - vec2<f32>(hp.x, -hp.y), vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
    return sum * 0.125;
}

@fragment
fn fs_upsample(in: VertexOut) -> @location(0) vec4<f32> {
    let hp = u_kawase.texel_offset.xy * u_kawase.texel_offset.z;
    var sum = textureSampleLevel(t_src, s_linear, clamp(in.uv + vec2<f32>(-hp.x * 2.0, 0.0), vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
    sum += textureSampleLevel(t_src, s_linear, clamp(in.uv + vec2<f32>(-hp.x, hp.y), vec2<f32>(0.0), vec2<f32>(1.0)), 0.0) * 2.0;
    sum += textureSampleLevel(t_src, s_linear, clamp(in.uv + vec2<f32>(0.0, hp.y * 2.0), vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
    sum += textureSampleLevel(t_src, s_linear, clamp(in.uv + vec2<f32>(hp.x, hp.y), vec2<f32>(0.0), vec2<f32>(1.0)), 0.0) * 2.0;
    sum += textureSampleLevel(t_src, s_linear, clamp(in.uv + vec2<f32>(hp.x * 2.0, 0.0), vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
    sum += textureSampleLevel(t_src, s_linear, clamp(in.uv + vec2<f32>(hp.x, -hp.y), vec2<f32>(0.0), vec2<f32>(1.0)), 0.0) * 2.0;
    sum += textureSampleLevel(t_src, s_linear, clamp(in.uv + vec2<f32>(0.0, -hp.y * 2.0), vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
    sum += textureSampleLevel(t_src, s_linear, clamp(in.uv + vec2<f32>(-hp.x, -hp.y), vec2<f32>(0.0), vec2<f32>(1.0)), 0.0) * 2.0;
    return sum / 12.0;
}
