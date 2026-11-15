struct GridBgUniforms {
    // xy = viewport_size, zw = unused
    viewport: vec4<f32>,
    // xyz = gradient_color (center), w = scale
    center_color_scale: vec4<f32>,
    // x = cos(-12deg) * scale, y = sin(-12deg) * scale, zw = unused
    tilt_cos_sin: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u_grid: GridBgUniforms;

@group(1) @binding(0)
var t_atlas: texture_2d<f32>;
@group(1) @binding(1)
var s_atlas: sampler;

struct BgVertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_grid_bg(@builtin(vertex_index) vid: u32) -> BgVertexOut {
    let x = f32((vid << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vid & 2u) * 2.0 - 1.0;
    var out: BgVertexOut;
    out.pos = vec4<f32>(x, -y, 0.0, 1.0);
    out.uv = vec2<f32>(x * 0.5 + 0.5, y * 0.5 + 0.5);
    return out;
}

@fragment
fn fs_grid_bg(in: BgVertexOut) -> @location(0) vec4<f32> {
    // CSS radial-gradient(circle, gradientColor 0%, #000000 100%) (farthest-corner)
    let aspect = u_grid.viewport.x / max(u_grid.viewport.y, 1.0);
    let delta = (in.uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let max_dist = length(vec2<f32>(aspect * 0.5, 0.5));
    let t = clamp(length(delta) / max(max_dist, 0.0001), 0.0, 1.0);
    let col = mix(u_grid.center_color_scale.xyz, vec3<f32>(0.0), t);
    return vec4<f32>(col, 1.0);
}

struct CardInstanceInput {
    @builtin(vertex_index) vertex_index: u32,
    // xy = center in unrotated grid-space relative to viewport center, zw = full width, height (px)
    @location(0) rect: vec4<f32>,
    // xyz = gradient stop 0
    @location(1) color_0: vec4<f32>,
    // xyz = gradient stop 1 (at t=1/3)
    @location(2) color_1: vec4<f32>,
    // xyz = gradient stop 2 (at t=2/3)
    @location(3) color_2: vec4<f32>,
    // xyz = gradient stop 3 (at t=1)
    @location(4) color_3: vec4<f32>,
    // xy = atlas uv min (u0, v0), zw = atlas uv max (u1, v1)
    @location(5) atlas_uv: vec4<f32>,
    // x = corner_radius_px, y = hover_zoom (1.0..1.06), z = brightness (1.0..1.08), w = has_image (0.0 gradient+label, 1.0 photo)
    @location(6) style: vec4<f32>,
};

struct CardVertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) local_px: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) color_0: vec4<f32>,
    @location(3) color_1: vec4<f32>,
    @location(4) color_2: vec4<f32>,
    @location(5) color_3: vec4<f32>,
    @location(6) atlas_uv: vec4<f32>,
    @location(7) style: vec4<f32>,
};

@vertex
fn vs_grid_card(in: CardInstanceInput) -> CardVertexOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0,  1.0)
    );
    let corner = corners[in.vertex_index % 6u];
    let half_size = in.rect.zw * 0.5;
    let shadow_pad = 24.0;
    let local_px = corner * (half_size + vec2<f32>(shadow_pad));

    // Position in grid-space relative to center, then rotate(-12deg) & scale
    let p_grid = in.rect.xy + local_px;
    let cs = u_grid.tilt_cos_sin.x;
    let sn = u_grid.tilt_cos_sin.y;
    let p_rot = vec2<f32>(
        p_grid.x * cs - p_grid.y * sn,
        p_grid.x * sn + p_grid.y * cs
    );
    let screen_px = u_grid.viewport.xy * 0.5 + p_rot;
    let ndc = vec2<f32>(
        (screen_px.x / max(u_grid.viewport.x, 1.0)) * 2.0 - 1.0,
        1.0 - (screen_px.y / max(u_grid.viewport.y, 1.0)) * 2.0
    );

    var out: CardVertexOut;
    out.clip_pos = vec4<f32>(ndc, 0.0, 1.0);
    out.local_px = local_px;
    out.half_size = half_size;
    out.color_0 = in.color_0;
    out.color_1 = in.color_1;
    out.color_2 = in.color_2;
    out.color_3 = in.color_3;
    out.atlas_uv = in.atlas_uv;
    out.style = in.style;
    return out;
}

fn sd_rounded_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - r;
}

