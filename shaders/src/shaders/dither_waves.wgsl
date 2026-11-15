struct VertexOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_dither_waves(@builtin(vertex_index) vid: u32) -> VertexOut {
    let x = f32((vid << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vid & 2u) * 2.0 - 1.0;
    var out: VertexOut;
    out.pos = vec4<f32>(x, -y, 0.0, 1.0);
    return out;
}

struct DitherWavesUniforms {
    // xy = effective canvas resolution (pixels), z = aspect, w = time
    res_aspect_time: vec4<f32>,
    // x = wave_speed, y = wave_frequency, z = wave_amplitude, w = pixel_size
    wave_params: vec4<f32>,
    // xyz = wave_color, w = color_num
    wave_color_num: vec4<f32>,
    // xyz = background_color, w = enable_mouse_interaction (0.0 or 1.0)
    bg_color_mouse: vec4<f32>,
    // xy = mouse_pos (effective pixels, top-left origin), z = mouse_radius, w = mouse_intensity
    mouse_params: vec4<f32>,
    // xy = physical target resolution (pixels), z = eff_dpr, w = inst_dpr
    raster_params: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u: DitherWavesUniforms;

const BAYER_MATRIX_8X8: array<f32, 64> = array<f32, 64>(
     0.0/64.0, 48.0/64.0, 12.0/64.0, 60.0/64.0,  3.0/64.0, 51.0/64.0, 15.0/64.0, 63.0/64.0,
    32.0/64.0, 16.0/64.0, 44.0/64.0, 28.0/64.0, 35.0/64.0, 19.0/64.0, 47.0/64.0, 31.0/64.0,
     8.0/64.0, 56.0/64.0,  4.0/64.0, 52.0/64.0, 11.0/64.0, 59.0/64.0,  7.0/64.0, 55.0/64.0,
    40.0/64.0, 24.0/64.0, 36.0/64.0, 20.0/64.0, 43.0/64.0, 27.0/64.0, 39.0/64.0, 23.0/64.0,
     2.0/64.0, 50.0/64.0, 14.0/64.0, 62.0/64.0,  1.0/64.0, 49.0/64.0, 13.0/64.0, 61.0/64.0,
    34.0/64.0, 18.0/64.0, 46.0/64.0, 30.0/64.0, 33.0/64.0, 17.0/64.0, 45.0/64.0, 29.0/64.0,
    10.0/64.0, 58.0/64.0,  6.0/64.0, 54.0/64.0,  9.0/64.0, 57.0/64.0,  5.0/64.0, 53.0/64.0,
    42.0/64.0, 26.0/64.0, 38.0/64.0, 22.0/64.0, 41.0/64.0, 25.0/64.0, 37.0/64.0, 21.0/64.0
);

fn glsl_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

fn dither(coord: vec2<f32>, color_in: vec3<f32>) -> vec3<f32> {
    var bayer = BAYER_MATRIX_8X8;
    let x = i32(glsl_mod(coord.x, 8.0));
    let y = i32(glsl_mod(coord.y, 8.0));
    let threshold = bayer[y * 8 + x] - 0.25;
    let color_num = max(2.0, u.wave_color_num.w);
    let step_val = 1.0 / (color_num - 1.0);
    var color = color_in + threshold * step_val;
    let bias = 0.2;
    color = clamp(color - bias, vec3<f32>(0.0), vec3<f32>(1.0));
    return floor(color * (color_num - 1.0) + 0.5) / (color_num - 1.0);
}

fn mod289(x: vec4<f32>) -> vec4<f32> {
    return x - floor(x * (1.0 / 289.0)) * 289.0;
}

fn permute(x: vec4<f32>) -> vec4<f32> {
    return mod289(((x * 34.0) + 1.0) * x);
}

fn taylor_inv_sqrt(r: vec4<f32>) -> vec4<f32> {
    return 1.79284291400159 - 0.85373472095314 * r;
}

fn fade(t: vec2<f32>) -> vec2<f32> {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

fn cnoise(P: vec2<f32>) -> f32 {
    var Pi = floor(P.xyxy) + vec4<f32>(0.0, 0.0, 1.0, 1.0);
    let Pf = fract(P.xyxy) - vec4<f32>(0.0, 0.0, 1.0, 1.0);
    Pi = mod289(Pi);
    let ix = Pi.xzxz;
    let iy = Pi.yyww;
    let fx = Pf.xzxz;
    let fy = Pf.yyww;
    let i = permute(permute(ix) + iy);
    var gx = fract(i * (1.0 / 41.0)) * 2.0 - 1.0;
    let gy = abs(gx) - 0.5;
    let tx = floor(gx + 0.5);
    gx = gx - tx;
    var g00 = vec2<f32>(gx.x, gy.x);
    var g10 = vec2<f32>(gx.y, gy.y);
    var g01 = vec2<f32>(gx.z, gy.z);
    var g11 = vec2<f32>(gx.w, gy.w);
    let norm = taylor_inv_sqrt(vec4<f32>(dot(g00, g00), dot(g01, g01), dot(g10, g10), dot(g11, g11)));
    g00 *= norm.x;
    g01 *= norm.y;
    g10 *= norm.z;
    g11 *= norm.w;
    let n00 = dot(g00, vec2<f32>(fx.x, fy.x));
    let n10 = dot(g10, vec2<f32>(fx.y, fy.y));
    let n01 = dot(g01, vec2<f32>(fx.z, fy.z));
    let n11 = dot(g11, vec2<f32>(fx.w, fy.w));
    let fade_xy = fade(Pf.xy);
    let n_x = mix(vec2<f32>(n00, n01), vec2<f32>(n10, n11), fade_xy.x);
    return 2.3 * mix(n_x.x, n_x.y, fade_xy.y);
}

fn fbm(p_in: vec2<f32>) -> f32 {
    var p = p_in;
    var value = 0.0;
    var amp = 1.0;
    let freq = u.wave_params.y;
    let wave_amp = u.wave_params.z;
    for (var i = 0; i < 4; i = i + 1) {
        value += amp * abs(cnoise(p));
        p *= freq;
        amp *= wave_amp;
    }
    return value;
}

fn pattern(p: vec2<f32>) -> f32 {
    let p2 = p - u.res_aspect_time.w * u.wave_params.x;
    return fbm(p + fbm(p2));
}

@fragment
fn fs_dither_waves(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let resolution = max(u.res_aspect_time.xy, vec2<f32>(1.0));
    let target_res = max(u.raster_params.xy, vec2<f32>(1.0));
    let aspect = u.res_aspect_time.z;
    let pixel_size = max(1.0, u.wave_params.w);

    // Map physical frag_coord through Chromium's `image-rendering: pixelated` canvas space
    // (`eff_res`): X rounds downward (-0.001) while bottom-up WebGL Y rounds downward from
    // bottom (+0.001 in top-down coordinates), preventing 15px beat stripes at 1.25x DPR.
    let mapped_xy = floor((frag_coord.xy - vec2<f32>(0.001, -0.001)) * resolution / target_res) + 0.5;

    // Match ditherWaves.js:112-113: ditherBlock uses top-down Y, pixelBlock/UV uses bottom-up Y
    let dither_block = floor(mapped_xy / pixel_size);
    let gl_frag_xy = vec2<f32>(mapped_xy.x, resolution.y - mapped_xy.y);
    let pixel_block = floor(gl_frag_xy / pixel_size);
    let sample_coord = (pixel_block * pixel_size + 0.5 * pixel_size) / resolution;
    var uv = sample_coord - 0.5;
    uv.x *= aspect;

    var f = pattern(uv);
    if (u.bg_color_mouse.w > 0.5) {
        var mouse_ndc = (u.mouse_params.xy / resolution - 0.5) * vec2<f32>(1.0, -1.0);
        mouse_ndc.x *= aspect;
        let dist = length(uv - mouse_ndc);
        let effect = 1.0 - smoothstep(0.0, u.mouse_params.z, dist);
        f -= u.mouse_params.w * effect;
    }

    var col = mix(u.bg_color_mouse.xyz, u.wave_color_num.xyz, f);
    col = dither(dither_block, col);
    return vec4<f32>(col, 1.0);
}
