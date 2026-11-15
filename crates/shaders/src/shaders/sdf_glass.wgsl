struct GlassGlobals {
    // xy = viewport_size (pixels), z = time (seconds), w = unused
    viewport_time: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u_glass: GlassGlobals;
@group(0) @binding(1)
var t_scene: texture_2d<f32>;
@group(0) @binding(2)
var t_blur4: texture_2d<f32>;
@group(0) @binding(3)
var t_blur8: texture_2d<f32>;
@group(0) @binding(4)
var t_blur16: texture_2d<f32>;
@group(0) @binding(5)
var t_blur32: texture_2d<f32>;
@group(0) @binding(6)
var s_linear: sampler;

struct GlassInstanceInput {
    @builtin(vertex_index) vertex_index: u32,
    // xy = rect center (px), zw = rect half_size (px)
    @location(0) rect: vec4<f32>,
    // per-corner radius (px): x = top_right, y = bottom_right, z = top_left, w = bottom_left
    @location(1) radii: vec4<f32>,
    // x = blur_px (0..32), y = saturation (1.0..1.9), z = border_width (px), w = skin_kind (0=none,1=dither,2=faulty,3=particles,4=gridmotion)
    @location(2) glass_params: vec4<f32>,
    // padding-box fill gradient stop 0..2 + angles
    @location(3) fill_color_0: vec4<f32>,
    @location(4) fill_color_1: vec4<f32>,
    // border-box gradient stop 0..2
    @location(5) border_color_0: vec4<f32>,
    @location(6) border_color_1: vec4<f32>,
    @location(7) border_color_2: vec4<f32>,
    // xy = shadow_offset (px), z = shadow_blur (px), w = shadow_spread (px)
    @location(8) shadow_geom: vec4<f32>,
    // rgba = shadow_color
    @location(9) shadow_color: vec4<f32>,
    // rgba = inset top 1px highlight color
    @location(10) inset_highlight: vec4<f32>,
    // x = fill_angle_deg, y = border_angle_deg, z = outer_ring_width, w = opacity
    @location(11) extra: vec4<f32>,
    // rgba = outer_ring_color
    @location(12) ring_color: vec4<f32>,
    // [x, y, w, h] clip rect in target px (w <= 0 disables fragment clip)
    @location(13) clip_rect: vec4<f32>,
};

struct GlassVertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) local_px: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) radii: vec4<f32>,
    @location(3) glass_params: vec4<f32>,
    @location(4) fill_color_0: vec4<f32>,
    @location(5) fill_color_1: vec4<f32>,
    @location(6) border_color_0: vec4<f32>,
    @location(7) border_color_1: vec4<f32>,
    @location(8) border_color_2: vec4<f32>,
    @location(9) shadow_geom: vec4<f32>,
    @location(10) shadow_color: vec4<f32>,
    @location(11) inset_highlight: vec4<f32>,
    @location(12) extra: vec4<f32>,
    @location(13) ring_color: vec4<f32>,
    @location(14) clip_rect: vec4<f32>,
};

@vertex
fn vs_sdf_glass(in: GlassInstanceInput) -> GlassVertexOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0,  1.0)
    );
    let corner = corners[in.vertex_index % 6u];
    let pad = max(in.shadow_geom.z * 1.5 + abs(in.shadow_geom.w) + max(abs(in.shadow_geom.x), abs(in.shadow_geom.y)), in.extra.z) + 4.0;
    let local_px = corner * (in.rect.zw + vec2<f32>(pad));
    let screen_px = in.rect.xy + local_px;
    let vp = max(u_glass.viewport_time.xy, vec2<f32>(1.0));
    let ndc = vec2<f32>(
        (screen_px.x / vp.x) * 2.0 - 1.0,
        1.0 - (screen_px.y / vp.y) * 2.0
    );

    var out: GlassVertexOut;
    out.clip_pos = vec4<f32>(ndc, 0.0, 1.0);
    out.local_px = local_px;
    out.half_size = in.rect.zw;
    out.radii = in.radii;
    out.glass_params = in.glass_params;
    out.fill_color_0 = in.fill_color_0;
    out.fill_color_1 = in.fill_color_1;
    out.border_color_0 = in.border_color_0;
    out.border_color_1 = in.border_color_1;
    out.border_color_2 = in.border_color_2;
    out.shadow_geom = in.shadow_geom;
    out.shadow_color = in.shadow_color;
    out.inset_highlight = in.inset_highlight;
    out.extra = in.extra;
    out.ring_color = in.ring_color;
    out.clip_rect = in.clip_rect;
    return out;
}

