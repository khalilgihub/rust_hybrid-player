struct ParticlesUniforms {
    model_matrix: mat4x4<f32>,
    view_matrix: mat4x4<f32>,
    projection_matrix: mat4x4<f32>,
    // x = time, y = spread, z = base_size (already * pixel_ratio), w = size_randomness
    params: vec4<f32>,
    // xy = viewport_size (pixels), z = alpha_particles (0.0 or 1.0), w = unused
    viewport_alpha: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u: ParticlesUniforms;

struct ParticleInput {
    // 6 vertices per billboard quad
    @builtin(vertex_index) vertex_index: u32,
    // Instance attributes
    @location(0) position: vec3<f32>,
    @location(1) random: vec4<f32>,
    @location(2) color: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) point_uv: vec2<f32>,
    @location(1) random: vec4<f32>,
    @location(2) color: vec3<f32>,
};

@vertex
fn vs_particles(in: ParticleInput) -> VertexOut {
    let quad_corners = array<vec2<f32>, 6>(
        vec2<f32>(-0.5, -0.5),
        vec2<f32>( 0.5, -0.5),
        vec2<f32>( 0.5,  0.5),
        vec2<f32>(-0.5, -0.5),
        vec2<f32>( 0.5,  0.5),
        vec2<f32>(-0.5,  0.5)
    );
    let corner = quad_corners[in.vertex_index % 6u];

    var pos = in.position * u.params.y;
    pos.z *= 10.0;

    var m_pos = u.model_matrix * vec4<f32>(pos, 1.0);
    let t = u.params.x;
    m_pos.x += sin(t * in.random.z + 6.28 * in.random.w) * mix(0.1, 1.5, in.random.x);
    m_pos.y += sin(t * in.random.y + 6.28 * in.random.x) * mix(0.1, 1.5, in.random.w);
    m_pos.z += sin(t * in.random.w + 6.28 * in.random.y) * mix(0.1, 1.5, in.random.z);

    let mv_pos = u.view_matrix * m_pos;
    let dist = max(length(mv_pos.xyz), 0.0001);

    var point_size_px: f32;
    if (u.params.w == 0.0) {
        point_size_px = u.params.z;
    } else {
        point_size_px = (u.params.z * (1.0 + u.params.w * (in.random.x - 0.5))) / dist;
    }

    let clip_center = u.projection_matrix * mv_pos;
    let viewport = max(u.viewport_alpha.xy, vec2<f32>(1.0));
    // Convert pixel size to NDC offset (wgpu clip space is [-1, 1])
    let ndc_offset = (corner * point_size_px * 2.0) / viewport;

    var out: VertexOut;
    out.clip_pos = vec4<f32>(
        clip_center.xy + ndc_offset * clip_center.w,
        clip_center.z,
        clip_center.w
    );
    // Match gl_PointCoord (0..1 top-to-bottom)
    out.point_uv = vec2<f32>(corner.x + 0.5, 0.5 - corner.y);
    out.random = in.random;
    out.color = in.color;
    return out;
}

@fragment
fn fs_particles(in: VertexOut) -> @location(0) vec4<f32> {
    let uv = in.point_uv;
    let d = length(uv - vec2<f32>(0.5));
    let col = in.color + 0.2 * sin(uv.yxx + vec3<f32>(u.params.x + in.random.y * 6.28));

    if (u.viewport_alpha.z < 0.5) {
        if (d > 0.5) {
            discard;
        }
        return vec4<f32>(col, 1.0);
    } else {
        let circle = smoothstep(0.5, 0.4, d) * 0.8;
        return vec4<f32>(col, circle);
    }
}