@fragment
fn fs_grid_card(in: CardVertexOut) -> @location(0) vec4<f32> {
    let radius = min(in.style.x, min(in.half_size.x, in.half_size.y));
    let d = sd_rounded_box(in.local_px, in.half_size, radius);

    // Shadow: 0 4px 20px rgba(0, 0, 0, 0.4)
    let d_shadow = sd_rounded_box(in.local_px - vec2<f32>(0.0, 4.0), in.half_size, radius);
    let shadow_alpha = smoothstep(20.0, -4.0, d_shadow) * 0.4;

    let card_alpha = smoothstep(0.85, -0.85, d);
    if (card_alpha <= 0.001 && shadow_alpha <= 0.001) {
        discard;
    }

    // Normalized card UV (0..1)
    let card_uv = (in.local_px / max(in.half_size * 2.0, vec2<f32>(1.0))) + vec2<f32>(0.5);
    let zoom = max(in.style.y, 1.0);
    let zoomed_uv = (card_uv - vec2<f32>(0.5)) / zoom + vec2<f32>(0.5);

    var fill_rgb: vec3<f32>;
    let has_atlas = (in.atlas_uv.z > in.atlas_uv.x) && (in.atlas_uv.w > in.atlas_uv.y);

    if (in.style.w > 0.5 && has_atlas) {
        // CSS background-size: cover; background-position: center
        let card_aspect = in.half_size.x / max(in.half_size.y, 1.0);
        let img_aspect = in.color_0.w;
        var cover_scale = vec2<f32>(1.0, 1.0);
        if (img_aspect > 0.001) {
            if (img_aspect > card_aspect) {
                cover_scale = vec2<f32>(card_aspect / img_aspect, 1.0);
            } else {
                cover_scale = vec2<f32>(1.0, img_aspect / card_aspect);
            }
        }
        let cover_uv = (zoomed_uv - vec2<f32>(0.5)) * cover_scale + vec2<f32>(0.5);
        let sample_uv = mix(in.atlas_uv.xy, in.atlas_uv.zw, clamp(cover_uv, vec2<f32>(0.0), vec2<f32>(1.0)));
        fill_rgb = textureSampleLevel(t_atlas, s_atlas, sample_uv, 0.0).rgb * in.style.z;
    } else {
        // True CSS linear-gradient(135deg, ...) projected onto non-square card geometry
        let denom = max(2.0 * (in.half_size.x + in.half_size.y), 0.0001);
        let grad_t = clamp((in.local_px.x + in.local_px.y) / denom + 0.5, 0.0, 1.0);
        if (grad_t < 0.3333333) {
            fill_rgb = mix(in.color_0.rgb, in.color_1.rgb, grad_t * 3.0);
        } else if (grad_t < 0.6666667) {
            fill_rgb = mix(in.color_1.rgb, in.color_2.rgb, (grad_t - 0.3333333) * 3.0);
        } else {
            fill_rgb = mix(in.color_2.rgb, in.color_3.rgb, (grad_t - 0.6666667) * 3.0);
        }
        if (has_atlas) {
            // Sample centered 256x64px label atlas at proportional 1:1 CSS scale
            let label_scale = clamp(in.half_size.x / 210.0, 0.35, 3.0);
            let label_half = vec2<f32>(128.0, 32.0) * label_scale;
            let label_uv = in.local_px / (label_half * 2.0) + vec2<f32>(0.5);
            if (label_uv.x >= 0.0 && label_uv.x <= 1.0 && label_uv.y >= 0.0 && label_uv.y <= 1.0) {
                let sample_uv = mix(in.atlas_uv.xy, in.atlas_uv.zw, label_uv);
                let label = textureSampleLevel(t_atlas, s_atlas, sample_uv, 0.0);
                fill_rgb = fill_rgb * (1.0 - label.a) + label.rgb;
            }
        }
    }

    // 1px border: rgba(255, 255, 255, 0.06)
    let border_cov = smoothstep(0.6, -0.6, d) - smoothstep(0.6, -0.6, d + 1.0);
    let card_col = mix(fill_rgb, vec3<f32>(1.0), border_cov * 0.06);

    // Combine shadow behind card
    let total_alpha = card_alpha + shadow_alpha * (1.0 - card_alpha);
    let out_rgb = mix(vec3<f32>(0.0), card_col, card_alpha / max(total_alpha, 0.0001));
    return vec4<f32>(out_rgb, total_alpha);
}