fn glsl_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// IQ SDF rounded box with per-corner radii:
// r.x = top_right, r.y = bottom_right, r.z = top_left, r.w = bottom_left
fn sd_rounded_box_4(p: vec2<f32>, b: vec2<f32>, r_in: vec4<f32>) -> f32 {
    var rx: vec2<f32>;
    if (p.x > 0.0) {
        rx = r_in.xy;
    } else {
        rx = r_in.zw;
    }
    var r: f32;
    if (p.y > 0.0) {
        r = rx.y;
    } else {
        r = rx.x;
    }
    r = min(r, min(b.x, b.y));
    let q = abs(p) - b + vec2<f32>(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - r;
}

fn sample_blur_pyramid(uv: vec2<f32>, blur_px: f32, saturation: f32) -> vec3<f32> {
    var col: vec3<f32>;
    if (blur_px <= 0.001) {
        col = textureSampleLevel(t_scene, s_linear, uv, 0.0).rgb;
    } else if (blur_px <= 4.0) {
        let t = blur_px / 4.0;
        let c0 = textureSampleLevel(t_scene, s_linear, uv, 0.0).rgb;
        let c1 = textureSampleLevel(t_blur4, s_linear, uv, 0.0).rgb;
        col = mix(c0, c1, t);
    } else if (blur_px <= 8.0) {
        let t = (blur_px - 4.0) / 4.0;
        let c0 = textureSampleLevel(t_blur4, s_linear, uv, 0.0).rgb;
        let c1 = textureSampleLevel(t_blur8, s_linear, uv, 0.0).rgb;
        col = mix(c0, c1, t);
    } else if (blur_px <= 16.0) {
        let t = (blur_px - 8.0) / 8.0;
        let c0 = textureSampleLevel(t_blur8, s_linear, uv, 0.0).rgb;
        let c1 = textureSampleLevel(t_blur16, s_linear, uv, 0.0).rgb;
        col = mix(c0, c1, t);
    } else {
        let t = clamp((blur_px - 16.0) / 16.0, 0.0, 1.0);
        let c0 = textureSampleLevel(t_blur16, s_linear, uv, 0.0).rgb;
        let c1 = textureSampleLevel(t_blur32, s_linear, uv, 0.0).rgb;
        col = mix(c0, c1, t);
    }

    let lum = dot(col, vec3<f32>(0.2126, 0.7152, 0.0722));
    return clamp(mix(vec3<f32>(lum), col, saturation), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn linear_gradient_t(local_px: vec2<f32>, half_size: vec2<f32>, angle_deg: f32) -> f32 {
    // CSS gradient angle: 0deg = bottom->top, 90deg = left->right, 180deg = top->bottom
    let rad = angle_deg * 0.01745329252;
    let dir = vec2<f32>(sin(rad), -cos(rad));
    let proj = dot(local_px, dir);
    let max_proj = abs(dir.x) * half_size.x + abs(dir.y) * half_size.y;
    return clamp(proj / max(2.0 * max_proj, 0.0001) + 0.5, 0.0, 1.0);
}

fn mix_premultiplied(c0: vec4<f32>, c1: vec4<f32>, t: f32) -> vec4<f32> {
    let pm0 = c0.rgb * c0.a;
    let pm1 = c1.rgb * c1.a;
    let pm = mix(pm0, pm1, t);
    let a = mix(c0.a, c1.a, t);
    if (a > 0.0001) {
        return vec4<f32>(pm / a, a);
    }
    return vec4<f32>(0.0);
}

fn eval_gradient_3(c0: vec4<f32>, c1: vec4<f32>, c2: vec4<f32>, t: f32) -> vec4<f32> {
    if (t < 0.5) {
        return mix_premultiplied(c0, c1, t * 2.0);
    }
    return mix_premultiplied(c1, c2, (t - 0.5) * 2.0);
}

@fragment
fn fs_sdf_glass(in: GlassVertexOut) -> @location(0) vec4<f32> {
    if (in.clip_rect.z > 0.0 && in.clip_rect.w > 0.0) {
        let cpos = in.clip_pos.xy;
        if (cpos.x < in.clip_rect.x || cpos.y < in.clip_rect.y
            || cpos.x > in.clip_rect.x + in.clip_rect.z
            || cpos.y > in.clip_rect.y + in.clip_rect.w) {
            discard;
        }
    }

    let vp = max(u_glass.viewport_time.xy, vec2<f32>(1.0));
    let screen_uv = clamp(in.clip_pos.xy / vp, vec2<f32>(0.0), vec2<f32>(1.0));

    let d = sd_rounded_box_4(in.local_px, in.half_size, in.radii);
    let border_w = max(in.glass_params.z, 0.0);

    // Outer SDF coverages with crisp screen-space AA
    let aa = 0.75;
    let outer_cov = smoothstep(aa, -aa, d);
    let inner_cov = smoothstep(aa, -aa, d + border_w);
    let border_cov = max(outer_cov - inner_cov, 0.0);

    // Drop shadow behind outer box
    var shadow_alpha = 0.0;
    if (in.shadow_color.a > 0.001 && in.shadow_geom.z > 0.001) {
        let s_center = in.local_px - in.shadow_geom.xy;
        let s_half = max(in.half_size + vec2<f32>(in.shadow_geom.w), vec2<f32>(1.0));
        let s_radii = max(in.radii + vec4<f32>(in.shadow_geom.w), vec4<f32>(0.0));
        let d_shadow = sd_rounded_box_4(s_center, s_half, s_radii);
        let sigma = max(in.shadow_geom.z * 0.45, 0.5);
        shadow_alpha = exp(-max(d_shadow, 0.0) * max(d_shadow, 0.0) / (2.0 * sigma * sigma)) * in.shadow_color.a;
        if (d_shadow < 0.0) {
            shadow_alpha = in.shadow_color.a;
        }
    }

    // Optional outer ring (e.g. 0 0 0 1px rgba(255,255,255,0.08))
    var ring_cov = 0.0;
    if (in.extra.z > 0.001 && in.ring_color.a > 0.001) {
        ring_cov = max(smoothstep(aa, -aa, d - in.extra.z) - outer_cov, 0.0);
    }

    if (outer_cov <= 0.001 && shadow_alpha <= 0.001 && ring_cov <= 0.001) {
        discard;
    }

    // Sample Pass 1 Dual-Kawase blur pyramid + saturate()
    let backdrop_rgb = sample_blur_pyramid(screen_uv, in.glass_params.x, in.glass_params.y);

    // Padding-box gradient fill (interpolated in premultiplied alpha space)
    let fill_t = linear_gradient_t(in.local_px, in.half_size, in.extra.x);
    var fill_col = mix_premultiplied(in.fill_color_0, in.fill_color_1, fill_t);

    // Chameleon overlays (settings.css:1388-1459, main.css:1817-1865)
    let skin_kind = i32(in.glass_params.w + 0.5);
    let top_left_px = in.local_px + in.half_size;
    if (skin_kind == 1 && fill_col.a > 0.001 && max(fill_col.r, max(fill_col.g, fill_col.b)) < 0.5) {
        // Dither crosshatch lines on dark non-IACVT surfaces (3px period H/V grid)
        if (glsl_mod(top_left_px.y, 3.0) < 1.0) {
            fill_col = vec4<f32>(mix(fill_col.rgb, vec3<f32>(1.0), 0.03), fill_col.a);
        }
        if (glsl_mod(top_left_px.x, 3.0) < 1.0) {
            fill_col = vec4<f32>(mix(fill_col.rgb, vec3<f32>(1.0), 0.025), fill_col.a);
        }
    } else if (skin_kind == 2 && max(fill_col.r, max(fill_col.g, fill_col.b)) < 0.5) {
        // Faulty CRT scanline pattern + animated faultyPickerSweep (2.8s linear infinite)
        if (glsl_mod(top_left_px.y, 3.0) < 1.0) {
            let scan_rgb = vec3<f32>(152.0 / 255.0, 1.0, 186.0 / 255.0);
            fill_col = vec4<f32>(mix(fill_col.rgb, scan_rgb, 0.08), fill_col.a);
        }
        let sweep_phase = fract(u_glass.viewport_time.z / 2.8) * 2.0 - 1.0;
        let rect_v = top_left_px.y / max(in.half_size.y * 2.0, 1.0) - sweep_phase;
        if (rect_v >= 0.0 && rect_v <= 1.0) {
            let band = 1.0 - abs(rect_v - 0.48) / 0.52;
            let sweep_alpha = clamp(band, 0.0, 1.0) * 0.3 * 0.25;
            let sweep_rgb = vec3<f32>(167.0 / 255.0, 239.0 / 255.0, 158.0 / 255.0);
            fill_col = vec4<f32>(mix(fill_col.rgb, sweep_rgb, sweep_alpha), fill_col.a);
        }
    }

    // Composite padding-box over blurred backdrop
    var interior_rgb = mix(backdrop_rgb, fill_col.rgb, fill_col.a);

    // Inset top 1px highlight (inset 0 1px 0 ...)
    if (in.inset_highlight.a > 0.001 && top_left_px.y >= border_w && top_left_px.y <= border_w + 1.25) {
        interior_rgb = mix(interior_rgb, in.inset_highlight.rgb, in.inset_highlight.a);
    }

    // Border-box gradient border composited over the darkened glass interior
    let border_t = linear_gradient_t(in.local_px, in.half_size, in.extra.y);
    let border_col = eval_gradient_3(in.border_color_0, in.border_color_1, in.border_color_2, border_t);
    let border_rgb = mix(interior_rgb, border_col.rgb, border_col.a);

    // Combine interior + border inside outer_cov
    let glass_rgb = mix(border_rgb, interior_rgb, inner_cov / max(outer_cov, 0.0001));
    let glass_alpha = outer_cov * in.extra.w;

    // Combine shadow + ring behind glass (sequential straight-alpha blend)
    let r_a = ring_cov * in.ring_color.a;
    let bg_effect_alpha = r_a + shadow_alpha * (1.0 - r_a);
    let bg_effect_rgb = mix(in.shadow_color.rgb, in.ring_color.rgb, r_a / max(bg_effect_alpha, 0.0001));

    let out_alpha = glass_alpha + bg_effect_alpha * (1.0 - glass_alpha);
    let out_rgb = mix(bg_effect_rgb, glass_rgb, glass_alpha / max(out_alpha, 0.0001));
    return vec4<f32>(out_rgb, out_alpha);
}
