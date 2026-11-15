struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_faulty_terminal(@builtin(vertex_index) vid: u32) -> VertexOut {
    let x = f32((vid << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vid & 2u) * 2.0 - 1.0;
    var out: VertexOut;
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    // Bottom-left (0,0) -> Top-right (1,1) matching WebGL OGL Triangle vUv
    out.uv = vec2<f32>(x * 0.5 + 0.5, y * 0.5 + 0.5);
    return out;
}

struct FaultyTerminalUniforms {
    // xyz = iResolution (width, height, aspect), w = iTime
    res_time: vec4<f32>,
    // xy = uGridMul, z = uScale, w = uDigitSize
    scale_grid_digit: vec4<f32>,
    // x = uScanlineIntensity, y = uGlitchAmount, z = uFlickerAmount, w = uNoiseAmp
    glitch_params: vec4<f32>,
    // x = uChromaticAberration, y = uDither, z = uCurvature, w = uBrightness
    post_params: vec4<f32>,
    // xyz = uTint, w = uUseMouse (0.0 or 1.0)
    tint_use_mouse: vec4<f32>,
    // xy = uMouse (0..1 bottom-up), z = uMouseStrength, w = uUsePageLoadAnimation
    mouse_load: vec4<f32>,
    // x = uPageLoadProgress, yzw = padding
    load_progress: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u: FaultyTerminalUniforms;

var<private> g_time: f32 = 0.0;

fn glsl_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

fn hash21(p_in: vec2<f32>) -> f32 {
    var p = fract(p_in * 234.56);
    p += vec2<f32>(dot(p, p + vec2<f32>(34.56)));
    return fract(p.x * p.y);
}

fn noise(p: vec2<f32>) -> f32 {
    return sin(p.x * 10.0) * sin(p.y * (3.0 + sin(g_time * 0.090909))) + 0.2;
}

fn rotate2d(angle: f32) -> mat2x2<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return mat2x2<f32>(vec2<f32>(c, -s), vec2<f32>(s, c));
}

fn fbm(p_in: vec2<f32>) -> f32 {
    var p = p_in * 1.1;
    var f = 0.0;
    var amp = 0.5 * u.glitch_params.w;

    let modify0 = rotate2d(g_time * 0.02);
    f += amp * noise(p);
    p = modify0 * p * 2.0;
    amp *= 0.454545;

    let modify1 = rotate2d(g_time * 0.02);
    f += amp * noise(p);
    p = modify1 * p * 2.0;
    amp *= 0.454545;

    let modify2 = rotate2d(g_time * 0.08);
    f += amp * noise(p);

    return f;
}

fn pattern(p: vec2<f32>) -> f32 {
    let offset1 = vec2<f32>(1.0);
    let offset0 = vec2<f32>(0.0);
    let rot01 = rotate2d(0.1 * g_time);
    let rot1 = rotate2d(0.1);

    let q = vec2<f32>(fbm(p + offset1), fbm(rot01 * p + offset1));
    let r = vec2<f32>(fbm(rot1 * q + offset0), fbm(q + offset0));
    return fbm(p + r);
}

fn digit(p_in: vec2<f32>) -> f32 {
    let grid = u.scale_grid_digit.xy * 15.0;
    let s = floor(p_in * grid) / grid;
    var p = p_in * grid;
    var intensity = pattern(s * 0.1) * 1.3 - 0.03;

    if (u.tint_use_mouse.w > 0.5) {
        let mouse_world = u.mouse_load.xy * u.scale_grid_digit.z;
        let dist_to_mouse = distance(s, mouse_world);
        let mouse_influence = exp(-dist_to_mouse * 8.0) * u.mouse_load.z * 10.0;
        intensity += mouse_influence;

        let ripple = sin(dist_to_mouse * 20.0 - u.res_time.w * 5.0) * 0.1 * mouse_influence;
        intensity += ripple;
    }

    if (u.mouse_load.w > 0.5) {
        let cell_random = fract(sin(dot(s, vec2<f32>(12.9898, 78.233))) * 43758.5453);
        let cell_delay = cell_random * 0.8;
        let cell_progress = clamp((u.load_progress.x - cell_delay) / 0.2, 0.0, 1.0);
        let fade_alpha = smoothstep(0.0, 1.0, cell_progress);
        intensity *= fade_alpha;
    }

    p = fract(p);
    p *= u.scale_grid_digit.w;

    let px5 = p.x * 5.0;
    let py5 = (1.0 - p.y) * 5.0;
    let x = fract(px5);
    let y = fract(py5);

    let i = floor(py5) - 2.0;
    let j = floor(px5) - 2.0;
    let n = i * i + j * j;
    let f = n * 0.0625;

    let is_on = step(0.1, intensity - f);
    let brightness = is_on * (0.2 + y * 0.8) * (0.75 + x * 0.25);

    return step(0.0, p.x) * step(p.x, 1.0) * step(0.0, p.y) * step(p.y, 1.0) * brightness;
}

fn on_off(a: f32, b: f32, c: f32) -> f32 {
    return step(c, sin(u.res_time.w + a * cos(u.res_time.w * b))) * u.glitch_params.z;
}

fn displace(look: vec2<f32>) -> f32 {
    let y = look.y - glsl_mod(u.res_time.w * 0.25, 1.0);
    let window = 1.0 / (1.0 + 50.0 * y * y);
    return sin(look.y * 20.0 + u.res_time.w) * 0.0125 * on_off(4.0, 2.0, 0.8) * (1.0 + cos(u.res_time.w * 60.0)) * window;
}

fn get_color(p_in: vec2<f32>) -> vec3<f32> {
    var p = p_in;
    var bar = step(glsl_mod(p.y + g_time * 20.0, 1.0), 0.2) * 0.4 + 1.0;
    bar *= u.glitch_params.x;

    let displacement = displace(p);
    p.x += displacement;

    if (u.glitch_params.y != 1.0) {
        let extra = displacement * (u.glitch_params.y - 1.0);
        p.x += extra;
    }

    let middle = digit(p);

    let off = 0.002;
    let sum = digit(p + vec2<f32>(-off, -off))
        + digit(p + vec2<f32>(0.0, -off))
        + digit(p + vec2<f32>(off, -off))
        + digit(p + vec2<f32>(-off, 0.0))
        + digit(p + vec2<f32>(0.0, 0.0))
        + digit(p + vec2<f32>(off, 0.0))
        + digit(p + vec2<f32>(-off, off))
        + digit(p + vec2<f32>(0.0, off))
        + digit(p + vec2<f32>(off, off));

    return vec3<f32>(0.9) * middle + sum * 0.1 * vec3<f32>(1.0) * bar;
}

fn barrel(uv: vec2<f32>) -> vec2<f32> {
    var c = uv * 2.0 - 1.0;
    let r2 = dot(c, c);
    c *= 1.0 + u.post_params.z * r2;
    return c * 0.5 + 0.5;
}

@fragment
fn fs_faulty_terminal(in: VertexOut) -> @location(0) vec4<f32> {
    g_time = u.res_time.w * 0.333333;
    // vs_faulty_terminal already emits bottom-up vUv matching WebGL OGL Triangle
    var uv = in.uv;

    if (u.post_params.z != 0.0) {
        uv = barrel(uv);
    }

    let p = uv * u.scale_grid_digit.z;
    var col = get_color(p);

    if (u.post_params.x != 0.0) {
        let ca = vec2<f32>(u.post_params.x) / max(u.res_time.xy, vec2<f32>(1.0));
        col.r = get_color(p + ca).r;
        col.b = get_color(p - ca).b;
    }

    col *= u.tint_use_mouse.xyz;
    col *= u.post_params.w;

    if (u.post_params.y > 0.0) {
        let rnd = hash21(in.pos.xy);
        col += (rnd - 0.5) * (u.post_params.y * 0.003922);
    }

    return vec4<f32>(col, 1.0);
}
