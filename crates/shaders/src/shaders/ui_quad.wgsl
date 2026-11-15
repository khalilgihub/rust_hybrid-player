struct UiQuadGlobals {
    // xy = viewport_size (pixels), zw = unused
    viewport: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u_ui: UiQuadGlobals;
@group(0) @binding(1)
var t_quad: texture_2d<f32>;
@group(0) @binding(2)
var s_quad: sampler;

struct UiQuadInstanceInput {
    @builtin(vertex_index) vertex_index: u32,
    // xy = top-left (px), zw = size (w, h px)
    @location(0) rect: vec4<f32>,
    // [u0, v0, u1, v1]
    @location(1) uv_rect: vec4<f32>,
    // rgba tint in 0..1
    @location(2) tint: vec4<f32>,
    // per-corner radii: [top_right, bottom_right, top_left, bottom_left]
    @location(3) radii: vec4<f32>,
    // border rgba
    @location(4) border_color: vec4<f32>,
    // x = sample_mode (0=Rgba, 1=MaskR8, 2=PremultipliedRgba, 3=SolidColor, 4=LinearGradient)
    // y = border_width_px, z = rotation_rad, w = gradient_angle_rad
    @location(5) params: vec4<f32>,
    // [x, y, w, h] clip rect in target px (w <= 0 disables fragment clip)
    @location(6) clip_rect: vec4<f32>,
    // clip corner radii: [top_right, bottom_right, top_left, bottom_left]
    @location(7) clip_radii: vec4<f32>,
};

struct UiQuadVertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) local_px: vec2<f32>,
    @location(2) half_size: vec2<f32>,
    @location(3) tint: vec4<f32>,
    @location(4) radii: vec4<f32>,
    @location(5) border_color: vec4<f32>,
    @location(6) params: vec4<f32>,
    @location(7) clip_rect: vec4<f32>,
    @location(8) clip_radii: vec4<f32>,
    @location(9) grad_color: vec4<f32>,
};

@vertex
fn vs_ui_quad(in: UiQuadInstanceInput) -> UiQuadVertexOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-0.5, -0.5),
        vec2<f32>( 0.5, -0.5),
        vec2<f32>( 0.5,  0.5),
        vec2<f32>(-0.5, -0.5),
        vec2<f32>( 0.5,  0.5),
        vec2<f32>(-0.5,  0.5)
    );
    let corner = corners[in.vertex_index % 6u];
    let size = max(in.rect.zw, vec2<f32>(0.0));
    let half_size = size * 0.5;
    let local_px = corner * size;
    let center = in.rect.xy + half_size;

    let rot = in.params.z;
    let c = cos(rot);
    let s = sin(rot);
    let rotated_px = vec2<f32>(
        local_px.x * c - local_px.y * s,
        local_px.x * s + local_px.y * c
    );
    let screen_px = center + rotated_px;

    let vp = max(u_ui.viewport.xy, vec2<f32>(1.0));
    let ndc = vec2<f32>(
        (screen_px.x / vp.x) * 2.0 - 1.0,
        1.0 - (screen_px.y / vp.y) * 2.0
    );

    let uv_t = corner + vec2<f32>(0.5);
    let uv = mix(in.uv_rect.xy, in.uv_rect.zw, uv_t);

    var out: UiQuadVertexOut;
    out.clip_pos = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = uv;
    out.local_px = local_px;
    out.half_size = half_size;
    out.tint = in.tint;
    out.radii = in.radii;
    out.border_color = in.border_color;
    out.params = in.params;
    out.clip_rect = in.clip_rect;
    out.clip_radii = in.clip_radii;
    out.grad_color = in.uv_rect;
    return out;
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

@fragment
fn fs_ui_quad(in: UiQuadVertexOut) -> @location(0) vec4<f32> {
    let mode = i32(in.params.x + 0.5);
    let tex_sample = textureSample(t_quad, s_quad, in.uv);

    var fill_col: vec4<f32>;
    if (mode == 1) {
        // R8 coverage glyph mask
        fill_col = vec4<f32>(in.tint.rgb, in.tint.a * tex_sample.r);
    } else if (mode == 2) {
        // Premultiplied RGBA (e.g. resvg tiny-skia pixmap) converted to straight alpha for ALPHA_BLENDING
        if (tex_sample.a > 0.0001) {
            let straight = min(tex_sample.rgb / tex_sample.a, vec3<f32>(1.0));
            fill_col = vec4<f32>(straight * in.tint.rgb, tex_sample.a * in.tint.a);
        } else {
            fill_col = vec4<f32>(0.0);
        }
    } else if (mode == 3) {
        // Solid untextured color
        fill_col = in.tint;
    } else if (mode == 4) {
        // CSS linear gradient in premultiplied alpha space (`0deg` = bottom->top, `180deg` = top->bottom)
        let angle_rad = in.params.w;
        let dir = vec2<f32>(sin(angle_rad), -cos(angle_rad));
        let proj = dot(in.local_px, dir);
        let max_proj = abs(dir.x) * in.half_size.x + abs(dir.y) * in.half_size.y;
        let grad_t = clamp(proj / max(2.0 * max_proj, 0.0001) + 0.5, 0.0, 1.0);
        let pm0 = in.tint.rgb * in.tint.a;
        let pm1 = in.grad_color.rgb * in.grad_color.a;
        let pm = mix(pm0, pm1, grad_t);
        let ga = mix(in.tint.a, in.grad_color.a, grad_t);
        if (ga > 0.0001) {
            fill_col = vec4<f32>(pm / ga, ga);
        } else {
            fill_col = vec4<f32>(0.0);
        }
    } else {
        // Standard straight-alpha RGBA
        fill_col = tex_sample * in.tint;
    }

    let aa = 0.75;
    let max_radius = max(max(in.radii.x, in.radii.y), max(in.radii.z, in.radii.w));
    let border_w = max(in.params.y, 0.0);

    var out_col = fill_col;
    if (max_radius > 0.001 || border_w > 0.001) {
        let d = sd_rounded_box_4(in.local_px, in.half_size, in.radii);
        let outer_cov = smoothstep(aa, -aa, d);
        if (outer_cov <= 0.001) {
            discard;
        }
        if (border_w > 0.001 && in.border_color.a > 0.001) {
            let inner_cov = smoothstep(aa, -aa, d + border_w);
            let border_cov = max(outer_cov - inner_cov, 0.0);
            let f_a = fill_col.a * outer_cov;
            let br_a = in.border_color.a * border_cov;
            let total_a = br_a + f_a * (1.0 - br_a);
            if (total_a <= 0.0001) {
                discard;
            }
            let premul_rgb = in.border_color.rgb * br_a + fill_col.rgb * (f_a * (1.0 - br_a));
            out_col = vec4<f32>(premul_rgb / total_a, total_a);
        } else {
            out_col = vec4<f32>(fill_col.rgb, fill_col.a * outer_cov);
        }
    }

    // Optional rounded or rectangular fragment clip region
    if (in.clip_rect.z > 0.0 && in.clip_rect.w > 0.0) {
        let clip_half = in.clip_rect.zw * 0.5;
        let clip_center = in.clip_rect.xy + clip_half;
        let d_clip = sd_rounded_box_4(in.clip_pos.xy - clip_center, clip_half, in.clip_radii);
        let clip_cov = smoothstep(aa, -aa, d_clip);
        if (clip_cov <= 0.001) {
            discard;
        }
        out_col.a *= clip_cov;
    }

    if (out_col.a <= 0.001) {
        discard;
    }
    return out_col;
}
