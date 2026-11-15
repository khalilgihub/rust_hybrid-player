struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vid: u32) -> VertexOut {
    // Fullscreen triangle covering [-1, 1] NDC with top-left UV (0, 0) -> bottom-right (1, 1)
    let x = f32((vid << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vid & 2u) * 2.0 - 1.0;
    var out: VertexOut;
    out.pos = vec4<f32>(x, -y, 0.0, 1.0);
    out.uv = vec2<f32>(x * 0.5 + 0.5, y * 0.5 + 0.5);
    return out;
}

struct SceneBlitUniforms {
    // xy = scale in NDC/UV (inverse aspect-fit scale), zw = offset
    uv_rect: vec4<f32>,
    // x = opacity, y = has_texture (1.0 or 0.0), zw = unused
    params: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u_blit: SceneBlitUniforms;
@group(0) @binding(1)
var t_scene: texture_2d<f32>;
@group(0) @binding(2)
var s_scene: sampler;

@fragment
fn fs_scene_blit(in: VertexOut) -> @location(0) vec4<f32> {
    if (u_blit.params.y < 0.5) {
        discard;
    }
    // Map viewport UV [0,1] into fitted video rect [0,1]
    let mapped_uv = (in.uv - u_blit.uv_rect.zw) / u_blit.uv_rect.xy;
    if (mapped_uv.x < 0.0 || mapped_uv.x > 1.0 || mapped_uv.y < 0.0 || mapped_uv.y > 1.0) {
        discard;
    }
    let col = textureSampleLevel(t_scene, s_scene, mapped_uv, 0.0);
    return vec4<f32>(col.rgb, col.a * u_blit.params.x);
}

@fragment
fn fs_copy_present(in: VertexOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(t_scene, s_scene, in.uv, 0.0);
}
