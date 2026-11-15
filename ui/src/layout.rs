//! Taffy flex/box layout & UI frame builder for Hybrid Player.
//! Translates `.rust-migration/tokens-and-metrics.md` 1:1 into `UiFrameOutput` render primitives,
//! responsive visibility rules, hit regions, Chameleon skins, and AccessKit tree updates.

use std::cell::RefCell;

use hybrid_core::equalizer::EqualizerPreset;
use hybrid_core::shortcuts::{
    format_combo_compact, format_combo_for_display, ShortcutManager, SHORTCUT_CATEGORIES,
};
use hybrid_shaders::{
    parse_hex_rgb, ChameleonSkin, CssBackdropRule, GlassRect, GridAspectRatio, QualityTier,
    ThemeMode, WelcomeBackgroundKind,
};
use taffy::prelude::*;

use crate::a11y::build_accessibility_tree;
use crate::fonts::{FontRole, TextEngine};
use crate::icons::SvgVariant;
use crate::motion::{cubic_bezier, evaluate_keyframe, KeyframeId, KeyframeSample, MotionProfile};
use crate::primitives::{
    BreakpointState, CursorHint, HitRegion, HitTarget, RectPx, TextAlign, UiFrameOutput,
    UiGlassPrimitive, UiImageSlot, UiShaderSlot, UiSolidRect, UiSvgQuad, UiTextRun, VirtualSlice,
};
use crate::state::{
    AbLoopState, BgControlInputType, CarouselMode, ColorPickerTarget, DropdownKind,
    FocusedTextField, ModalId, RecordingIndicatorMode, RepeatMode, ToastKind, UiState,
    WindowMaximizeState, BG_CONTROLS_CATALOG, COLOR_PICKER_SWATCHES, HOVER_TOOLTIP_DELAY_SECS,
    MAX_VISIBLE_TOASTS, PLAYLIST_ITEM_HEIGHT, SPEED_PRESETS,
};
use crate::svg_catalog::SvgId;
use crate::tokens::{
    parse_hex_rgb_u8, rgba, rgba8, ChameleonTokens, FontConfig, RootTokens, ThemeTokens, ZIndex,
};

const TITLEBAR_HEIGHT: f32 = 36.0;

thread_local! {
    static LAYOUT_TEXT_ENGINE: RefCell<TextEngine> = RefCell::new(TextEngine::new());
}

fn measure_text_width(
    text: &str,
    role: FontRole,
    font_size_px: f32,
    weight: u16,
    letter_spacing_px: f32,
    cfg: FontConfig,
) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let line_height_px = (font_size_px * 1.25).round();
    LAYOUT_TEXT_ENGINE.with(|cell| {
        let mut engine = cell.borrow_mut();
        engine
            .measure_with_options(
                text,
                font_size_px,
                line_height_px,
                None,
                role,
                weight,
                cfg,
                letter_spacing_px,
                false,
            )
            .width
    })
}

#[inline]
fn rgba_f32_to_u8(c: [f32; 4]) -> [u8; 4] {
    [
        (c[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (c[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (c[2].clamp(0.0, 1.0) * 255.0).round() as u8,
        (c[3].clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

struct FrameBuilder {
    hovered: Option<HitTarget>,
    shader_slots: Vec<UiShaderSlot>,
    glass_panels: Vec<UiGlassPrimitive>,
    solid_rects: Vec<UiSolidRect>,
    image_slots: Vec<UiImageSlot>,
    text_runs: Vec<UiTextRun>,
    svg_quads: Vec<UiSvgQuad>,
    hit_regions: Vec<HitRegion>,
}

#[derive(Clone, Copy)]
struct FrameCheckpoint {
    shader: usize,
    glass: usize,
    solid: usize,
    img: usize,
    text: usize,
    svg: usize,
    hit: usize,
}

impl FrameBuilder {
    fn new(hovered: Option<HitTarget>) -> Self {
        Self {
            hovered,
            shader_slots: Vec::new(),
            glass_panels: Vec::new(),
            solid_rects: Vec::new(),
            image_slots: Vec::new(),
            text_runs: Vec::new(),
            svg_quads: Vec::new(),
            hit_regions: Vec::new(),
        }
    }

    fn checkpoint(&self) -> FrameCheckpoint {
        FrameCheckpoint {
            shader: self.shader_slots.len(),
            glass: self.glass_panels.len(),
            solid: self.solid_rects.len(),
            img: self.image_slots.len(),
            text: self.text_runs.len(),
            svg: self.svg_quads.len(),
            hit: self.hit_regions.len(),
        }
    }

    fn apply_subtree_anim(
        &mut self,
        cp: FrameCheckpoint,
        origin_x: f32,
        origin_y: f32,
        kf: KeyframeSample,
        apply_rotate: bool,
    ) {
        let sx = kf.scale_x;
        let sy = kf.scale_y;
        let tx = kf.translate_x;
        let ty = kf.translate_y;
        let op = kf.opacity.clamp(0.0, 1.0);
        if (sx - 1.0).abs() < 1e-4
            && (sy - 1.0).abs() < 1e-4
            && tx.abs() < 1e-3
            && ty.abs() < 1e-3
            && (op - 1.0).abs() < 1e-4
            && (!apply_rotate || kf.rotate_deg.abs() < 1e-3)
        {
            return;
        }
        let transform_rect = |r: RectPx| -> RectPx {
            RectPx::new(
                origin_x + (r.x - origin_x) * sx + tx,
                origin_y + (r.y - origin_y) * sy + ty,
                r.width * sx,
                r.height * sy,
            )
        };
        let font_scale = ((sx + sy) * 0.5).max(0.1);
        for slot in &mut self.shader_slots[cp.shader..] {
            slot.rect = transform_rect(slot.rect);
            slot.opacity *= op;
            slot.clip_rect = slot.clip_rect.map(transform_rect);
        }
        for g in &mut self.glass_panels[cp.glass..] {
            let r = transform_rect(RectPx::new(
                g.glass.x,
                g.glass.y,
                g.glass.width,
                g.glass.height,
            ));
            g.glass.x = r.x;
            g.glass.y = r.y;
            g.glass.width = r.width;
            g.glass.height = r.height;
            g.glass.opacity *= op;
            g.glass.shadow_color[3] *= op;
            g.clip_rect = g.clip_rect.map(transform_rect);
        }
        for s in &mut self.solid_rects[cp.solid..] {
            s.rect = transform_rect(s.rect);
            s.color[3] *= op;
            s.border_color[3] *= op;
            if let Some((ref mut grad, _)) = s.gradient {
                grad[3] *= op;
            }
            s.clip_rect = s.clip_rect.map(transform_rect);
        }
        for im in &mut self.image_slots[cp.img..] {
            im.rect = transform_rect(im.rect);
            im.opacity *= op;
            im.clip_rect = im.clip_rect.map(transform_rect);
        }
        for t in &mut self.text_runs[cp.text..] {
            t.rect = transform_rect(t.rect);
            t.font_size_px *= font_scale;
            t.line_height_px *= font_scale;
            t.color[3] *= op;
            t.clip_rect = t.clip_rect.map(transform_rect);
        }
        for q in &mut self.svg_quads[cp.svg..] {
            q.rect = transform_rect(q.rect);
            q.opacity *= op;
            if apply_rotate {
                q.rotation_deg += kf.rotate_deg;
            }
            q.clip_rect = q.clip_rect.map(transform_rect);
        }
        for h in &mut self.hit_regions[cp.hit..] {
            h.rect = transform_rect(h.rect);
        }
    }

    #[inline]
    fn is_hovered(&self, target: HitTarget) -> bool {
        self.hovered.as_ref() == Some(&target)
    }

    #[allow(clippy::too_many_arguments)]
    fn push_glass(
        &mut self,
        id: impl Into<String>,
        z_index: i32,
        rect: RectPx,
        radius: f32,
        rule: Option<CssBackdropRule>,
        skin: ChameleonSkin,
        theme: &ThemeTokens,
        clip_rect: Option<RectPx>,
    ) {
        let mut glass = GlassRect::new(rect.x, rect.y, rect.width, rect.height, radius, theme.mode)
            .with_chameleon_skin(skin, theme.mode);
        if let Some(r) = rule {
            glass = glass.with_backdrop_rule(r, theme.mode);
        }
        self.glass_panels.push(UiGlassPrimitive {
            id: id.into(),
            z_index,
            glass,
            backdrop_rule: rule,
            chameleon_skin: skin,
            clip_rect,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn push_custom_glass(
        &mut self,
        id: impl Into<String>,
        z_index: i32,
        glass: GlassRect,
        backdrop_rule: Option<CssBackdropRule>,
        clip_rect: Option<RectPx>,
    ) {
        let skin = glass.skin;
        self.glass_panels.push(UiGlassPrimitive {
            id: id.into(),
            z_index,
            glass,
            backdrop_rule,
            chameleon_skin: skin,
            clip_rect,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn push_solid(
        &mut self,
        id: impl Into<String>,
        z_index: i32,
        rect: RectPx,
        color: [f32; 4],
        corner_radii: [f32; 4],
        border_width: f32,
        border_color: [f32; 4],
        clip_rect: Option<RectPx>,
    ) {
        self.solid_rects.push(UiSolidRect {
            id: id.into(),
            z_index,
            rect,
            color,
            gradient: None,
            corner_radii,
            border_width,
            border_color,
            clip_rect,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn push_gradient(
        &mut self,
        id: impl Into<String>,
        z_index: i32,
        rect: RectPx,
        start_color: [f32; 4],
        end_color: [f32; 4],
        angle_deg: f32,
        corner_radii: [f32; 4],
        border_width: f32,
        border_color: [f32; 4],
        clip_rect: Option<RectPx>,
    ) {
        self.solid_rects.push(UiSolidRect {
            id: id.into(),
            z_index,
            rect,
            color: start_color,
            gradient: Some((end_color, angle_deg)),
            corner_radii,
            border_width,
            border_color,
            clip_rect,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn push_text(
        &mut self,
        id: impl Into<String>,
        z_index: i32,
        rect: RectPx,
        text: impl Into<String>,
        font_role: FontRole,
        font_size_px: f32,
        weight: u16,
        color: [f32; 4],
        align: TextAlign,
        clip_rect: Option<RectPx>,
    ) {
        self.push_text_spaced(
            id,
            z_index,
            rect,
            text,
            font_role,
            font_size_px,
            weight,
            0.0,
            color,
            align,
            clip_rect,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn push_text_spaced(
        &mut self,
        id: impl Into<String>,
        z_index: i32,
        rect: RectPx,
        text: impl Into<String>,
        font_role: FontRole,
        font_size_px: f32,
        weight: u16,
        letter_spacing_px: f32,
        color: [f32; 4],
        align: TextAlign,
        clip_rect: Option<RectPx>,
    ) {
        self.text_runs.push(UiTextRun {
            id: id.into(),
            z_index,
            rect,
            text: text.into(),
            font_role,
            font_size_px,
            line_height_px: (font_size_px * 1.25).round(),
            weight,
            letter_spacing_px,
            color,
            align,
            allow_wrap: false,
            clip_rect,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn push_text_wrapped(
        &mut self,
        id: impl Into<String>,
        z_index: i32,
        rect: RectPx,
        text: impl Into<String>,
        font_role: FontRole,
        font_size_px: f32,
        weight: u16,
        color: [f32; 4],
        align: TextAlign,
        clip_rect: Option<RectPx>,
    ) {
        self.text_runs.push(UiTextRun {
            id: id.into(),
            z_index,
            rect,
            text: text.into(),
            font_role,
            font_size_px,
            line_height_px: (font_size_px * 1.35).round(),
            weight,
            letter_spacing_px: 0.0,
            color,
            align,
            allow_wrap: true,
            clip_rect,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn push_svg(
        &mut self,
        id: impl Into<String>,
        z_index: i32,
        svg: SvgId,
        variant: SvgVariant,
        rect: RectPx,
        tint_rgba: [u8; 4],
        clip_rect: Option<RectPx>,
    ) {
        self.svg_quads.push(UiSvgQuad {
            id: id.into(),
            z_index,
            svg,
            variant,
            rect,
            tint_rgba,
            opacity: 1.0,
            rotation_deg: 0.0,
            clip_rect,
        });
    }

    fn push_hit(&mut self, target: HitTarget, z_index: i32, rect: RectPx, cursor: CursorHint) {
        self.hit_regions.push(HitRegion {
            target,
            z_index,
            rect,
            cursor,
        });
    }
}

#[inline]
fn is_hovered(state: &UiState, target: &HitTarget) -> bool {
    state.hovered_target.as_ref() == Some(target)
}

#[inline]
fn parse_hex_rgba(hex: &str) -> Option<[f32; 4]> {
    parse_hex_rgb_u8(hex)
        .map(|[r, g, b]| [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
}

#[inline]
fn with_alpha(c: [f32; 4], alpha: f32) -> [f32; 4] {
    [c[0], c[1], c[2], alpha]
}

/// Computes the complete `UiFrameOutput` for one frame using `taffy` flexbox/box layout.
pub fn build_ui_frame(state: &UiState) -> UiFrameOutput {
    let w = state.viewport_w.max(320.0);
    let h = state.viewport_h.max(240.0);
    let bp = BreakpointState::evaluate(w, h, state.playlist.open);
    let theme = state.theme_tokens();
    let mut b = FrameBuilder::new(state.hovered_target.clone());

    let raw_pw = bp.playlist_width();
    let playlist_eased = cubic_bezier(0.16, 1.0, 0.3, 1.0, state.anim.playlist_t);
    let playlist_w = if state.playlist.open {
        raw_pw * playlist_eased
    } else if state.anim.playlist_closing && state.anim.playlist_t < 1.0 {
        raw_pw * (1.0 - playlist_eased)
    } else {
        0.0
    };
    let main_w = (w - playlist_w).max(0.0);

    // 1. Welcome Shader Backdrop (full window behind UI when welcome screen active)
    if state.welcome_active && state.welcome_background != WelcomeBackgroundKind::None {
        b.shader_slots.push(UiShaderSlot {
            id: "welcome-shader-backdrop".to_string(),
            z_index: ZIndex::ACTIVE_CANVAS,
            kind: state.welcome_background,
            rect: RectPx::new(0.0, 0.0, w, h),
            is_preview: false,
            opacity: 1.0,
            clip_rect: None,
        });
    }

    // 2. Titlebar (`height: 36px`, `right: playlist_w` when playlist open, `z: 1400`)
    // Matches `main.css:180-183, 264-269, 434-455, 1095-1100`: hidden when locked, always visible
    // when playlist open, and synced with `auto_hide_controls` + fullscreen/maximized slide-up.
    if !state.locked {
        let tb_hidden = state.auto_hide_controls && !state.playlist.open;
        if !tb_hidden || state.anim.controls_t > 0.005 {
            let tb_cp = b.checkpoint();
            build_titlebar(&mut b, state, &bp, &theme, main_w);
            if tb_hidden && state.anim.controls_t < 0.999 {
                let tb_alpha = state.anim.controls_t.clamp(0.0, 1.0);
                let slide_y = if matches!(
                    state.window_maximize_state,
                    WindowMaximizeState::Fullscreen | WindowMaximizeState::Maximized
                ) {
                    -(1.0 - tb_alpha) * TITLEBAR_HEIGHT
                } else {
                    0.0
                };
                b.apply_subtree_anim(
                    tb_cp,
                    main_w * 0.5,
                    0.0,
                    KeyframeSample {
                        opacity: tb_alpha,
                        translate_y: slide_y,
                        ..Default::default()
                    },
                    false,
                );
                b.hit_regions.truncate(tb_cp.hit);
            }
        }
    }

    // 3. Welcome Screen (`z: 15`, hidden `.welcome-content` when BG Settings drawer open per `main.css:2983-2986`)
    // Float playlist sidebar over welcome screen without shrinking `welcome-content` (`main.css:2405-2435`).
    if state.welcome_active && state.active_modal != Some(ModalId::BgSettings) {
        build_welcome_screen(&mut b, state, &bp, &theme, w, h);
    }

    // 4. Control Bar & Progress Bar (`z: 1350`, or Unlock Pill `z: 56` when locked)
    // Control bar visible on BOTH welcome and playback (golden PNGs confirm welcome shows bar).
    if state.locked {
        if state.show_unlock_button {
            let rect = RectPx::new((main_w - 48.0) * 0.5, h - 72.0, 48.0, 48.0);
            b.push_glass(
                "unlock-overlay",
                ZIndex::UNLOCK_OVERLAY,
                rect,
                RootTokens::RADIUS_FULL,
                None,
                ChameleonSkin::None,
                &theme,
                None,
            );
            b.push_svg(
                "unlock-icon",
                ZIndex::UNLOCK_OVERLAY,
                SvgId::UnlockPadlock,
                SvgVariant::Default,
                RectPx::new(rect.x + 14.0, rect.y + 14.0, 20.0, 20.0),
                rgba_f32_to_u8(theme.text_primary),
                None,
            );
            b.push_hit(
                HitTarget::UnlockControls,
                ZIndex::UNLOCK_OVERLAY,
                rect,
                CursorHint::Pointer,
            );
        }
    } else if !state.auto_hide_controls || state.anim.controls_t > 0.005 {
        let ctrl_cp = b.checkpoint();
        build_controls_bar(&mut b, state, &bp, &theme, main_w, h);
        if state.auto_hide_controls && state.anim.controls_t >= 0.999 {
            // Unanimated direct test override: keep hidden
        } else if state.anim.controls_t < 0.999 {
            let ctrl_alpha = state.anim.controls_t.clamp(0.0, 1.0);
            b.apply_subtree_anim(
                ctrl_cp,
                main_w * 0.5,
                h,
                KeyframeSample {
                    opacity: ctrl_alpha,
                    translate_y: (1.0 - ctrl_alpha) * 12.0,
                    ..Default::default()
                },
                false,
            );
            if state.auto_hide_controls {
                b.hit_regions.truncate(ctrl_cp.hit);
            }
        }
    }

    // 5. Playlist Sidebar (`z: 1800`, width `320px` or `min(320, w)`, `40px` stride, `>80` virtualization)
    let playlist_slice =
        if state.playlist.open || (state.anim.playlist_closing && state.anim.playlist_t < 1.0) {
            let pl_cp = b.checkpoint();
            let slice = build_playlist_sidebar(&mut b, state, &bp, &theme, w, h);
            let slide_x = if state.playlist.open {
                (1.0 - playlist_eased) * raw_pw
            } else {
                playlist_eased * raw_pw
            };
            if slide_x.abs() > 0.05 {
                b.apply_subtree_anim(
                    pl_cp,
                    w - raw_pw,
                    0.0,
                    KeyframeSample {
                        translate_x: slide_x,
                        ..Default::default()
                    },
                    false,
                );
            }
            if state.anim.playlist_closing {
                b.hit_regions.truncate(pl_cp.hit);
            }
            slice
        } else {
            VirtualSlice::default()
        };

    // 6. OSDs, Recording Indicator (burst vs flush bezel tab), Screenshot Preview / Carousel, Toasts, Drop
    build_overlays(&mut b, state, &bp, &theme, w, h, playlist_w);

    // 7. All 7 Modals + `#bgAccordionPopover` (`z: 2000` / `2100`)
    if let Some(modal) =
        state
            .active_modal
            .or(if state.anim.modal_closing && state.anim.modal_t < 1.0 {
                state.closing_modal
            } else {
                None
            })
    {
        build_modal(&mut b, state, &bp, &theme, w, h, playlist_w, modal);
    }

    // 7b. Floating select dropdowns & interactive color picker popovers
    build_floating_popovers(&mut b, state, &theme, w, h);

    // 7c. Keyboard :focus-visible outline ring & delayed hover tooltip pill
    build_focus_ring_and_tooltip(&mut b, state, &theme, w, h);

    // 8. AccessKit accessibility tree (respecting modal inertness & hit bounds)
    let a11y_tree = build_accessibility_tree(state, &b.hit_regions, &playlist_slice);

    // Ensure consistent font config usage (tracked in state)
    let _ = FontConfig::default();

    UiFrameOutput {
        clear_context: state.clear_context(),
        breakpoints: bp,
        playlist_slice,
        shader_slots: b.shader_slots,
        glass_panels: b.glass_panels,
        solid_rects: b.solid_rects,
        image_slots: b.image_slots,
        text_runs: b.text_runs,
        svg_quads: b.svg_quads,
        hit_regions: b.hit_regions,
        a11y_tree,
    }
}

/// Returns the highest-z-index `HitRegion` containing `(x, y)` (top-most modal/overlay wins).
pub fn hit_test_frame(frame: &UiFrameOutput, x: f32, y: f32) -> Option<&HitRegion> {
    frame
        .hit_regions
        .iter()
        .enumerate()
        .filter(|(_, region)| region.rect.contains(x, y))
        .max_by_key(|(idx, region)| (region.z_index, *idx))
        .map(|(_, region)| region)
}

fn build_titlebar(
    b: &mut FrameBuilder,
    state: &UiState,
    bp: &BreakpointState,
    theme: &ThemeTokens,
    main_w: f32,
) {
    let mut taffy = TaffyTree::<()>::new();
    let btn_w = bp.titlebar_btn_width();

    let brand_node = taffy
        .new_leaf(Style {
            size: Size {
                width: length(20.0),
                height: length(20.0),
            },
            margin: Rect {
                left: length(14.0),
                right: length(10.0),
                top: zero(),
                bottom: zero(),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .unwrap();

    let title_measured_w = measure_text_width(
        &state.window_title,
        FontRole::Display,
        12.0,
        600,
        0.3,
        state.font_config,
    );
    let max_title_w =
        (main_w - 44.0 - btn_w * 3.0 - if state.is_live_stream { 58.0 } else { 12.0 }).max(40.0);
    let title_node = taffy
        .new_leaf(Style {
            flex_grow: if state.is_live_stream { 0.0 } else { 1.0 },
            flex_shrink: 1.0,
            size: Size {
                width: if state.is_live_stream {
                    length(title_measured_w.min(max_title_w))
                } else {
                    auto()
                },
                height: length(20.0),
            },
            min_size: Size {
                width: length(40.0),
                height: length(20.0),
            },
            ..Default::default()
        })
        .unwrap();

    let live_node = if state.is_live_stream {
        Some(
            taffy
                .new_leaf(Style {
                    size: Size {
                        width: length(44.0),
                        height: length(18.0),
                    },
                    margin: Rect {
                        left: length(8.0),
                        right: length(8.0),
                        top: zero(),
                        bottom: zero(),
                    },
                    flex_shrink: 0.0,
                    ..Default::default()
                })
                .unwrap(),
        )
    } else {
        None
    };

    let live_spacer_node = if state.is_live_stream {
        Some(
            taffy
                .new_leaf(Style {
                    flex_grow: 1.0,
                    ..Default::default()
                })
                .unwrap(),
        )
    } else {
        None
    };

    let mut win_btns = Vec::new();
    for _ in 0..3 {
        win_btns.push(
            taffy
                .new_leaf(Style {
                    size: Size {
                        width: length(btn_w),
                        height: length(TITLEBAR_HEIGHT),
                    },
                    flex_shrink: 0.0,
                    ..Default::default()
                })
                .unwrap(),
        );
    }

    let mut children = vec![brand_node, title_node];
    if let Some(ln) = live_node {
        children.push(ln);
    }
    if let Some(sn) = live_spacer_node {
        children.push(sn);
    }
    children.extend_from_slice(&win_btns);

    let root = taffy
        .new_with_children(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::CENTER,
                size: Size {
                    width: length(main_w),
                    height: length(TITLEBAR_HEIGHT),
                },
                ..Default::default()
            },
            &children,
        )
        .unwrap();

    taffy
        .compute_layout(
            root,
            Size {
                width: AvailableSpace::Definite(main_w),
                height: AvailableSpace::Definite(TITLEBAR_HEIGHT),
            },
        )
        .unwrap();

    let tb_colors = theme.titlebar_colors(state.welcome_active, state.active_modal.is_some());
    let over_video_chrome = !state.welcome_active && state.active_modal.is_none();
    // Electron keeps the titlebar fully transparent over video (`main.css:155-163`); only the text
    // and icon shadows below provide legibility. Do not paint a full-width dark scrim here.

    // Drag region stops before window buttons (`.titlebar` has `background: transparent` per main.css:160)
    let drag_w = (main_w - btn_w * 3.0).max(0.0);
    b.push_hit(
        HitTarget::TitlebarDrag,
        ZIndex::TITLEBAR,
        RectPx::new(0.0, 0.0, drag_w, TITLEBAR_HEIGHT),
        CursorHint::Default,
    );

    let brand_l = taffy.layout(brand_node).unwrap();
    b.push_svg(
        "titlebar-brand-mark",
        ZIndex::TITLEBAR,
        SvgId::BrandMarkTitlebar,
        SvgVariant::Default,
        RectPx::new(brand_l.location.x, brand_l.location.y, 20.0, 20.0),
        rgba_f32_to_u8(theme.accent),
        None,
    );

    let title_l = taffy.layout(title_node).unwrap();
    let title_rect = RectPx::new(
        title_l.location.x,
        title_l.location.y,
        title_l.size.width,
        20.0,
    );
    if tb_colors.has_text_shadow {
        b.push_text_spaced(
            "titlebar-title-shadow",
            ZIndex::TITLEBAR,
            RectPx::new(
                title_rect.x,
                title_rect.y + 1.0,
                title_rect.width,
                title_rect.height,
            ),
            state.window_title.clone(),
            FontRole::Display,
            12.0,
            600,
            0.3,
            [0.0, 0.0, 0.0, 0.75],
            TextAlign::Left,
            None,
        );
    }
    b.push_text_spaced(
        "titlebar-title",
        ZIndex::TITLEBAR,
        title_rect,
        state.window_title.clone(),
        FontRole::Display,
        12.0,
        600,
        0.3,
        tb_colors.title_fg,
        TextAlign::Left,
        None,
    );

    if let Some(ln) = live_node {
        let ll = taffy.layout(ln).unwrap();
        let lrect = RectPx::new(ll.location.x, ll.location.y, ll.size.width, ll.size.height);
        b.push_solid(
            "titlebar-live-badge",
            ZIndex::TITLEBAR,
            lrect,
            rgba(239, 68, 68, 0.16),
            [4.0; 4],
            1.0,
            rgba(239, 68, 68, 0.55),
            None,
        );
        b.push_text(
            "titlebar-live-text",
            ZIndex::TITLEBAR,
            lrect,
            "LIVE",
            FontRole::Display,
            10.0,
            700,
            rgba8(0xf8, 0x71, 0x71, 255),
            TextAlign::Center,
            None,
        );
    }

    let max_svg = match state.window_maximize_state {
        WindowMaximizeState::Restored => SvgId::TitlebarMaximizeSquare,
        WindowMaximizeState::Maximized => SvgId::TitlebarRestoreStacked,
        WindowMaximizeState::Fullscreen => SvgId::TitlebarExitFullscreenMonitor,
    };
    let btn_specs = [
        (
            "btnMinimize",
            SvgId::TitlebarMinimize,
            HitTarget::TitlebarMinimize,
        ),
        ("btnMaximize", max_svg, HitTarget::TitlebarMaximize),
        ("btnClose", SvgId::TitlebarClose, HitTarget::TitlebarClose),
    ];
    let btn_tint = rgba_f32_to_u8(tb_colors.btn_fg);
    for (i, (id, svg, target)) in btn_specs.into_iter().enumerate() {
        let bl = taffy.layout(win_btns[i]).unwrap();
        let rect = RectPx::new(bl.location.x, bl.location.y, bl.size.width, bl.size.height);
        let hovered = is_hovered(state, &target);
        if hovered {
            let hover_bg = if target == HitTarget::TitlebarClose {
                rgba8(0xe8, 0x11, 0x23, 255)
            } else if over_video_chrome {
                rgba(0, 0, 0, 0.52)
            } else {
                rgba(255, 255, 255, 0.16)
            };
            b.push_solid(
                format!("{id}-hover"),
                ZIndex::TITLEBAR,
                rect,
                hover_bg,
                [0.0; 4],
                0.0,
                [0.0; 4],
                None,
            );
        }
        let svg_rect = RectPx::new(
            rect.x + (rect.width - 12.0) * 0.5,
            rect.y + 12.0,
            12.0,
            12.0,
        );
        // Parity with Electron `.titlebar-btn` (`main.css:271-299`): the button surface is
        // transparent at rest, so no rounded dark backing is drawn here. Legibility over bright
        // video comes from the icon drop shadow below.
        if tb_colors.has_text_shadow && !(hovered && target == HitTarget::TitlebarClose) {
            b.push_svg(
                format!("{id}-shadow"),
                ZIndex::TITLEBAR,
                svg,
                SvgVariant::Default,
                RectPx::new(
                    svg_rect.x,
                    svg_rect.y + 1.0,
                    svg_rect.width,
                    svg_rect.height,
                ),
                [0, 0, 0, 200],
                None,
            );
        }
        let icon_tint = if hovered && target == HitTarget::TitlebarClose {
            [255, 255, 255, 255]
        } else {
            btn_tint
        };
        b.push_svg(
            id,
            ZIndex::TITLEBAR,
            svg,
            SvgVariant::Default,
            svg_rect,
            icon_tint,
            None,
        );
        b.push_hit(target, ZIndex::TITLEBAR, rect, CursorHint::Default);
    }
}

fn build_welcome_screen(
    b: &mut FrameBuilder,
    state: &UiState,
    bp: &BreakpointState,
    theme: &ThemeTokens,
    main_w: f32,
    h: f32,
) {
    let mut taffy = TaffyTree::<()>::new();
    // Content area starts below titlebar
    let content_h = (h - TITLEBAR_HEIGHT).max(100.0);
    // Bottom padding matches main.css `.welcome-content`: 96px normal, 88px at max-h: 620px
    let pad_bottom = if bp.max_h_620 { 88.0 } else { 96.0 };
    // Gap between buttons row and recent pill: 12px normal, 8px at max-h: 620px
    let actions_gap = if bp.max_h_620 { 8.0 } else { 12.0 };
    // Button geometry: Open File `151x43` (`main.css:2522`, `130x37` at `max-h: 620`),
    // Open Link `142x45.5` (`main.css:2351,2510`, `125x40` at `max-h: 620`).
    let (btn_w, btn_h, link_btn_w, link_btn_h, btn_font) = if bp.max_h_620 {
        (130.0, 37.0, 125.0, 40.0, 14.0)
    } else {
        (151.0, 43.0, 142.0, 45.5, 15.0)
    };

    let open_file_node = taffy
        .new_leaf(Style {
            size: Size {
                width: length(btn_w),
                height: length(btn_h),
            },
            ..Default::default()
        })
        .unwrap();
    let open_link_node = taffy
        .new_leaf(Style {
            size: Size {
                width: length(link_btn_w),
                height: length(link_btn_h),
            },
            ..Default::default()
        })
        .unwrap();
    let actions_row = taffy
        .new_with_children(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::CENTER,
                gap: Size {
                    width: length(10.0),
                    height: zero(),
                },
                flex_shrink: 0.0,
                ..Default::default()
            },
            &[open_file_node, open_link_node],
        )
        .unwrap();

    // Recent files pill (`index.html:326`, `main.css:2377-2390`): the container stays in flow
    // with `padding: 12px` (`10px` at max-height: 620px) + 1px border even when empty,
    // offsetting `.welcome-actions` upward via `.welcome-content`'s gap.
    let recent_count = state.recent_files.len().min(5);
    let recent_w = (main_w - 12.0).clamp(220.0, 440.0);
    let rec_pad = if bp.max_h_620 { 10.0 } else { 12.0 };
    let max_recent_h = if bp.max_h_620 {
        (h * 0.26).max(60.0)
    } else {
        (h * 0.32).min((h - 260.0).max(60.0)).max(60.0)
    };
    let recent_h = if recent_count > 0 {
        let content_h_rec = rec_pad * 2.0
            + 24.0
            + (recent_count as f32) * 34.0
            + ((recent_count.saturating_sub(1)) as f32) * 6.0;
        content_h_rec.min(max_recent_h)
    } else {
        rec_pad * 2.0 + 2.0
    };
    let recent_node = Some(
        taffy
            .new_leaf(Style {
                size: Size {
                    width: length(recent_w),
                    height: length(recent_h),
                },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .unwrap(),
    );

    // Root: flex-col, align-center, justify FLEX_END (bottom), gap & padding-bottom match CSS
    let children_vec: Vec<_> = if let Some(rn) = recent_node {
        vec![actions_row, rn]
    } else {
        vec![actions_row]
    };
    let root = taffy
        .new_with_children(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::CENTER,
                justify_content: JustifyContent::FLEX_END,
                gap: Size {
                    width: length(0.0),
                    height: length(actions_gap),
                },
                size: Size {
                    width: length(main_w),
                    height: length(content_h),
                },
                padding: Rect {
                    bottom: length(pad_bottom),
                    ..Rect::zero()
                },
                ..Default::default()
            },
            &children_vec,
        )
        .unwrap();

    taffy
        .compute_layout(
            root,
            Size {
                width: AvailableSpace::Definite(main_w),
                height: AvailableSpace::Definite(content_h),
            },
        )
        .unwrap();

    let y_off = TITLEBAR_HEIGHT;
    let row_l = taffy.layout(actions_row).unwrap();
    let row_rect = RectPx::new(
        row_l.location.x,
        y_off + row_l.location.y,
        row_l.size.width,
        row_l.size.height,
    );
    b.push_hit(
        HitTarget::WelcomeContentSurface,
        ZIndex::WELCOME_SCREEN,
        row_rect,
        CursorHint::Default,
    );
    let of_l = taffy.layout(open_file_node).unwrap();
    let of_rect = RectPx::new(
        row_l.location.x + of_l.location.x,
        y_off + row_l.location.y + of_l.location.y,
        of_l.size.width,
        of_l.size.height,
    );
    let of_hover = is_hovered(state, &HitTarget::OpenFile);
    let of_icon_y = of_rect.y + (of_rect.height - 18.0) * 0.5;
    let of_txt_y = of_rect.y + (of_rect.height - 20.0) * 0.5;
    let of_lbl_w = 68.0 * (btn_font / 15.0);
    let of_start_x = of_rect.x + (of_rect.width - (18.0 + 8.0 + of_lbl_w)) * 0.5;
    b.push_solid(
        "btnOpenFile-bg",
        ZIndex::WELCOME_SCREEN,
        of_rect,
        if of_hover {
            theme.accent_hover
        } else {
            theme.accent
        },
        [12.0; 4],
        0.0,
        [0.0; 4],
        None,
    );
    b.push_svg(
        "btnOpenFile-icon",
        ZIndex::WELCOME_SCREEN,
        SvgId::WelcomeOpenFile,
        SvgVariant::Default,
        RectPx::new(of_start_x, of_icon_y, 18.0, 18.0),
        [255; 4],
        None,
    );
    b.push_text(
        "btnOpenFile-label",
        ZIndex::WELCOME_SCREEN,
        RectPx::new(of_start_x + 26.0, of_txt_y, of_lbl_w + 16.0, 20.0),
        "Open File",
        FontRole::Ui,
        btn_font,
        600,
        [1.0; 4],
        TextAlign::Left,
        None,
    );
    b.push_hit(
        HitTarget::OpenFile,
        ZIndex::WELCOME_SCREEN,
        of_rect,
        CursorHint::Pointer,
    );

    let ol_l = taffy.layout(open_link_node).unwrap();
    let ol_rect = RectPx::new(
        row_l.location.x + ol_l.location.x,
        y_off + row_l.location.y + ol_l.location.y,
        ol_l.size.width,
        ol_l.size.height,
    );
    // `.btn-open-link` (`main.css:2351-2375`): always navy translucent gradient + ice-blue border/text
    let ol_hover = is_hovered(state, &HitTarget::OpenLink);
    b.push_gradient(
        "btnOpenLink-bg",
        ZIndex::WELCOME_SCREEN,
        ol_rect,
        if ol_hover {
            rgba(16, 32, 64, 0.96)
        } else {
            rgba(11, 26, 53, 0.90)
        },
        if ol_hover {
            rgba(11, 22, 45, 0.95)
        } else {
            rgba(9, 18, 36, 0.92)
        },
        180.0,
        [12.0; 4],
        1.0,
        if ol_hover {
            rgba(167, 214, 255, 0.52)
        } else {
            rgba(144, 188, 255, 0.36)
        },
        None,
    );
    let ol_fg = rgba(238, 246, 255, 0.96);
    let ol_icon_y = ol_rect.y + (ol_rect.height - 18.0) * 0.5;
    let ol_txt_y = ol_rect.y + (ol_rect.height - 20.0) * 0.5;
    let ol_lbl_w = 72.0 * (btn_font / 15.0);
    let ol_start_x = ol_rect.x + (ol_rect.width - (18.0 + 8.0 + ol_lbl_w)) * 0.5;
    b.push_svg(
        "btnOpenLink-icon",
        ZIndex::WELCOME_SCREEN,
        SvgId::WelcomeOpenLink,
        SvgVariant::Default,
        RectPx::new(ol_start_x, ol_icon_y, 18.0, 18.0),
        rgba_f32_to_u8(ol_fg),
        None,
    );
    b.push_text(
        "btnOpenLink-label",
        ZIndex::WELCOME_SCREEN,
        RectPx::new(ol_start_x + 26.0, ol_txt_y, ol_lbl_w + 16.0, 20.0),
        "Open Link",
        FontRole::Ui,
        btn_font,
        600,
        ol_fg,
        TextAlign::Left,
        None,
    );
    b.push_hit(
        HitTarget::OpenLink,
        ZIndex::WELCOME_SCREEN,
        ol_rect,
        CursorHint::Pointer,
    );

    // Recent files pill (hidden when no recent files)
    if let Some(recent_node) = recent_node {
        let rec_l = taffy.layout(recent_node).unwrap();
        let rec_box = RectPx::new(
            rec_l.location.x,
            y_off + rec_l.location.y,
            rec_l.size.width,
            rec_l.size.height,
        );
        // Outer capsule (`main.css:2386-2389`): var(--bg-glass-heavy), blur(8px), border 1px rgba(255,255,255,0.08)
        let mut rec_glass = GlassRect::new(
            rec_box.x,
            rec_box.y,
            rec_box.width,
            rec_box.height,
            14.0,
            theme.mode,
        )
        .with_backdrop_rule(CssBackdropRule::WelcomeRecent, theme.mode);
        let rec_border = if theme.mode == hybrid_shaders::ThemeMode::Light {
            rgba(0, 0, 0, 0.08)
        } else {
            rgba(255, 255, 255, 0.08)
        };
        rec_glass.border_color_0 = rec_border;
        rec_glass.border_color_1 = rec_border;
        rec_glass.border_color_2 = rec_border;
        rec_glass.shadow_blur = 0.0;
        rec_glass.shadow_color = [0.0; 4];
        b.push_custom_glass(
            "welcome-recent-bg",
            ZIndex::WELCOME_SCREEN,
            rec_glass,
            Some(CssBackdropRule::WelcomeRecent),
            None,
        );
        b.push_hit(
            HitTarget::WelcomeContentSurface,
            ZIndex::WELCOME_SCREEN,
            rec_box,
            CursorHint::Default,
        );
        let is_light = theme.mode == hybrid_shaders::ThemeMode::Light;
        if recent_count > 0 {
            let heading_col = if is_light {
                rgba(26, 26, 46, 0.88)
            } else {
                rgba(234, 240, 255, 0.88)
            };
            b.push_text(
                "recent-header",
                ZIndex::WELCOME_SCREEN,
                RectPx::new(
                    rec_box.x + rec_pad,
                    rec_box.y + rec_pad,
                    (rec_box.width - rec_pad * 2.0).max(40.0),
                    16.0,
                ),
                "RECENTLY PLAYED",
                FontRole::Ui,
                12.0,
                700,
                heading_col,
                TextAlign::Left,
                Some(rec_box),
            );
            for (i, item) in state.recent_files.iter().take(recent_count).enumerate() {
                let row_rect = RectPx::new(
                    rec_box.x + rec_pad,
                    rec_box.y + rec_pad + 24.0 + (i as f32) * 40.0,
                    (rec_box.width - rec_pad * 2.0).max(40.0),
                    34.0,
                );
                if row_rect.y + row_rect.height > rec_box.y + rec_box.height + 0.5 {
                    break;
                }
                let row_hover = is_hovered(state, &HitTarget::RecentFile(i));
                let row_bg = match (is_light, row_hover) {
                    (true, false) => rgba(0, 0, 0, 0.04),
                    (true, true) => rgba(0, 0, 0, 0.08),
                    (false, false) => rgba(255, 255, 255, 0.03),
                    (false, true) => rgba(255, 255, 255, 0.11),
                };
                let row_fg = match (is_light, row_hover) {
                    (true, false) => rgba8(0x1a, 0x1a, 0x2e, 255),
                    (true, true) => [0.0, 0.0, 0.0, 1.0],
                    (false, false) => rgba(236, 241, 255, 0.90),
                    (false, true) => [1.0, 1.0, 1.0, 1.0],
                };
                b.push_solid(
                    format!("recent-item-bg-{i}"),
                    ZIndex::WELCOME_SCREEN,
                    row_rect,
                    row_bg,
                    [8.0; 4],
                    0.0,
                    [0.0; 4],
                    Some(rec_box),
                );
                b.push_svg(
                    format!("recent-play-{i}"),
                    ZIndex::WELCOME_SCREEN,
                    SvgId::WelcomeRecentPlay,
                    SvgVariant::Default,
                    RectPx::new(row_rect.x + 12.0, row_rect.y + 9.0, 16.0, 16.0),
                    rgba_f32_to_u8(row_fg),
                    Some(rec_box),
                );
                b.push_text(
                    format!("recent-title-{i}"),
                    ZIndex::WELCOME_SCREEN,
                    RectPx::new(
                        row_rect.x + 38.0,
                        row_rect.y + 7.0,
                        row_rect.width - 48.0,
                        20.0,
                    ),
                    item.name.clone(),
                    FontRole::Ui,
                    13.0,
                    500,
                    row_fg,
                    TextAlign::Left,
                    Some(rec_box),
                );
                b.push_hit(
                    HitTarget::RecentFile(i),
                    ZIndex::WELCOME_SCREEN,
                    row_rect,
                    CursorHint::Pointer,
                );
            }
        }
    }
}

fn control_svg_dimensions(svg: SvgId, btn_size: f32, max_w_1000: bool) -> (f32, f32) {
    let (base_w, base_h) = if max_w_1000 {
        match svg {
            SvgId::CtrlPlay | SvgId::CtrlPause => (24.0, 24.0),
            SvgId::CtrlRepeat => (20.0, 10.0),
            _ => (18.0, 18.0),
        }
    } else {
        match svg {
            SvgId::CtrlPlay | SvgId::CtrlPause => (32.0, 32.0),
            SvgId::CtrlRepeat => (24.0, 12.0),
            SvgId::CtrlPlaylist | SvgId::CtrlShuffle | SvgId::CtrlAbLoop => (20.0, 20.0),
            _ => (22.0, 22.0),
        }
    };
    let max_fit = (btn_size - 2.0).max(12.0);
    if base_w > max_fit || base_h > max_fit {
        let s = (max_fit / base_w.max(base_h)).min(1.0);
        ((base_w * s).round(), (base_h * s).round())
    } else {
        (base_w, base_h)
    }
}

fn build_controls_bar(
    b: &mut FrameBuilder,
    state: &UiState,
    bp: &BreakpointState,
    theme: &ThemeTokens,
    main_w: f32,
    h: f32,
) {
    let pad_x = if bp.max_w_1000 { 8.0 } else { 14.0 };
    let pad_bottom = if bp.max_w_1000 { 8.0 } else { 12.0 };
    let bar_pad_x = if bp.max_w_1150_playlist {
        6.0
    } else if bp.max_w_760 {
        8.0
    } else {
        0.0
    };
    let bar_pad_y = if bp.max_w_1150_playlist || bp.max_w_760 {
        4.0
    } else {
        0.0
    };
    let content_x = pad_x + bar_pad_x;
    let bar_h = if bp.max_w_1150_playlist {
        28.0
    } else if bp.max_w_760 {
        32.0
    } else if bp.max_w_1000 {
        38.0
    } else {
        44.0
    };
    let bar_top_y = h - (bar_h + bar_pad_y * 2.0) - pad_bottom;
    let bar_y = bar_top_y + bar_pad_y;
    let prog_hover = is_hovered(state, &HitTarget::ProgressSeek)
        || matches!(state.hovered_target, Some(HitTarget::ChapterMarker(_)))
        || state.is_dragging_progress;
    let prog_h = if prog_hover { 6.0 } else { 4.0 };
    let dpr = state.dpr.max(1.0);
    let prog_y = ((bar_top_y - 10.0 - 4.0) * dpr).round() / dpr;
    // `.controls-wrapper` (`controls.css:16`): `padding: 0 14px 12px` (0 top padding)
    let wrapper_y = prog_y;
    let wrapper_h = (h - wrapper_y).max(52.0);

    // Multi-stop `theme.bg_controls` gradient backdrop (`controls.css:17`, `themes.css`)
    // Rendered via `UiQuadPass` (`push_gradient`) in strict Z-order without X-raying underlying panes.
    let mid_stop = 1.0 - theme.bg_controls.mid.map(|(_, s)| s).unwrap_or(0.5);
    let mid_y = (wrapper_y + wrapper_h * mid_stop).round();
    let mid_col = theme
        .bg_controls
        .mid
        .map(|(c, _)| c)
        .unwrap_or(theme.bg_controls.start);
    let top_rect = RectPx::new(0.0, wrapper_y, main_w, (mid_y - wrapper_y).max(1.0));
    let bot_rect = RectPx::new(0.0, mid_y, main_w, (h - mid_y).max(1.0));
    b.push_gradient(
        "controls-bar-backdrop-top",
        ZIndex::CONTROLS_BAR,
        top_rect,
        theme.bg_controls.end,
        mid_col,
        180.0,
        [0.0; 4],
        0.0,
        [0.0; 4],
        // No self-clip: the anti-aliased fragment clip would feather this rect's bottom edge and
        // the abutting bottom rect's top edge, leaving a 1px bright seam across the video.
        None,
    );
    b.push_gradient(
        "controls-bar-backdrop",
        ZIndex::CONTROLS_BAR,
        bot_rect,
        mid_col,
        theme.bg_controls.start,
        180.0,
        [0.0; 4],
        0.0,
        [0.0; 4],
        // See `controls-bar-backdrop-top`: self-clipping both halves creates a feathered seam.
        None,
    );

    let prog_w = (main_w - pad_x * 2.0).max(80.0);
    b.push_hit(
        HitTarget::ControlsBarSurface,
        ZIndex::CONTROLS_BAR,
        RectPx::new(pad_x, bar_top_y, prog_w, bar_h + bar_pad_y * 2.0),
        CursorHint::Default,
    );
    let prog_rect = RectPx::new(pad_x, prog_y - (prog_h - 4.0) * 0.5, prog_w, prog_h);

    // Progress track + buffer + played fill + handle + chapters + hover thumb
    b.push_solid(
        "progress-track",
        ZIndex::CONTROLS_WRAPPER,
        prog_rect,
        theme.progress_bg,
        [prog_h * 0.5; 4],
        0.0,
        [0.0; 4],
        None,
    );
    let dur = state.duration_secs.max(0.001);
    let buf_ratio = (state.buffered_secs / dur).clamp(0.0, 1.0) as f32;
    let play_ratio = if state.is_live_stream {
        1.0
    } else {
        (state.current_secs / dur).clamp(0.0, 1.0) as f32
    };
    b.push_solid(
        "progress-buffer",
        ZIndex::CONTROLS_WRAPPER,
        RectPx::new(prog_rect.x, prog_rect.y, prog_w * buf_ratio, prog_h),
        theme.progress_buffer,
        [prog_h * 0.5; 4],
        0.0,
        [0.0; 4],
        None,
    );
    b.push_solid(
        "progress-fill",
        ZIndex::CONTROLS_WRAPPER,
        RectPx::new(prog_rect.x, prog_rect.y, prog_w * play_ratio, prog_h),
        theme.accent,
        [prog_h * 0.5; 4],
        0.0,
        [0.0; 4],
        None,
    );
    if prog_hover {
        let hx = prog_rect.x + prog_w * play_ratio - 7.0;
        let hy = prog_rect.y + prog_h * 0.5 - 7.0;
        b.push_solid(
            "progress-handle",
            ZIndex::CONTROLS_WRAPPER + 1,
            RectPx::new(hx, hy, 14.0, 14.0),
            theme.accent,
            [7.0; 4],
            0.0,
            [0.0; 4],
            None,
        );
    }
    b.push_hit(
        HitTarget::ProgressSeek,
        ZIndex::CONTROLS_WRAPPER,
        RectPx::new(prog_rect.x, prog_rect.y - 6.0, prog_w, 18.0),
        CursorHint::Pointer,
    );

    for (i, ch) in state.chapter_positions_norm.iter().enumerate() {
        let cx = prog_rect.x + prog_w * ch.clamp(0.0, 1.0);
        let ch_target = HitTarget::ChapterMarker(i);
        let ch_active = state.current_chapter == Some(i) || is_hovered(state, &ch_target);
        b.push_solid(
            format!("chapter-marker-{i}"),
            ZIndex::CONTROLS_WRAPPER + 2,
            RectPx::new(cx - 1.0, prog_rect.y - 1.0, 2.0, prog_h + 2.0),
            if ch_active {
                theme.accent
            } else {
                rgba(255, 255, 255, 0.4)
            },
            [1.0; 4],
            0.0,
            [0.0; 4],
            None,
        );
        b.push_hit(
            ch_target,
            ZIndex::CONTROLS_WRAPPER + 2,
            RectPx::new(cx - 4.0, prog_rect.y - 4.0, 8.0, prog_h + 8.0),
            CursorHint::Pointer,
        );
    }

    if let Some(hover_norm) = state.hover_thumb_norm.filter(|_| {
        state.hover_thumb_has_frame && !state.is_live_stream && !state.is_dragging_progress
    }) {
        let hover_cp = b.checkpoint();
        let norm = hover_norm.clamp(0.0, 1.0);
        let hx = (prog_rect.x + prog_w * norm - 96.0).clamp(8.0, (main_w - 200.0).max(8.0));
        let thumb_rect = RectPx::new(hx, prog_rect.y - 120.0, 192.0, 108.0);
        b.push_glass(
            "progress-hover-thumb",
            ZIndex::SEEK_INDICATOR,
            thumb_rect,
            10.0,
            None,
            ChameleonSkin::None,
            theme,
            None,
        );
        b.image_slots.push(UiImageSlot {
            id: "progress-hover-thumb-img".to_string(),
            z_index: ZIndex::SEEK_INDICATOR,
            rect: thumb_rect,
            corner_radius: 10.0,
            opacity: if state.hover_thumb_loading { 0.62 } else { 1.0 },
            key: format!("hover-thumb:{norm:.6}"),
            clip_rect: None,
        });
        if state.hover_thumb_loading {
            b.push_solid(
                "progress-hover-thumb-dim",
                ZIndex::SEEK_INDICATOR,
                thumb_rect,
                rgba(8, 9, 11, 0.38),
                [10.0; 4],
                0.0,
                [0.0; 4],
                None,
            );
            let shimmer_w = thumb_rect.width * 0.45;
            let shimmer_x = thumb_rect.x - shimmer_w
                + state.hover_thumb_shimmer_phase * (thumb_rect.width + shimmer_w);
            b.push_solid(
                "progress-hover-thumb-shimmer",
                ZIndex::SEEK_INDICATOR,
                RectPx::new(shimmer_x, thumb_rect.y, shimmer_w, thumb_rect.height),
                rgba(255, 255, 255, 0.13),
                [0.0; 4],
                0.0,
                [0.0; 4],
                Some(thumb_rect),
            );
        }
        let hover_time_label = UiState::format_time(norm as f64 * state.duration_secs);
        let measured_w = measure_text_width(
            &hover_time_label,
            FontRole::Mono,
            11.0,
            600,
            0.0,
            state.font_config,
        );
        let badge_w = (measured_w + 14.0).clamp(46.0, thumb_rect.width - 12.0);
        let badge_r = RectPx::new(
            thumb_rect.x + (thumb_rect.width - badge_w) * 0.5,
            thumb_rect.y + thumb_rect.height - 27.0,
            badge_w,
            20.0,
        );
        b.push_solid(
            "progress-hover-thumb-time-bg",
            ZIndex::SEEK_INDICATOR + 1,
            badge_r,
            rgba(0, 0, 0, 0.88),
            [5.0; 4],
            1.0,
            rgba(255, 255, 255, 0.14),
            None,
        );
        b.push_text(
            "progress-hover-thumb-time",
            ZIndex::SEEK_INDICATOR + 1,
            badge_r,
            hover_time_label,
            FontRole::Mono,
            11.0,
            700,
            [1.0; 4],
            TextAlign::Center,
            None,
        );
        if state.anim.hover_thumb_t < 0.999 {
            let kf = evaluate_keyframe(
                KeyframeId::HoverThumbIn,
                MotionProfile::ease_emphasized(state.anim.hover_thumb_t),
                state.motion_profile,
            );
            b.apply_subtree_anim(
                hover_cp,
                thumb_rect.x + thumb_rect.width * 0.5,
                thumb_rect.y + thumb_rect.height,
                kf,
                false,
            );
        }
    }

    // Main controls bar using Taffy row layout
    let bar_w = (main_w - content_x * 2.0).max(200.0);
    let btn_sz = bp.ctrl_btn_size_for(state.welcome_active);
    let play_sz = bp.ctrl_play_btn_size_for(state.welcome_active);
    let base_vol_w = bp.volume_slider_width();
    let spring_tx = state.volume_spring.transforms(base_vol_w);
    let vol_w = (base_vol_w * spring_tx.scale_x).max(24.0);

    #[derive(Clone)]
    struct BtnItem {
        id: &'static str,
        svg: SvgId,
        variant: SvgVariant,
        target: HitTarget,
        size: f32,
        active: bool,
    }

    let mut pre_vol_items: Vec<BtnItem> = Vec::new();
    if bp.show_prev_next() {
        pre_vol_items.push(BtnItem {
            id: "btnPrev",
            svg: SvgId::CtrlPrev,
            variant: SvgVariant::Default,
            target: HitTarget::PrevTrack,
            size: btn_sz,
            active: false,
        });
    }
    pre_vol_items.push(BtnItem {
        id: "btnPlay",
        svg: if state.playing {
            SvgId::CtrlPause
        } else {
            SvgId::CtrlPlay
        },
        variant: SvgVariant::Default,
        target: HitTarget::PlayPause,
        size: play_sz,
        active: false,
    });
    if bp.show_prev_next() {
        pre_vol_items.push(BtnItem {
            id: "btnNext",
            svg: SvgId::CtrlNext,
            variant: SvgVariant::Default,
            target: HitTarget::NextTrack,
            size: btn_sz,
            active: false,
        });
    }
    let vol_svg = if state.muted || state.volume_pct == 0 {
        SvgId::CtrlVolumeMute
    } else if state.volume_pct < 50 {
        SvgId::CtrlVolumeLow
    } else {
        SvgId::CtrlVolumeHigh
    };
    let vol_btn_item = BtnItem {
        id: "btnVolume",
        svg: vol_svg,
        variant: SvgVariant::Default,
        target: HitTarget::MuteToggle,
        size: btn_sz,
        active: false,
    };

    let mut right_items: Vec<BtnItem> = Vec::new();
    // Matches `controls.js:1988-2002` + `controls.css:714`:
    // On welcome screen: `bgSettingsToggle` (hidden <=860px), `btnPlaylist`, `btnSettings`, `btnFullscreen`.
    // Unlike the Electron CSS (`body.playlist-open #bgSettingsToggle { display: none }`), keep the
    // welcome-effects button mounted while the playlist drawer is open so background settings
    // remain reachable with the queue on screen.
    // During playback: playback controls (`btnSpeed`, `btnEqualizer`, `btnAudioTrack`, `btnSubtitles`,
    // `btnRepeat`, `btnShuffle`, `btnABLoop`, `btnLock`, optional `btnYoutubeQuality`) + `btnPlaylist`, `btnSettings`, `btnFullscreen`.
    // Like `bgSettingsToggle`, the secondary playback toggles are intentionally NOT suppressed by
    // `body.playlist-open` (`controls.css:248-256`): repeat/shuffle/A-B loop/lock must stay reachable
    // with the queue open. They still hide at `<=860px` where the bar cannot fit them.
    if state.welcome_active && !bp.max_w_860 {
        right_items.push(BtnItem {
            id: "bgSettingsToggle",
            svg: SvgId::CtrlBgSparkles,
            variant: SvgVariant::Default,
            target: HitTarget::BgSettingsToggle,
            size: btn_sz,
            active: state.active_modal == Some(ModalId::BgSettings),
        });
    }
    if !state.welcome_active && bp.show_speed_btn() {
        right_items.push(BtnItem {
            id: "btnSpeed",
            svg: SvgId::CtrlSettings,
            variant: SvgVariant::Default,
            target: HitTarget::SpeedButton,
            size: btn_sz,
            active: (state.speed - 1.0).abs() > 0.01,
        });
    }
    if !state.welcome_active && bp.show_eq_and_subs() {
        right_items.push(BtnItem {
            id: "btnEqualizer",
            svg: SvgId::CtrlEqualizer,
            variant: SvgVariant::Default,
            target: HitTarget::EqualizerButton,
            size: btn_sz,
            active: state.active_modal == Some(ModalId::Equalizer),
        });
    }
    if !state.welcome_active && bp.show_audio_track() {
        right_items.push(BtnItem {
            id: "btnAudioTrack",
            svg: SvgId::CtrlAudioTrack,
            variant: SvgVariant::Default,
            target: HitTarget::AudioTrackButton,
            size: btn_sz,
            active: state.active_modal == Some(ModalId::Audio),
        });
    }
    if !state.welcome_active && bp.show_eq_and_subs() {
        right_items.push(BtnItem {
            id: "btnSubtitles",
            svg: SvgId::CtrlSubtitles,
            variant: SvgVariant::Default,
            target: HitTarget::SubtitlesButton,
            size: btn_sz,
            active: state.active_modal == Some(ModalId::Subtitles),
        });
    }
    right_items.push(BtnItem {
        id: "btnPlaylist",
        svg: SvgId::CtrlPlaylist,
        variant: SvgVariant::Default,
        target: HitTarget::PlaylistToggle,
        size: btn_sz,
        active: state.playlist.open,
    });
    if !state.welcome_active && bp.show_secondary_toggles() {
        right_items.push(BtnItem {
            id: "btnRepeat",
            svg: SvgId::CtrlRepeat,
            variant: if state.repeat == RepeatMode::One {
                SvgVariant::RepeatOne
            } else {
                SvgVariant::Default
            },
            target: HitTarget::RepeatToggle,
            size: btn_sz,
            active: state.repeat != RepeatMode::None,
        });
        right_items.push(BtnItem {
            id: "btnShuffle",
            svg: SvgId::CtrlShuffle,
            variant: SvgVariant::Default,
            target: HitTarget::ShuffleToggle,
            size: btn_sz,
            active: state.shuffle,
        });
        right_items.push(BtnItem {
            id: "btnABLoop",
            svg: SvgId::CtrlAbLoop,
            variant: match state.ab_loop {
                AbLoopState::Off => SvgVariant::Default,
                AbLoopState::PointASet(_) => SvgVariant::AbLoopA,
                AbLoopState::Active(_, _) => SvgVariant::AbLoopAb,
            },
            target: HitTarget::AbLoopToggle,
            size: btn_sz,
            active: state.ab_loop != AbLoopState::Off,
        });
    }
    // Note: `#btnScreenshot` (`bp.show_screenshot_btn() == false`) is NEVER mounted!
    let _ = bp.show_screenshot_btn();
    if !state.welcome_active && bp.show_secondary_toggles() {
        right_items.push(BtnItem {
            id: "btnLock",
            svg: SvgId::CtrlLock,
            variant: SvgVariant::Default,
            target: HitTarget::LockToggle,
            size: btn_sz,
            active: state.locked,
        });
    }
    if !state.welcome_active && bp.show_youtube_quality(state.is_youtube) {
        right_items.push(BtnItem {
            id: "btnYoutubeQuality",
            svg: SvgId::CtrlSettings,
            variant: SvgVariant::Default,
            target: HitTarget::YoutubeQualityButton,
            size: btn_sz,
            active: false,
        });
    }
    right_items.push(BtnItem {
        id: "btnSettings",
        svg: SvgId::CtrlSettings,
        variant: SvgVariant::Default,
        target: HitTarget::SettingsButton,
        size: btn_sz,
        active: false,
    });
    right_items.push(BtnItem {
        id: "btnFullscreen",
        svg: if state.window_maximize_state == WindowMaximizeState::Fullscreen {
            SvgId::CtrlFullscreenExit
        } else {
            SvgId::CtrlFullscreenEnter
        },
        variant: SvgVariant::Default,
        target: HitTarget::FullscreenToggle,
        size: btn_sz,
        active: state.window_maximize_state == WindowMaximizeState::Fullscreen,
    });

    let mut taffy = TaffyTree::<()>::new();
    // The compact playlist tier is playback-only (the welcome screen keeps full-size controls).
    let compact_playback_playlist = bp.max_w_1280_playlist && !state.welcome_active;
    let left_gap = if bp.max_w_1000 || compact_playback_playlist {
        2.0
    } else {
        4.0
    };
    let right_gap = if bp.max_w_1000 || compact_playback_playlist {
        2.0
    } else {
        4.0
    };
    let vol_margin_left = if bp.max_w_1150_playlist {
        2.0
    } else if bp.max_w_1000 {
        4.0
    } else {
        10.0
    };
    let vol_inner_gap = if bp.max_w_860 || bp.max_w_1150_playlist {
        2.0
    } else {
        0.0
    };
    let vol_slider_margin_left = if bp.max_w_1150_playlist {
        2.0
    } else if bp.max_w_1000 {
        4.0
    } else {
        8.0
    };
    let vol_val_w = if bp.max_w_1000 { 34.0 } else { 40.0 };
    let vol_val_margin_left = if bp.max_w_1000 { 6.0 } else { 12.0 };
    let time_margin_left = if bp.max_w_1000 || bp.max_w_1150_playlist {
        4.0
    } else {
        8.0
    };
    let time_font = if bp.max_w_1150_playlist {
        10.5
    } else if bp.max_w_760 {
        11.0
    } else if bp.max_w_1000 {
        11.5
    } else {
        13.0
    };

    let mut pre_vol_nodes = Vec::new();
    for item in &pre_vol_items {
        pre_vol_nodes.push(
            taffy
                .new_leaf(Style {
                    size: Size {
                        width: length(item.size),
                        height: length(item.size),
                    },
                    flex_shrink: 0.0,
                    ..Default::default()
                })
                .unwrap(),
        );
    }

    let vol_btn_node = taffy
        .new_leaf(Style {
            size: Size {
                width: length(vol_btn_item.size),
                height: length(vol_btn_item.size),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .unwrap();
    let vol_slider_node = taffy
        .new_leaf(Style {
            size: Size {
                width: length(vol_w),
                height: length(20.0),
            },
            margin: Rect {
                left: length(vol_slider_margin_left),
                ..Rect::zero()
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .unwrap();
    let vol_val_node = if bp.show_volume_value() {
        Some(
            taffy
                .new_leaf(Style {
                    size: Size {
                        width: length(vol_val_w),
                        height: length(18.0),
                    },
                    margin: Rect {
                        left: length(vol_val_margin_left),
                        ..Rect::zero()
                    },
                    flex_shrink: 0.0,
                    ..Default::default()
                })
                .unwrap(),
        )
    } else {
        None
    };
    let mut vol_children = vec![vol_btn_node, vol_slider_node];
    if let Some(vn) = vol_val_node {
        vol_children.push(vn);
    }
    let vol_container = taffy
        .new_with_children(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::CENTER,
                gap: Size {
                    width: length(vol_inner_gap),
                    height: zero(),
                },
                margin: Rect {
                    left: length(vol_margin_left),
                    ..Rect::zero()
                },
                flex_shrink: 0.0,
                ..Default::default()
            },
            &vol_children,
        )
        .unwrap();

    let time_node = if bp.show_time_display() {
        Some(
            taffy
                .new_leaf(Style {
                    size: Size {
                        width: length(110.0),
                        height: length(20.0),
                    },
                    margin: Rect {
                        left: length(time_margin_left),
                        ..Rect::zero()
                    },
                    flex_shrink: 0.0,
                    ..Default::default()
                })
                .unwrap(),
        )
    } else {
        None
    };
    let mut left_children = pre_vol_nodes.clone();
    left_children.push(vol_container);
    if let Some(tn) = time_node {
        left_children.push(tn);
    }
    let left_row = taffy
        .new_with_children(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::CENTER,
                gap: Size {
                    width: length(left_gap),
                    height: zero(),
                },
                flex_shrink: 0.0,
                ..Default::default()
            },
            &left_children,
        )
        .unwrap();

    let spacer = taffy
        .new_leaf(Style {
            flex_grow: 1.0,
            ..Default::default()
        })
        .unwrap();
    let mut right_nodes = Vec::new();
    for item in &right_items {
        let item_w = if item.id == "btnYoutubeQuality" {
            let lbl_w = measure_text_width(
                &state.youtube_quality_label,
                FontRole::Ui,
                12.0,
                600,
                0.0,
                state.font_config,
            );
            (lbl_w + 16.0).clamp(item.size, 72.0)
        } else {
            item.size
        };
        right_nodes.push(
            taffy
                .new_leaf(Style {
                    size: Size {
                        width: length(item_w),
                        height: length(item.size),
                    },
                    flex_shrink: 0.0,
                    ..Default::default()
                })
                .unwrap(),
        );
    }
    let right_row = taffy
        .new_with_children(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::CENTER,
                gap: Size {
                    width: length(right_gap),
                    height: zero(),
                },
                flex_shrink: 0.0,
                ..Default::default()
            },
            &right_nodes,
        )
        .unwrap();

    let root = taffy
        .new_with_children(
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::CENTER,
                size: Size {
                    width: length(bar_w),
                    height: length(bar_h),
                },
                ..Default::default()
            },
            &[left_row, spacer, right_row],
        )
        .unwrap();

    taffy
        .compute_layout(
            root,
            Size {
                width: AvailableSpace::Definite(bar_w),
                height: AvailableSpace::Definite(bar_h),
            },
        )
        .unwrap();

    let ctrl_z = ZIndex::CONTROLS_BAR + 1;
    let white_u8 = [255, 255, 255, 255];
    let accent_u8 = rgba_f32_to_u8(theme.accent);
    let left_l = taffy.layout(left_row).unwrap();
    let vol_cont_l = taffy.layout(vol_container).unwrap();
    let right_l = taffy.layout(right_row).unwrap();

    b.push_hit(
        HitTarget::VolumeContainerSurface,
        ctrl_z,
        RectPx::new(
            content_x + left_l.location.x + vol_cont_l.location.x,
            bar_y + left_l.location.y + vol_cont_l.location.y,
            vol_cont_l.size.width,
            vol_cont_l.size.height,
        ),
        CursorHint::Default,
    );

    for (item, node) in pre_vol_items.iter().zip(pre_vol_nodes.iter()) {
        let l = taffy.layout(*node).unwrap();
        let rect = RectPx::new(
            content_x + left_l.location.x + l.location.x,
            bar_y + left_l.location.y + l.location.y,
            l.size.width,
            l.size.height,
        );
        if is_hovered(state, &item.target) {
            b.push_solid(
                format!("{}-hover-bg", item.id),
                ctrl_z,
                rect,
                rgba(255, 255, 255, 0.10),
                [8.0; 4],
                0.0,
                [0.0; 4],
                None,
            );
        }
        let (svg_w, svg_h) = control_svg_dimensions(item.svg, rect.width, bp.max_w_1000);
        let sx = rect.x + (rect.width - svg_w) * 0.5;
        let sy = rect.y + (rect.height - svg_h) * 0.5;
        b.push_svg(
            item.id,
            ctrl_z,
            item.svg,
            item.variant,
            RectPx::new(sx, sy, svg_w, svg_h),
            if item.active { accent_u8 } else { white_u8 },
            None,
        );
        b.push_hit(item.target.clone(), ctrl_z, rect, CursorHint::Pointer);
    }

    // `#btnVolume` inside `.volume-container`
    {
        let l = taffy.layout(vol_btn_node).unwrap();
        let mut rect = RectPx::new(
            content_x + left_l.location.x + vol_cont_l.location.x + l.location.x,
            bar_y + left_l.location.y + vol_cont_l.location.y + l.location.y,
            l.size.width,
            l.size.height,
        );
        rect.x += spring_tx.btn_volume_translate_x;
        if is_hovered(state, &vol_btn_item.target) {
            b.push_solid(
                "btnVolume-hover-bg",
                ctrl_z,
                rect,
                rgba(255, 255, 255, 0.10),
                [8.0; 4],
                0.0,
                [0.0; 4],
                None,
            );
        }
        let (svg_w, svg_h) = control_svg_dimensions(vol_btn_item.svg, rect.width, bp.max_w_1000);
        let sx = rect.x + (rect.width - svg_w) * 0.5;
        let sy = rect.y + (rect.height - svg_h) * 0.5;
        b.push_svg(
            vol_btn_item.id,
            ctrl_z,
            vol_btn_item.svg,
            vol_btn_item.variant,
            RectPx::new(sx, sy, svg_w, svg_h),
            white_u8,
            None,
        );
        b.push_hit(
            vol_btn_item.target.clone(),
            ctrl_z,
            rect,
            CursorHint::Pointer,
        );
    }

    let vs_l = taffy.layout(vol_slider_node).unwrap();
    let vs_rect = RectPx::new(
        content_x + left_l.location.x + vol_cont_l.location.x + vs_l.location.x,
        bar_y + left_l.location.y + vol_cont_l.location.y + vs_l.location.y,
        vs_l.size.width,
        vs_l.size.height,
    );
    let vs_track_x = vs_rect.x + 4.0;
    let vs_track_w = (vs_rect.width - 8.0).max(8.0);
    b.push_solid(
        "volume-slider-track",
        ctrl_z,
        RectPx::new(vs_track_x, vs_rect.y + 8.0, vs_track_w, 4.0),
        rgba(255, 255, 255, 0.22),
        [2.0; 4],
        0.0,
        [0.0; 4],
        None,
    );
    let max_vol = if state.volume_boost_enabled {
        200.0
    } else {
        100.0
    };
    let vol_fill_ratio = if state.muted {
        0.0
    } else {
        (state.volume_pct as f32 / max_vol).clamp(0.0, 1.0)
    };
    b.push_solid(
        "volume-slider-fill",
        ctrl_z,
        RectPx::new(
            vs_track_x,
            vs_rect.y + 8.0,
            vs_track_w * vol_fill_ratio,
            4.0,
        ),
        if state.volume_pct > 100 {
            theme.ytdlp_pending
        } else {
            [1.0, 1.0, 1.0, 1.0]
        },
        [2.0; 4],
        0.0,
        [0.0; 4],
        None,
    );
    // White circular volume thumb (matches golden PNGs & controls.css:511-521)
    let thumb_cx = vs_track_x + vs_track_w * vol_fill_ratio;
    b.push_solid(
        "volume-slider-thumb",
        ctrl_z,
        RectPx::new(thumb_cx - 6.0, vs_rect.y + 4.0, 12.0, 12.0),
        [1.0, 1.0, 1.0, 1.0],
        [6.0; 4],
        0.0,
        [0.0; 4],
        None,
    );
    b.push_hit(
        HitTarget::VolumeSlider,
        ctrl_z,
        vs_rect,
        CursorHint::Pointer,
    );

    if let Some(vn) = vol_val_node {
        let l = taffy.layout(vn).unwrap();
        let rect = RectPx::new(
            content_x
                + left_l.location.x
                + vol_cont_l.location.x
                + l.location.x
                + spring_tx.volume_value_translate_x,
            bar_y + left_l.location.y + vol_cont_l.location.y + l.location.y,
            l.size.width,
            l.size.height,
        );
        b.push_text(
            "volume-value",
            ctrl_z,
            rect,
            format!("{}%", state.volume_pct),
            FontRole::Ui,
            11.0,
            600,
            [1.0; 4],
            TextAlign::Left,
            None,
        );
    }

    if let Some(tn) = time_node {
        let l = taffy.layout(tn).unwrap();
        let rect = RectPx::new(
            content_x + left_l.location.x + l.location.x + spring_tx.time_display_translate_x,
            bar_y + left_l.location.y + l.location.y + (20.0 - time_font * 1.35) * 0.5,
            l.size.width,
            20.0,
        );
        let left_str = UiState::format_time(state.current_secs);
        let dur_str = if state.is_live_stream {
            "LIVE".to_string()
        } else if state.time_remaining_mode && state.duration_secs > 0.0 {
            format!(
                "-{}",
                UiState::format_time((state.duration_secs - state.current_secs).max(0.0))
            )
        } else {
            UiState::format_time(state.duration_secs)
        };
        let est_left_w = measure_text_width(
            &left_str,
            FontRole::Ui,
            time_font,
            500,
            0.0,
            state.font_config,
        );
        let sep_pad = if bp.max_w_1000 { 4.0 } else { 6.0 };
        let sep_w = measure_text_width("/", FontRole::Ui, time_font, 500, 0.0, state.font_config);
        let dur_w = measure_text_width(
            &dur_str,
            FontRole::Ui,
            time_font,
            500,
            0.0,
            state.font_config,
        )
        .max(24.0);
        b.push_text(
            "time-display",
            ctrl_z,
            rect,
            left_str,
            FontRole::Ui,
            time_font,
            500,
            [1.0; 4],
            TextAlign::Left,
            None,
        );
        b.push_text(
            "time-separator",
            ctrl_z,
            RectPx::new(rect.x + est_left_w + sep_pad, rect.y, sep_w + 4.0, 20.0),
            "/",
            FontRole::Ui,
            time_font,
            500,
            rgba(255, 255, 255, 0.5),
            TextAlign::Left,
            None,
        );
        let total_rect = RectPx::new(
            rect.x + est_left_w + sep_pad * 2.0 + sep_w,
            rect.y,
            dur_w + 4.0,
            20.0,
        );
        b.push_text(
            "time-total",
            ctrl_z,
            total_rect,
            dur_str,
            FontRole::Ui,
            time_font,
            500,
            [1.0; 4],
            TextAlign::Left,
            None,
        );
        b.push_hit(
            HitTarget::TimeRemainingToggle,
            ctrl_z,
            total_rect,
            CursorHint::Pointer,
        );
    }

    for (item, node) in right_items.iter().zip(right_nodes.iter()) {
        let l = taffy.layout(*node).unwrap();
        let rect = RectPx::new(
            content_x + right_l.location.x + l.location.x,
            bar_y + right_l.location.y + l.location.y,
            l.size.width,
            l.size.height,
        );
        if item.active {
            b.push_solid(
                format!("{}-active-bg", item.id),
                ctrl_z,
                rect,
                rgba(255, 255, 255, 0.08),
                [8.0; 4],
                0.0,
                [0.0; 4],
                None,
            );
        } else if is_hovered(state, &item.target) {
            b.push_solid(
                format!("{}-hover-bg", item.id),
                ctrl_z,
                rect,
                rgba(255, 255, 255, 0.10),
                [8.0; 4],
                0.0,
                [0.0; 4],
                None,
            );
        }
        if item.id == "btnSpeed" {
            // `#btnSpeed` renders text (`1x`, `1.5x`, etc.) matching golden PNG & index.html `.ctrl-btn-text`
            let speed_txt = if (state.speed - state.speed.round()).abs() < 0.01 {
                format!("{}x", state.speed.round() as i32)
            } else {
                format!("{:.2}x", state.speed)
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_string()
                    + "x"
            };
            b.push_text(
                "btnSpeed-label",
                ctrl_z,
                RectPx::new(
                    rect.x,
                    rect.y + (rect.height - 16.0) * 0.5,
                    rect.width,
                    16.0,
                ),
                speed_txt,
                FontRole::Ui,
                12.0,
                600,
                if item.active { theme.accent } else { [1.0; 4] },
                TextAlign::Center,
                None,
            );
        } else if item.id == "btnYoutubeQuality" {
            b.push_text(
                "btnYoutubeQuality-label",
                ctrl_z,
                RectPx::new(
                    rect.x,
                    rect.y + (rect.height - 16.0) * 0.5,
                    rect.width,
                    16.0,
                ),
                state.youtube_quality_label.clone(),
                FontRole::Ui,
                12.0,
                600,
                if item.active { theme.accent } else { [1.0; 4] },
                TextAlign::Center,
                None,
            );
        } else {
            let (svg_w, svg_h) = control_svg_dimensions(item.svg, rect.width, bp.max_w_1000);
            let sx = rect.x + (rect.width - svg_w) * 0.5;
            let sy = rect.y + (rect.height - svg_h) * 0.5;
            b.push_svg(
                item.id,
                ctrl_z,
                item.svg,
                item.variant,
                RectPx::new(sx, sy, svg_w, svg_h),
                if item.active { accent_u8 } else { white_u8 },
                None,
            );
        }
        b.push_hit(item.target.clone(), ctrl_z, rect, CursorHint::Pointer);
    }
}

fn build_playlist_sidebar(
    b: &mut FrameBuilder,
    state: &UiState,
    bp: &BreakpointState,
    theme: &ThemeTokens,
    w: f32,
    h: f32,
) -> VirtualSlice {
    let pw = bp.playlist_width();
    let px = (w - pw).max(0.0);
    let panel_rect = RectPx::new(px, 0.0, pw, h);

    // Opaque sidebar surface + `box-shadow: -12px 0 32px rgba(0,0,0,0.35)` (`playlist.css:1-28`)
    let shadow_w = 32.0_f32.min(px);
    if shadow_w > 1.0 {
        b.push_gradient(
            "playlist-sidebar-shadow",
            ZIndex::PLAYLIST_SIDEBAR,
            RectPx::new(px - shadow_w, 0.0, shadow_w, h),
            rgba(0, 0, 0, 0.0),
            rgba(0, 0, 0, 0.28),
            90.0,
            [0.0; 4],
            0.0,
            [0.0; 4],
            None,
        );
    }
    b.push_solid(
        "playlist-sidebar",
        ZIndex::PLAYLIST_SIDEBAR,
        panel_rect,
        theme.bg_secondary,
        [0.0; 4],
        0.0,
        [0.0; 4],
        None,
    );
    b.push_hit(
        HitTarget::PlaylistSurface,
        ZIndex::PLAYLIST_SIDEBAR,
        panel_rect,
        CursorHint::Default,
    );
    b.push_solid(
        "playlist-sidebar-border",
        ZIndex::PLAYLIST_SIDEBAR,
        RectPx::new(px, 0.0, 1.0, h),
        theme.border_color,
        [0.0; 4],
        0.0,
        [0.0; 4],
        None,
    );

    // Header (height 36px, left `>` close icon + `"Playlist"`, matching `playlist.css:45-83` & golden PNG)
    let close_rect = RectPx::new(px + 12.0, 4.0, 28.0, 28.0);
    let close_hovered = b.is_hovered(HitTarget::PlaylistClose);
    if close_hovered {
        b.push_solid(
            "btnClosePlaylist-bg",
            ZIndex::PLAYLIST_SIDEBAR,
            close_rect,
            theme.bg_tertiary,
            [6.0; 4],
            0.0,
            [0.0; 4],
            Some(panel_rect),
        );
    }
    b.push_svg(
        "btnClosePlaylist",
        ZIndex::PLAYLIST_SIDEBAR,
        SvgId::PlaylistClose,
        SvgVariant::Default,
        RectPx::new(close_rect.x + 6.0, close_rect.y + 6.0, 16.0, 16.0),
        rgba_f32_to_u8(if close_hovered {
            theme.text_primary
        } else {
            theme.text_secondary
        }),
        Some(panel_rect),
    );
    b.push_hit(
        HitTarget::PlaylistClose,
        ZIndex::PLAYLIST_SIDEBAR,
        close_rect,
        CursorHint::Pointer,
    );
    let hdr_rect = RectPx::new(px + 46.0, 8.0, pw - 60.0, 20.0);
    b.push_text(
        "playlist-header",
        ZIndex::PLAYLIST_SIDEBAR,
        hdr_rect,
        "Playlist",
        FontRole::Display,
        13.0,
        600,
        theme.text_primary,
        TextAlign::Left,
        Some(panel_rect),
    );
    b.push_solid(
        "playlist-header-divider",
        ZIndex::PLAYLIST_SIDEBAR,
        RectPx::new(px, 36.0, pw, 1.0),
        theme.border_color,
        [0.0; 4],
        0.0,
        [0.0; 4],
        Some(panel_rect),
    );

    // 4 borderless toolbar icon buttons (36x36, `padding: 6px 16px`, `gap: 6px`)
    let repeat_variant = if state.repeat == RepeatMode::One {
        SvgVariant::RepeatOne
    } else {
        SvgVariant::Default
    };
    let tools = [
        (
            "btnAddToPlaylist",
            SvgId::PlaylistAddFiles,
            SvgVariant::Default,
            HitTarget::PlaylistAddFiles,
            false,
            18.0,
            18.0,
        ),
        (
            "btnShufflePlaylist",
            SvgId::PlaylistShuffle,
            SvgVariant::Default,
            HitTarget::PlaylistShuffle,
            state.shuffle,
            18.0,
            18.0,
        ),
        (
            "btnRepeatPlaylist",
            SvgId::PlaylistRepeat,
            repeat_variant,
            HitTarget::PlaylistRepeat,
            state.repeat != RepeatMode::None,
            22.0,
            11.0,
        ),
        (
            "btnClearPlaylist",
            SvgId::PlaylistClear,
            SvgVariant::Default,
            HitTarget::PlaylistClear,
            false,
            18.0,
            18.0,
        ),
    ];
    for (i, (id, svg, variant, target, active, sw, sh)) in tools.into_iter().enumerate() {
        let bx = px + 16.0 + (i as f32) * 42.0;
        let brect = RectPx::new(bx, 43.0, 36.0, 36.0);
        let hovered = b.is_hovered(target.clone());
        if active || hovered {
            b.push_solid(
                format!("{id}-bg"),
                ZIndex::PLAYLIST_SIDEBAR,
                brect,
                if active {
                    theme.accent_dim
                } else {
                    theme.bg_tertiary
                },
                [8.0; 4],
                0.0,
                [0.0; 4],
                Some(panel_rect),
            );
        }
        b.push_svg(
            id,
            ZIndex::PLAYLIST_SIDEBAR,
            svg,
            variant,
            RectPx::new(
                brect.x + (brect.width - sw) * 0.5,
                brect.y + (brect.height - sh) * 0.5,
                sw,
                sh,
            ),
            if active {
                rgba_f32_to_u8(theme.accent)
            } else if hovered {
                rgba_f32_to_u8(theme.text_primary)
            } else {
                rgba_f32_to_u8(theme.text_secondary)
            },
            Some(panel_rect),
        );
        b.push_hit(target, ZIndex::PLAYLIST_SIDEBAR, brect, CursorHint::Pointer);
    }
    b.push_solid(
        "playlist-toolbar-divider",
        ZIndex::PLAYLIST_SIDEBAR,
        RectPx::new(px, 85.0, pw, 1.0),
        theme.border_color,
        [0.0; 4],
        0.0,
        [0.0; 4],
        Some(panel_rect),
    );

    // Search input (y: 94, h: 34)
    let search_rect = RectPx::new(px + 16.0, 94.0, (pw - 32.0).max(40.0), 34.0);
    draw_text_input_box(
        b,
        state,
        theme,
        "playlist-search",
        ZIndex::PLAYLIST_SIDEBAR,
        search_rect,
        theme.bg_tertiary,
        theme.border_color,
        8.0,
        10.0,
        13.0,
        "Search playlist...",
        FocusedTextField::PlaylistSearch,
        HitTarget::PlaylistSearch,
        Some(panel_rect),
    );

    b.push_solid(
        "playlist-search-divider",
        ZIndex::PLAYLIST_SIDEBAR,
        RectPx::new(px, 136.0, pw, 1.0),
        theme.border_color,
        [0.0; 4],
        0.0,
        [0.0; 4],
        Some(panel_rect),
    );

    // Virtualized playlist viewport (flat 39px rows + 3px active left bar, matching `playlist.css:154-245`)
    let list_y = 138.0;
    let footer_h = 36.0;
    let list_h = (h - list_y - footer_h).max(40.0);
    let list_rect = RectPx::new(px + 1.0, list_y, (pw - 1.0).max(40.0), list_h);
    let slice = state.playlist.virtual_window(list_h);
    let filtered = state.playlist.filtered_items();

    for vis_idx in slice.start_index..slice.end_index {
        if let Some((orig_idx, item)) = filtered.get(vis_idx) {
            let row_y =
                list_y + (vis_idx as f32) * PLAYLIST_ITEM_HEIGHT - state.playlist.scroll_top_px;
            let row_rect = RectPx::new(list_rect.x, row_y, list_rect.width, 39.0);
            let is_active = state.playlist.current_index == Some(*orig_idx);
            let row_hovered = b.is_hovered(HitTarget::PlaylistRow(*orig_idx))
                || b.is_hovered(HitTarget::PlaylistRowRemove(*orig_idx));
            if is_active || row_hovered {
                b.push_solid(
                    format!("playlist-row-{orig_idx}"),
                    ZIndex::PLAYLIST_SIDEBAR,
                    row_rect,
                    if is_active {
                        theme.accent_dim
                    } else {
                        theme.bg_tertiary
                    },
                    [0.0; 4],
                    0.0,
                    [0.0; 4],
                    Some(list_rect),
                );
            }
            if is_active {
                b.push_solid(
                    format!("playlist-row-accent-{orig_idx}"),
                    ZIndex::PLAYLIST_SIDEBAR,
                    RectPx::new(row_rect.x, row_rect.y, 3.0, row_rect.height),
                    theme.accent,
                    [0.0; 4],
                    0.0,
                    [0.0; 4],
                    Some(list_rect),
                );
            }
            b.push_text(
                format!("playlist-row-index-{orig_idx}"),
                ZIndex::PLAYLIST_SIDEBAR,
                RectPx::new(row_rect.x + 12.0, row_rect.y + 10.0, 22.0, 18.0),
                if is_active {
                    "▶".to_string()
                } else {
                    format!("{}", orig_idx + 1)
                },
                if is_active {
                    FontRole::Ui
                } else {
                    FontRole::Mono
                },
                11.5,
                if is_active { 600 } else { 400 },
                if is_active {
                    theme.accent
                } else {
                    theme.text_tertiary
                },
                TextAlign::Left,
                Some(list_rect),
            );
            b.push_text(
                format!("playlist-row-title-{orig_idx}"),
                ZIndex::PLAYLIST_SIDEBAR,
                RectPx::new(
                    row_rect.x + 36.0,
                    row_rect.y + 10.0,
                    (row_rect.width - 68.0).max(20.0),
                    19.0,
                ),
                item.name.clone(),
                FontRole::Ui,
                13.0,
                if is_active { 600 } else { 400 },
                if is_active {
                    theme.text_primary
                } else {
                    theme.text_secondary
                },
                TextAlign::Left,
                Some(list_rect),
            );
            let rm_rect = RectPx::new(
                row_rect.x + row_rect.width - 30.0,
                row_rect.y + (row_rect.height - 22.0) * 0.5,
                22.0,
                22.0,
            );
            let rm_hovered = b.is_hovered(HitTarget::PlaylistRowRemove(*orig_idx));
            if row_hovered || is_active {
                if rm_hovered {
                    b.push_solid(
                        format!("playlist-row-remove-bg-{orig_idx}"),
                        ZIndex::PLAYLIST_SIDEBAR + 1,
                        rm_rect,
                        with_alpha(theme.accent, 0.16),
                        [5.0; 4],
                        0.0,
                        [0.0; 4],
                        Some(list_rect),
                    );
                }
                b.push_svg(
                    format!("playlist-row-remove-{orig_idx}"),
                    ZIndex::PLAYLIST_SIDEBAR + 1,
                    SvgId::ModalCloseSubtitle,
                    SvgVariant::Default,
                    RectPx::new(rm_rect.x + 6.0, rm_rect.y + 6.0, 10.0, 10.0),
                    rgba_f32_to_u8(if rm_hovered {
                        theme.accent
                    } else {
                        theme.text_tertiary
                    }),
                    Some(list_rect),
                );
            }
            b.push_hit(
                HitTarget::PlaylistRow(*orig_idx),
                ZIndex::PLAYLIST_SIDEBAR,
                row_rect,
                CursorHint::Pointer,
            );
            b.push_hit(
                HitTarget::PlaylistRowRemove(*orig_idx),
                ZIndex::PLAYLIST_SIDEBAR + 1,
                rm_rect,
                CursorHint::Pointer,
            );
        }
    }

    if state.playlist.items.is_empty() {
        let cy = list_rect.y + (list_rect.height - 96.0).max(16.0) * 0.45;
        b.push_svg(
            "playlist-empty-icon",
            ZIndex::PLAYLIST_SIDEBAR,
            SvgId::PlaylistEmptyList,
            SvgVariant::Default,
            RectPx::new(list_rect.x + (list_rect.width - 40.0) * 0.5, cy, 40.0, 40.0),
            rgba_f32_to_u8(with_alpha(theme.text_tertiary, 0.72)),
            Some(list_rect),
        );
        b.push_text(
            "playlist-empty-title",
            ZIndex::PLAYLIST_SIDEBAR,
            RectPx::new(list_rect.x + 16.0, cy + 50.0, list_rect.width - 32.0, 20.0),
            "Playlist is empty",
            FontRole::Ui,
            13.5,
            600,
            theme.text_secondary,
            TextAlign::Center,
            Some(list_rect),
        );
        b.push_text(
            "playlist-empty-hint",
            ZIndex::PLAYLIST_SIDEBAR,
            RectPx::new(list_rect.x + 16.0, cy + 72.0, list_rect.width - 32.0, 18.0),
            "Drop files or click + to add",
            FontRole::Ui,
            12.0,
            400,
            theme.text_tertiary,
            TextAlign::Center,
            Some(list_rect),
        );
    } else if filtered.is_empty() {
        let cy = list_rect.y + (list_rect.height - 48.0).max(16.0) * 0.45;
        b.push_text(
            "playlist-no-matches-title",
            ZIndex::PLAYLIST_SIDEBAR,
            RectPx::new(list_rect.x + 16.0, cy, list_rect.width - 32.0, 20.0),
            "No matches",
            FontRole::Ui,
            13.5,
            600,
            theme.text_secondary,
            TextAlign::Center,
            Some(list_rect),
        );
        b.push_text(
            "playlist-no-matches-hint",
            ZIndex::PLAYLIST_SIDEBAR,
            RectPx::new(list_rect.x + 16.0, cy + 22.0, list_rect.width - 32.0, 18.0),
            "Try a different search",
            FontRole::Ui,
            12.0,
            400,
            theme.text_tertiary,
            TextAlign::Center,
            Some(list_rect),
        );
    }

    let total_list_h = (filtered.len() as f32) * PLAYLIST_ITEM_HEIGHT;
    if total_list_h > list_rect.height + 0.5 {
        let track_r = RectPx::new(
            list_rect.x + list_rect.width - 6.0,
            list_rect.y + 3.0,
            4.0,
            (list_rect.height - 6.0).max(20.0),
        );
        let max_scroll = (total_list_h - list_rect.height).max(1.0);
        let thumb_h =
            ((list_rect.height / total_list_h) * track_r.height).clamp(24.0, track_r.height);
        let thumb_y = track_r.y
            + (state.playlist.scroll_top_px.clamp(0.0, max_scroll) / max_scroll)
                * (track_r.height - thumb_h);
        b.push_solid(
            "playlist-scroll-thumb",
            ZIndex::PLAYLIST_SIDEBAR + 1,
            RectPx::new(track_r.x, thumb_y, track_r.width, thumb_h),
            with_alpha(theme.text_tertiary, 0.35),
            [2.0; 4],
            0.0,
            [0.0; 4],
            Some(list_rect),
        );
    }

    // Sidebar footer (`border-top`, `"0 items"` / `"N items"` / `"N of M items"`, matching `playlist.css:254-261`, `playlist.js`)
    let footer_y = (h - 36.0).max(list_y);
    b.push_solid(
        "playlist-footer-divider",
        ZIndex::PLAYLIST_SIDEBAR,
        RectPx::new(px, footer_y, pw, 1.0),
        theme.border_color,
        [0.0; 4],
        0.0,
        [0.0; 4],
        Some(panel_rect),
    );
    let count = filtered.len();
    let total = state.playlist.items.len();
    let total_label = if total == 1 {
        "1 item".to_string()
    } else {
        format!("{total} items")
    };
    let count_label = if !state.playlist.search_query.trim().is_empty() {
        format!("{count} of {total_label}")
    } else {
        total_label
    };
    b.push_text(
        "playlist-footer-count",
        ZIndex::PLAYLIST_SIDEBAR,
        RectPx::new(px + 16.0, footer_y + 9.0, pw - 32.0, 18.0),
        count_label,
        FontRole::Ui,
        12.0,
        400,
        theme.text_tertiary,
        TextAlign::Left,
        Some(panel_rect),
    );

    slice
}

fn build_overlays(
    b: &mut FrameBuilder,
    state: &UiState,
    _bp: &BreakpointState,
    theme: &ThemeTokens,
    w: f32,
    h: f32,
    playlist_w: f32,
) {
    let main_w = (w - playlist_w).max(0.0);

    // Top-right Volume OSD (`.top-right-volume-osd`: `{val}%` with bold num + cyan `%`, radius 20px)
    if state.volume_osd.visible {
        let top = if state.ab_loop != AbLoopState::Off {
            138.0
        } else if state.window_maximize_state == WindowMaximizeState::Fullscreen {
            28.0
        } else {
            48.0
        };
        let val_str = format!("{}", state.volume_osd.percent);
        let val_w = measure_text_width(&val_str, FontRole::Ui, 28.0, 900, -0.04, state.font_config);
        let pct_w = measure_text_width("%", FontRole::Ui, 16.0, 700, 0.0, state.font_config);
        let pill_w = (28.0 + val_w + 3.0 + pct_w).max(68.0);
        let pill_h = 40.0;
        let right_offset = 28.0 + playlist_w;
        let rect = RectPx::new((w - right_offset - pill_w).max(8.0), top, pill_w, pill_h);
        b.push_glass(
            "top-right-volume-osd-glass",
            ZIndex::TOP_RIGHT_VOLUME_OSD,
            rect,
            20.0,
            Some(CssBackdropRule::TopRightVolumeOsd),
            ChameleonSkin::None,
            theme,
            None,
        );
        b.push_solid(
            "top-right-volume-osd",
            ZIndex::TOP_RIGHT_VOLUME_OSD,
            rect,
            theme.volume_osd_bg,
            [20.0; 4],
            1.0,
            theme.volume_osd_border,
            None,
        );
        b.push_text(
            "top-right-volume-osd-val",
            ZIndex::TOP_RIGHT_VOLUME_OSD,
            RectPx::new(rect.x + 14.0, rect.y + 4.0, val_w + 2.0, 30.0),
            val_str,
            FontRole::Ui,
            28.0,
            900,
            theme.volume_osd_fg,
            TextAlign::Left,
            None,
        );
        b.push_text(
            "top-right-volume-osd-pct",
            ZIndex::TOP_RIGHT_VOLUME_OSD,
            RectPx::new(
                rect.x + 14.0 + val_w + 3.0,
                rect.y + 13.0,
                pct_w + 2.0,
                20.0,
            ),
            "%",
            FontRole::Ui,
            16.0,
            700,
            theme.volume_osd_pct,
            TextAlign::Left,
            None,
        );
    }

    // Loading Spinner (`main.css:482-517`: 48x48 ring + animated accent arc dots)
    if state.loading_visible {
        let rect = RectPx::new((main_w - 48.0) * 0.5, (h - 48.0) * 0.5, 48.0, 48.0);
        b.push_solid(
            "loading-spinner",
            ZIndex::LOADING_SPINNER,
            rect,
            rgba(0, 0, 0, 0.45),
            [24.0; 4],
            3.0,
            rgba(255, 255, 255, 0.10),
            None,
        );
        let cx = rect.x + rect.width * 0.5;
        let cy = rect.y + rect.height * 0.5;
        let base_angle = state.anim.spinner_phase * std::f32::consts::TAU;
        for step in 0..8 {
            let t = (step as f32) / 8.0;
            let angle = base_angle - t * (std::f32::consts::PI * 1.15);
            let dot_r = 18.0;
            let dx = cx + angle.cos() * dot_r - 2.5;
            let dy = cy + angle.sin() * dot_r - 2.5;
            let alpha = (1.0 - t * 0.85).clamp(0.15, 1.0);
            b.push_solid(
                format!("loading-spinner-dot-{step}"),
                ZIndex::LOADING_SPINNER,
                RectPx::new(dx, dy, 5.0, 5.0),
                with_alpha(theme.accent, alpha),
                [2.5; 4],
                0.0,
                [0.0; 4],
                None,
            );
        }
    }

    // Seek Indicator
    if state.seek_indicator.visible {
        let rect = RectPx::new((main_w - 160.0) * 0.5, h * 0.25, 160.0, 40.0);
        b.push_glass(
            "seek-indicator",
            ZIndex::SEEK_INDICATOR,
            rect,
            RootTokens::RADIUS_MD,
            None,
            ChameleonSkin::None,
            theme,
            None,
        );
        b.push_text(
            "seek-indicator-label",
            ZIndex::SEEK_INDICATOR,
            rect,
            state.seek_indicator.label.clone(),
            FontRole::Mono,
            14.0,
            600,
            [1.0; 4],
            TextAlign::Center,
            None,
        );
    }

    // Skip Indicator (`main.css:519-560`: centered pill `rgba(14,17,24,0.94)` + 34x34 badge + `+{N}s`/`-{N}s`)
    if state.skip_indicator.visible {
        let skip_cp = b.checkpoint();
        let secs = state.skip_indicator.accumulated_secs.abs();
        let label = if state.skip_indicator.forward {
            format!("+{secs}s")
        } else {
            format!("-{secs}s")
        };
        let rect = RectPx::new((main_w - 134.0) * 0.5, (h - 52.0) * 0.5, 134.0, 52.0);
        b.push_solid(
            "skip-indicator",
            ZIndex::SKIP_INDICATOR,
            rect,
            rgba(14, 17, 24, 0.94),
            [26.0; 4],
            1.0,
            rgba(255, 255, 255, 0.14),
            None,
        );
        let badge_r = RectPx::new(rect.x + 10.0, rect.y + 9.0, 34.0, 34.0);
        b.push_solid(
            "skip-indicator-badge",
            ZIndex::SKIP_INDICATOR,
            badge_r,
            rgba(255, 255, 255, 0.08),
            [17.0; 4],
            0.0,
            [0.0; 4],
            None,
        );
        b.push_svg(
            "skip-indicator-arrows",
            ZIndex::SKIP_INDICATOR,
            if state.skip_indicator.forward {
                SvgId::SkipForwardArrows
            } else {
                SvgId::SkipBackwardArrows
            },
            SvgVariant::Default,
            RectPx::new(badge_r.x + 8.0, badge_r.y + 8.0, 18.0, 18.0),
            [255; 4],
            None,
        );
        b.push_text(
            "skip-indicator-label",
            ZIndex::SKIP_INDICATOR,
            RectPx::new(rect.x + 52.0, rect.y + 15.0, 70.0, 22.0),
            label,
            FontRole::Mono,
            16.0,
            700,
            rgba(255, 255, 255, 0.96),
            TextAlign::Left,
            None,
        );
        if state.anim.skip_t < 0.999 {
            let kf_id = if state.skip_indicator.forward {
                KeyframeId::SkipIndicatorForwardPop
            } else {
                KeyframeId::SkipIndicatorBackwardPop
            };
            let kf = evaluate_keyframe(kf_id, state.anim.skip_t, state.motion_profile);
            b.apply_subtree_anim(
                skip_cp,
                rect.x + rect.width * 0.5,
                rect.y + rect.height * 0.5,
                kf,
                false,
            );
        }
    }

    // Stats Overlay (`top: 50, left: 15, z: 50`, 7 telemetry rows)
    if state.stats_overlay.visible {
        let fallback = |s: &str| {
            if s.trim().is_empty() {
                "-".to_string()
            } else {
                s.to_string()
            }
        };
        let speed_str = if (state.speed - state.speed.round()).abs() < 1e-4 {
            format!("{}x", state.speed.round() as i32)
        } else {
            format!(
                "{}x",
                format!("{:.2}", state.speed)
                    .trim_end_matches('0')
                    .trim_end_matches('.')
            )
        };
        let stats_rows = [
            ("Resolution:", fallback(&state.stats_overlay.resolution)),
            ("Bitrate:", fallback(&state.stats_overlay.bitrate)),
            (
                "Dropped Frames:",
                state.stats_overlay.dropped_frames.to_string(),
            ),
            ("FPS:", fallback(&state.stats_overlay.fps)),
            ("Codec:", fallback(&state.stats_overlay.codec)),
            ("Speed:", speed_str),
            ("Buffer:", format!("{:.1}s", state.stats_overlay.cache_secs)),
        ];
        let max_val_w = stats_rows
            .iter()
            .map(|(_, v)| measure_text_width(v, FontRole::Mono, 12.0, 600, 0.0, state.font_config))
            .fold(0.0_f32, f32::max);
        let overlay_w = (156.0 + max_val_w).clamp(260.0, (main_w - 30.0).max(260.0));
        let rect = RectPx::new(15.0, 50.0, overlay_w, 186.0);
        b.push_glass(
            "stats-overlay",
            ZIndex::STATS_AND_AB_LOOP,
            rect,
            10.0,
            Some(CssBackdropRule::StatsOverlay),
            ChameleonSkin::None,
            theme,
            None,
        );
        b.push_solid(
            "stats-overlay-bg",
            ZIndex::STATS_AND_AB_LOOP,
            rect,
            rgba(10, 12, 18, 0.88),
            [10.0; 4],
            1.0,
            rgba(255, 255, 255, 0.10),
            None,
        );
        for (idx, (k, v)) in stats_rows.iter().enumerate() {
            let ry = rect.y + 12.0 + (idx as f32) * 23.0;
            b.push_text(
                format!("stats-row-key-{idx}"),
                ZIndex::STATS_AND_AB_LOOP,
                RectPx::new(rect.x + 14.0, ry, 118.0, 18.0),
                *k,
                FontRole::Mono,
                12.0,
                500,
                rgba(255, 255, 255, 0.62),
                TextAlign::Left,
                Some(rect),
            );
            b.push_text(
                format!("stats-row-val-{idx}"),
                ZIndex::STATS_AND_AB_LOOP,
                RectPx::new(rect.x + 126.0, ry, rect.width - 140.0, 18.0),
                v.clone(),
                FontRole::Mono,
                12.0,
                600,
                rgba(255, 255, 255, 0.95),
                TextAlign::Right,
                Some(rect),
            );
        }
    }

    // A-B Loop Indicator: Electron `.ab-loop-indicator` (`main.css:2636-2656`) uses
    // `top: 102px; right: 15px; padding: 6px 14px; border-radius: 20px; font-size: 12px;
    // font-weight: 600` and inherits `--font-ui` (not the mono face). It has no min-width, so the
    // pill hugs the label. `animation: pulse` is opacity-only (`animations.css:123-126`) on every
    // visible state. Scaling the subtree (the old `PulseGlow`) re-rasterized the label at a new
    // fractional font size every frame, which made the text shimmer and shake.
    if state.ab_loop != AbLoopState::Off {
        let ab_cp = b.checkpoint();
        let right = 15.0 + playlist_w;
        let (pw, label) = match state.ab_loop {
            AbLoopState::Off => (0.0, String::new()),
            AbLoopState::PointASet(a_ms) => {
                let text = format!("A: {}", UiState::format_time((a_ms as f64) / 1000.0));
                let pw = measure_text_width(&text, FontRole::Ui, 12.0, 600, 0.0, state.font_config)
                    + 28.0;
                (pw, text)
            }
            AbLoopState::Active(a_ms, b_ms) => {
                let text = format!(
                    "A-B: {} - {}",
                    UiState::format_time((a_ms as f64) / 1000.0),
                    UiState::format_time((b_ms as f64) / 1000.0)
                );
                let pw = measure_text_width(&text, FontRole::Ui, 12.0, 600, 0.0, state.font_config)
                    + 28.0;
                (pw, text)
            }
        };
        let rect = RectPx::new((w - right - pw).max(8.0), 102.0, pw, 28.0);
        b.push_solid(
            "ab-loop-indicator",
            ZIndex::STATS_AND_AB_LOOP,
            rect,
            theme.accent,
            [20.0; 4],
            0.0,
            [0.0; 4],
            None,
        );
        b.push_text(
            "ab-loop-indicator-label",
            ZIndex::STATS_AND_AB_LOOP,
            RectPx::new(rect.x + 14.0, rect.y + 5.0, rect.width - 28.0, 18.0),
            label,
            FontRole::Ui,
            12.0,
            600,
            [1.0; 4],
            TextAlign::Center,
            None,
        );
        if state.anim.ab_pulse_phase > 1e-4 {
            let kf = evaluate_keyframe(
                KeyframeId::Pulse,
                state.anim.ab_pulse_phase,
                state.motion_profile,
            );
            b.apply_subtree_anim(
                ab_cp,
                rect.x + rect.width * 0.5,
                rect.y + rect.height * 0.5,
                kf,
                false,
            );
        }
    }

    // Dual-Geometry Recording Indicator: Burst split-flap pill (`z: 54`, `top: 48`, `height: 30`) vs Clip flush bezel tab (`z: 2000`, `top: 0`, `height: 28`, stop btn `z: 2501`)
    let rec_breath = evaluate_keyframe(
        KeyframeId::RecordingRubyBreath,
        state.anim.recording_pulse_phase,
        state.motion_profile,
    );
    match state.recording_indicator {
        RecordingIndicatorMode::Hidden => {}
        RecordingIndicatorMode::BurstActive { count } => {
            let count_str = count.to_string();
            let count_w = measure_text_width(
                &count_str,
                FontRole::Mono,
                12.0,
                700,
                0.0,
                state.font_config,
            )
            .max(16.0);
            let unit = if count == 1 { "frame" } else { "frames" };
            let unit_w =
                measure_text_width(unit, FontRole::Ui, 11.0, 600, 0.0, state.font_config).max(32.0);
            let chip_w = 8.0 + count_w + 4.0 + unit_w + 11.0;
            let title_w = 64.0;
            // `.recording-pill-inner` overlaps `.recording-view-saved` in grid slot `1/1`, sizing pill >=229.6px.
            let pill_w = (12.0 + 14.0 + 18.0 + 55.0 + 9.0 + chip_w + 12.0).max(229.6);
            let rect = RectPx::new((main_w - pill_w) * 0.5, 48.0, pill_w, 30.0);
            b.push_glass(
                "recording-burst-pill",
                ZIndex::RECORDING_BURST_PILL,
                rect,
                RootTokens::RADIUS_FULL,
                Some(CssBackdropRule::RecordingIndicator),
                ChameleonSkin::None,
                theme,
                None,
            );
            let cam_alpha = (255.0 * rec_breath.opacity).round().clamp(120.0, 255.0) as u8;
            b.push_svg(
                "recording-cam",
                ZIndex::RECORDING_BURST_PILL,
                SvgId::RecordingCam,
                SvgVariant::Default,
                RectPx::new(rect.x + 12.0, rect.y + 8.0, 14.0, 14.0),
                [0xff, 0x3b, 0x30, cam_alpha],
                None,
            );
            b.push_text(
                "recording-burst-title",
                ZIndex::RECORDING_BURST_PILL,
                RectPx::new(rect.x + 44.4, rect.y + 6.0, title_w, 18.0),
                "Recording",
                FontRole::Ui,
                12.0,
                600,
                [1.0; 4],
                TextAlign::Left,
                None,
            );
            let badge_r = RectPx::new(rect.x + 109.0, rect.y + 5.0, chip_w, 20.0);
            b.push_solid(
                "recording-burst-badge-bg",
                ZIndex::RECORDING_BURST_PILL,
                badge_r,
                rgba(255, 255, 255, 0.10),
                [999.0; 4],
                1.0,
                rgba(255, 255, 255, 0.10),
                None,
            );
            b.push_text(
                "recording-burst-count",
                ZIndex::RECORDING_BURST_PILL,
                RectPx::new(badge_r.x + 8.0, badge_r.y + 2.0, count_w, 16.0),
                format!("{count}"),
                FontRole::Ui,
                11.5,
                700,
                [1.0; 4],
                TextAlign::Right,
                None,
            );
            b.push_text(
                "recording-burst-unit",
                ZIndex::RECORDING_BURST_PILL,
                RectPx::new(
                    badge_r.x + 8.0 + count_w + 4.0,
                    badge_r.y + 2.5,
                    unit_w,
                    15.0,
                ),
                unit,
                FontRole::Ui,
                10.5,
                500,
                rgba(255, 255, 255, 0.65),
                TextAlign::Left,
                None,
            );
        }
        RecordingIndicatorMode::ClipActive { elapsed_ds } => {
            // Flush top-center bezel tab (`border-radius: 0 0 10px 10px`, `main.css:768-860`)
            let rect = RectPx::new((main_w - 110.0) * 0.5, 0.0, 110.0, 28.0);
            let mut glass =
                GlassRect::new(rect.x, rect.y, rect.width, rect.height, 0.0, theme.mode)
                    .with_chameleon_skin(ChameleonSkin::None, theme.mode)
                    .with_backdrop_rule(CssBackdropRule::RecordingClipActive, theme.mode);
            glass.corner_radii = [0.0, 0.0, 10.0, 10.0];
            glass.border_width = 0.0;
            b.push_custom_glass(
                "recording-clip-bezel-tab",
                ZIndex::MODAL_OVERLAY,
                glass,
                Some(CssBackdropRule::RecordingClipActive),
                None,
            );
            // `border-top: none !important` (`main.css:775`): extend y=-1 clipped to y>=0 so no top border draws.
            b.push_solid(
                "recording-clip-bezel-bg",
                ZIndex::MODAL_OVERLAY,
                RectPx::new(rect.x, -1.0, rect.width, rect.height + 1.0),
                rgba(10, 12, 18, 0.92),
                [0.0, 0.0, 10.0, 10.0],
                1.0,
                rgba(255, 255, 255, 0.15),
                Some(RectPx::new(
                    rect.x - 1.0,
                    0.0,
                    rect.width + 2.0,
                    rect.height + 1.0,
                )),
            );
            let mins = elapsed_ds / 600;
            let secs = (elapsed_ds % 600) / 10;
            let tenths = elapsed_ds % 10;
            b.push_text_spaced(
                "recording-clip-timer",
                ZIndex::MODAL_OVERLAY,
                RectPx::new(rect.x + 10.0, 5.0, 54.0, 18.0),
                format!("{mins:02}:{secs:02}.{tenths}"),
                FontRole::Mono,
                12.0,
                700,
                0.48,
                [1.0; 4],
                TextAlign::Left,
                None,
            );
            b.push_solid(
                "recording-clip-divider",
                ZIndex::MODAL_OVERLAY,
                RectPx::new(rect.x + 69.0, 8.0, 1.0, 12.0),
                rgba(255, 255, 255, 0.2),
                [0.0; 4],
                0.0,
                [0.0; 4],
                None,
            );
            let stop_rect = RectPx::new(rect.x + rect.width - 30.0, 4.0, 20.0, 20.0);
            let stop_alpha = rec_breath.opacity.clamp(0.65, 1.0);
            b.push_solid(
                "recording-stop-btn-bg",
                ZIndex::RECORDING_STOP_BTN,
                stop_rect,
                rgba(225, 29, 72, stop_alpha),
                [5.0; 4],
                0.0,
                [0.0; 4],
                None,
            );
            b.push_svg(
                "recording-stop-square",
                ZIndex::RECORDING_STOP_BTN,
                SvgId::RecordingStopSquare,
                SvgVariant::Default,
                RectPx::new(stop_rect.x + 5.0, stop_rect.y + 5.0, 10.0, 10.0),
                [255; 4],
                None,
            );
            b.push_hit(
                HitTarget::RecordingStopClip,
                ZIndex::RECORDING_STOP_BTN,
                stop_rect.expanded(6.0),
                CursorHint::Pointer,
            );
        }
        RecordingIndicatorMode::ClipSaving => {
            let rect = RectPx::new((main_w - 124.0) * 0.5, 0.0, 124.0, 28.0);
            let mut glass =
                GlassRect::new(rect.x, rect.y, rect.width, rect.height, 0.0, theme.mode)
                    .with_chameleon_skin(ChameleonSkin::None, theme.mode)
                    .with_backdrop_rule(CssBackdropRule::RecordingClipActive, theme.mode);
            glass.corner_radii = [0.0, 0.0, 10.0, 10.0];
            glass.border_width = 0.0;
            b.push_custom_glass(
                "recording-clip-bezel-tab",
                ZIndex::MODAL_OVERLAY,
                glass,
                Some(CssBackdropRule::RecordingClipActive),
                None,
            );
            b.push_solid(
                "recording-clip-bezel-bg",
                ZIndex::MODAL_OVERLAY,
                RectPx::new(rect.x, -1.0, rect.width, rect.height + 1.0),
                rgba(10, 12, 18, 0.92),
                [0.0, 0.0, 10.0, 10.0],
                1.0,
                rgba(255, 255, 255, 0.15),
                Some(RectPx::new(
                    rect.x - 1.0,
                    0.0,
                    rect.width + 2.0,
                    rect.height + 1.0,
                )),
            );
            b.push_text(
                "recording-clip-saving-lbl",
                ZIndex::MODAL_OVERLAY,
                RectPx::new(rect.x + 8.0, 5.0, rect.width - 16.0, 18.0),
                "Saving clip...",
                FontRole::Ui,
                11.5,
                600,
                [1.0; 4],
                TextAlign::Center,
                None,
            );
        }
        RecordingIndicatorMode::Saved { label_frames } => {
            if label_frames {
                let n = state.recording_saved_frame_count.max(1);
                let count_txt = if n == 1 {
                    "1 Frame".to_string()
                } else {
                    format!("{n} Frames")
                };
                let pill_w = 248.0;
                let rect = RectPx::new((main_w - pill_w) * 0.5, 48.0, pill_w, 30.0);
                b.push_glass(
                    "recording-saved-pill",
                    ZIndex::RECORDING_BURST_PILL,
                    rect,
                    RootTokens::RADIUS_FULL,
                    Some(CssBackdropRule::RecordingIndicator),
                    ChameleonSkin::None,
                    theme,
                    None,
                );
                b.push_svg(
                    "recording-saved-folder",
                    ZIndex::RECORDING_BURST_PILL,
                    SvgId::RecordingSavedFolder,
                    SvgVariant::Default,
                    RectPx::new(rect.x + 12.0, rect.y + 8.0, 14.0, 14.0),
                    [0x34, 0xd3, 0x99, 255],
                    None,
                );
                b.push_text(
                    "recording-saved-text",
                    ZIndex::RECORDING_BURST_PILL,
                    RectPx::new(rect.x + 32.0, rect.y + 6.0, 142.0, 18.0),
                    "Saved to Screenshots",
                    FontRole::Ui,
                    12.0,
                    600,
                    [1.0; 4],
                    TextAlign::Left,
                    None,
                );
                let badge_r = RectPx::new(rect.x + 174.0, rect.y + 5.0, 64.0, 20.0);
                b.push_solid(
                    "recording-saved-count-bg",
                    ZIndex::RECORDING_BURST_PILL,
                    badge_r,
                    rgba(52, 211, 153, 0.16),
                    [999.0; 4],
                    1.0,
                    rgba(52, 211, 153, 0.35),
                    None,
                );
                b.push_text(
                    "recording-saved-count",
                    ZIndex::RECORDING_BURST_PILL,
                    RectPx::new(badge_r.x + 4.0, badge_r.y + 2.0, badge_r.width - 8.0, 16.0),
                    count_txt,
                    FontRole::Ui,
                    10.5,
                    600,
                    rgba8(0x34, 0xd3, 0x99, 255),
                    TextAlign::Center,
                    None,
                );
                b.push_hit(
                    HitTarget::RecordingOpenFolder,
                    ZIndex::RECORDING_BURST_PILL,
                    rect,
                    CursorHint::Pointer,
                );
            } else {
                // Clip saved morph (`main.css:908-935`, `controls.js:1798-1840`): green folder morph in flush bezel tab
                let rect = RectPx::new((main_w - 110.0) * 0.5, 0.0, 110.0, 28.0);
                let mut glass =
                    GlassRect::new(rect.x, rect.y, rect.width, rect.height, 0.0, theme.mode)
                        .with_chameleon_skin(ChameleonSkin::None, theme.mode)
                        .with_backdrop_rule(CssBackdropRule::RecordingClipActive, theme.mode);
                glass.corner_radii = [0.0, 0.0, 10.0, 10.0];
                glass.border_width = 0.0;
                b.push_custom_glass(
                    "recording-saved-pill",
                    ZIndex::MODAL_OVERLAY,
                    glass,
                    Some(CssBackdropRule::RecordingClipActive),
                    None,
                );
                b.push_solid(
                    "recording-clip-saved-bg",
                    ZIndex::MODAL_OVERLAY,
                    RectPx::new(rect.x, -1.0, rect.width, rect.height + 1.0),
                    rgba(10, 12, 18, 0.92),
                    [0.0, 0.0, 10.0, 10.0],
                    1.0,
                    rgba(16, 185, 129, 0.45),
                    Some(RectPx::new(
                        rect.x - 1.0,
                        0.0,
                        rect.width + 2.0,
                        rect.height + 1.0,
                    )),
                );
                b.push_text(
                    "recording-saved-text",
                    ZIndex::MODAL_OVERLAY,
                    RectPx::new(rect.x + 10.0, 5.0, 58.0, 18.0),
                    "Saved",
                    FontRole::Ui,
                    12.0,
                    600,
                    rgba8(0x34, 0xd3, 0x99, 255),
                    TextAlign::Left,
                    None,
                );
                let folder_btn = RectPx::new(rect.x + rect.width - 30.0, 4.0, 20.0, 20.0);
                b.push_solid(
                    "recording-saved-folder-bg",
                    ZIndex::RECORDING_STOP_BTN,
                    folder_btn,
                    rgba8(0x10, 0xb9, 0x81, 255),
                    [5.0; 4],
                    0.0,
                    [0.0; 4],
                    None,
                );
                b.push_svg(
                    "recording-saved-folder",
                    ZIndex::RECORDING_STOP_BTN,
                    SvgId::RecordingStopFolder,
                    SvgVariant::Default,
                    RectPx::new(folder_btn.x + 4.0, folder_btn.y + 4.0, 12.0, 12.0),
                    [255; 4],
                    None,
                );
                b.push_hit(
                    HitTarget::RecordingOpenFolder,
                    ZIndex::RECORDING_STOP_BTN,
                    rect,
                    CursorHint::Pointer,
                );
            }
        }
    }

    // Screenshot Preview & Delete/Restore Carousel
    let top = if state.stats_overlay.visible {
        296.0
    } else {
        86.0
    };
    let mut notice_anchor_y: Option<f32> = None;
    let preview_aspect = state.video_aspect_ratio.clamp(0.5, 2.5);
    let preview_img_w = 252.0_f32;
    let preview_img_h = (preview_img_w / preview_aspect).clamp(90.0, 260.0);
    let preview_card_h = preview_img_h + 8.0;
    match state.carousel_mode {
        CarouselMode::Hidden => {}
        CarouselMode::SinglePreview => {
            let preview_cp = b.checkpoint();
            let rect = RectPx::new(24.0, top, 260.0, preview_card_h);
            notice_anchor_y = Some(rect.y + rect.height + 10.0);
            b.push_glass(
                "screenshot-preview",
                ZIndex::SCREENSHOT_PREVIEW,
                rect,
                RootTokens::RADIUS_MD,
                None,
                ChameleonSkin::None,
                theme,
                None,
            );
            b.image_slots.push(UiImageSlot {
                id: "screenshot-preview-img".to_string(),
                z_index: ZIndex::SCREENSHOT_PREVIEW,
                rect: RectPx::new(rect.x + 4.0, rect.y + 4.0, preview_img_w, preview_img_h),
                corner_radius: RootTokens::RADIUS_SM,
                opacity: 1.0,
                key: if state.anim.screenshot_deleting {
                    format!("screenshot-deleted:{}", state.carousel_index)
                } else {
                    format!("screenshot:{}", state.carousel_index)
                },
                clip_rect: Some(rect),
            });
            if let Some((badge_text, is_deleted)) = &state.screenshot_badge {
                let badge_upper = badge_text.to_uppercase();
                let bw = (measure_text_width(
                    &badge_upper,
                    FontRole::Ui,
                    11.0,
                    600,
                    0.5,
                    state.font_config,
                ) + 20.0)
                    .clamp(58.0, 124.0);
                let badge_r = RectPx::new(rect.x + 10.0, rect.y + 10.0, bw, 22.0);
                let (badge_bg, badge_br, badge_fg) = if *is_deleted {
                    (
                        rgba(45, 12, 14, 0.94),
                        rgba(255, 77, 77, 0.6),
                        rgba8(0xff, 0x6b, 0x6b, 255),
                    )
                } else {
                    (
                        rgba(10, 36, 20, 0.94),
                        rgba(52, 211, 153, 0.6),
                        rgba8(0x34, 0xd3, 0x99, 255),
                    )
                };
                b.push_solid(
                    "screenshot-preview-badge-bg",
                    ZIndex::SCREENSHOT_PREVIEW + 1,
                    badge_r,
                    badge_bg,
                    [6.0; 4],
                    1.0,
                    badge_br,
                    Some(rect),
                );
                b.push_text_spaced(
                    "screenshot-preview-badge-text",
                    ZIndex::SCREENSHOT_PREVIEW + 1,
                    badge_r,
                    badge_upper,
                    FontRole::Ui,
                    11.0,
                    600,
                    0.5,
                    badge_fg,
                    TextAlign::Center,
                    Some(rect),
                );
            }
            if !state.anim.screenshot_deleting {
                b.push_hit(
                    HitTarget::SinglePreviewOpenFolder,
                    ZIndex::SCREENSHOT_PREVIEW,
                    rect,
                    CursorHint::Pointer,
                );
            }
            if state.anim.screenshot_t < 0.999 {
                let kf_id = if state.anim.screenshot_deleting {
                    KeyframeId::PreviewDeletePop
                } else if state.screenshot_badge.is_some() {
                    KeyframeId::PreviewRestorePop
                } else {
                    KeyframeId::SlideUp
                };
                let kf = evaluate_keyframe(kf_id, state.anim.screenshot_t, state.motion_profile);
                b.apply_subtree_anim(
                    preview_cp,
                    rect.x + rect.width * 0.5,
                    rect.y + rect.height * 0.5,
                    kf,
                    state.anim.screenshot_deleting,
                );
            }
        }
        CarouselMode::DeleteCarousel | CarouselMode::RestoreCarousel => {
            // Compact top-left widget (`top: 86px, left: 24px, width: 260px`)
            // matching golden `overlay-screenshot-carousel-1280x720.png`
            let carousel_cp = b.checkpoint();
            let cw = 260.0_f32;
            let ch = preview_card_h;
            let rect = RectPx::new(24.0, top, cw, ch);
            notice_anchor_y = Some(rect.y + ch + 10.0);
            let carousel_hovered = matches!(
                state.hovered_target,
                Some(
                    HitTarget::CarouselPrev
                        | HitTarget::CarouselNext
                        | HitTarget::CarouselConfirm
                        | HitTarget::CarouselClose
                )
            );
            let carousel_opacity = if carousel_hovered { 1.0 } else { 0.82 };
            b.push_glass(
                "screenshot-carousel",
                ZIndex::SCREENSHOT_CAROUSEL,
                rect,
                RootTokens::RADIUS_MD,
                Some(CssBackdropRule::CarouselMode),
                ChameleonSkin::None,
                theme,
                None,
            );
            if let Some(last) = b.glass_panels.last_mut() {
                last.glass.opacity *= carousel_opacity;
                last.glass.shadow_color[3] *= carousel_opacity;
            }
            b.image_slots.push(UiImageSlot {
                id: "screenshot-carousel-img".to_string(),
                z_index: ZIndex::SCREENSHOT_CAROUSEL,
                rect: RectPx::new(rect.x + 4.0, rect.y + 4.0, cw - 8.0, ch - 8.0),
                corner_radius: RootTokens::RADIUS_SM,
                opacity: carousel_opacity,
                key: format!("carousel:{}:{}", state.carousel_index, state.carousel_total),
                clip_rect: Some(rect),
            });
            // Floating dark pill badge "N / M" at top-left of preview image
            let badge_r = RectPx::new(rect.x + 10.0, rect.y + 10.0, 44.0, 20.0);
            b.push_solid(
                "carousel-badge-bg",
                ZIndex::SCREENSHOT_CAROUSEL + 1,
                badge_r,
                rgba(10, 12, 18, 0.72 * carousel_opacity),
                [10.0; 4],
                0.0,
                [0.0; 4],
                Some(rect),
            );
            b.push_text(
                "carousel-badge",
                ZIndex::SCREENSHOT_CAROUSEL + 1,
                RectPx::new(badge_r.x + 2.0, badge_r.y + 1.0, badge_r.width - 4.0, 18.0),
                format!(
                    "{} / {}",
                    state.carousel_index + 1,
                    state.carousel_total.max(1)
                ),
                FontRole::Mono,
                11.0,
                600,
                [1.0, 1.0, 1.0, carousel_opacity],
                TextAlign::Center,
                Some(rect),
            );
            let prev_r = RectPx::new(rect.x + 10.0, rect.y + ch * 0.5 - 11.0, 22.0, 22.0);
            let next_r = RectPx::new(rect.x + cw - 32.0, rect.y + ch * 0.5 - 11.0, 22.0, 22.0);
            let close_r = RectPx::new(rect.x + cw - 30.0, rect.y + 8.0, 22.0, 22.0);
            for (bg_id, r) in [
                ("carousel-prev-bg", prev_r),
                ("carousel-next-bg", next_r),
                ("carousel-close-bg", close_r),
            ] {
                b.push_solid(
                    bg_id,
                    ZIndex::SCREENSHOT_CAROUSEL + 1,
                    r,
                    rgba(10, 12, 18, 0.68 * carousel_opacity),
                    [11.0; 4],
                    0.0,
                    [0.0; 4],
                    Some(rect),
                );
            }
            let svg_tint_alpha = (255.0 * carousel_opacity).round() as u8;
            b.push_svg(
                "carousel-prev",
                ZIndex::SCREENSHOT_CAROUSEL + 1,
                SvgId::CarouselPrevArrow,
                SvgVariant::Default,
                RectPx::new(prev_r.x + 3.0, prev_r.y + 3.0, 16.0, 16.0),
                [255, 255, 255, svg_tint_alpha],
                Some(rect),
            );
            b.push_svg(
                "carousel-next",
                ZIndex::SCREENSHOT_CAROUSEL + 1,
                SvgId::CarouselNextArrow,
                SvgVariant::Default,
                RectPx::new(next_r.x + 3.0, next_r.y + 3.0, 16.0, 16.0),
                [255, 255, 255, svg_tint_alpha],
                Some(rect),
            );
            b.push_svg(
                "carousel-close",
                ZIndex::SCREENSHOT_CAROUSEL + 1,
                SvgId::ModalCloseSubtitle,
                SvgVariant::Default,
                RectPx::new(close_r.x + 5.0, close_r.y + 5.0, 12.0, 12.0),
                [255, 255, 255, svg_tint_alpha],
                Some(rect),
            );
            // Clickable action confirm pill ("Delete" / "Restore")
            let is_restore = state.carousel_mode == CarouselMode::RestoreCarousel;
            let confirm_lbl = if is_restore { "Restore" } else { "Delete" };
            let confirm_r = RectPx::new(rect.x + cw - 72.0, rect.y + ch - 30.0, 62.0, 22.0);
            let confirm_bg = if is_restore {
                rgba(34, 197, 94, 0.90 * carousel_opacity)
            } else {
                rgba(229, 9, 20, 0.90 * carousel_opacity)
            };
            b.push_solid(
                "carousel-confirm-bg",
                ZIndex::SCREENSHOT_CAROUSEL + 1,
                confirm_r,
                confirm_bg,
                [11.0; 4],
                0.0,
                [0.0; 4],
                Some(rect),
            );
            b.push_text(
                "carousel-confirm-text",
                ZIndex::SCREENSHOT_CAROUSEL + 1,
                confirm_r,
                confirm_lbl,
                FontRole::Ui,
                11.0,
                700,
                [1.0, 1.0, 1.0, carousel_opacity],
                TextAlign::Center,
                Some(rect),
            );
            b.push_hit(
                HitTarget::CarouselPrev,
                ZIndex::SCREENSHOT_CAROUSEL + 1,
                prev_r,
                CursorHint::Pointer,
            );
            b.push_hit(
                HitTarget::CarouselNext,
                ZIndex::SCREENSHOT_CAROUSEL + 1,
                next_r,
                CursorHint::Pointer,
            );
            b.push_hit(
                HitTarget::CarouselConfirm,
                ZIndex::SCREENSHOT_CAROUSEL + 1,
                confirm_r,
                CursorHint::Pointer,
            );
            b.push_hit(
                HitTarget::CarouselClose,
                ZIndex::SCREENSHOT_CAROUSEL + 1,
                close_r,
                CursorHint::Pointer,
            );
            if state.anim.screenshot_t < 0.999 {
                let kf = evaluate_keyframe(
                    if state.anim.screenshot_deleting {
                        KeyframeId::PreviewDeletePop
                    } else {
                        KeyframeId::PreviewRestorePop
                    },
                    state.anim.screenshot_t,
                    state.motion_profile,
                );
                b.apply_subtree_anim(
                    carousel_cp,
                    rect.x + rect.width * 0.5,
                    rect.y + rect.height * 0.5,
                    kf,
                    false,
                );
            }
        }
    }
    if let Some(notice) = &state.screenshot_notice {
        let notice_cp = b.checkpoint();
        let ny = notice_anchor_y.unwrap_or(top);
        let nw = (measure_text_width(notice, FontRole::Ui, 12.0, 500, 0.2, state.font_config)
            + 28.0)
            .clamp(120.0, 260.0);
        let nrect = RectPx::new(24.0, ny, nw, 30.0);
        b.push_solid(
            "screenshot-notice",
            ZIndex::SCREENSHOT_NOTICE,
            nrect,
            theme.bg_glass_heavy,
            [9.0; 4],
            1.0,
            theme.border_color,
            None,
        );
        b.push_text_spaced(
            "screenshot-notice-text",
            ZIndex::SCREENSHOT_NOTICE,
            RectPx::new(
                nrect.x + 14.0,
                nrect.y,
                (nrect.width - 28.0).max(24.0),
                nrect.height,
            ),
            notice.clone(),
            FontRole::Ui,
            12.0,
            500,
            0.2,
            theme.text_primary,
            TextAlign::Left,
            None,
        );
        if state.anim.screenshot_t < 0.999 {
            let kf = evaluate_keyframe(
                KeyframeId::ScreenshotNoticeIn,
                MotionProfile::ease_emphasized(state.anim.screenshot_t),
                state.motion_profile,
            );
            b.apply_subtree_anim(
                notice_cp,
                nrect.x + nrect.width * 0.5,
                nrect.y + nrect.height * 0.5,
                kf,
                false,
            );
        }
    }

    // Toast Stack (max 3, bottom: 80px centered horizontally per `animations.css:178-202`)
    for (i, toast) in state.toasts.iter().take(MAX_VISIBLE_TOASTS).enumerate() {
        let toast_cp = b.checkpoint();
        let is_update_avail = toast.kind == ToastKind::UpdateAvailable;
        let is_update_ok = toast.kind == ToastKind::UpdateSuccess;
        let is_update = is_update_avail || is_update_ok;
        let text_w = measure_text_width(
            &toast.message,
            FontRole::Ui,
            13.0,
            500,
            0.0,
            state.font_config,
        );
        let ver_str = toast.version_tag.as_deref().map(|v| {
            if v.starts_with('v') {
                v.to_string()
            } else {
                format!("v{v}")
            }
        });
        let ver_w = ver_str.as_deref().map_or(0.0, |v| {
            measure_text_width(v, FontRole::Mono, 11.0, 600, 0.0, state.font_config) + 20.0
        });
        let action_lbl = if toast.downloading {
            "Updating..."
        } else {
            "Update Now"
        };
        let action_w = if is_update_avail {
            measure_text_width(action_lbl, FontRole::Ui, 11.5, 600, 0.0, state.font_config) + 34.0
        } else {
            0.0
        };
        let tw = if is_update_avail {
            (40.0 + text_w + 10.0 + ver_w + action_w + 36.0).clamp(280.0, (w - 24.0).min(560.0))
        } else if is_update_ok {
            (38.0 + text_w + 36.0).clamp(180.0, (w - 24.0).min(460.0))
        } else {
            (text_w + 40.0).clamp(120.0, 320.0).min(w - 24.0)
        };
        let th = 40.0;
        let rect = RectPx::new((w - tw) * 0.5, h - 80.0 - th - (i as f32) * 48.0, tw, th);
        let (bg, border, fg, rule) = if is_update {
            let (bg, br, fg) = theme.update_pill_colors();
            (bg, br, fg, Some(CssBackdropRule::ToastUpdatePill))
        } else {
            let (bg, br, fg) = theme.toast_colors();
            (bg, br, fg, None)
        };
        if let Some(r) = rule {
            let _ = (bg, border);
            b.push_glass(
                format!("toast-{}", toast.id),
                ZIndex::TOAST_CONTAINER,
                rect,
                if is_update { 20.0 } else { 10.0 },
                Some(r),
                ChameleonSkin::None,
                theme,
                None,
            );
        } else {
            b.push_solid(
                format!("toast-{}", toast.id),
                ZIndex::TOAST_CONTAINER,
                rect,
                bg,
                [10.0; 4],
                1.0,
                border,
                None,
            );
        }
        if is_update_avail {
            let badge_r = RectPx::new(rect.x + 10.0, rect.y + 8.0, 24.0, 24.0);
            b.push_solid(
                format!("toast-icon-badge-{}", toast.id),
                ZIndex::TOAST_CONTAINER,
                badge_r,
                with_alpha(theme.accent, 0.16),
                [12.0; 4],
                0.0,
                [0.0; 4],
                Some(rect),
            );
            b.push_svg(
                format!("toast-icon-{}", toast.id),
                ZIndex::TOAST_CONTAINER,
                SvgId::ToastUpdateBadge,
                SvgVariant::Default,
                RectPx::new(badge_r.x + 5.5, badge_r.y + 5.5, 13.0, 13.0),
                rgba_f32_to_u8(theme.accent),
                Some(rect),
            );
            let close_r = RectPx::new(rect.x + rect.width - 28.0, rect.y + 9.0, 22.0, 22.0);
            let act_r = RectPx::new(close_r.x - 6.0 - action_w, rect.y + 7.0, action_w, 26.0);
            let msg_max_w = (act_r.x - (badge_r.x + badge_r.width + 8.0) - ver_w - 6.0).max(40.0);
            let msg_w = text_w.min(msg_max_w);
            b.push_text(
                format!("toast-msg-{}", toast.id),
                ZIndex::TOAST_CONTAINER,
                RectPx::new(badge_r.x + badge_r.width + 8.0, rect.y + 10.0, msg_w, 20.0),
                toast.message.clone(),
                FontRole::Ui,
                13.0,
                500,
                fg,
                TextAlign::Left,
                Some(rect),
            );
            if let Some(ver) = &ver_str {
                let ver_r = RectPx::new(
                    badge_r.x + badge_r.width + 14.0 + msg_w,
                    rect.y + 10.0,
                    (ver_w - 6.0).max(32.0),
                    20.0,
                );
                b.push_solid(
                    format!("toast-ver-bg-{}", toast.id),
                    ZIndex::TOAST_CONTAINER,
                    ver_r,
                    rgba(255, 255, 255, 0.08),
                    [6.0; 4],
                    1.0,
                    rgba(255, 255, 255, 0.14),
                    Some(rect),
                );
                b.push_text(
                    format!("toast-ver-txt-{}", toast.id),
                    ZIndex::TOAST_CONTAINER,
                    RectPx::new(ver_r.x + 2.0, ver_r.y + 2.0, ver_r.width - 4.0, 16.0),
                    ver.clone(),
                    FontRole::Mono,
                    11.0,
                    600,
                    fg,
                    TextAlign::Center,
                    Some(rect),
                );
            }
            let act_hov = !toast.downloading && b.is_hovered(HitTarget::ToastUpdateNow);
            b.push_solid(
                format!("toast-update-btn-{}", toast.id),
                ZIndex::TOAST_CONTAINER,
                act_r,
                if toast.downloading {
                    with_alpha(theme.accent, 0.45)
                } else if act_hov {
                    theme.accent_hover
                } else {
                    theme.accent
                },
                [13.0; 4],
                0.0,
                [0.0; 4],
                Some(rect),
            );
            b.push_svg(
                format!("toast-update-ico-{}", toast.id),
                ZIndex::TOAST_CONTAINER,
                SvgId::ToastUpdateAction,
                SvgVariant::Default,
                RectPx::new(act_r.x + 9.0, act_r.y + 7.0, 12.0, 12.0),
                [255; 4],
                Some(rect),
            );
            b.push_text(
                format!("toast-update-lbl-{}", toast.id),
                ZIndex::TOAST_CONTAINER,
                RectPx::new(act_r.x + 24.0, act_r.y + 4.0, act_r.width - 30.0, 18.0),
                action_lbl,
                FontRole::Ui,
                11.5,
                600,
                [1.0; 4],
                TextAlign::Left,
                Some(rect),
            );
            if !toast.downloading {
                b.push_hit(
                    HitTarget::ToastUpdateNow,
                    ZIndex::TOAST_CONTAINER + 1,
                    act_r,
                    CursorHint::Pointer,
                );
            }
            b.push_svg(
                format!("toast-close-{}", toast.id),
                ZIndex::TOAST_CONTAINER,
                SvgId::ToastUpdateClose,
                SvgVariant::Default,
                RectPx::new(close_r.x + 5.5, close_r.y + 5.5, 11.0, 11.0),
                rgba_f32_to_u8(fg),
                Some(rect),
            );
            b.push_hit(
                HitTarget::ToastDismiss(toast.id),
                ZIndex::TOAST_CONTAINER + 1,
                close_r,
                CursorHint::Pointer,
            );
        } else if is_update_ok {
            b.push_svg(
                format!("toast-ok-ico-{}", toast.id),
                ZIndex::TOAST_CONTAINER,
                SvgId::ToastSuccessCheck,
                SvgVariant::Default,
                RectPx::new(rect.x + 12.0, rect.y + 13.0, 14.0, 14.0),
                [0x22, 0xc5, 0x5e, 255],
                Some(rect),
            );
            let close_r = RectPx::new(rect.x + rect.width - 28.0, rect.y + 9.0, 22.0, 22.0);
            b.push_text(
                format!("toast-msg-{}", toast.id),
                ZIndex::TOAST_CONTAINER,
                RectPx::new(rect.x + 32.0, rect.y + 10.0, rect.width - 66.0, 20.0),
                toast.message.clone(),
                FontRole::Ui,
                13.0,
                500,
                fg,
                TextAlign::Left,
                Some(rect),
            );
            b.push_svg(
                format!("toast-close-{}", toast.id),
                ZIndex::TOAST_CONTAINER,
                SvgId::ToastUpdateClose,
                SvgVariant::Default,
                RectPx::new(close_r.x + 5.5, close_r.y + 5.5, 11.0, 11.0),
                rgba_f32_to_u8(fg),
                Some(rect),
            );
            b.push_hit(
                HitTarget::ToastDismiss(toast.id),
                ZIndex::TOAST_CONTAINER + 1,
                close_r,
                CursorHint::Pointer,
            );
        } else {
            b.push_text_wrapped(
                format!("toast-msg-{}", toast.id),
                ZIndex::TOAST_CONTAINER,
                RectPx::new(rect.x + 12.0, rect.y + 10.0, rect.width - 24.0, 30.0),
                toast.message.clone(),
                FontRole::Ui,
                13.0,
                500,
                fg,
                TextAlign::Center,
                Some(rect),
            );
            b.push_hit(
                HitTarget::ToastDismiss(toast.id),
                ZIndex::TOAST_CONTAINER,
                rect,
                CursorHint::Pointer,
            );
        }
        if toast.closing {
            let exit_p = MotionProfile::ease_standard(toast.exit_t);
            let kf = KeyframeSample {
                opacity: (1.0 - exit_p).clamp(0.0, 1.0),
                scale_x: 1.0 - 0.05 * exit_p,
                scale_y: 1.0 - 0.05 * exit_p,
                ..Default::default()
            };
            b.apply_subtree_anim(
                toast_cp,
                rect.x + rect.width * 0.5,
                rect.y + rect.height * 0.5,
                kf,
                false,
            );
            b.hit_regions.truncate(toast_cp.hit);
        } else {
            let visible_len = state.toasts.len().min(MAX_VISIBLE_TOASTS);
            let enter_t = if toast.enter_t < 0.999 {
                toast.enter_t
            } else if i + 1 == visible_len && state.anim.toast_t < 0.999 {
                state.anim.toast_t
            } else {
                1.0
            };
            if enter_t < 0.999 {
                let kf = evaluate_keyframe(
                    KeyframeId::ToastIn,
                    MotionProfile::ease_emphasized(enter_t),
                    state.motion_profile,
                );
                b.apply_subtree_anim(
                    toast_cp,
                    rect.x + rect.width * 0.5,
                    rect.y + rect.height * 0.5,
                    kf,
                    false,
                );
            }
        }
    }

    // Drag-and-drop Overlay (`z: 10060`, `player.js:300-316`, `main.css`)
    if state.drop_overlay_active {
        let rect = RectPx::new(0.0, 0.0, w, h);
        b.push_glass(
            "drop-overlay",
            ZIndex::DROP_OVERLAY,
            rect,
            0.0,
            Some(CssBackdropRule::DropOverlay),
            ChameleonSkin::None,
            theme,
            None,
        );
        let card_r = RectPx::new((w - 280.0) * 0.5, (h - 132.0) * 0.5, 280.0, 132.0);
        b.push_solid(
            "drop-overlay-content",
            ZIndex::DROP_OVERLAY,
            card_r,
            rgba(18, 22, 32, 0.86),
            [16.0; 4],
            2.0,
            with_alpha(theme.accent, 0.55),
            None,
        );
        b.push_svg(
            "drop-overlay-icon",
            ZIndex::DROP_OVERLAY,
            SvgId::WelcomeOpenFile,
            SvgVariant::Default,
            RectPx::new(
                card_r.x + (card_r.width - 40.0) * 0.5,
                card_r.y + 24.0,
                40.0,
                40.0,
            ),
            rgba_f32_to_u8(theme.accent),
            Some(card_r),
        );
        b.push_text(
            "drop-overlay-text",
            ZIndex::DROP_OVERLAY,
            RectPx::new(card_r.x + 16.0, card_r.y + 80.0, card_r.width - 32.0, 26.0),
            "Drop media file to play",
            FontRole::Display,
            16.0,
            600,
            [1.0; 4],
            TextAlign::Center,
            Some(card_r),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn build_modal(
    b: &mut FrameBuilder,
    state: &UiState,
    bp: &BreakpointState,
    theme: &ThemeTokens,
    w: f32,
    h: f32,
    playlist_w: f32,
    modal: ModalId,
) {
    // Modal backdrop (`z: 2000`, `rgba(0,0,0,0.85)` per `main.css:2666`, `0.2` for BgSettings per `main.css:2978`)
    let pre_modal_hit_cp = b.checkpoint().hit;
    let backdrop_fade = if state.anim.modal_closing {
        evaluate_keyframe(
            KeyframeId::FadeOut,
            state.anim.modal_t,
            state.motion_profile,
        )
        .opacity
    } else if state.anim.modal_t < 0.999 {
        evaluate_keyframe(KeyframeId::FadeIn, state.anim.modal_t, state.motion_profile).opacity
    } else {
        1.0
    };
    let backdrop_alpha = (if modal == ModalId::BgSettings {
        0.2
    } else {
        0.85
    }) * backdrop_fade;
    b.push_solid(
        "modal-backdrop",
        ZIndex::MODAL_OVERLAY,
        RectPx::new(0.0, 0.0, w, h),
        rgba(0, 0, 0, backdrop_alpha),
        [0.0; 4],
        0.0,
        [0.0; 4],
        None,
    );
    b.push_hit(
        HitTarget::ModalBackdrop,
        ZIndex::MODAL_OVERLAY,
        RectPx::new(0.0, 0.0, w, h),
        CursorHint::Default,
    );
    let modal_panel_cp = b.checkpoint();

    let is_light = state.theme == ThemeMode::Light;
    let card_bg = if is_light {
        rgba(0, 0, 0, 0.025)
    } else {
        rgba(255, 255, 255, 0.025)
    };
    let card_border = if is_light {
        theme.border_color
    } else {
        rgba(255, 255, 255, 0.05)
    };
    let pill_idle_bg = if is_light {
        rgba(0, 0, 0, 0.04)
    } else {
        rgba(255, 255, 255, 0.045)
    };
    let pill_idle_border = if is_light {
        theme.border_color
    } else {
        rgba(255, 255, 255, 0.08)
    };

    let sub_extra_h = {
        let mw = (w * 0.92).min(470.0);
        let cw = mw - 36.0;
        let max_inner_w = (cw - 28.0).max(60.0);
        let mut px = 50.0_f32;
        let mut rows = 1usize;
        for trk in &state.subtitle_tracks {
            let lbl = if trk.label.is_empty() {
                format!("Track {}", trk.id)
            } else {
                trk.label.clone()
            };
            let lbl_w = measure_text_width(&lbl, FontRole::Ui, 11.5, 600, 0.0, state.font_config);
            let pw = (lbl_w + 22.0).clamp(52.0, max_inner_w.min(180.0));
            if px > 0.0 && px + pw > max_inner_w {
                rows += 1;
                px = 0.0;
            }
            px += pw + 6.0;
        }
        ((rows.saturating_sub(1)) as f32) * 36.0
    };
    let audio_extra_h = {
        let mw = (w * 0.92).min(470.0);
        let cw = mw - 38.0;
        let max_inner_w = (cw - 28.0).max(70.0);
        let mut px = 0.0_f32;
        let mut rows = 1usize;
        for trk in &state.audio_tracks {
            let lbl = if trk.label.is_empty() {
                format!("Track {}", trk.id)
            } else {
                trk.label.clone()
            };
            let lbl_w = measure_text_width(&lbl, FontRole::Ui, 11.0, 600, 0.0, state.font_config);
            let tw = (lbl_w + 22.0).clamp(64.0, max_inner_w.min(190.0));
            if px > 0.0 && px + tw > max_inner_w {
                rows += 1;
                px = 0.0;
            }
            px += tw + 6.0;
        }
        ((rows.saturating_sub(1)) as f32) * 36.0
    };

    let z = ZIndex::MODAL_OVERLAY + 1;
    let panel_rect = match modal {
        ModalId::Equalizer => {
            let mw = (w * 0.94).min(920.0);
            let mh = (h * 0.88).min(557.0);
            RectPx::new((w - mw) * 0.5, (h - mh) * 0.5, mw, mh)
        }
        ModalId::Speed => {
            let mw = (w * 0.92).min(440.0);
            let mh = (h * 0.86).min(372.0);
            RectPx::new((w - mw) * 0.5, (h - mh) * 0.5, mw, mh)
        }
        ModalId::Subtitles => {
            let mw = (w * 0.92).min(470.0);
            let mh = (h * 0.88)
                .min(580.0 + sub_extra_h)
                .min((h - 24.0).max(320.0));
            RectPx::new((w - mw) * 0.5, (h - mh) * 0.5, mw, mh)
        }
        ModalId::Audio => {
            let mw = (w * 0.92).min(470.0);
            let mh = (h * 0.84)
                .min(367.0 + audio_extra_h)
                .min((h - 24.0).max(280.0));
            RectPx::new((w - mw) * 0.5, (h - mh) * 0.5, mw, mh)
        }
        ModalId::Settings => {
            let mw = (w * 0.92).min(520.0);
            let mh = h * 0.88;
            RectPx::new((w - mw) * 0.5, (h - mh) * 0.5, mw, mh)
        }
        ModalId::NetworkStream => {
            let mw = (w * 0.90).min(520.0);
            let mh = (h * 0.78).min(234.0);
            RectPx::new((w - mw) * 0.5, (h - mh) * 0.5, mw, mh)
        }
        ModalId::BgSettings => {
            let mw = if bp.max_w_950 {
                (w - 32.0).min(340.0)
            } else {
                (w * 0.92).min(360.0)
            };
            let right_pad = 20.0 + playlist_w;
            let mh = (h - 124.0).clamp(220.0, 640.0);
            RectPx::new((w - right_pad - mw).max(16.0), h - 80.0 - mh, mw, mh)
        }
    };

    let (backdrop_rule, skin) = match modal {
        ModalId::Settings => (Some(CssBackdropRule::SettingsModal), ChameleonSkin::None),
        ModalId::BgSettings => (
            Some(CssBackdropRule::BgSettingsPanel),
            ChameleonSkin::from(state.welcome_background),
        ),
        _ => (Some(CssBackdropRule::ModalPanel), ChameleonSkin::None),
    };
    let panel_radius = match modal {
        ModalId::Speed | ModalId::Subtitles | ModalId::Audio | ModalId::Settings => 20.0,
        _ => RootTokens::RADIUS_LG,
    };

    b.push_glass(
        format!("modal-{modal:?}"),
        z,
        panel_rect,
        panel_radius,
        backdrop_rule,
        skin,
        theme,
        None,
    );
    if modal == ModalId::BgSettings {
        if let Some(last) = b.glass_panels.last_mut() {
            if let Some((bg, border, _)) = theme.bg_panel_light_override() {
                last.glass.fill_color_0 = bg;
                last.glass.fill_color_1 = bg;
                last.glass.border_color_0 = border;
                last.glass.border_color_1 = border;
                last.glass.border_color_2 = border;
                last.glass.border_width = 1.0;
                last.glass.inset_highlight = [0.0; 4];
            }
            last.glass.apply_backdrop_dim(backdrop_alpha);
        }
    }
    b.push_hit(
        HitTarget::ModalPanelSurface,
        z,
        panel_rect,
        CursorHint::Default,
    );

    let close_svg = match modal {
        ModalId::Equalizer => SvgId::ModalCloseEqualizer,
        ModalId::Speed => SvgId::ModalCloseSpeed,
        ModalId::Subtitles => SvgId::ModalCloseSubtitle,
        ModalId::Settings => SvgId::ModalCloseSettings,
        ModalId::NetworkStream => SvgId::ModalCloseNetworkStream,
        ModalId::BgSettings => SvgId::ModalCloseBgSettings,
        ModalId::Audio => SvgId::ModalCloseSubtitle,
    };
    let modal_title = match modal {
        ModalId::Equalizer => "Audio Equalizer",
        ModalId::Speed => "Playback Speed",
        ModalId::Subtitles => "Subtitles",
        ModalId::Audio => "Audio & Voice",
        ModalId::Settings => "Settings",
        ModalId::NetworkStream => "Open Link",
        ModalId::BgSettings => "Background",
    };
    let title_x = if modal == ModalId::BgSettings {
        34.0
    } else {
        23.0
    };
    let title_y = if modal == ModalId::BgSettings {
        28.0
    } else {
        24.0
    };
    let title_right_pad = if modal == ModalId::BgSettings {
        100.0
    } else {
        77.0
    };
    b.push_text(
        "modal-title",
        z,
        RectPx::new(
            panel_rect.x + title_x,
            panel_rect.y + title_y,
            panel_rect.width - title_right_pad,
            22.0,
        ),
        modal_title,
        FontRole::Display,
        16.0,
        700,
        theme.text_primary,
        TextAlign::Left,
        Some(panel_rect),
    );
    if modal == ModalId::BgSettings {
        let sub = match state.welcome_background {
            WelcomeBackgroundKind::Dither => "Dither Waves · animated bayer dither",
            WelcomeBackgroundKind::Particles => "Particles · drifting particle field",
            WelcomeBackgroundKind::Faulty => "Faulty Terminal · CRT glitch & scanlines",
            WelcomeBackgroundKind::Gridmotion => "Grid Motion · interactive gliding item lattice",
            WelcomeBackgroundKind::None => "Default · pure black backdrop",
        };
        b.push_text(
            "modal-subtitle",
            z,
            RectPx::new(
                panel_rect.x + 34.0,
                panel_rect.y + 54.0,
                panel_rect.width - 100.0,
                16.0,
            ),
            sub,
            FontRole::Ui,
            11.0,
            500,
            theme.text_secondary,
            TextAlign::Left,
            Some(panel_rect),
        );
    }
    let header_h = if modal == ModalId::BgSettings {
        89.0
    } else {
        69.0
    };
    let close_rect = RectPx::new(
        panel_rect.x + panel_rect.width
            - if modal == ModalId::BgSettings {
                66.0
            } else {
                54.0
            },
        panel_rect.y
            + if modal == ModalId::BgSettings {
                34.0
            } else {
                19.0
            },
        32.0,
        32.0,
    );
    let close_hovered = b.is_hovered(HitTarget::ModalClose);
    let close_border = pill_idle_border;
    b.push_solid(
        "modal-close-bg",
        z,
        close_rect,
        if close_hovered {
            if is_light {
                rgba(0, 0, 0, 0.08)
            } else {
                rgba(255, 255, 255, 0.08)
            }
        } else {
            pill_idle_bg
        },
        [9.0; 4],
        1.0,
        close_border,
        Some(panel_rect),
    );
    b.push_svg(
        "modal-close",
        z,
        close_svg,
        SvgVariant::Default,
        RectPx::new(close_rect.x + 8.0, close_rect.y + 8.0, 16.0, 16.0),
        rgba_f32_to_u8(if close_hovered {
            theme.text_primary
        } else {
            theme.text_secondary
        }),
        Some(panel_rect),
    );
    b.push_hit(HitTarget::ModalClose, z, close_rect, CursorHint::Pointer);
    let div_inset = if modal == ModalId::BgSettings {
        12.0
    } else {
        1.0
    };
    b.push_solid(
        "modal-header-divider",
        z,
        RectPx::new(
            panel_rect.x + div_inset,
            panel_rect.y + header_h,
            panel_rect.width - div_inset * 2.0,
            1.0,
        ),
        theme.border_color,
        [0.0; 4],
        0.0,
        [0.0; 4],
        Some(panel_rect),
    );

    match modal {
        ModalId::Equalizer => {
            let preset_r = RectPx::new(
                panel_rect.x + 23.0,
                panel_rect.y + 85.0,
                panel_rect.width - 46.0,
                36.0,
            );
            let preset_hovered = b.is_hovered(HitTarget::EqPresetSelect);
            b.push_solid(
                "eq-preset-bg",
                z,
                preset_r,
                theme.bg_secondary,
                [8.0; 4],
                1.0,
                if preset_hovered {
                    theme.border_focus
                } else {
                    theme.border_color
                },
                Some(panel_rect),
            );
            b.push_text(
                "eq-preset-label",
                z,
                RectPx::new(
                    preset_r.x + 12.0,
                    preset_r.y + 8.0,
                    preset_r.width - 40.0,
                    20.0,
                ),
                eq_preset_label(state.equalizer.preset),
                FontRole::Ui,
                13.0,
                500,
                theme.text_primary,
                TextAlign::Left,
                Some(panel_rect),
            );
            b.push_svg(
                "eq-preset-chevron",
                z,
                SvgId::BgPickerChevron,
                SvgVariant::Default,
                RectPx::new(
                    preset_r.x + preset_r.width - 26.0,
                    preset_r.y + 10.0,
                    16.0,
                    16.0,
                ),
                rgba_f32_to_u8(theme.text_secondary),
                Some(panel_rect),
            );
            b.push_hit(HitTarget::EqPresetSelect, z, preset_r, CursorHint::Pointer);
            b.push_solid(
                "eq-preset-divider",
                z,
                RectPx::new(panel_rect.x, panel_rect.y + 137.0, panel_rect.width, 1.0),
                theme.border_color,
                [0.0; 4],
                0.0,
                [0.0; 4],
                Some(panel_rect),
            );

            let band_labels = [
                "31Hz", "62Hz", "125Hz", "250Hz", "500Hz", "1K", "2K", "4K", "8K", "16K",
            ];
            let band_gap = 8.0_f32;
            let band_w = ((panel_rect.width - 46.0 - 9.0 * band_gap) / 10.0).max(20.0);
            let tracks_top = panel_rect.y + 191.0;
            let track_h = if bp.max_h_620 {
                (panel_rect.height - 259.0).clamp(80.0, 120.0)
            } else {
                (panel_rect.height - 279.0).clamp(140.0, 264.0)
            };
            let track_bg = theme.border_focus;
            for (idx, freq_lbl) in band_labels.iter().enumerate() {
                let col_x = panel_rect.x + 23.0 + (idx as f32) * (band_w + band_gap);
                let db = state.equalizer.bands.get(idx).copied().unwrap_or(0.0);
                let db_text = if db.abs() < 0.05 {
                    "0 dB".to_string()
                } else if (db - db.round()).abs() < 0.05 {
                    format!("{:+.0} dB", db.round())
                } else {
                    format!("{db:+.1} dB")
                };
                b.push_text(
                    format!("eq-val-{idx}"),
                    z,
                    RectPx::new(col_x, tracks_top - 22.0, band_w, 18.0),
                    db_text,
                    FontRole::Mono,
                    11.0,
                    700,
                    theme.text_primary,
                    TextAlign::Center,
                    Some(panel_rect),
                );
                let track_x = col_x + (band_w - 6.0) * 0.5;
                let track_top = tracks_top + 10.0;
                let track_len = (track_h - 20.0).max(16.0);
                let srect = RectPx::new(col_x + 4.0, tracks_top, band_w - 8.0, track_h);
                b.push_solid(
                    format!("eq-band-{idx}"),
                    z,
                    RectPx::new(track_x, track_top, 6.0, track_len),
                    track_bg,
                    [3.0; 4],
                    0.0,
                    [0.0; 4],
                    Some(panel_rect),
                );
                // Thumb position (-12 dB bottom .. +12 dB top) & accent fill bar from 0 dB center
                let norm = ((db + 12.0) / 24.0).clamp(0.0, 1.0) as f32;
                let center_y = track_top + 0.5 * track_len;
                let thumb_center_y = track_top + (1.0 - norm) * track_len;
                let fill_top = center_y.min(thumb_center_y);
                let fill_h = (center_y - thumb_center_y).abs();
                if fill_h > 1.0 {
                    b.push_solid(
                        format!("eq-band-fill-{idx}"),
                        z,
                        RectPx::new(track_x, fill_top, 6.0, fill_h),
                        theme.accent,
                        [3.0; 4],
                        0.0,
                        [0.0; 4],
                        Some(panel_rect),
                    );
                }
                let thumb_y = thumb_center_y - 8.0;
                b.push_solid(
                    format!("eq-thumb-{idx}"),
                    z,
                    RectPx::new(col_x + (band_w - 16.0) * 0.5, thumb_y, 16.0, 16.0),
                    theme.accent,
                    [8.0; 4],
                    0.0,
                    [0.0; 4],
                    Some(panel_rect),
                );
                b.push_hit(HitTarget::EqBandSlider(idx), z, srect, CursorHint::Grab);
                b.push_text(
                    format!("eq-freq-{idx}"),
                    z,
                    RectPx::new(col_x, tracks_top + track_h + 6.0, band_w, 18.0),
                    *freq_lbl,
                    FontRole::Ui,
                    11.0,
                    600,
                    theme.text_secondary,
                    TextAlign::Center,
                    Some(panel_rect),
                );
            }

            let footer_y = panel_rect.y + panel_rect.height - 62.0;
            b.push_solid(
                "eq-footer-divider",
                z,
                RectPx::new(panel_rect.x, footer_y, panel_rect.width, 1.0),
                theme.border_color,
                [0.0; 4],
                0.0,
                [0.0; 4],
                Some(panel_rect),
            );
            let reset_r = RectPx::new(
                panel_rect.x + panel_rect.width - 98.0,
                footer_y + 12.0,
                76.0,
                32.0,
            );
            let reset_hovered = b.is_hovered(HitTarget::EqReset);
            b.push_solid(
                "eq-reset-bg",
                z,
                reset_r,
                if reset_hovered {
                    if is_light {
                        rgba(0, 0, 0, 0.08)
                    } else {
                        rgba(255, 255, 255, 0.08)
                    }
                } else {
                    pill_idle_bg
                },
                [8.0; 4],
                1.0,
                pill_idle_border,
                Some(panel_rect),
            );
            b.push_text(
                "eq-reset-label",
                z,
                RectPx::new(reset_r.x, reset_r.y + 6.0, reset_r.width, 20.0),
                "Reset",
                FontRole::Ui,
                12.0,
                600,
                theme.text_primary,
                TextAlign::Center,
                Some(panel_rect),
            );
            b.push_hit(HitTarget::EqReset, z, reset_r, CursorHint::Pointer);
        }
        ModalId::Speed => {
            let cw = panel_rect.width - 40.0;
            let cx = panel_rect.x + 20.0;
            let speed_style = |active: bool, hovered: bool| {
                if active {
                    if is_light {
                        (theme.accent_dim, theme.accent, theme.accent)
                    } else {
                        (
                            rgba(0, 242, 254, 0.22),
                            rgba(0, 242, 254, 0.60),
                            rgba(0, 242, 254, 1.0),
                        )
                    }
                } else if hovered {
                    (
                        if is_light {
                            rgba(0, 0, 0, 0.08)
                        } else {
                            rgba(255, 255, 255, 0.09)
                        },
                        if is_light {
                            theme.border_focus
                        } else {
                            rgba(255, 255, 255, 0.16)
                        },
                        theme.text_primary,
                    )
                } else {
                    (pill_idle_bg, pill_idle_border, theme.text_primary)
                }
            };

            b.push_text(
                "speed-slow-hdr",
                z,
                RectPx::new(cx, panel_rect.y + 85.0, cw, 16.0),
                "SLOW",
                FontRole::Ui,
                10.0,
                700,
                theme.text_secondary,
                TextAlign::Left,
                Some(panel_rect),
            );
            let col_w = (cw - 16.0) / 3.0;
            for (i, lbl) in ["0.25x", "0.5x", "0.75x"].iter().enumerate() {
                let r = RectPx::new(
                    cx + (i as f32) * (col_w + 8.0),
                    panel_rect.y + 105.5,
                    col_w,
                    33.0,
                );
                let target = HitTarget::SpeedPreset(i);
                let active = (state.speed - SPEED_PRESETS[i]).abs() < 0.02;
                let (bg, br, fg) = speed_style(active, b.is_hovered(target.clone()));
                b.push_solid(
                    format!("speed-slow-{i}"),
                    z,
                    r,
                    bg,
                    [8.0; 4],
                    1.0,
                    br,
                    Some(panel_rect),
                );
                b.push_text(
                    format!("speed-slow-lbl-{i}"),
                    z,
                    RectPx::new(r.x, r.y + 8.0, r.width, 18.0),
                    *lbl,
                    FontRole::Ui,
                    12.5,
                    if active { 700 } else { 600 },
                    fg,
                    TextAlign::Center,
                    Some(panel_rect),
                );
                b.push_hit(target, z, r, CursorHint::Pointer);
            }
            let norm_r = RectPx::new(cx, panel_rect.y + 152.0, cw, 33.0);
            let is_normal = (state.speed - SPEED_PRESETS[3]).abs() < 0.02;
            let (nbg, nbr, nfg) = speed_style(is_normal, b.is_hovered(HitTarget::SpeedPreset(3)));
            if is_normal && !is_light {
                b.push_gradient(
                    "speed-normal-bg",
                    z,
                    norm_r,
                    rgba(0, 242, 254, 0.22),
                    rgba(79, 172, 254, 0.30),
                    135.0,
                    [9.0; 4],
                    1.0,
                    rgba(0, 242, 254, 0.60),
                    Some(panel_rect),
                );
            } else {
                b.push_solid(
                    "speed-normal-bg",
                    z,
                    norm_r,
                    nbg,
                    [9.0; 4],
                    1.0,
                    nbr,
                    Some(panel_rect),
                );
            }
            b.push_text(
                "speed-normal-lbl",
                z,
                RectPx::new(norm_r.x, norm_r.y + 7.5, norm_r.width, 18.0),
                "Normal (1.0x)",
                FontRole::Ui,
                13.0,
                700,
                nfg,
                TextAlign::Center,
                Some(panel_rect),
            );
            b.push_hit(HitTarget::SpeedPreset(3), z, norm_r, CursorHint::Pointer);

            b.push_text(
                "speed-fast-hdr",
                z,
                RectPx::new(cx, panel_rect.y + 199.0, cw, 16.0),
                "FAST",
                FontRole::Ui,
                10.0,
                700,
                theme.text_secondary,
                TextAlign::Left,
                Some(panel_rect),
            );
            for (i, lbl) in ["1.25x", "1.5x", "1.75x", "2x", "3x", "4x"]
                .iter()
                .enumerate()
            {
                let preset_idx = i + 4;
                let row = i / 3;
                let col = i % 3;
                let r = RectPx::new(
                    cx + (col as f32) * (col_w + 8.0),
                    panel_rect.y + 219.0 + (row as f32) * 37.8,
                    col_w,
                    33.0,
                );
                let target = HitTarget::SpeedPreset(preset_idx);
                let active = (state.speed - SPEED_PRESETS[preset_idx]).abs() < 0.02;
                let (bg, br, fg) = speed_style(active, b.is_hovered(target.clone()));
                b.push_solid(
                    format!("speed-fast-{i}"),
                    z,
                    r,
                    bg,
                    [8.0; 4],
                    1.0,
                    br,
                    Some(panel_rect),
                );
                b.push_text(
                    format!("speed-fast-lbl-{i}"),
                    z,
                    RectPx::new(r.x, r.y + 7.0, r.width, 18.0),
                    *lbl,
                    FontRole::Ui,
                    12.5,
                    if active { 700 } else { 600 },
                    fg,
                    TextAlign::Center,
                    Some(panel_rect),
                );
                b.push_hit(target, z, r, CursorHint::Pointer);
            }
            let custom_r = RectPx::new(cx, panel_rect.y + panel_rect.height - 68.0, cw, 50.0);
            b.push_solid(
                "speed-custom-card",
                z,
                custom_r,
                card_bg,
                [12.0; 4],
                1.0,
                card_border,
                Some(panel_rect),
            );
            b.push_text(
                "speed-custom-lbl",
                z,
                RectPx::new(custom_r.x + 14.0, custom_r.y + 16.0, 92.0, 18.0),
                "Custom Speed",
                FontRole::Ui,
                12.5,
                600,
                theme.text_primary,
                TextAlign::Left,
                Some(panel_rect),
            );
            let slider_r = RectPx::new(custom_r.x + 114.0, custom_r.y + 13.0, cw - 190.0, 24.0);
            b.push_solid(
                "speed-slider-track",
                z,
                RectPx::new(slider_r.x, custom_r.y + 23.0, slider_r.width, 4.0),
                if is_light {
                    rgba(0, 0, 0, 0.16)
                } else {
                    rgba(255, 255, 255, 0.2)
                },
                [2.0; 4],
                0.0,
                [0.0; 4],
                Some(panel_rect),
            );
            let snorm = ((state.speed - 0.1) / 3.9).clamp(0.0, 1.0) as f32;
            b.push_solid(
                "speed-slider-thumb",
                z,
                RectPx::new(
                    slider_r.x + slider_r.width * snorm - 7.0,
                    custom_r.y + 18.0,
                    14.0,
                    14.0,
                ),
                theme.accent,
                [7.0; 4],
                0.0,
                [0.0; 4],
                Some(panel_rect),
            );
            b.push_hit(HitTarget::SpeedSlider, z, slider_r, CursorHint::Pointer);
            let val_badge = RectPx::new(custom_r.x + cw - 66.0, custom_r.y + 13.0, 54.0, 24.0);
            b.push_solid(
                "speed-custom-val-bg",
                z,
                val_badge,
                if is_light {
                    theme.accent_dim
                } else {
                    rgba(0, 242, 254, 0.08)
                },
                [6.0; 4],
                1.0,
                if is_light {
                    theme.accent
                } else {
                    rgba(0, 242, 254, 0.25)
                },
                Some(panel_rect),
            );
            b.push_text(
                "speed-custom-val",
                z,
                RectPx::new(val_badge.x, val_badge.y + 3.0, val_badge.width, 18.0),
                format!("{:.2}x", state.speed),
                FontRole::Mono,
                11.0,
                700,
                if is_light {
                    theme.accent
                } else {
                    rgba(0, 242, 254, 1.0)
                },
                TextAlign::Center,
                Some(panel_rect),
            );
        }
        ModalId::Subtitles => {
            let cx = panel_rect.x + 18.0;
            let cw = panel_rect.width - 36.0;
            let cyan = if is_light {
                theme.accent
            } else {
                rgba(0, 242, 254, 1.0)
            };
            let cyan_bg = if is_light {
                theme.accent_dim
            } else {
                rgba(0, 242, 254, 0.12)
            };
            let sub_div = if is_light {
                theme.border_color
            } else {
                rgba(255, 255, 255, 0.04)
            };

            // Card 1: SUBTITLE TRACK
            let c1 = RectPx::new(cx, panel_rect.y + 84.0, cw, 122.0 + sub_extra_h);
            b.push_solid(
                "sub-card-1",
                z,
                c1,
                card_bg,
                [14.0; 4],
                1.0,
                card_border,
                Some(panel_rect),
            );
            b.push_text(
                "sub-track-hdr",
                z,
                RectPx::new(c1.x + 14.0, c1.y + 12.0, 140.0, 16.0),
                "SUBTITLE TRACK",
                FontRole::Ui,
                10.5,
                700,
                theme.text_secondary,
                TextAlign::Left,
                Some(panel_rect),
            );

            let has_selected_sub = state.subtitle_tracks.iter().any(|t| t.selected);
            let start_x = c1.x + 14.0;
            let max_pill_x = c1.x + c1.width - 14.0;
            let max_pill_w = (max_pill_x - start_x).max(60.0);
            let mut pill_x = start_x;
            let mut pill_y = c1.y + 36.0;
            let off_r = RectPx::new(pill_x, pill_y, 44.0, 30.0);
            b.push_solid(
                "sub-off-pill",
                z,
                off_r,
                if !has_selected_sub {
                    cyan_bg
                } else {
                    pill_idle_bg
                },
                [8.0; 4],
                1.0,
                if !has_selected_sub {
                    cyan
                } else {
                    pill_idle_border
                },
                Some(panel_rect),
            );
            b.push_text(
                "sub-off-lbl",
                z,
                RectPx::new(off_r.x, off_r.y + 6.0, off_r.width, 18.0),
                "Off",
                FontRole::Ui,
                11.5,
                600,
                if !has_selected_sub {
                    cyan
                } else {
                    theme.text_primary
                },
                TextAlign::Center,
                Some(panel_rect),
            );
            b.push_hit(
                HitTarget::SubtitleTrackSelect(-1),
                z,
                off_r,
                CursorHint::Pointer,
            );
            pill_x += 50.0;

            for (idx, trk) in state.subtitle_tracks.iter().enumerate() {
                let lbl = if trk.label.is_empty() {
                    format!("Track {}", trk.id)
                } else {
                    trk.label.clone()
                };
                let lbl_w =
                    measure_text_width(&lbl, FontRole::Ui, 11.5, 600, 0.0, state.font_config);
                let pw = (lbl_w + 22.0).clamp(52.0, max_pill_w.min(180.0));
                if pill_x > start_x && pill_x + pw > max_pill_x {
                    pill_x = start_x;
                    pill_y += 36.0;
                }
                let tr = RectPx::new(pill_x, pill_y, pw, 30.0);
                b.push_solid(
                    format!("sub-trk-pill-{idx}"),
                    z,
                    tr,
                    if trk.selected { cyan_bg } else { pill_idle_bg },
                    [8.0; 4],
                    1.0,
                    if trk.selected { cyan } else { pill_idle_border },
                    Some(panel_rect),
                );
                b.push_text(
                    format!("sub-trk-lbl-{idx}"),
                    z,
                    RectPx::new(tr.x + 6.0, tr.y + 6.0, tr.width - 12.0, 18.0),
                    lbl,
                    FontRole::Ui,
                    11.5,
                    600,
                    if trk.selected {
                        cyan
                    } else {
                        theme.text_primary
                    },
                    TextAlign::Center,
                    Some(tr),
                );
                b.push_hit(
                    HitTarget::SubtitleTrackSelect(trk.id),
                    z,
                    tr,
                    CursorHint::Pointer,
                );
                pill_x += pw + 6.0;
            }

            let load_r = RectPx::new(c1.x + 14.0, c1.y + 78.0 + sub_extra_h, cw - 28.0, 33.0);
            b.push_solid(
                "sub-load-bg",
                z,
                load_r,
                pill_idle_bg,
                [8.0; 4],
                0.0,
                [0.0; 4],
                Some(panel_rect),
            );
            // Dashed border (`settings.css` dashed outline around Load External Subtitle)
            let dash_col = if is_light {
                theme.border_focus
            } else {
                rgba(255, 255, 255, 0.16)
            };
            let dash_step = 8.0_f32;
            let mut dx = load_r.x + 4.0;
            let mut di = 0u32;
            while dx + 4.0 < load_r.x + load_r.width - 4.0 {
                let seg_w = 4.0_f32.min(load_r.x + load_r.width - 4.0 - dx);
                b.push_solid(
                    format!("sub-load-dt-{di}"),
                    z,
                    RectPx::new(dx, load_r.y, seg_w, 1.0),
                    dash_col,
                    [0.0; 4],
                    0.0,
                    [0.0; 4],
                    Some(panel_rect),
                );
                b.push_solid(
                    format!("sub-load-db-{di}"),
                    z,
                    RectPx::new(dx, load_r.y + load_r.height - 1.0, seg_w, 1.0),
                    dash_col,
                    [0.0; 4],
                    0.0,
                    [0.0; 4],
                    Some(panel_rect),
                );
                dx += dash_step;
                di += 1;
            }
            let mut dy = load_r.y + 4.0;
            while dy + 4.0 < load_r.y + load_r.height - 4.0 {
                let seg_h = 4.0_f32.min(load_r.y + load_r.height - 4.0 - dy);
                b.push_solid(
                    format!("sub-load-dl-{di}"),
                    z,
                    RectPx::new(load_r.x, dy, 1.0, seg_h),
                    dash_col,
                    [0.0; 4],
                    0.0,
                    [0.0; 4],
                    Some(panel_rect),
                );
                b.push_solid(
                    format!("sub-load-dr-{di}"),
                    z,
                    RectPx::new(load_r.x + load_r.width - 1.0, dy, 1.0, seg_h),
                    dash_col,
                    [0.0; 4],
                    0.0,
                    [0.0; 4],
                    Some(panel_rect),
                );
                dy += dash_step;
                di += 1;
            }
            b.push_text(
                "sub-load-lbl",
                z,
                RectPx::new(load_r.x + 14.0, load_r.y + 8.0, load_r.width - 28.0, 18.0),
                "Load External Subtitle",
                FontRole::Ui,
                12.0,
                600,
                theme.text_primary,
                TextAlign::Left,
                Some(panel_rect),
            );
            b.push_hit(
                HitTarget::SubtitleLoadExternal,
                z,
                load_r,
                CursorHint::Pointer,
            );

            // Card 2: SYNC ADJUSTMENT
            let c2 = RectPx::new(cx, panel_rect.y + 217.0 + sub_extra_h, cw, 138.0);
            b.push_solid(
                "sub-card-2",
                z,
                c2,
                card_bg,
                [14.0; 4],
                1.0,
                card_border,
                Some(panel_rect),
            );
            b.push_text(
                "sub-sync-hdr",
                z,
                RectPx::new(c2.x + 14.0, c2.y + 12.0, 150.0, 16.0),
                "SYNC ADJUSTMENT",
                FontRole::Ui,
                10.5,
                700,
                theme.text_secondary,
                TextAlign::Left,
                Some(panel_rect),
            );
            let sync_badge = RectPx::new(c2.x + cw - 72.0, c2.y + 10.0, 58.0, 24.0);
            b.push_solid(
                "sub-sync-val-bg",
                z,
                sync_badge,
                cyan_bg,
                [7.0; 4],
                1.0,
                if is_light {
                    theme.accent
                } else {
                    rgba(0, 242, 254, 0.45)
                },
                Some(panel_rect),
            );
            b.push_text(
                "sub-sync-val",
                z,
                RectPx::new(sync_badge.x, sync_badge.y + 3.0, sync_badge.width, 18.0),
                format!("{:.1}s", state.subtitle_delay_secs),
                FontRole::Mono,
                11.5,
                700,
                cyan,
                TextAlign::Center,
                Some(panel_rect),
            );
            b.push_text(
                "sub-sync-min",
                z,
                RectPx::new(c2.x + 14.0, c2.y + 51.0, 32.0, 16.0),
                "-10s",
                FontRole::Mono,
                10.5,
                500,
                theme.text_tertiary,
                TextAlign::Left,
                Some(panel_rect),
            );
            let sslider = RectPx::new(c2.x + 48.0, c2.y + 48.0, cw - 96.0, 22.0);
            b.push_solid(
                "sub-slider-track",
                z,
                RectPx::new(sslider.x, sslider.y + 9.0, sslider.width, 4.0),
                if is_light {
                    rgba(0, 0, 0, 0.16)
                } else {
                    rgba(255, 255, 255, 0.2)
                },
                [2.0; 4],
                0.0,
                [0.0; 4],
                Some(panel_rect),
            );
            let snorm = ((state.subtitle_delay_secs + 10.0) / 20.0).clamp(0.0, 1.0) as f32;
            b.push_solid(
                "sub-slider-thumb",
                z,
                RectPx::new(
                    sslider.x + (sslider.width - 14.0) * snorm,
                    sslider.y + 4.0,
                    14.0,
                    14.0,
                ),
                theme.accent,
                [7.0; 4],
                0.0,
                [0.0; 4],
                Some(panel_rect),
            );
            b.push_hit(HitTarget::SubDelaySlider, z, sslider, CursorHint::Pointer);
            b.push_text(
                "sub-sync-max",
                z,
                RectPx::new(c2.x + cw - 46.0, c2.y + 51.0, 32.0, 16.0),
                "+10s",
                FontRole::Mono,
                10.5,
                500,
                theme.text_tertiary,
                TextAlign::Right,
                Some(panel_rect),
            );

            let sbw = (cw - 52.0) / 5.0;
            for (i, (lbl, step)) in [
                ("-0.5s", -500),
                ("-0.1s", -100),
                ("Reset", 0),
                ("+0.1s", 100),
                ("+0.5s", 500),
            ]
            .iter()
            .enumerate()
            {
                let br = RectPx::new(
                    c2.x + 14.0 + (i as f32) * (sbw + 6.0),
                    c2.y + 77.0,
                    sbw,
                    29.0,
                );
                b.push_solid(
                    format!("sub-btn-{i}"),
                    z,
                    br,
                    pill_idle_bg,
                    [8.0; 4],
                    1.0,
                    pill_idle_border,
                    Some(panel_rect),
                );
                b.push_text(
                    format!("sub-btn-lbl-{i}"),
                    z,
                    RectPx::new(br.x, br.y + 5.5, br.width, 18.0),
                    *lbl,
                    FontRole::Ui,
                    11.5,
                    600,
                    theme.text_primary,
                    TextAlign::Center,
                    Some(panel_rect),
                );
                b.push_hit(HitTarget::SubDelayStep(*step), z, br, CursorHint::Pointer);
            }
            b.push_text(
                "sub-sync-earlier",
                z,
                RectPx::new(c2.x + 14.0, c2.y + 114.0, 80.0, 14.0),
                "Earlier (-)",
                FontRole::Ui,
                10.0,
                500,
                theme.text_tertiary,
                TextAlign::Left,
                Some(panel_rect),
            );
            b.push_text(
                "sub-sync-later",
                z,
                RectPx::new(c2.x + cw - 94.0, c2.y + 114.0, 80.0, 14.0),
                "Later (+)",
                FontRole::Ui,
                10.0,
                500,
                theme.text_tertiary,
                TextAlign::Right,
                Some(panel_rect),
            );

            // Card 3: APPEARANCE (4 rows: Font Size, Font Color, Background Color, Background Opacity)
            let c3_y = panel_rect.y + 366.0 + sub_extra_h;
            let c3_h = (panel_rect.height - (386.0 + sub_extra_h)).max(176.0);
            let c3 = RectPx::new(cx, c3_y, cw, c3_h);
            b.push_solid(
                "sub-card-3",
                z,
                c3,
                card_bg,
                [14.0; 4],
                1.0,
                card_border,
                Some(panel_rect),
            );
            b.push_text(
                "sub-app-hdr",
                z,
                RectPx::new(c3.x + 14.0, c3.y + 12.0, 120.0, 16.0),
                "APPEARANCE",
                FontRole::Ui,
                10.5,
                700,
                theme.text_secondary,
                TextAlign::Left,
                Some(panel_rect),
            );

            let row_h = 38.0;
            let r1_y = c3.y + 33.0;
            // Row 1: Font Size (12..=64 slider matching `index.html:785`)
            b.push_text(
                "sub-fs-lbl",
                z,
                RectPx::new(c3.x + 14.0, r1_y + 10.0, 110.0, 18.0),
                "Font Size",
                FontRole::Ui,
                12.5,
                500,
                theme.text_primary,
                TextAlign::Left,
                Some(panel_rect),
            );
            let fs_slider = RectPx::new(c3.x + 201.0, r1_y + 8.0, cw - 246.0, 22.0);
            let fs_norm = (((state.subtitle_font_size as f32) - 12.0) / 52.0).clamp(0.0, 1.0);
            b.push_solid(
                "sub-fs-track",
                z,
                RectPx::new(fs_slider.x, fs_slider.y + 9.0, fs_slider.width, 4.0),
                if is_light {
                    rgba(0, 0, 0, 0.16)
                } else {
                    rgba(255, 255, 255, 0.2)
                },
                [2.0; 4],
                0.0,
                [0.0; 4],
                Some(panel_rect),
            );
            b.push_solid(
                "sub-fs-thumb",
                z,
                RectPx::new(
                    fs_slider.x + (fs_slider.width - 14.0) * fs_norm,
                    fs_slider.y + 4.0,
                    14.0,
                    14.0,
                ),
                theme.accent,
                [7.0; 4],
                0.0,
                [0.0; 4],
                Some(panel_rect),
            );
            b.push_hit(
                HitTarget::SubtitleFontSizeSlider,
                z,
                fs_slider,
                CursorHint::Pointer,
            );
            b.push_text(
                "sub-fs-val",
                z,
                RectPx::new(c3.x + cw - 58.0, r1_y + 10.0, 44.0, 18.0),
                format!("{}px", state.subtitle_font_size),
                FontRole::Mono,
                11.5,
                600,
                theme.text_secondary,
                TextAlign::Right,
                Some(panel_rect),
            );

            // Row 2: Font Color (outer dark frame + inset color swatch, 28x28 rounded 7px)
            let r2_y = r1_y + row_h;
            b.push_solid(
                "sub-div-1",
                z,
                RectPx::new(c3.x + 14.0, r2_y, cw - 28.0, 1.0),
                sub_div,
                [0.0; 4],
                0.0,
                [0.0; 4],
                Some(panel_rect),
            );
            b.push_text(
                "sub-fc-lbl",
                z,
                RectPx::new(c3.x + 14.0, r2_y + 10.0, 140.0, 18.0),
                "Font Color",
                FontRole::Ui,
                12.5,
                500,
                theme.text_primary,
                TextAlign::Left,
                Some(panel_rect),
            );
            let fc_frame = RectPx::new(c3.x + cw - 42.0, r2_y + 5.0, 28.0, 28.0);
            b.push_solid(
                "sub-fc-frame",
                z,
                fc_frame,
                if is_light {
                    rgba(0, 0, 0, 0.04)
                } else {
                    rgba(0, 0, 0, 0.0)
                },
                [7.0; 4],
                1.0,
                pill_idle_border,
                Some(panel_rect),
            );
            let fc_rgba =
                parse_hex_rgba(&state.subtitle_font_color).unwrap_or([1.0, 1.0, 1.0, 1.0]);
            b.push_solid(
                "sub-fc-swatch",
                z,
                RectPx::new(
                    fc_frame.x + 3.0,
                    fc_frame.y + 3.0,
                    fc_frame.width - 6.0,
                    fc_frame.height - 6.0,
                ),
                fc_rgba,
                [4.0; 4],
                0.0,
                [0.0; 4],
                Some(panel_rect),
            );
            b.push_hit(
                HitTarget::SubtitleFontColorPicker,
                z,
                fc_frame,
                CursorHint::Pointer,
            );

            // Row 3: Background Color (outer dark frame + inset color swatch, 28x28 rounded 7px)
            let r3_y = r2_y + row_h;
            b.push_solid(
                "sub-div-2",
                z,
                RectPx::new(c3.x + 14.0, r3_y, cw - 28.0, 1.0),
                sub_div,
                [0.0; 4],
                0.0,
                [0.0; 4],
                Some(panel_rect),
            );
            b.push_text(
                "sub-bc-lbl",
                z,
                RectPx::new(c3.x + 14.0, r3_y + 10.0, 150.0, 18.0),
                "Background Color",
                FontRole::Ui,
                12.5,
                500,
                theme.text_primary,
                TextAlign::Left,
                Some(panel_rect),
            );
            let bc_frame = RectPx::new(c3.x + cw - 42.0, r3_y + 5.0, 28.0, 28.0);
            b.push_solid(
                "sub-bc-frame",
                z,
                bc_frame,
                if is_light {
                    rgba(0, 0, 0, 0.04)
                } else {
                    rgba(0, 0, 0, 0.0)
                },
                [7.0; 4],
                1.0,
                pill_idle_border,
                Some(panel_rect),
            );
            let bc_rgba = parse_hex_rgba(&state.subtitle_bg_color).unwrap_or([0.0, 0.0, 0.0, 1.0]);
            b.push_solid(
                "sub-bc-swatch",
                z,
                RectPx::new(
                    bc_frame.x + 3.0,
                    bc_frame.y + 3.0,
                    bc_frame.width - 6.0,
                    bc_frame.height - 6.0,
                ),
                bc_rgba,
                [4.0; 4],
                0.0,
                [0.0; 4],
                Some(panel_rect),
            );
            b.push_hit(
                HitTarget::SubtitleBgColorPicker,
                z,
                bc_frame,
                CursorHint::Pointer,
            );

            // Row 4: Background Opacity
            let r4_y = r3_y + row_h;
            b.push_solid(
                "sub-div-3",
                z,
                RectPx::new(c3.x + 14.0, r4_y, cw - 28.0, 1.0),
                sub_div,
                [0.0; 4],
                0.0,
                [0.0; 4],
                Some(panel_rect),
            );
            b.push_text(
                "sub-bo-lbl",
                z,
                RectPx::new(c3.x + 14.0, r4_y + 10.0, 130.0, 18.0),
                "Background Opacity",
                FontRole::Ui,
                12.5,
                500,
                theme.text_primary,
                TextAlign::Left,
                Some(panel_rect),
            );
            let bo_slider = RectPx::new(c3.x + 201.0, r4_y + 8.0, cw - 246.0, 22.0);
            let bo_norm = ((state.subtitle_bg_opacity_pct as f32) / 100.0).clamp(0.0, 1.0);
            b.push_solid(
                "sub-bo-track",
                z,
                RectPx::new(bo_slider.x, bo_slider.y + 9.0, bo_slider.width, 4.0),
                if is_light {
                    rgba(0, 0, 0, 0.16)
                } else {
                    rgba(255, 255, 255, 0.2)
                },
                [2.0; 4],
                0.0,
                [0.0; 4],
                Some(panel_rect),
            );
            b.push_solid(
                "sub-bo-thumb",
                z,
                RectPx::new(
                    bo_slider.x + (bo_slider.width - 14.0) * bo_norm,
                    bo_slider.y + 4.0,
                    14.0,
                    14.0,
                ),
                theme.accent,
                [7.0; 4],
                0.0,
                [0.0; 4],
                Some(panel_rect),
            );
            b.push_hit(
                HitTarget::SubtitleBgOpacitySlider,
                z,
                bo_slider,
                CursorHint::Pointer,
            );
            b.push_text(
                "sub-bo-val",
                z,
                RectPx::new(c3.x + cw - 58.0, r4_y + 10.0, 44.0, 18.0),
                format!("{}%", state.subtitle_bg_opacity_pct),
                FontRole::Mono,
                11.5,
                600,
                theme.text_secondary,
                TextAlign::Right,
                Some(panel_rect),
            );
        }
        ModalId::Audio => {
            let cx = panel_rect.x + 19.0;
            let cw = panel_rect.width - 38.0;
            // Card 1: ANIME PRESET (fit-content left-aligned pills)
            let c1 = RectPx::new(cx, panel_rect.y + 83.0, cw, 84.0);
            b.push_solid(
                "audio-card-1",
                z,
                c1,
                card_bg,
                [14.0; 4],
                1.0,
                card_border,
                Some(panel_rect),
            );
            b.push_text(
                "audio-preset-hdr",
                z,
                RectPx::new(c1.x + 14.0, c1.y + 11.0, 220.0, 16.0),
                "ANIME PRESET (SUBS / DUBS)",
                FontRole::Ui,
                10.0,
                700,
                theme.text_secondary,
                TextAlign::Left,
                Some(panel_rect),
            );
            let subs_w = 174.0_f32.min((cw - 36.0) * 0.56);
            let dubs_w = 132.0_f32.min((cw - 36.0) - subs_w);
            let r_subs = RectPx::new(c1.x + 14.0, c1.y + 37.0, subs_w, 32.0);
            let r_dubs = RectPx::new(c1.x + 22.0 + subs_w, c1.y + 37.0, dubs_w, 32.0);
            let current_audio_opt = state
                .audio_tracks
                .iter()
                .find(|t| t.selected)
                .or_else(|| state.audio_tracks.first());
            let (is_jap_audio, is_eng_audio) = current_audio_opt
                .map(|t| {
                    let mt = hybrid_core::tracks::MediaTrack {
                        id: t.id,
                        lang: t.lang.clone(),
                        title: t.label.clone(),
                        selected: t.selected,
                        external: t.external,
                    };
                    (
                        hybrid_core::tracks::is_japanese_track(&mt),
                        hybrid_core::tracks::is_english_track(&mt),
                    )
                })
                .unwrap_or((false, false));

            for (id, r, lbl, tgt, active) in [
                (
                    "audio-subs",
                    r_subs,
                    "Subs (Japanese + Subtitles)",
                    HitTarget::AudioPresetSubs,
                    is_jap_audio,
                ),
                (
                    "audio-dubs",
                    r_dubs,
                    "Dubs (English Voice)",
                    HitTarget::AudioPresetDubs,
                    is_eng_audio,
                ),
            ] {
                let hovered = b.is_hovered(tgt.clone());
                b.push_solid(
                    format!("{id}-bg"),
                    z,
                    r,
                    if active {
                        theme.accent_dim
                    } else if hovered {
                        if is_light {
                            rgba(0, 0, 0, 0.08)
                        } else {
                            rgba(255, 255, 255, 0.09)
                        }
                    } else {
                        pill_idle_bg
                    },
                    [8.0; 4],
                    1.0,
                    if active {
                        theme.accent
                    } else {
                        pill_idle_border
                    },
                    Some(panel_rect),
                );
                b.push_text(
                    format!("{id}-lbl"),
                    z,
                    RectPx::new(r.x + 6.0, r.y + 7.0, r.width - 12.0, 18.0),
                    lbl,
                    FontRole::Ui,
                    11.0,
                    600,
                    if active {
                        theme.accent
                    } else {
                        theme.text_primary
                    },
                    TextAlign::Center,
                    Some(panel_rect),
                );
                b.push_hit(tgt, z, r, CursorHint::Pointer);
            }

            // Card 2: VOICE / AUDIO TRACK
            let c2 = RectPx::new(cx, panel_rect.y + 177.0, cw, 78.0 + audio_extra_h);
            b.push_solid(
                "audio-card-2",
                z,
                c2,
                card_bg,
                [14.0; 4],
                1.0,
                card_border,
                Some(panel_rect),
            );
            b.push_text(
                "audio-trk-hdr",
                z,
                RectPx::new(c2.x + 14.0, c2.y + 11.0, 200.0, 16.0),
                "VOICE / AUDIO TRACK",
                FontRole::Ui,
                10.0,
                700,
                theme.text_secondary,
                TextAlign::Left,
                Some(panel_rect),
            );
            if state.audio_tracks.is_empty() {
                b.push_text(
                    "audio-trk-msg",
                    z,
                    RectPx::new(c2.x + 14.0, c2.y + 38.0, cw - 28.0, 18.0),
                    "No audio tracks found",
                    FontRole::Ui,
                    12.0,
                    400,
                    theme.text_secondary,
                    TextAlign::Left,
                    Some(panel_rect),
                );
            } else {
                let start_x = c2.x + 14.0;
                let max_tx = c2.x + c2.width - 14.0;
                let max_tw = (max_tx - start_x).max(70.0);
                let mut tx = start_x;
                let mut ty = c2.y + 34.0;
                for (idx, trk) in state.audio_tracks.iter().enumerate() {
                    let lbl = if trk.label.is_empty() {
                        format!("Track {}", trk.id)
                    } else {
                        trk.label.clone()
                    };
                    let lbl_w =
                        measure_text_width(&lbl, FontRole::Ui, 11.0, 600, 0.0, state.font_config);
                    let tw = (lbl_w + 22.0).clamp(64.0, max_tw.min(190.0));
                    if tx > start_x && tx + tw > max_tx {
                        tx = start_x;
                        ty += 36.0;
                    }
                    let tr = RectPx::new(tx, ty, tw, 30.0);
                    b.push_solid(
                        format!("audio-trk-pill-{idx}"),
                        z,
                        tr,
                        if trk.selected {
                            theme.accent_dim
                        } else {
                            pill_idle_bg
                        },
                        [8.0; 4],
                        1.0,
                        if trk.selected {
                            theme.accent
                        } else {
                            pill_idle_border
                        },
                        Some(panel_rect),
                    );
                    b.push_text(
                        format!("audio-trk-lbl-{idx}"),
                        z,
                        RectPx::new(tr.x + 6.0, tr.y + 6.0, tr.width - 12.0, 18.0),
                        lbl,
                        FontRole::Ui,
                        11.5,
                        600,
                        if trk.selected {
                            theme.accent
                        } else {
                            theme.text_primary
                        },
                        TextAlign::Center,
                        Some(tr),
                    );
                    b.push_hit(
                        HitTarget::AudioTrackSelect(trk.id),
                        z,
                        tr,
                        CursorHint::Pointer,
                    );
                    tx += tw + 6.0;
                }
            }

            // Card 3: AUDIO SYNC
            let c3 = RectPx::new(cx, panel_rect.y + 265.0 + audio_extra_h, cw, 82.0);
            b.push_solid(
                "audio-card-3",
                z,
                c3,
                card_bg,
                [14.0; 4],
                1.0,
                card_border,
                Some(panel_rect),
            );
            b.push_text(
                "audio-sync-hdr",
                z,
                RectPx::new(c3.x + 14.0, c3.y + 12.0, 140.0, 16.0),
                "AUDIO SYNC",
                FontRole::Ui,
                10.0,
                700,
                theme.text_secondary,
                TextAlign::Left,
                Some(panel_rect),
            );
            let sync_gap = 8.0_f32;
            let bw = (cw - 28.0 - 3.0 * sync_gap) / 4.0;
            let delay_ms = (state.audio_delay_secs * 1000.0).round() as i32;
            let btns = [
                ("-100ms", -100, false),
                (&format!("{delay_ms}ms") as &str, 0, true),
                ("+100ms", 100, false),
                ("Reset", 0, false),
            ];
            for (i, (lbl, step, is_val)) in btns.into_iter().enumerate() {
                let r = RectPx::new(
                    c3.x + 14.0 + (i as f32) * (bw + sync_gap),
                    c3.y + 38.0,
                    bw,
                    30.0,
                );
                b.push_solid(
                    format!("audio-sync-btn-{i}"),
                    z,
                    r,
                    if is_val {
                        theme.accent_dim
                    } else {
                        pill_idle_bg
                    },
                    [if is_val { 6.0 } else { 8.0 }; 4],
                    if is_val { 0.0 } else { 1.0 },
                    if is_val { [0.0; 4] } else { pill_idle_border },
                    Some(panel_rect),
                );
                b.push_text(
                    format!("audio-sync-lbl-{i}"),
                    z,
                    RectPx::new(r.x, r.y + 6.0, r.width, 18.0),
                    lbl.to_string(),
                    FontRole::Mono,
                    11.0,
                    600,
                    if is_val {
                        theme.accent
                    } else {
                        theme.text_primary
                    },
                    TextAlign::Center,
                    Some(panel_rect),
                );
                if !is_val {
                    b.push_hit(HitTarget::AudioDelayStep(step), z, r, CursorHint::Pointer);
                }
            }
        }
        ModalId::Settings => {
            // Settings modal layout: continuous scrollable layout clipped to `body_clip`.
            // At scroll_y=0: APPEARANCE, BEHAVIOR, SCREENSHOTS at top; shortcuts & footer below clip.
            let content_x = panel_rect.x + 20.0;
            let content_w = panel_rect.width - 40.0;
            let filtered = state.filtered_shortcuts();
            let mut sc_list_h = 0.0_f32;
            let mut first_cat = true;
            for cat in SHORTCUT_CATEGORIES {
                let count = filtered.iter().filter(|(_, c, _, _)| *c == cat).count();
                if count == 0 {
                    continue;
                }
                if !first_cat {
                    sc_list_h += 12.0;
                }
                first_cat = false;
                sc_list_h += 20.0 + (count as f32) * 38.0 - 4.0;
            }
            let sc_viewport_h = if filtered.is_empty() {
                54.0
            } else {
                sc_list_h.clamp(80.0, 290.0)
            };
            let sc_card_h = 104.0 + sc_viewport_h;
            let body_clip = RectPx::new(
                panel_rect.x,
                panel_rect.y + 70.0,
                panel_rect.width,
                (panel_rect.height - 71.0).max(40.0),
            );
            let total_settings_h = 14.0
                + 173.0
                + 11.0
                + 251.0
                + 11.0
                + 264.0
                + 14.0
                + sc_card_h
                + 14.0
                + 346.0
                + 14.0
                + 250.0
                + 20.0;
            let max_settings_scroll = (total_settings_h - body_clip.height).max(0.0);
            let scroll_y = state.settings_scroll_y.clamp(0.0, max_settings_scroll);
            let is_visible = |y: f32, h_box: f32| -> bool {
                y + h_box > body_clip.y && y < body_clip.y + body_clip.height
            };

            let draw_switch = |b: &mut FrameBuilder, id: &str, rx: f32, ry: f32, on: bool| {
                let sw_r = RectPx::new(rx, ry, 44.0, 24.0);
                b.push_solid(
                    format!("{id}-track"),
                    z,
                    sw_r,
                    if on {
                        theme.accent
                    } else if is_light {
                        rgba(0, 0, 0, 0.14)
                    } else {
                        rgba(255, 255, 255, 0.14)
                    },
                    [12.0; 4],
                    if on { 0.0 } else { 1.0 },
                    theme.border_color,
                    Some(body_clip),
                );
                let knob_x = if on { sw_r.x + 23.0 } else { sw_r.x + 3.0 };
                b.push_solid(
                    format!("{id}-knob"),
                    z,
                    RectPx::new(knob_x, sw_r.y + 3.0, 18.0, 18.0),
                    [1.0; 4],
                    [9.0; 4],
                    0.0,
                    [0.0; 4],
                    Some(body_clip),
                );
            };

            let draw_select =
                |b: &mut FrameBuilder, id: &str, rx: f32, ry: f32, rw: f32, val: &str| {
                    let sr = RectPx::new(rx, ry, rw, 33.0);
                    b.push_solid(
                        format!("{id}-box"),
                        z,
                        sr,
                        if is_light {
                            rgba(0, 0, 0, 0.04)
                        } else {
                            rgba(255, 255, 255, 0.04)
                        },
                        [7.0; 4],
                        1.0,
                        theme.border_color,
                        Some(body_clip),
                    );
                    b.push_text(
                        format!("{id}-val"),
                        z,
                        RectPx::new(sr.x + 10.0, sr.y + 7.5, sr.width - 28.0, 18.0),
                        val.to_string(),
                        FontRole::Ui,
                        12.0,
                        500,
                        theme.text_primary,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                    b.push_svg(
                        format!("{id}-chev"),
                        z,
                        SvgId::BgPickerChevron,
                        SvgVariant::Default,
                        RectPx::new(sr.x + sr.width - 20.0, sr.y + 10.5, 12.0, 12.0),
                        rgba_f32_to_u8(theme.text_secondary),
                        Some(body_clip),
                    );
                };

            let draw_action_btn = |b: &mut FrameBuilder,
                                   id: &str,
                                   rect: RectPx,
                                   label: &str,
                                   target: HitTarget,
                                   danger: bool| {
                let hovered = b.is_hovered(target.clone());
                let (bg, br, fg) = if danger {
                    (
                        if hovered {
                            rgba(229, 9, 20, 0.22)
                        } else {
                            rgba(229, 9, 20, 0.12)
                        },
                        rgba(229, 9, 20, 0.40),
                        rgba(248, 113, 113, 1.0),
                    )
                } else {
                    (
                        if hovered {
                            if is_light {
                                rgba(0, 0, 0, 0.08)
                            } else {
                                rgba(255, 255, 255, 0.09)
                            }
                        } else {
                            pill_idle_bg
                        },
                        pill_idle_border,
                        theme.text_primary,
                    )
                };
                b.push_solid(
                    format!("{id}-bg"),
                    z,
                    rect,
                    bg,
                    [7.0; 4],
                    1.0,
                    br,
                    Some(body_clip),
                );
                b.push_text(
                    format!("{id}-lbl"),
                    z,
                    RectPx::new(
                        rect.x + 6.0,
                        rect.y + (rect.height - 18.0) * 0.5,
                        rect.width - 12.0,
                        18.0,
                    ),
                    label.to_string(),
                    FontRole::Ui,
                    12.0,
                    600,
                    fg,
                    TextAlign::Center,
                    Some(body_clip),
                );
                b.push_hit(target, z, rect, CursorHint::Pointer);
            };

            let mut cursor_y = panel_rect.y + 84.0 - scroll_y;
            let s_card_bg = if is_light {
                theme.bg_secondary
            } else {
                rgba(255, 255, 255, 0.025)
            };
            let s_card_br = if is_light {
                theme.border_color
            } else {
                rgba(255, 255, 255, 0.08)
            };

            {
                // 1. APPEARANCE (section header inside card, no inner row dividers, 44x24 switch, framed 26x24 swatch)
                let app_card_y = cursor_y;
                let app_card_h = 173.0;
                if is_visible(app_card_y, app_card_h) {
                    b.push_solid(
                        "settings-appearance-card",
                        z,
                        RectPx::new(content_x, app_card_y, content_w, app_card_h),
                        s_card_bg,
                        [14.0; 4],
                        1.0,
                        s_card_br,
                        Some(body_clip),
                    );
                    b.push_text(
                        "settings-hdr-appearance",
                        z,
                        RectPx::new(content_x + 16.0, app_card_y + 14.0, 160.0, 16.0),
                        "APPEARANCE",
                        FontRole::Ui,
                        11.0,
                        700,
                        theme.text_secondary,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                    let r1_y = app_card_y + 41.0;
                    b.push_text(
                        "settings-theme-label",
                        z,
                        RectPx::new(content_x + 16.0, r1_y + 10.5, 120.0, 20.0),
                        "Theme",
                        FontRole::Ui,
                        13.0,
                        500,
                        theme.text_primary,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                    let theme_name = match state.theme {
                        hybrid_shaders::ThemeMode::Dark => "Dark",
                        hybrid_shaders::ThemeMode::Oled => "OLED Black",
                        hybrid_shaders::ThemeMode::Light => "Light",
                    };
                    draw_select(
                        b,
                        "settings-theme-sel",
                        content_x + content_w - 150.0,
                        r1_y + 4.0,
                        134.0,
                        theme_name,
                    );
                    b.push_hit(
                        HitTarget::SettingsThemeSelect,
                        z,
                        RectPx::new(content_x + content_w - 150.0, r1_y + 4.0, 134.0, 33.0),
                        CursorHint::Pointer,
                    );
                    let r2_y = app_card_y + 85.0;
                    b.push_text(
                        "settings-accent-label",
                        z,
                        RectPx::new(content_x + 16.0, r2_y + 8.0, 120.0, 20.0),
                        "Accent Color",
                        FontRole::Ui,
                        13.0,
                        500,
                        theme.text_primary,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                    let swatch_frame =
                        RectPx::new(content_x + content_w - 42.0, r2_y + 6.0, 26.0, 24.0);
                    b.push_solid(
                        "settings-accent-frame",
                        z,
                        swatch_frame,
                        if is_light {
                            rgba(0, 0, 0, 0.04)
                        } else {
                            rgba(0, 0, 0, 0.4)
                        },
                        [6.0; 4],
                        1.0,
                        theme.border_color,
                        Some(body_clip),
                    );
                    b.push_solid(
                        "settings-accent-swatch",
                        z,
                        RectPx::new(
                            swatch_frame.x + 4.0,
                            swatch_frame.y + 4.0,
                            swatch_frame.width - 8.0,
                            swatch_frame.height - 8.0,
                        ),
                        theme.accent,
                        [3.0; 4],
                        0.0,
                        [0.0; 4],
                        Some(body_clip),
                    );
                    b.push_hit(
                        HitTarget::SettingsAccentPicker,
                        z,
                        swatch_frame,
                        CursorHint::Pointer,
                    );
                    let r3_y = app_card_y + 125.0;
                    b.push_text(
                        "settings-fonts-label",
                        z,
                        RectPx::new(content_x + 16.0, r3_y + 8.0, 120.0, 20.0),
                        "Brand Fonts",
                        FontRole::Ui,
                        13.0,
                        500,
                        theme.text_primary,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                    draw_switch(
                        b,
                        "settings-fonts-sw",
                        content_x + content_w - 60.0,
                        r3_y + 6.0,
                        state.font_config.brand_font_enabled,
                    );
                    b.push_hit(
                        HitTarget::SettingsBrandFontsToggle,
                        z,
                        RectPx::new(content_x + content_w - 60.0, r3_y + 6.0, 44.0, 24.0),
                        CursorHint::Pointer,
                    );
                }
                cursor_y = app_card_y + app_card_h + 11.0;

                // 2. BEHAVIOR (header inside card, no inner row dividers)
                let beh_card_y = cursor_y;
                let beh_card_h = 251.0;
                if is_visible(beh_card_y, beh_card_h) {
                    b.push_solid(
                        "settings-behavior-card",
                        z,
                        RectPx::new(content_x, beh_card_y, content_w, beh_card_h),
                        s_card_bg,
                        [14.0; 4],
                        1.0,
                        s_card_br,
                        Some(body_clip),
                    );
                    b.push_text(
                        "settings-hdr-behavior",
                        z,
                        RectPx::new(content_x + 16.0, beh_card_y + 14.0, 160.0, 16.0),
                        "BEHAVIOR",
                        FontRole::Ui,
                        11.0,
                        700,
                        theme.text_secondary,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                    let motion_lbl = match state.motion_profile {
                        crate::motion::MotionProfile::Balanced => "Balanced",
                        crate::motion::MotionProfile::Reduced => "Reduced",
                        crate::motion::MotionProfile::Showcase => "Showcase",
                    };
                    let seek_lbl = if state.seek_step_secs == 5 {
                        "5 seconds (Default)".to_string()
                    } else {
                        format!("{} seconds", state.seek_step_secs)
                    };
                    enum BehCtrl {
                        Switch(bool),
                        Select(String, f32),
                    }
                    let rows = [
                        (
                            "Auto Resume Playback",
                            "auto-resume",
                            HitTarget::SettingsAutoResumeToggle,
                            BehCtrl::Switch(state.auto_resume),
                        ),
                        (
                            "Motion Profile",
                            "motion-profile",
                            HitTarget::SettingsMotionSelect,
                            BehCtrl::Select(motion_lbl.to_string(), 120.0),
                        ),
                        (
                            "Seek Step (Arrow Keys)",
                            "seek-step",
                            HitTarget::SettingsSeekStepSelect,
                            BehCtrl::Select(seek_lbl, 156.0),
                        ),
                        (
                            "Double-Click to Fullscreen",
                            "dbl-click",
                            HitTarget::SettingsDoubleClickToggle,
                            BehCtrl::Switch(state.double_click_fullscreen),
                        ),
                        (
                            "Show Unlock Button When Locked",
                            "unlock-vis",
                            HitTarget::SettingsUnlockVisToggle,
                            BehCtrl::Switch(state.show_unlock_button),
                        ),
                    ];
                    let mut beh_y = beh_card_y + 44.8;
                    for (label, id, target, ctrl) in rows {
                        let (lbl_dy, step_h, hit_r) = match ctrl {
                            BehCtrl::Switch(on) => {
                                let sx = content_x + content_w - 60.0;
                                draw_switch(b, id, sx, beh_y, on);
                                (2.0, 36.0, RectPx::new(sx, beh_y, 44.0, 24.0))
                            }
                            BehCtrl::Select(val, sw) => {
                                let sx = content_x + content_w - 16.0 - sw;
                                draw_select(b, id, sx, beh_y, sw, &val);
                                (6.5, 46.0, RectPx::new(sx, beh_y, sw, 33.0))
                            }
                        };
                        b.push_text(
                            format!("settings-beh-label-{id}"),
                            z,
                            RectPx::new(content_x + 16.0, beh_y + lbl_dy, content_w - 190.0, 20.0),
                            label,
                            FontRole::Ui,
                            13.0,
                            500,
                            theme.text_primary,
                            TextAlign::Left,
                            Some(body_clip),
                        );
                        b.push_hit(target, z, hit_r, CursorHint::Pointer);
                        beh_y += step_h;
                    }
                }
                cursor_y = beh_card_y + beh_card_h + 11.0;

                // 3. SCREENSHOTS (header inside card, Save Directory + Browse/Open/Reset, Image Format, Auto-Organize, Mouse Screenshot)
                let ss_card_y = cursor_y;
                let ss_card_h = 264.0;
                if is_visible(ss_card_y, ss_card_h) {
                    b.push_solid(
                        "settings-screenshots-card",
                        z,
                        RectPx::new(content_x, ss_card_y, content_w, ss_card_h),
                        s_card_bg,
                        [14.0; 4],
                        1.0,
                        s_card_br,
                        Some(body_clip),
                    );
                    b.push_text(
                        "settings-hdr-screenshots",
                        z,
                        RectPx::new(content_x + 16.0, ss_card_y + 14.0, 160.0, 16.0),
                        "SCREENSHOTS",
                        FontRole::Ui,
                        11.0,
                        700,
                        theme.text_secondary,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                    b.push_text(
                        "settings-ss-dir-label",
                        z,
                        RectPx::new(content_x + 16.0, ss_card_y + 42.0, 140.0, 20.0),
                        "Save Directory",
                        FontRole::Ui,
                        13.0,
                        500,
                        theme.text_primary,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                    let dir_input =
                        RectPx::new(content_x + 16.0, ss_card_y + 68.0, content_w - 32.0, 32.0);
                    b.push_solid(
                        "settings-ss-dir-input",
                        z,
                        dir_input,
                        if is_light {
                            theme.bg_tertiary
                        } else {
                            rgba8(0x0d, 0x0f, 0x17, 255)
                        },
                        [7.0; 4],
                        1.0,
                        s_card_br,
                        Some(body_clip),
                    );
                    let default_dir_label;
                    let dir_txt = if state.screenshot_dir.is_empty() {
                        if state.default_screenshot_dir.is_empty() {
                            "Default (App Data/Screenshots)"
                        } else {
                            default_dir_label =
                                format!("{} (Default)", state.default_screenshot_dir);
                            default_dir_label.as_str()
                        }
                    } else {
                        state.screenshot_dir.as_str()
                    };
                    b.push_text(
                        "settings-ss-dir-val",
                        z,
                        RectPx::new(
                            dir_input.x + 10.0,
                            dir_input.y + 7.0,
                            dir_input.width - 20.0,
                            18.0,
                        ),
                        dir_txt,
                        FontRole::Ui,
                        12.0,
                        400,
                        theme.text_secondary,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                    b.push_hit(
                        HitTarget::SettingsScreenshotDirPick,
                        z,
                        dir_input,
                        CursorHint::Pointer,
                    );

                    let btn_w = (content_w - 48.0) / 3.0;
                    let btn_y = ss_card_y + 108.0;
                    draw_action_btn(
                        b,
                        "settings-ss-browse",
                        RectPx::new(content_x + 16.0, btn_y, btn_w, 26.0),
                        "Browse...",
                        HitTarget::SettingsScreenshotDirPick,
                        false,
                    );
                    draw_action_btn(
                        b,
                        "settings-ss-open",
                        RectPx::new(content_x + 24.0 + btn_w, btn_y, btn_w, 26.0),
                        "Open Folder",
                        HitTarget::SettingsScreenshotDirOpen,
                        false,
                    );
                    draw_action_btn(
                        b,
                        "settings-ss-reset",
                        RectPx::new(content_x + 32.0 + btn_w * 2.0, btn_y, btn_w, 26.0),
                        "Reset",
                        HitTarget::SettingsScreenshotDirReset,
                        false,
                    );

                    // Row: Image Format
                    let fmt_y = ss_card_y + 144.0;
                    b.push_text(
                        "settings-ss-fmt-label",
                        z,
                        RectPx::new(content_x + 16.0, fmt_y + 6.5, 150.0, 20.0),
                        "Image Format",
                        FontRole::Ui,
                        13.0,
                        500,
                        theme.text_primary,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                    let fmt_val = match state.screenshot_format.to_ascii_lowercase().as_str() {
                        "png" => "PNG (.png)",
                        "webp" => "WebP (.webp)",
                        _ => "JPEG (.jpg)",
                    };
                    draw_select(
                        b,
                        "settings-ss-fmt",
                        content_x + content_w - 156.0,
                        fmt_y,
                        140.0,
                        fmt_val,
                    );
                    b.push_hit(
                        HitTarget::SettingsScreenshotFormatSelect,
                        z,
                        RectPx::new(content_x + content_w - 156.0, fmt_y, 140.0, 33.0),
                        CursorHint::Pointer,
                    );

                    // Row: Auto-Organize Folders
                    let org_y = ss_card_y + 186.0;
                    b.push_text(
                        "settings-ss-org-label",
                        z,
                        RectPx::new(content_x + 16.0, org_y + 2.0, 200.0, 20.0),
                        "Auto-Organize Folders",
                        FontRole::Ui,
                        13.0,
                        500,
                        theme.text_primary,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                    draw_switch(
                        b,
                        "settings-ss-org-sw",
                        content_x + content_w - 60.0,
                        org_y,
                        state.auto_organize_screenshots,
                    );
                    b.push_hit(
                        HitTarget::SettingsAutoOrganizeToggle,
                        z,
                        RectPx::new(content_x + content_w - 60.0, org_y, 44.0, 24.0),
                        CursorHint::Pointer,
                    );

                    // Row: Mouse Screenshot Button
                    let ms_y = ss_card_y + 222.0;
                    b.push_text(
                        "settings-ss-mouse-label",
                        z,
                        RectPx::new(content_x + 16.0, ms_y + 6.5, 200.0, 20.0),
                        "Mouse Screenshot Button",
                        FontRole::Ui,
                        13.0,
                        500,
                        theme.text_primary,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                    let ms_val = match state.screenshot_mouse_button {
                        1 => "Middle Click",
                        3 => "Mouse 4 (Back Button)",
                        4 => "Mouse 5 (Forward Button)",
                        _ => "Off",
                    };
                    draw_select(
                        b,
                        "settings-ss-mouse",
                        content_x + content_w - 200.0,
                        ms_y,
                        184.0,
                        ms_val,
                    );
                    b.push_hit(
                        HitTarget::SettingsMouseScreenshotSelect,
                        z,
                        RectPx::new(content_x + content_w - 200.0, ms_y, 184.0, 33.0),
                        CursorHint::Pointer,
                    );
                }
                cursor_y = ss_card_y + ss_card_h + 14.0;
            }

            // 4. KEYBOARD SHORTCUTS (`#settGroupShortcuts`, 290px scrollable container)
            let sc_card_y = cursor_y;
            let search_r = RectPx::new(content_x + 16.0, sc_card_y + 44.0, content_w - 32.0, 34.0);
            let sc_box = RectPx::new(
                content_x + 16.0,
                sc_card_y + 88.0,
                content_w - 32.0,
                sc_viewport_h,
            );

            if is_visible(sc_card_y, sc_card_h) {
                b.push_solid(
                    "settings-shortcuts-card",
                    z,
                    RectPx::new(content_x, sc_card_y, content_w, sc_card_h),
                    s_card_bg,
                    [14.0; 4],
                    1.0,
                    s_card_br,
                    Some(body_clip),
                );
                b.push_text(
                    "settings-hdr-shortcuts",
                    z,
                    RectPx::new(content_x + 16.0, sc_card_y + 15.0, 180.0, 16.0),
                    "KEYBOARD SHORTCUTS",
                    FontRole::Ui,
                    11.0,
                    700,
                    theme.text_secondary,
                    TextAlign::Left,
                    Some(body_clip),
                );
                draw_action_btn(
                    b,
                    "shortcut-reset-all",
                    RectPx::new(content_x + content_w - 102.0, sc_card_y + 11.0, 86.0, 24.0),
                    "Reset All",
                    HitTarget::ShortcutResetAll,
                    false,
                );
                draw_text_input_box(
                    b,
                    state,
                    theme,
                    "shortcut-search",
                    z,
                    search_r,
                    if is_light {
                        theme.bg_tertiary
                    } else {
                        rgba8(0x0d, 0x0f, 0x17, 255)
                    },
                    s_card_br,
                    8.0,
                    12.0,
                    12.5,
                    "Search shortcuts or keys (e.g. Screenshot, S, Play)...",
                    FocusedTextField::ShortcutSearch,
                    HitTarget::ShortcutSearch,
                    Some(body_clip),
                );
            }

            if is_visible(sc_box.y, sc_box.height) {
                let cx0 = body_clip.x.max(sc_box.x);
                let cy0 = body_clip.y.max(sc_box.y);
                let cx1 = (body_clip.x + body_clip.width).min(sc_box.x + sc_box.width);
                let cy1 = (body_clip.y + body_clip.height).min(sc_box.y + sc_box.height);
                if cx1 > cx0 && cy1 > cy0 {
                    let sc_clip = RectPx::new(cx0, cy0, cx1 - cx0, cy1 - cy0);
                    b.push_hit(
                        HitTarget::ShortcutScrollArea,
                        z,
                        sc_clip,
                        CursorHint::Default,
                    );

                    if filtered.is_empty() {
                        b.push_text(
                            "shortcut-empty-state",
                            z,
                            RectPx::new(sc_box.x + 8.0, sc_box.y + 16.0, sc_box.width - 16.0, 20.0),
                            "No shortcuts match your search.",
                            FontRole::Ui,
                            12.0,
                            400,
                            theme.text_tertiary,
                            TextAlign::Center,
                            Some(sc_clip),
                        );
                    } else {
                        let max_sc_scroll = (sc_list_h - sc_viewport_h).max(0.0);
                        let sc_scroll = state.shortcut_scroll_y.clamp(0.0, max_sc_scroll);
                        let inner_w = sc_box.width - if max_sc_scroll > 0.5 { 10.0 } else { 0.0 };
                        let mut sy = sc_box.y - sc_scroll;
                        let mut first_c = true;

                        for cat in SHORTCUT_CATEGORIES {
                            let items: Vec<_> =
                                filtered.iter().filter(|(_, c, _, _)| *c == cat).collect();
                            if items.is_empty() {
                                continue;
                            }
                            if !first_c {
                                sy += 12.0;
                            }
                            first_c = false;

                            if sy + 20.0 > sc_clip.y && sy < sc_clip.y + sc_clip.height {
                                b.push_text_spaced(
                                    format!("shortcut-cat-{cat}"),
                                    z,
                                    RectPx::new(sc_box.x + 2.0, sy + 2.0, inner_w - 4.0, 16.0),
                                    cat.to_ascii_uppercase(),
                                    FontRole::Ui,
                                    10.5,
                                    700,
                                    0.9,
                                    if is_light {
                                        rgba(0, 0, 0, 0.48)
                                    } else {
                                        rgba(255, 255, 255, 0.42)
                                    },
                                    TextAlign::Left,
                                    Some(sc_clip),
                                );
                            }
                            sy += 20.0;

                            for (idx_in_cat, (id, _cat, label, combo)) in items.iter().enumerate() {
                                let item_h = 34.0;
                                if sy + item_h > sc_clip.y && sy < sc_clip.y + sc_clip.height {
                                    let is_recording =
                                        state.recording_shortcut_id.as_deref() == Some(*id);
                                    let is_customized = state.custom_shortcuts.contains_key(*id);
                                    let is_unbound = combo.trim().is_empty();
                                    let rrect = RectPx::new(sc_box.x, sy, inner_w, item_h);
                                    let row_hov =
                                        b.is_hovered(HitTarget::ShortcutRebind((*id).to_string()));
                                    let row_bg = if row_hov {
                                        if is_light {
                                            rgba(0, 0, 0, 0.045)
                                        } else {
                                            rgba(255, 255, 255, 0.045)
                                        }
                                    } else if is_light {
                                        rgba(0, 0, 0, 0.02)
                                    } else {
                                        rgba(255, 255, 255, 0.02)
                                    };
                                    let row_br = if is_customized {
                                        theme.accent_dim
                                    } else if row_hov {
                                        if is_light {
                                            rgba(0, 0, 0, 0.12)
                                        } else {
                                            rgba(255, 255, 255, 0.09)
                                        }
                                    } else if is_light {
                                        rgba(0, 0, 0, 0.06)
                                    } else {
                                        rgba(255, 255, 255, 0.04)
                                    };
                                    b.push_solid(
                                        format!("shortcut-item-bg-{id}"),
                                        z,
                                        rrect,
                                        row_bg,
                                        [9.0; 4],
                                        1.0,
                                        row_br,
                                        Some(sc_clip),
                                    );

                                    let disp_combo = if is_recording {
                                        state
                                            .recording_shortcut_preview
                                            .clone()
                                            .unwrap_or_else(|| "Press key...".to_string())
                                    } else {
                                        format_combo_for_display(combo)
                                    };
                                    let combo_w = measure_text_width(
                                        &disp_combo,
                                        FontRole::Mono,
                                        11.5,
                                        600,
                                        0.0,
                                        state.font_config,
                                    );
                                    let key_w = (combo_w + 20.0).clamp(78.0, 148.0);
                                    let reset_one = RectPx::new(
                                        rrect.x + rrect.width - 32.0,
                                        sy + 5.0,
                                        24.0,
                                        24.0,
                                    );
                                    let key_r = RectPx::new(
                                        reset_one.x - 6.0 - key_w,
                                        sy + 4.0,
                                        key_w,
                                        26.0,
                                    );

                                    let mut lbl_x = rrect.x + 8.0;
                                    if is_customized {
                                        b.push_solid(
                                            format!("shortcut-mod-dot-{id}"),
                                            z,
                                            RectPx::new(lbl_x, sy + 14.0, 6.0, 6.0),
                                            theme.accent,
                                            [3.0; 4],
                                            0.0,
                                            [0.0; 4],
                                            Some(sc_clip),
                                        );
                                        lbl_x += 13.0;
                                    }
                                    let lbl_max_w = (key_r.x - 10.0 - lbl_x).max(40.0);
                                    b.push_text(
                                        format!("shortcut-label-{id}"),
                                        z,
                                        RectPx::new(lbl_x, sy + 7.5, lbl_max_w, 19.0),
                                        (*label).to_string(),
                                        FontRole::Ui,
                                        12.5,
                                        500,
                                        theme.text_primary,
                                        TextAlign::Left,
                                        Some(sc_clip),
                                    );

                                    let key_bg = if is_recording {
                                        theme.accent_dim
                                    } else if is_light {
                                        [1.0; 4]
                                    } else {
                                        rgba(255, 255, 255, 0.06)
                                    };
                                    let key_br = if is_recording {
                                        theme.accent
                                    } else if is_light {
                                        rgba(0, 0, 0, 0.16)
                                    } else {
                                        rgba(255, 255, 255, 0.14)
                                    };
                                    b.push_solid(
                                        format!("shortcut-key-bg-{id}"),
                                        z,
                                        key_r,
                                        key_bg,
                                        [7.0; 4],
                                        if is_unbound && !is_recording {
                                            0.0
                                        } else {
                                            1.0
                                        },
                                        key_br,
                                        Some(sc_clip),
                                    );
                                    if is_unbound && !is_recording {
                                        let dcol = if is_light {
                                            rgba(0, 0, 0, 0.20)
                                        } else {
                                            rgba(255, 255, 255, 0.18)
                                        };
                                        let mut dx = key_r.x + 4.0;
                                        let mut di = 0u32;
                                        while dx + 3.0 < key_r.x + key_r.width - 4.0 {
                                            let seg = 4.0_f32.min(key_r.x + key_r.width - 4.0 - dx);
                                            b.push_solid(
                                                format!("sc-ub-t-{id}-{di}"),
                                                z,
                                                RectPx::new(dx, key_r.y, seg, 1.0),
                                                dcol,
                                                [0.0; 4],
                                                0.0,
                                                [0.0; 4],
                                                Some(sc_clip),
                                            );
                                            b.push_solid(
                                                format!("sc-ub-b-{id}-{di}"),
                                                z,
                                                RectPx::new(
                                                    dx,
                                                    key_r.y + key_r.height - 2.0,
                                                    seg,
                                                    2.0,
                                                ),
                                                dcol,
                                                [0.0; 4],
                                                0.0,
                                                [0.0; 4],
                                                Some(sc_clip),
                                            );
                                            dx += 7.0;
                                            di += 1;
                                        }
                                    } else {
                                        // 2px keycap bottom lip (`border-bottom-width: 2px`)
                                        b.push_solid(
                                            format!("shortcut-key-lip-{id}"),
                                            z,
                                            RectPx::new(
                                                key_r.x + 4.0,
                                                key_r.y + key_r.height - 2.0,
                                                (key_r.width - 8.0).max(4.0),
                                                1.5,
                                            ),
                                            key_br,
                                            [0.0; 4],
                                            0.0,
                                            [0.0; 4],
                                            Some(sc_clip),
                                        );
                                    }
                                    b.push_text(
                                        format!("shortcut-key-lbl-{id}"),
                                        z,
                                        RectPx::new(
                                            key_r.x + 6.0,
                                            key_r.y + 4.5,
                                            key_r.width - 12.0,
                                            18.0,
                                        ),
                                        disp_combo,
                                        FontRole::Mono,
                                        11.5,
                                        if is_unbound && !is_recording {
                                            500
                                        } else {
                                            600
                                        },
                                        if is_recording {
                                            [1.0; 4]
                                        } else if is_unbound {
                                            if is_light {
                                                rgba(0, 0, 0, 0.4)
                                            } else {
                                                rgba(255, 255, 255, 0.4)
                                            }
                                        } else {
                                            theme.text_primary
                                        },
                                        TextAlign::Center,
                                        Some(sc_clip),
                                    );
                                    b.push_hit(
                                        HitTarget::ShortcutRebind((*id).to_string()),
                                        z,
                                        RectPx::new(
                                            rrect.x,
                                            rrect.y,
                                            reset_one.x - rrect.x - 2.0,
                                            rrect.height,
                                        ),
                                        CursorHint::Pointer,
                                    );

                                    let reset_alpha = if is_customized { 1.0 } else { 0.22 };
                                    b.push_solid(
                                        format!("shortcut-reset-bg-{id}"),
                                        z,
                                        reset_one,
                                        if is_light {
                                            rgba(0, 0, 0, 0.04 * reset_alpha)
                                        } else {
                                            rgba(255, 255, 255, 0.04 * reset_alpha)
                                        },
                                        [6.0; 4],
                                        1.0,
                                        if is_light {
                                            rgba(0, 0, 0, 0.10 * reset_alpha)
                                        } else {
                                            rgba(255, 255, 255, 0.08 * reset_alpha)
                                        },
                                        Some(sc_clip),
                                    );
                                    let mut icon_col = rgba_f32_to_u8(theme.text_secondary);
                                    icon_col[3] =
                                        ((icon_col[3] as f32) * reset_alpha).round() as u8;
                                    b.push_svg(
                                        format!("shortcut-reset-{id}"),
                                        z,
                                        SvgId::ShortcutRowReset,
                                        SvgVariant::Default,
                                        RectPx::new(
                                            reset_one.x + 5.0,
                                            reset_one.y + 5.0,
                                            14.0,
                                            14.0,
                                        ),
                                        icon_col,
                                        Some(sc_clip),
                                    );
                                    if is_customized {
                                        b.push_hit(
                                            HitTarget::ShortcutResetOne((*id).to_string()),
                                            z,
                                            reset_one,
                                            CursorHint::Pointer,
                                        );
                                    }
                                }
                                let _ = idx_in_cat;
                                sy += 38.0;
                            }
                            sy -= 4.0;
                        }

                        if max_sc_scroll > 0.5 {
                            let track_r = RectPx::new(
                                sc_box.x + sc_box.width - 5.0,
                                sc_box.y + 2.0,
                                5.0,
                                (sc_box.height - 4.0).max(20.0),
                            );
                            let thumb_h = ((sc_viewport_h / sc_list_h) * track_r.height)
                                .clamp(24.0, track_r.height);
                            let thumb_y = track_r.y
                                + (sc_scroll / max_sc_scroll) * (track_r.height - thumb_h);
                            b.push_solid(
                                "shortcut-scroll-thumb",
                                z,
                                RectPx::new(track_r.x, thumb_y, 5.0, thumb_h),
                                if is_light {
                                    rgba(0, 0, 0, 0.18)
                                } else {
                                    rgba(255, 255, 255, 0.16)
                                },
                                [2.5; 4],
                                0.0,
                                [0.0; 4],
                                Some(sc_clip),
                            );
                        }
                    }
                }
            }
            cursor_y = sc_card_y + sc_card_h + 14.0;

            {
                // 5. STREAMING ENGINE (yt-dlp) & YOUTUBE ACCOUNT
                let st_card_y = cursor_y;
                let st_card_h = 346.0;
                if is_visible(st_card_y, st_card_h) {
                    b.push_solid(
                        "settings-streaming-card",
                        z,
                        RectPx::new(content_x, st_card_y, content_w, st_card_h),
                        s_card_bg,
                        [14.0; 4],
                        1.0,
                        s_card_br,
                        Some(body_clip),
                    );
                    b.push_text(
                        "settings-hdr-streaming",
                        z,
                        RectPx::new(content_x + 16.0, st_card_y + 14.0, 220.0, 16.0),
                        "STREAMING ENGINE (YT-DLP)",
                        FontRole::Ui,
                        11.0,
                        700,
                        theme.text_secondary,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                    // Row 1: Engine Version
                    let r_ver_y = st_card_y + 42.0;
                    b.push_text(
                        "ytdlp-ver-lbl",
                        z,
                        RectPx::new(content_x + 16.0, r_ver_y + 3.0, 140.0, 18.0),
                        "Engine Version",
                        FontRole::Ui,
                        13.0,
                        500,
                        theme.text_primary,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                    let ver_txt = if state.ytdlp_version.trim().is_empty() {
                        "Bundled"
                    } else {
                        state.ytdlp_version.as_str()
                    };
                    let ver_w = (measure_text_width(
                        ver_txt,
                        FontRole::Mono,
                        11.0,
                        600,
                        0.0,
                        state.font_config,
                    ) + 20.0)
                        .clamp(84.0, 170.0);
                    let ver_badge_r =
                        RectPx::new(content_x + content_w - 16.0 - ver_w, r_ver_y, ver_w, 24.0);
                    b.push_solid(
                        "ytdlp-ver-badge",
                        z,
                        ver_badge_r,
                        pill_idle_bg,
                        [7.0; 4],
                        1.0,
                        pill_idle_border,
                        Some(body_clip),
                    );
                    b.push_text(
                        "ytdlp-ver-val",
                        z,
                        RectPx::new(
                            ver_badge_r.x + 6.0,
                            ver_badge_r.y + 4.0,
                            ver_badge_r.width - 12.0,
                            16.0,
                        ),
                        ver_txt,
                        FontRole::Mono,
                        11.0,
                        600,
                        theme.text_secondary,
                        TextAlign::Center,
                        Some(body_clip),
                    );

                    // Row 2: Status
                    let r_st_y = st_card_y + 74.0;
                    let st_lower = state.ytdlp_status_text.to_ascii_lowercase();
                    let ytdlp_status_color = if state.ytdlp_update_available {
                        theme.ytdlp_info
                    } else if st_lower.contains("fail")
                        || st_lower.contains("error")
                        || st_lower.contains("not found")
                        || st_lower.contains("missing")
                    {
                        theme.ytdlp_error
                    } else if st_lower.contains("checking")
                        || st_lower.contains("updating")
                        || st_lower.contains("downloading")
                    {
                        theme.ytdlp_pending
                    } else {
                        theme.ytdlp_ok
                    };
                    b.push_text(
                        "ytdlp-st-lbl",
                        z,
                        RectPx::new(content_x + 16.0, r_st_y + 2.0, 90.0, 18.0),
                        "Status",
                        FontRole::Ui,
                        13.0,
                        500,
                        theme.text_primary,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                    b.push_text(
                        "ytdlp-status",
                        z,
                        RectPx::new(content_x + 116.0, r_st_y + 2.0, content_w - 132.0, 18.0),
                        state.ytdlp_status_text.clone(),
                        FontRole::Ui,
                        12.0,
                        500,
                        ytdlp_status_color,
                        TextAlign::Right,
                        Some(body_clip),
                    );

                    // Row 3: Actions
                    let r_act_y = st_card_y + 106.0;
                    b.push_text(
                        "ytdlp-act-lbl",
                        z,
                        RectPx::new(content_x + 16.0, r_act_y + 4.0, 90.0, 18.0),
                        "Actions",
                        FontRole::Ui,
                        13.0,
                        500,
                        theme.text_primary,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                    if state.ytdlp_update_available {
                        let upd_label = if let Some(latest) = &state.ytdlp_latest_version {
                            let clean = latest.strip_prefix('v').unwrap_or(latest.as_str());
                            format!("Update to v{clean}")
                        } else {
                            "Update Engine".to_string()
                        };
                        let upd_w = (measure_text_width(
                            &upd_label,
                            FontRole::Ui,
                            12.0,
                            600,
                            0.0,
                            state.font_config,
                        ) + 22.0)
                            .clamp(106.0, 168.0);
                        let upd_x = content_x + content_w - 16.0 - upd_w;
                        draw_action_btn(
                            b,
                            "ytdlp-check-btn",
                            RectPx::new(upd_x - 134.0, r_act_y, 128.0, 27.0),
                            "Check for Updates",
                            HitTarget::YtdlpCheckUpdate,
                            false,
                        );
                        draw_action_btn(
                            b,
                            "ytdlp-update-btn",
                            RectPx::new(upd_x, r_act_y, upd_w, 27.0),
                            &upd_label,
                            HitTarget::YtdlpPerformUpdate,
                            false,
                        );
                    } else {
                        draw_action_btn(
                            b,
                            "ytdlp-check-btn",
                            RectPx::new(content_x + content_w - 144.0, r_act_y, 128.0, 27.0),
                            "Check for Updates",
                            HitTarget::YtdlpCheckUpdate,
                            false,
                        );
                    }

                    // Dedicated YouTube Obsidian Card (`.yt-cookies-card`)
                    let yt_card =
                        RectPx::new(content_x + 14.0, st_card_y + 144.0, content_w - 28.0, 188.0);
                    let yt_bg = match state.theme {
                        ThemeMode::Oled => [0.0, 0.0, 0.0, 1.0],
                        ThemeMode::Light => rgba(0, 0, 0, 0.02),
                        ThemeMode::Dark => rgba(255, 255, 255, 0.025),
                    };
                    let yt_br = match state.theme {
                        ThemeMode::Oled => rgba(255, 255, 255, 0.10),
                        ThemeMode::Light => rgba(0, 0, 0, 0.08),
                        ThemeMode::Dark => rgba(255, 255, 255, 0.07),
                    };
                    b.push_solid(
                        "yt-obsidian-card",
                        z,
                        yt_card,
                        yt_bg,
                        [12.0; 4],
                        1.0,
                        yt_br,
                        Some(body_clip),
                    );

                    let badge_r = RectPx::new(yt_card.x + 15.0, yt_card.y + 13.0, 32.0, 32.0);
                    b.push_solid(
                        "yt-icon-badge",
                        z,
                        badge_r,
                        match state.theme {
                            ThemeMode::Oled => rgba8(0x08, 0x08, 0x08, 255),
                            ThemeMode::Light => rgba(0, 0, 0, 0.035),
                            ThemeMode::Dark => rgba(255, 255, 255, 0.04),
                        },
                        [9.0; 4],
                        1.0,
                        if is_light {
                            rgba(0, 0, 0, 0.08)
                        } else {
                            rgba(255, 255, 255, 0.08)
                        },
                        Some(body_clip),
                    );
                    b.push_svg(
                        "yt-cookies-logo",
                        z,
                        SvgId::SettingsYoutubeLogo,
                        SvgVariant::Default,
                        RectPx::new(badge_r.x + 5.0, badge_r.y + 8.0, 22.0, 16.0),
                        rgba_f32_to_u8(theme.accent),
                        Some(body_clip),
                    );
                    b.push_text(
                        "yt-account-title",
                        z,
                        RectPx::new(badge_r.x + 43.0, yt_card.y + 12.0, 160.0, 18.0),
                        "YouTube Account",
                        FontRole::Display,
                        13.0,
                        600,
                        theme.text_primary,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                    b.push_text(
                        "yt-account-subtitle",
                        z,
                        RectPx::new(badge_r.x + 43.0, yt_card.y + 30.0, 170.0, 15.0),
                        "Authentication & Cookies",
                        FontRole::Ui,
                        11.0,
                        400,
                        theme.text_tertiary,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                    let status_r = RectPx::new(
                        yt_card.x + yt_card.width - 118.0,
                        yt_card.y + 16.0,
                        104.0,
                        22.0,
                    );
                    b.push_solid(
                        "yt-status-pill",
                        z,
                        status_r,
                        if state.youtube_signed_in {
                            rgba(34, 197, 94, 0.12)
                        } else if is_light {
                            rgba(0, 0, 0, 0.04)
                        } else {
                            rgba(255, 255, 255, 0.035)
                        },
                        [11.0; 4],
                        1.0,
                        if state.youtube_signed_in {
                            rgba(34, 197, 94, 0.30)
                        } else if is_light {
                            rgba(0, 0, 0, 0.10)
                        } else {
                            rgba(255, 255, 255, 0.08)
                        },
                        Some(body_clip),
                    );
                    b.push_solid(
                        "yt-status-dot",
                        z,
                        RectPx::new(status_r.x + 10.0, status_r.y + 8.0, 6.0, 6.0),
                        if state.youtube_signed_in {
                            rgba8(0x4a, 0xde, 0x80, 255)
                        } else {
                            theme.text_tertiary
                        },
                        [3.0; 4],
                        0.0,
                        [0.0; 4],
                        Some(body_clip),
                    );
                    b.push_text(
                        "yt-status-lbl",
                        z,
                        RectPx::new(
                            status_r.x + 18.0,
                            status_r.y + 3.0,
                            status_r.width - 22.0,
                            16.0,
                        ),
                        if state.youtube_signed_in {
                            "Signed in"
                        } else {
                            "Not signed in"
                        },
                        FontRole::Ui,
                        11.0,
                        500,
                        if state.youtube_signed_in {
                            rgba8(0x4a, 0xde, 0x80, 255)
                        } else {
                            theme.text_secondary
                        },
                        TextAlign::Center,
                        Some(body_clip),
                    );
                    b.push_text_wrapped(
                        "yt-account-hint",
                        z,
                        RectPx::new(yt_card.x + 15.0, yt_card.y + 54.0, yt_card.width - 30.0, 44.0),
                        "Click \"Sign in to YouTube\" to authenticate via a clean browser window. Session cookies sync automatically for 4K and restricted streams.",
                        FontRole::Ui, 11.5, 400, theme.text_secondary, TextAlign::Left, Some(body_clip),
                    );
                    if state.youtube_signed_in {
                        let hw = (yt_card.width - 38.0) * 0.5;
                        draw_action_btn(
                            b,
                            "yt-switch-btn",
                            RectPx::new(yt_card.x + 15.0, yt_card.y + 108.0, hw, 30.0),
                            "Switch Account",
                            HitTarget::YoutubeSwitchAccount,
                            false,
                        );
                        draw_action_btn(
                            b,
                            "yt-signout-btn",
                            RectPx::new(yt_card.x + 23.0 + hw, yt_card.y + 108.0, hw, 30.0),
                            "Sign Out",
                            HitTarget::YoutubeSignOut,
                            true,
                        );
                    } else {
                        let signin_r = RectPx::new(
                            yt_card.x + 15.0,
                            yt_card.y + 108.0,
                            yt_card.width - 30.0,
                            30.0,
                        );
                        b.push_solid(
                            "yt-signin-bg",
                            z,
                            signin_r,
                            theme.accent,
                            [8.0; 4],
                            0.0,
                            [0.0; 4],
                            Some(body_clip),
                        );
                        b.push_text(
                            "yt-signin-lbl",
                            z,
                            RectPx::new(signin_r.x, signin_r.y + 6.0, signin_r.width, 18.0),
                            "Sign in to YouTube",
                            FontRole::Ui,
                            12.0,
                            700,
                            [1.0; 4],
                            TextAlign::Center,
                            Some(body_clip),
                        );
                        b.push_hit(HitTarget::YoutubeSignIn, z, signin_r, CursorHint::Pointer);
                    }
                    let optb_w = (yt_card.width - 38.0) * 0.5;
                    draw_action_btn(
                        b,
                        "yt-import-browser-btn",
                        RectPx::new(yt_card.x + 15.0, yt_card.y + 146.0, optb_w, 27.0),
                        "Import Browser Cookies",
                        HitTarget::YoutubeImportBrowserCookies,
                        false,
                    );
                    draw_action_btn(
                        b,
                        "yt-import-file-btn",
                        RectPx::new(yt_card.x + 23.0 + optb_w, yt_card.y + 146.0, optb_w, 27.0),
                        "Import cookies.txt",
                        HitTarget::YoutubeImportCookiesFile,
                        false,
                    );
                }
                cursor_y = st_card_y + st_card_h + 14.0;

                // 6. PLAYER ACTIONS (5 full-width stacked `.settings-action-btn` items)
                let pa_card_y = cursor_y;
                let pa_card_h = 250.0;
                if is_visible(pa_card_y, pa_card_h) {
                    b.push_solid(
                        "settings-actions-card",
                        z,
                        RectPx::new(content_x, pa_card_y, content_w, pa_card_h),
                        s_card_bg,
                        [14.0; 4],
                        1.0,
                        s_card_br,
                        Some(body_clip),
                    );
                    b.push_text(
                        "settings-hdr-actions",
                        z,
                        RectPx::new(content_x + 16.0, pa_card_y + 14.0, 180.0, 16.0),
                        "PLAYER ACTIONS",
                        FontRole::Ui,
                        11.0,
                        700,
                        theme.text_secondary,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                    let list_x = content_x + 16.0;
                    let list_w = content_w - 32.0;
                    let actions = [
                        ("pa-open-file", "Open File", HitTarget::OpenFile, false),
                        (
                            "pa-open-multi",
                            "Open Multiple Files",
                            HitTarget::SettingsOpenMultiple,
                            false,
                        ),
                        (
                            "pa-open-folder",
                            "Open Folder",
                            HitTarget::SettingsOpenFolder,
                            false,
                        ),
                        (
                            "pa-open-stream",
                            "Open Network Stream",
                            HitTarget::OpenLink,
                            false,
                        ),
                        ("pa-quit", "Quit Player", HitTarget::SettingsQuit, true),
                    ];
                    for (idx, (aid, alabel, atarget, danger)) in actions.into_iter().enumerate() {
                        let by = pa_card_y + 42.0 + (idx as f32) * 39.0;
                        let rect = RectPx::new(list_x, by, list_w, 32.0);
                        let hovered = b.is_hovered(atarget.clone());
                        let (bg, br, fg) = if danger {
                            (
                                if hovered {
                                    rgba(255, 74, 74, 0.14)
                                } else {
                                    rgba(255, 74, 74, 0.05)
                                },
                                rgba(255, 74, 74, 0.25),
                                rgba8(0xff, 0x4a, 0x4a, 255),
                            )
                        } else {
                            (
                                if hovered {
                                    if is_light {
                                        rgba(0, 0, 0, 0.08)
                                    } else {
                                        rgba(255, 255, 255, 0.08)
                                    }
                                } else if is_light {
                                    rgba(0, 0, 0, 0.035)
                                } else {
                                    rgba(255, 255, 255, 0.035)
                                },
                                if hovered {
                                    if is_light {
                                        rgba(0, 0, 0, 0.16)
                                    } else {
                                        rgba(255, 255, 255, 0.18)
                                    }
                                } else if is_light {
                                    rgba(0, 0, 0, 0.07)
                                } else {
                                    rgba(255, 255, 255, 0.07)
                                },
                                if hovered {
                                    theme.text_primary
                                } else {
                                    theme.text_secondary
                                },
                            )
                        };
                        b.push_solid(
                            format!("{aid}-bg"),
                            z,
                            rect,
                            bg,
                            [10.0; 4],
                            1.0,
                            br,
                            Some(body_clip),
                        );
                        b.push_text(
                            format!("{aid}-lbl"),
                            z,
                            RectPx::new(rect.x + 12.0, rect.y + 7.0, rect.width - 24.0, 18.0),
                            alabel,
                            FontRole::Ui,
                            12.5,
                            500,
                            fg,
                            TextAlign::Left,
                            Some(body_clip),
                        );
                        b.push_hit(atarget, z, rect, CursorHint::Pointer);
                    }
                }
            }
        }
        ModalId::NetworkStream => {
            let cx = panel_rect.x + 22.0;
            let cw = panel_rect.width - 44.0;
            b.push_text(
                "net-url-label",
                z,
                RectPx::new(cx, panel_rect.y + 87.0, cw, 18.0),
                "YouTube / Stream URL",
                FontRole::Ui,
                13.0,
                500,
                theme.text_secondary,
                TextAlign::Left,
                Some(panel_rect),
            );
            let inp = RectPx::new(cx, panel_rect.y + 115.0, cw, 39.0);
            draw_text_input_box(
                b,
                state,
                theme,
                "network-stream-input",
                z,
                inp,
                if is_light {
                    theme.bg_tertiary
                } else {
                    rgba8(0x1a, 0x1a, 0x25, 255)
                },
                theme.border_color,
                10.0,
                12.0,
                14.0,
                "https://www.youtube.com/watch?v=...",
                FocusedTextField::NetworkStreamUrl,
                HitTarget::NetworkStreamInput,
                Some(panel_rect),
            );
            let cancel_r = RectPx::new(cx + cw - 175.0, panel_rect.y + 169.6, 78.0, 35.0);
            let cancel_hovered = b.is_hovered(HitTarget::ModalClose);
            b.push_solid(
                "net-cancel-bg",
                z,
                cancel_r,
                if cancel_hovered {
                    if is_light {
                        rgba(0, 0, 0, 0.08)
                    } else {
                        rgba(255, 255, 255, 0.09)
                    }
                } else {
                    theme.bg_tertiary
                },
                [10.0; 4],
                1.0,
                theme.border_color,
                Some(panel_rect),
            );
            b.push_text(
                "net-cancel-lbl",
                z,
                RectPx::new(cancel_r.x, cancel_r.y + 7.5, cancel_r.width, 20.0),
                "Cancel",
                FontRole::Ui,
                13.0,
                600,
                theme.text_primary,
                TextAlign::Center,
                Some(panel_rect),
            );
            b.push_hit(HitTarget::ModalClose, z, cancel_r, CursorHint::Pointer);

            let play_r = RectPx::new(cx + cw - 88.0, panel_rect.y + 165.6, 88.0, 43.0);
            b.push_solid(
                "net-play-bg",
                z,
                play_r,
                theme.accent,
                [12.0; 4],
                0.0,
                [0.0; 4],
                Some(panel_rect),
            );
            b.push_text(
                "net-play-lbl",
                z,
                RectPx::new(play_r.x, play_r.y + 11.5, play_r.width, 20.0),
                "Play",
                FontRole::Ui,
                13.0,
                700,
                [1.0; 4],
                TextAlign::Center,
                Some(panel_rect),
            );
            b.push_hit(HitTarget::NetworkStreamOpen, z, play_r, CursorHint::Pointer);
        }
        ModalId::BgSettings => {
            let chameleon = ChameleonTokens::resolve(
                state.welcome_background,
                theme,
                state.welcome_quality == QualityTier::Low,
                state.welcome_quality == QualityTier::Medium,
            );
            let divider_col = if is_light {
                theme.border_color
            } else {
                rgba(255, 255, 255, 0.08)
            };
            let body_clip = RectPx::new(
                panel_rect.x,
                panel_rect.y + 89.0,
                panel_rect.width,
                (panel_rect.height - 99.0).max(40.0),
            );
            let scroll_y = state.bg_settings_scroll_y;
            let cx = panel_rect.x + 33.0;
            let cw = panel_rect.width - 68.0;
            let mut cy = body_clip.y + 21.0 - scroll_y;

            // Global Background picker row (`.bg-settings-global`, `settings.css:743-755, 1492-1561`)
            if cy + 36.0 > body_clip.y && cy < body_clip.y + body_clip.height {
                b.push_text(
                    "bg-picker-row-lbl",
                    z,
                    RectPx::new(cx, cy + 7.0, 96.0, 16.0),
                    "BACKGROUND",
                    FontRole::Ui,
                    10.0,
                    700,
                    chameleon.fg_dim,
                    TextAlign::Left,
                    Some(body_clip),
                );
                let picker_w = 170.0_f32.min((cw - 104.0).max(120.0));
                let picker_btn = RectPx::new(cx + cw - picker_w, cy, picker_w, 30.0);
                let picker_hovered = b.is_hovered(HitTarget::BgEffectPickerToggle);
                let picker_bg = if picker_hovered {
                    if is_light {
                        rgba(0, 0, 0, 0.10)
                    } else {
                        rgba(255, 255, 255, 0.14)
                    }
                } else if is_light {
                    rgba(0, 0, 0, 0.05)
                } else {
                    rgba(255, 255, 255, 0.07)
                };
                let (picker_bw, picker_br) = if state.bg_accordion_open {
                    (2.0, chameleon.accent)
                } else if picker_hovered {
                    (
                        1.0,
                        if is_light {
                            theme.border_focus
                        } else {
                            rgba(255, 255, 255, 0.40)
                        },
                    )
                } else {
                    (
                        1.0,
                        if is_light {
                            theme.border_color
                        } else {
                            rgba(255, 255, 255, 0.14)
                        },
                    )
                };
                b.push_solid(
                    "bg-effect-picker-btn",
                    z,
                    picker_btn,
                    picker_bg,
                    [12.0; 4],
                    picker_bw,
                    picker_br,
                    Some(body_clip),
                );
                b.push_text(
                    "bg-picker-btn-lbl",
                    z,
                    RectPx::new(
                        picker_btn.x + 12.0,
                        picker_btn.y + 6.0,
                        picker_btn.width - 36.0,
                        18.0,
                    ),
                    "Change Background",
                    FontRole::Ui,
                    12.0,
                    600,
                    theme.text_primary,
                    TextAlign::Left,
                    Some(body_clip),
                );
                b.push_svg(
                    "bg-picker-chevron",
                    z,
                    SvgId::BgPickerChevron,
                    if state.bg_accordion_open {
                        SvgVariant::ChevronUp
                    } else {
                        SvgVariant::Default
                    },
                    RectPx::new(
                        picker_btn.x + picker_btn.width - 24.0,
                        picker_btn.y + 8.0,
                        14.0,
                        14.0,
                    ),
                    rgba_f32_to_u8(if state.bg_accordion_open {
                        chameleon.accent
                    } else {
                        chameleon.fg_dim
                    }),
                    Some(body_clip),
                );
                b.push_hit(
                    HitTarget::BgEffectPickerToggle,
                    z,
                    picker_btn,
                    CursorHint::Pointer,
                );
            }
            cy += 30.0 + 24.0;
            if cy > body_clip.y && cy < body_clip.y + body_clip.height {
                b.push_solid(
                    "bg-global-divider",
                    z,
                    RectPx::new(cx, cy, cw, 1.0),
                    divider_col,
                    [0.0; 4],
                    0.0,
                    [0.0; 4],
                    Some(body_clip),
                );
            }
            cy += 12.0;

            // Group Header (`.bg-group-head`, `index.html:1200-1356`, `settings.css:1096-1146`)
            let group_name = match state.welcome_background {
                WelcomeBackgroundKind::Dither => "DITHER",
                WelcomeBackgroundKind::Particles => "PARTICLES",
                WelcomeBackgroundKind::Faulty => "FAULTY TERMINAL",
                WelcomeBackgroundKind::Gridmotion => "GRID MOTION",
                WelcomeBackgroundKind::None => "DEFAULT",
            };
            if cy + 44.0 > body_clip.y && cy < body_clip.y + body_clip.height {
                b.push_text(
                    "bg-group-head-tag",
                    z,
                    RectPx::new(cx, cy + 20.0, 180.0, 16.0),
                    group_name,
                    FontRole::Ui,
                    11.0,
                    700,
                    theme.text_primary,
                    TextAlign::Left,
                    Some(body_clip),
                );
                if state.welcome_background != WelcomeBackgroundKind::None {
                    let head_reset = RectPx::new(cx + cw - 54.0, cy + 14.0, 54.0, 28.0);
                    let reset_hov = b.is_hovered(HitTarget::BgResetCurrent);
                    b.push_solid(
                        "bg-group-reset-bg",
                        z,
                        head_reset,
                        if reset_hov {
                            rgba(255, 255, 255, 0.04)
                        } else {
                            [0.0; 4]
                        },
                        [6.0; 4],
                        1.0,
                        if reset_hov {
                            chameleon.accent
                        } else if is_light {
                            theme.border_color
                        } else {
                            rgba(255, 255, 255, 0.14)
                        },
                        Some(body_clip),
                    );
                    b.push_text(
                        "bg-group-reset-lbl",
                        z,
                        RectPx::new(head_reset.x, head_reset.y + 6.0, head_reset.width, 16.0),
                        "RESET",
                        FontRole::Ui,
                        10.0,
                        700,
                        if reset_hov {
                            chameleon.accent
                        } else {
                            chameleon.fg_dim
                        },
                        TextAlign::Center,
                        Some(body_clip),
                    );
                    b.push_hit(
                        HitTarget::BgResetCurrent,
                        z,
                        head_reset,
                        CursorHint::Pointer,
                    );
                }
            }
            if cy + 50.0 > body_clip.y && cy + 50.0 < body_clip.y + body_clip.height {
                b.push_solid(
                    "bg-group-divider",
                    z,
                    RectPx::new(cx, cy + 50.0, cw, 1.0),
                    divider_col,
                    [0.0; 4],
                    0.0,
                    [0.0; 4],
                    Some(body_clip),
                );
            }
            cy += 52.0;

            let font_role = if chameleon.uses_monospace_body {
                FontRole::Mono
            } else {
                FontRole::Ui
            };
            let track_idle_col = if is_light {
                rgba(0, 0, 0, 0.14)
            } else {
                rgba(255, 255, 255, 0.14)
            };
            let subhdr_col = with_alpha(chameleon.fg_dim, chameleon.fg_dim[3] * 0.72);

            let push_subhdr = |b: &mut FrameBuilder, id: &str, cy_ref: &mut f32, title: &str| {
                if *cy_ref + 28.0 > body_clip.y && *cy_ref < body_clip.y + body_clip.height {
                    b.push_text(
                        format!("bg-subhdr-{id}"),
                        z,
                        RectPx::new(cx, *cy_ref + 21.0, cw, 14.0),
                        title,
                        font_role,
                        10.0,
                        700,
                        subhdr_col,
                        TextAlign::Left,
                        Some(body_clip),
                    );
                }
                *cy_ref += 33.0;
            };

            let push_bg_control = |b: &mut FrameBuilder, cy_ref: &mut f32, ctrl_id: &str| {
                let Some(ctrl) = BG_CONTROLS_CATALOG.iter().find(|c| c.id == ctrl_id) else {
                    return;
                };
                let row_h = match ctrl.input_type {
                    BgControlInputType::Color => 49.0,
                    _ => 42.0,
                };
                if *cy_ref + row_h > body_clip.y && *cy_ref < body_clip.y + body_clip.height {
                    let row_lbl = ctrl.label.to_uppercase();
                    match ctrl.input_type {
                        BgControlInputType::Range { min, max, .. } => {
                            let val_f: f32 = match ctrl.id {
                                "bgDitherSpeed" => state.dither_cfg.wave_speed,
                                "bgDitherFrequency" => state.dither_cfg.wave_frequency,
                                "bgDitherAmplitude" => state.dither_cfg.wave_amplitude,
                                "bgDitherPixelSize" => state.dither_cfg.pixel_size,
                                "bgDitherColorNum" => state.dither_cfg.color_num,
                                "bgParticlesCount" => state.particles_cfg.particle_count as f32,
                                "bgParticlesSpeed" => state.particles_cfg.speed,
                                "bgParticlesSpread" => state.particles_cfg.particle_spread,
                                "bgParticlesSize" => state.particles_cfg.particle_base_size,
                                "bgFaultyScale" => state.faulty_cfg.scale,
                                "bgFaultyGlitch" => state.faulty_cfg.glitch_amount,
                                "bgFaultyScanlines" => state.faulty_cfg.scanline_intensity,
                                "bgFaultyFlicker" => state.faulty_cfg.flicker_amount,
                                "bgFaultyAberration" => state.faulty_cfg.chromatic_aberration,
                                "bgFaultyCurvature" => state.faulty_cfg.curvature,
                                "bgFaultyBrightness" => state.faulty_cfg.brightness,
                                "bgGridmotionScale" => state.gridmotion_cfg.scale,
                                "bgGridmotionSpeed" => state.gridmotion_cfg.speed,
                                "bgGridmotionMaxMove" => state.gridmotion_cfg.max_move,
                                _ => ctrl.default_display.parse().unwrap_or(0.0),
                            };
                            let min_f = min.parse::<f32>().unwrap_or(0.0);
                            let max_f = max.parse::<f32>().unwrap_or(1.0);
                            let norm =
                                ((val_f - min_f) / (max_f - min_f).max(1e-4)).clamp(0.0, 1.0);
                            let val_str = if (val_f - val_f.round()).abs() < 0.005 {
                                format!("{}", val_f.round() as i32)
                            } else {
                                let s = format!("{val_f:.2}");
                                if s.ends_with('0') && s.contains('.') {
                                    s[..s.len() - 1].to_string()
                                } else {
                                    s
                                }
                            };
                            b.push_text(
                                format!("bg-ctrl-label-{}", ctrl.id),
                                z,
                                RectPx::new(cx, *cy_ref + 20.0, 108.0, 16.0),
                                row_lbl,
                                font_role,
                                11.0,
                                600,
                                theme.text_primary,
                                TextAlign::Left,
                                Some(body_clip),
                            );
                            let track_x = cx + 118.0;
                            let track_w = (cw - 118.0 - 16.0 - 38.0).max(40.0);
                            let track_y = *cy_ref + 25.0;
                            b.push_solid(
                                format!("bg-ctrl-track-{}", ctrl.id),
                                z,
                                RectPx::new(track_x, track_y, track_w, 6.0),
                                track_idle_col,
                                [3.0; 4],
                                0.0,
                                [0.0; 4],
                                Some(body_clip),
                            );
                            let thumb_off = (track_w - 16.0) * norm;
                            if norm > 0.001 {
                                b.push_solid(
                                    format!("bg-ctrl-fill-{}", ctrl.id),
                                    z,
                                    RectPx::new(track_x, track_y, (8.0 + thumb_off).max(3.0), 6.0),
                                    theme.accent,
                                    [3.0; 4],
                                    0.0,
                                    [0.0; 4],
                                    Some(body_clip),
                                );
                            }
                            b.push_solid(
                                format!("bg-ctrl-thumb-{}", ctrl.id),
                                z,
                                RectPx::new(track_x + thumb_off, track_y - 5.0, 16.0, 16.0),
                                chameleon.accent,
                                [8.0; 4],
                                0.0,
                                [0.0; 4],
                                Some(body_clip),
                            );
                            b.push_text(
                                format!("bg-ctrl-val-{}", ctrl.id),
                                z,
                                RectPx::new(cx + cw - 38.0, *cy_ref + 20.0, 38.0, 16.0),
                                val_str,
                                FontRole::Mono,
                                11.0,
                                500,
                                chameleon.fg_dim,
                                TextAlign::Right,
                                Some(body_clip),
                            );
                            b.push_hit(
                                HitTarget::BgControlInput(ctrl.id.to_string()),
                                z,
                                RectPx::new(track_x, *cy_ref, track_w, row_h),
                                CursorHint::Pointer,
                            );
                        }
                        BgControlInputType::Color => {
                            let rgb = match ctrl.id {
                                "bgDitherColor" => state.dither_cfg.wave_color,
                                "bgDitherBgColor" => state.dither_cfg.bg_color,
                                "bgParticlesColor" => state
                                    .particles_cfg
                                    .particle_colors
                                    .first()
                                    .copied()
                                    .unwrap_or([1.0, 1.0, 1.0]),
                                "bgFaultyTint" => state.faulty_cfg.tint,
                                "bgGridmotionGradientColor" => state.gridmotion_cfg.gradient_color,
                                _ => [0.5, 0.5, 0.5],
                            };
                            let hex_label = format!(
                                "#{:02x}{:02x}{:02x}",
                                (rgb[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                                (rgb[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                                (rgb[2].clamp(0.0, 1.0) * 255.0).round() as u8,
                            );
                            b.push_text(
                                format!("bg-ctrl-label-{}", ctrl.id),
                                z,
                                RectPx::new(cx, *cy_ref + 24.0, cw - 114.0, 16.0),
                                row_lbl,
                                font_role,
                                11.0,
                                600,
                                theme.text_primary,
                                TextAlign::Left,
                                Some(body_clip),
                            );
                            let hex_x = cx + cw - 60.0;
                            let swatch_r =
                                RectPx::new(hex_x - 11.0 - 36.0, *cy_ref + 14.0, 36.0, 36.0);
                            let swatch_hovered =
                                b.is_hovered(HitTarget::BgControlInput(ctrl.id.to_string()));
                            let swatch_border = if swatch_hovered {
                                if is_light {
                                    rgba(0, 0, 0, 0.35)
                                } else {
                                    rgba(255, 255, 255, 0.40)
                                }
                            } else if is_light {
                                rgba(0, 0, 0, 0.16)
                            } else {
                                rgba(255, 255, 255, 0.20)
                            };
                            b.push_solid(
                                format!("bg-ctrl-swatch-{}", ctrl.id),
                                z,
                                swatch_r,
                                [rgb[0], rgb[1], rgb[2], 1.0],
                                [10.0; 4],
                                2.0,
                                swatch_border,
                                Some(body_clip),
                            );
                            b.push_text(
                                format!("bg-ctrl-hex-{}", ctrl.id),
                                z,
                                RectPx::new(hex_x, *cy_ref + 24.0, 60.0, 16.0),
                                hex_label,
                                FontRole::Mono,
                                11.0,
                                500,
                                chameleon.fg_dim,
                                TextAlign::Left,
                                Some(body_clip),
                            );
                            b.push_hit(
                                HitTarget::BgControlInput(ctrl.id.to_string()),
                                z,
                                RectPx::new(swatch_r.x, *cy_ref, 104.0, row_h),
                                CursorHint::Pointer,
                            );
                        }
                        BgControlInputType::Checkbox => {
                            let on = match ctrl.id {
                                "bgParticlesAlpha" => state.particles_cfg.alpha_particles,
                                "bgGridmotionAutoMove" => state.gridmotion_cfg.auto_move,
                                _ => false,
                            };
                            b.push_text(
                                format!("bg-ctrl-label-{}", ctrl.id),
                                z,
                                RectPx::new(cx, *cy_ref + 13.0, cw - 52.0, 16.0),
                                row_lbl,
                                font_role,
                                11.0,
                                600,
                                theme.text_primary,
                                TextAlign::Left,
                                Some(body_clip),
                            );
                            let sw_r = RectPx::new(cx + cw - 38.0, *cy_ref + 10.0, 38.0, 22.0);
                            b.push_solid(
                                format!("bg-ctrl-sw-{}", ctrl.id),
                                z,
                                sw_r,
                                if on { chameleon.accent } else { track_idle_col },
                                [11.0; 4],
                                0.0,
                                [0.0; 4],
                                Some(body_clip),
                            );
                            let kx = if on { sw_r.x + 18.0 } else { sw_r.x + 2.0 };
                            b.push_solid(
                                format!("bg-ctrl-knob-{}", ctrl.id),
                                z,
                                RectPx::new(kx, sw_r.y + 2.0, 18.0, 18.0),
                                if on {
                                    [1.0; 4]
                                } else {
                                    rgba(255, 255, 255, 0.85)
                                },
                                [9.0; 4],
                                0.0,
                                [0.0; 4],
                                Some(body_clip),
                            );
                            b.push_hit(
                                HitTarget::BgControlInput(ctrl.id.to_string()),
                                z,
                                sw_r,
                                CursorHint::Pointer,
                            );
                        }
                        BgControlInputType::Select => {
                            b.push_text(
                                format!("bg-ctrl-label-{}", ctrl.id),
                                z,
                                RectPx::new(cx, *cy_ref + 13.0, 108.0, 16.0),
                                row_lbl,
                                font_role,
                                11.0,
                                600,
                                theme.text_primary,
                                TextAlign::Left,
                                Some(body_clip),
                            );
                            let ratio_lbl = match state.gridmotion_cfg.aspect_ratio {
                                hybrid_shaders::GridAspectRatio::Ratio16x9 => "16:9 Widescreen",
                                hybrid_shaders::GridAspectRatio::Ratio1x1 => "1:1 Square",
                                hybrid_shaders::GridAspectRatio::Ratio3x4 => "3:4 Portrait",
                                hybrid_shaders::GridAspectRatio::Ratio4x3 => "4:3 Standard",
                                hybrid_shaders::GridAspectRatio::Auto => "Auto Fill",
                            };
                            let sel_r = RectPx::new(
                                cx + 116.0,
                                *cy_ref + 3.0,
                                (cw - 116.0).max(96.0),
                                36.0,
                            );
                            b.push_solid(
                                format!("bg-ctrl-sel-{}", ctrl.id),
                                z,
                                sel_r,
                                rgba(8, 12, 26, 0.68),
                                [12.0; 4],
                                1.0,
                                rgba(255, 255, 255, 0.22),
                                Some(body_clip),
                            );
                            b.push_text(
                                format!("bg-ctrl-sel-val-{}", ctrl.id),
                                z,
                                RectPx::new(
                                    sel_r.x + 12.0,
                                    sel_r.y + 9.0,
                                    sel_r.width - 36.0,
                                    18.0,
                                ),
                                ratio_lbl,
                                FontRole::Ui,
                                12.5,
                                600,
                                theme.text_primary,
                                TextAlign::Left,
                                Some(body_clip),
                            );
                            b.push_svg(
                                format!("bg-ctrl-sel-chev-{}", ctrl.id),
                                z,
                                SvgId::BgPickerChevron,
                                SvgVariant::Default,
                                RectPx::new(
                                    sel_r.x + sel_r.width - 24.0,
                                    sel_r.y + 11.0,
                                    14.0,
                                    14.0,
                                ),
                                rgba_f32_to_u8(chameleon.fg_dim),
                                Some(body_clip),
                            );
                            b.push_hit(
                                HitTarget::BgControlInput(ctrl.id.to_string()),
                                z,
                                sel_r,
                                CursorHint::Pointer,
                            );
                        }
                    }
                }
                *cy_ref += row_h;
            };

            match state.welcome_background {
                WelcomeBackgroundKind::Dither => {
                    push_subhdr(b, "dither-motion", &mut cy, "MOTION");
                    push_bg_control(b, &mut cy, "bgDitherSpeed");
                    push_bg_control(b, &mut cy, "bgDitherFrequency");
                    push_bg_control(b, &mut cy, "bgDitherAmplitude");
                    cy += 9.0;
                    push_subhdr(b, "dither-render", &mut cy, "RENDER");
                    push_bg_control(b, &mut cy, "bgDitherColor");
                    push_bg_control(b, &mut cy, "bgDitherBgColor");
                    push_bg_control(b, &mut cy, "bgDitherPixelSize");
                    push_bg_control(b, &mut cy, "bgDitherColorNum");
                }
                WelcomeBackgroundKind::Particles => {
                    push_subhdr(b, "particles-motion", &mut cy, "MOTION");
                    push_bg_control(b, &mut cy, "bgParticlesCount");
                    push_bg_control(b, &mut cy, "bgParticlesSpeed");
                    push_bg_control(b, &mut cy, "bgParticlesSpread");
                    cy += 8.0;
                    push_subhdr(b, "particles-app", &mut cy, "APPEARANCE");
                    push_bg_control(b, &mut cy, "bgParticlesColor");
                    push_bg_control(b, &mut cy, "bgParticlesSize");
                    push_bg_control(b, &mut cy, "bgParticlesAlpha");
                }
                WelcomeBackgroundKind::Faulty => {
                    push_subhdr(b, "faulty-layout", &mut cy, "LAYOUT & GLITCH");
                    push_bg_control(b, &mut cy, "bgFaultyScale");
                    push_bg_control(b, &mut cy, "bgFaultyGlitch");
                    push_bg_control(b, &mut cy, "bgFaultyFlicker");
                    cy += 8.0;
                    push_subhdr(b, "faulty-scan", &mut cy, "SCAN");
                    push_bg_control(b, &mut cy, "bgFaultyScanlines");
                    push_bg_control(b, &mut cy, "bgFaultyAberration");
                    push_bg_control(b, &mut cy, "bgFaultyCurvature");
                    cy += 8.0;
                    push_subhdr(b, "faulty-render", &mut cy, "RENDER");
                    push_bg_control(b, &mut cy, "bgFaultyTint");
                    push_bg_control(b, &mut cy, "bgFaultyBrightness");
                }
                WelcomeBackgroundKind::Gridmotion => {
                    push_subhdr(b, "grid-motion", &mut cy, "MOTION");
                    push_bg_control(b, &mut cy, "bgGridmotionAutoMove");
                    push_bg_control(b, &mut cy, "bgGridmotionScale");
                    push_bg_control(b, &mut cy, "bgGridmotionSpeed");
                    push_bg_control(b, &mut cy, "bgGridmotionMaxMove");
                    cy += 8.0;
                    push_subhdr(b, "grid-render", &mut cy, "RENDER");
                    push_bg_control(b, &mut cy, "bgGridmotionAspectRatio");
                    push_bg_control(b, &mut cy, "bgGridmotionGradientColor");
                    if cy + 42.0 > body_clip.y && cy < body_clip.y + body_clip.height {
                        b.push_text(
                            "bg-grid-pics-lbl",
                            z,
                            RectPx::new(cx, cy + 13.0, 108.0, 16.0),
                            "GRID PICTURES",
                            font_role,
                            11.0,
                            600,
                            theme.text_primary,
                            TextAlign::Left,
                            Some(body_clip),
                        );
                        let has_pics = !state.gridmotion_custom_pictures.is_empty();
                        let add_w = if has_pics {
                            (cw - 124.0) * 0.52
                        } else {
                            cw - 116.0
                        };
                        let add_r = RectPx::new(cx + 116.0, cy + 6.0, add_w, 30.0);
                        b.push_solid(
                            "bg-grid-add-bg",
                            z,
                            add_r,
                            pill_idle_bg,
                            [7.0; 4],
                            1.0,
                            pill_idle_border,
                            Some(body_clip),
                        );
                        b.push_text(
                            "bg-grid-add-lbl",
                            z,
                            RectPx::new(add_r.x, add_r.y + 6.0, add_r.width, 18.0),
                            "Add Images...",
                            FontRole::Ui,
                            11.5,
                            600,
                            theme.text_primary,
                            TextAlign::Center,
                            Some(body_clip),
                        );
                        b.push_hit(
                            HitTarget::BgGridmotionAddImages,
                            z,
                            add_r,
                            CursorHint::Pointer,
                        );

                        if has_pics {
                            let clr_r = RectPx::new(
                                add_r.x + add_r.width + 8.0,
                                cy + 6.0,
                                cw - 116.0 - add_r.width - 8.0,
                                30.0,
                            );
                            b.push_solid(
                                "bg-grid-clr-bg",
                                z,
                                clr_r,
                                pill_idle_bg,
                                [7.0; 4],
                                1.0,
                                pill_idle_border,
                                Some(body_clip),
                            );
                            b.push_text(
                                "bg-grid-clr-lbl",
                                z,
                                RectPx::new(clr_r.x, clr_r.y + 6.0, clr_r.width, 18.0),
                                "Clear All",
                                FontRole::Ui,
                                11.5,
                                600,
                                theme.text_secondary,
                                TextAlign::Center,
                                Some(body_clip),
                            );
                            b.push_hit(
                                HitTarget::BgGridmotionClearImages,
                                z,
                                clr_r,
                                CursorHint::Pointer,
                            );
                        }
                    }
                    cy += 42.0;
                    if !state.gridmotion_custom_pictures.is_empty() {
                        cy += 6.0;
                        let cols = 4usize;
                        let gap = 6.0;
                        let thumb_w = ((cw - gap * ((cols - 1) as f32)) / (cols as f32)).max(24.0);
                        let thumb_h = 44.0;
                        for (idx, _path) in state.gridmotion_custom_pictures.iter().enumerate() {
                            let row = idx / cols;
                            let col = idx % cols;
                            let tx = cx + (col as f32) * (thumb_w + gap);
                            let ty = cy + (row as f32) * (thumb_h + gap);
                            if ty + thumb_h > body_clip.y && ty < body_clip.y + body_clip.height {
                                let tr = RectPx::new(tx, ty, thumb_w, thumb_h);
                                b.push_solid(
                                    format!("bg-grid-thumb-bg-{idx}"),
                                    z,
                                    tr,
                                    pill_idle_bg,
                                    [6.0; 4],
                                    1.0,
                                    pill_idle_border,
                                    Some(body_clip),
                                );
                                b.image_slots.push(UiImageSlot {
                                    id: format!("bg-grid-thumb-img-{idx}"),
                                    z_index: z,
                                    rect: RectPx::new(
                                        tr.x + 1.0,
                                        tr.y + 1.0,
                                        (tr.width - 2.0).max(4.0),
                                        (tr.height - 2.0).max(4.0),
                                    ),
                                    corner_radius: 5.0,
                                    opacity: 1.0,
                                    key: format!("bg-thumb:{idx}"),
                                    clip_rect: Some(body_clip),
                                });
                                let rm_r =
                                    RectPx::new(tr.x + tr.width - 18.0, tr.y + 2.0, 16.0, 16.0);
                                b.push_solid(
                                    format!("bg-grid-thumb-rm-bg-{idx}"),
                                    z + 1,
                                    rm_r,
                                    rgba(10, 12, 18, 0.78),
                                    [8.0; 4],
                                    0.0,
                                    [0.0; 4],
                                    Some(body_clip),
                                );
                                b.push_svg(
                                    format!("bg-grid-thumb-rm-{idx}"),
                                    z + 1,
                                    SvgId::ModalCloseSubtitle,
                                    SvgVariant::Default,
                                    RectPx::new(rm_r.x + 3.5, rm_r.y + 3.5, 9.0, 9.0),
                                    [255; 4],
                                    Some(body_clip),
                                );
                                b.push_hit(
                                    HitTarget::BgGridmotionRemoveImage(idx),
                                    z + 1,
                                    rm_r,
                                    CursorHint::Pointer,
                                );
                            }
                        }
                    }
                }
                WelcomeBackgroundKind::None => {
                    if cy + 36.0 > body_clip.y && cy < body_clip.y + body_clip.height {
                        b.push_text(
                            "bg-none-hint",
                            z,
                            RectPx::new(cx, cy + 18.0, cw, 24.0),
                            "No effect applied — pure black backdrop.",
                            font_role,
                            12.0,
                            400,
                            chameleon.fg_dim,
                            TextAlign::Center,
                            Some(body_clip),
                        );
                    }
                }
            }

            // `#bgAccordionPopover` (`z: 2100`, horizontal expanding flex cards 1.0 / 2.0 / 3.8 + top-right × + 22px red check circle)
            if state.bg_accordion_open
                || (state.anim.bg_accordion_closing && state.anim.bg_accordion_t < 1.0)
            {
                let acc_cp = b.checkpoint();
                let right_pad = (if bp.max_w_950 { 16.0 } else { 20.0 }) + playlist_w;
                let dpr = if state.dpr.is_finite() && state.dpr > 0.0 {
                    state.dpr
                } else {
                    1.0
                };
                let snap = |v: f32| -> f32 { (v * dpr).round() / dpr };
                let border_1px = (1.0 * dpr).floor().max(1.0) / dpr;
                let (pop_x_raw, pop_w_raw, pop_bottom, gallery_h) = if bp.max_w_950 {
                    let avail = (w - right_pad - panel_rect.width - 28.0).max(240.0);
                    (16.0, avail, 72.0, (h - 210.0).clamp(120.0, 200.0))
                } else {
                    let pw = (w - 410.0).min(w - 32.0).clamp(260.0, 860.0);
                    let px = (w - right_pad - panel_rect.width - 14.0 - pw).max(16.0);
                    (px, pw, 80.0, 240.0)
                };
                let pop_x = snap(pop_x_raw);
                let pop_w = snap(pop_w_raw);
                let pop_h = 16.0 + 45.6 + 14.0 + gallery_h + 22.0;
                let pop_rect = RectPx::new(
                    pop_x,
                    snap((h - pop_bottom - pop_h).max(48.0)),
                    pop_w,
                    pop_h,
                );
                b.push_glass(
                    "bg-accordion-popover",
                    ZIndex::BG_ACCORDION_POPOVER,
                    pop_rect,
                    RootTokens::RADIUS_LG,
                    Some(CssBackdropRule::BgAccordionPopover),
                    ChameleonSkin::None,
                    theme,
                    None,
                );
                if let Some(last) = b.glass_panels.last_mut() {
                    if let Some((bg, border, _)) = theme.bg_panel_light_override() {
                        last.glass.fill_color_0 = bg;
                        last.glass.fill_color_1 = bg;
                        last.glass.border_color_0 = border;
                        last.glass.border_color_1 = border;
                        last.glass.border_color_2 = border;
                        last.glass.inset_highlight = [0.0; 4];
                    }
                    last.glass.apply_backdrop_dim(backdrop_alpha);
                    last.glass.shadow_color[3] *= 0.5;
                    last.glass.border_width = border_1px;
                }
                b.push_hit(
                    HitTarget::BgAccordionSurface,
                    ZIndex::BG_ACCORDION_POPOVER,
                    pop_rect,
                    CursorHint::Default,
                );
                let content_left = pop_rect.x + border_1px + snap(18.0);
                b.push_text(
                    "bg-pop-title",
                    ZIndex::BG_ACCORDION_POPOVER,
                    RectPx::new(content_left, pop_rect.y + 18.0, pop_rect.width - 70.0, 20.0),
                    "Background Effects",
                    FontRole::Display,
                    14.0,
                    700,
                    theme.text_primary,
                    TextAlign::Left,
                    Some(pop_rect),
                );
                b.push_text(
                    "bg-pop-sub",
                    ZIndex::BG_ACCORDION_POPOVER,
                    RectPx::new(content_left, pop_rect.y + 40.5, pop_rect.width - 70.0, 16.0),
                    "Select an effect to customize parameters",
                    FontRole::Ui,
                    11.0,
                    400,
                    chameleon.fg_dim,
                    TextAlign::Left,
                    Some(pop_rect),
                );
                b.push_solid(
                    "bg-pop-divider",
                    ZIndex::BG_ACCORDION_POPOVER,
                    RectPx::new(
                        content_left,
                        pop_rect.y + 64.0,
                        pop_rect.width - 36.0,
                        border_1px,
                    ),
                    divider_col,
                    [0.0; 4],
                    0.0,
                    [0.0; 4],
                    Some(pop_rect),
                );
                let pop_close = RectPx::new(
                    pop_rect.x + pop_rect.width - 44.0,
                    pop_rect.y + 21.0,
                    26.0,
                    26.0,
                );
                let pop_close_hov = b.is_hovered(HitTarget::BgEffectPickerToggle);
                b.push_svg(
                    "bg-pop-close-svg",
                    ZIndex::BG_ACCORDION_POPOVER,
                    SvgId::ModalCloseBgSettings,
                    SvgVariant::Default,
                    RectPx::new(pop_close.x + 7.0, pop_close.y + 7.0, 12.0, 12.0),
                    rgba_f32_to_u8(if pop_close_hov {
                        theme.text_primary
                    } else {
                        chameleon.fg_dim
                    }),
                    Some(pop_rect),
                );
                b.push_hit(
                    HitTarget::BgEffectPickerToggle,
                    ZIndex::BG_ACCORDION_POPOVER + 1,
                    pop_close,
                    CursorHint::Pointer,
                );

                let kinds = [
                    (
                        WelcomeBackgroundKind::Dither,
                        SvgId::AccordionCheckDither,
                        "Dither Waves",
                        "Animated bayer dither",
                    ),
                    (
                        WelcomeBackgroundKind::Particles,
                        SvgId::AccordionCheckParticles,
                        "Particles",
                        "Drifting particle field",
                    ),
                    (
                        WelcomeBackgroundKind::Faulty,
                        SvgId::AccordionCheckFaulty,
                        "Faulty Terminal",
                        "CRT glitch & scanlines",
                    ),
                    (
                        WelcomeBackgroundKind::Gridmotion,
                        SvgId::AccordionCheckGridmotion,
                        "Grid Motion",
                        "Gliding item lattice",
                    ),
                    (
                        WelcomeBackgroundKind::None,
                        SvgId::AccordionCheckNone,
                        "Default",
                        "Pure black backdrop",
                    ),
                ];
                let mut target_weights = [1.0_f32; 5];
                let mut target_reveals = [0.0_f32; 5];
                for (i, (kind, _, _, _)) in kinds.iter().enumerate() {
                    let is_sel = *kind == state.welcome_background;
                    let is_hov = b.is_hovered(HitTarget::BgAccordionCard(*kind));
                    target_weights[i] = if is_sel {
                        3.8
                    } else if is_hov {
                        2.0
                    } else {
                        1.0
                    };
                    target_reveals[i] = if is_sel || is_hov { 1.0 } else { 0.0 };
                }
                let weights = if state.anim.accordion_weights_initialized {
                    state.anim.accordion_card_weights
                } else {
                    target_weights
                };
                let reveals = if state.anim.accordion_weights_initialized {
                    state.anim.accordion_card_reveal
                } else {
                    target_reveals
                };
                let weight_sum: f32 = weights.iter().sum();
                let gap = 8.0;
                let avail_w = (pop_rect.width - 38.0 - gap * 4.0).max(120.0);
                let content_avail = (avail_w - 2.0 * kinds.len() as f32).max(20.0);
                let card_h = snap((gallery_h - 8.0).max(80.0));
                let card_y = snap(pop_rect.y + 83.2);
                let mut cur_x = content_left;
                for (idx, ((kind, check_svg, name, desc), flex_w)) in
                    kinds.into_iter().zip(weights).enumerate()
                {
                    let raw_card_w = (content_avail * (flex_w / weight_sum) + 2.0).max(24.0);
                    let card_start = snap(cur_x);
                    let card_end = snap(cur_x + raw_card_w);
                    let card_w = (card_end - card_start).max(24.0);
                    let card_rect = RectPx::new(card_start, card_y, card_w, card_h);
                    cur_x += raw_card_w + gap;
                    let is_sel = kind == state.welcome_background;
                    let is_hov = b.is_hovered(HitTarget::BgAccordionCard(kind));
                    let reveal = reveals[idx].clamp(0.0, 1.0);
                    if is_sel {
                        let mut drop_shadow = GlassRect::new(
                            card_rect.x,
                            card_rect.y,
                            card_rect.width,
                            card_rect.height,
                            12.0,
                            theme.mode,
                        );
                        drop_shadow.blur_px = 0.0;
                        drop_shadow.saturation = 1.0;
                        drop_shadow.fill_color_0 = [0.0; 4];
                        drop_shadow.fill_color_1 = [0.0; 4];
                        drop_shadow.border_width = 0.0;
                        drop_shadow.inset_highlight = [0.0; 4];
                        drop_shadow.shadow_offset = [0.0, 10.0];
                        drop_shadow.shadow_blur = 28.0;
                        drop_shadow.shadow_spread = 0.0;
                        drop_shadow.shadow_color = rgba(0, 0, 0, 0.30);
                        b.push_custom_glass(
                            format!("bg-card-drop-{kind:?}"),
                            ZIndex::BG_ACCORDION_POPOVER,
                            drop_shadow,
                            None,
                            Some(pop_rect),
                        );

                        let mut glow = GlassRect::new(
                            card_rect.x,
                            card_rect.y,
                            card_rect.width,
                            card_rect.height,
                            12.0,
                            theme.mode,
                        );
                        glow.blur_px = 0.0;
                        glow.saturation = 1.0;
                        glow.fill_color_0 = [0.0; 4];
                        glow.fill_color_1 = [0.0; 4];
                        glow.border_width = 0.0;
                        glow.inset_highlight = [0.0; 4];
                        glow.shadow_offset = [0.0, 0.0];
                        glow.shadow_blur = 16.0;
                        glow.shadow_spread = 0.0;
                        glow.shadow_color = with_alpha(theme.accent, 0.14);
                        b.push_custom_glass(
                            format!("bg-card-glow-{kind:?}"),
                            ZIndex::BG_ACCORDION_POPOVER,
                            glow,
                            None,
                            Some(pop_rect),
                        );
                    }
                    let card_border = if is_sel {
                        theme.accent
                    } else if is_hov {
                        rgba(255, 255, 255, 0.45)
                    } else {
                        rgba(255, 255, 255, 0.14)
                    };
                    b.push_solid(
                        format!("bg-card-frame-{kind:?}"),
                        ZIndex::BG_ACCORDION_POPOVER,
                        card_rect,
                        rgba8(0x05, 0x05, 0x08, 255),
                        [12.0; 4],
                        border_1px,
                        card_border,
                        Some(pop_rect),
                    );
                    if kind != WelcomeBackgroundKind::None {
                        // Procedural CSS preview gradient underneath live GPU canvas (`settings.css:1767-1827`)
                        let (g0, g1, angle) = match kind {
                            WelcomeBackgroundKind::Dither => {
                                (rgba(26, 26, 46, 1.0), rgba(15, 52, 96, 1.0), 135.0)
                            }
                            WelcomeBackgroundKind::Particles => {
                                (rgba(13, 17, 23, 1.0), rgba(30, 58, 95, 1.0), 160.0)
                            }
                            WelcomeBackgroundKind::Faulty => {
                                (rgba(7, 24, 15, 1.0), rgba(16, 62, 34, 1.0), 135.0)
                            }
                            WelcomeBackgroundKind::Gridmotion => {
                                (rgba(45, 22, 28, 1.0), rgba(14, 14, 24, 1.0), 135.0)
                            }
                            WelcomeBackgroundKind::None => {
                                (rgba(5, 5, 8, 1.0), rgba(5, 5, 8, 1.0), 135.0)
                            }
                        };
                        b.push_gradient(
                            format!("bg-accordion-preview-{kind:?}"),
                            ZIndex::BG_ACCORDION_POPOVER,
                            RectPx::new(
                                card_rect.x + border_1px,
                                card_rect.y + border_1px,
                                (card_rect.width - 2.0 * border_1px).max(1.0),
                                (card_rect.height - 2.0 * border_1px).max(1.0),
                            ),
                            g0,
                            g1,
                            angle,
                            [11.0; 4],
                            0.0,
                            [0.0; 4],
                            Some(pop_rect),
                        );
                        let inner_clip = RectPx::new(
                            card_rect.x + border_1px,
                            card_rect.y + border_1px,
                            (card_rect.width - 2.0 * border_1px).max(1.0),
                            (card_rect.height - 2.0 * border_1px).max(1.0),
                        );
                        let preview_scale = 1.0 + 0.04 * reveal;
                        let pw = 400.0 * preview_scale;
                        let ph = 240.0 * preview_scale;
                        b.shader_slots.push(UiShaderSlot {
                            id: format!("bg-preview-{kind:?}"),
                            z_index: ZIndex::BG_ACCORDION_POPOVER + 1,
                            kind,
                            rect: RectPx::new(
                                snap(card_rect.x + (card_rect.width - pw) * 0.5),
                                snap(
                                    card_rect.y
                                        + border_1px
                                        + ((card_rect.height - 2.0 * border_1px) - ph) * 0.5,
                                ),
                                pw,
                                ph,
                            ),
                            is_preview: true,
                            opacity: 0.85 + 0.15 * reveal,
                            clip_rect: Some(inner_clip),
                        });
                        b.push_solid(
                            format!("bg-card-border-{kind:?}"),
                            ZIndex::BG_ACCORDION_POPOVER + 2,
                            card_rect,
                            [0.0; 4],
                            [12.0; 4],
                            border_1px,
                            card_border,
                            Some(pop_rect),
                        );
                    }
                    if reveal > 0.01 {
                        let slide_y = (1.0 - reveal) * 8.0;
                        let grad_rect = RectPx::new(
                            card_rect.x + border_1px,
                            card_rect.y + card_rect.height - 72.0,
                            card_rect.width - 2.0 * border_1px,
                            72.0 - border_1px,
                        );
                        b.push_gradient(
                            format!("bg-card-grad-{kind:?}"),
                            ZIndex::BG_ACCORDION_POPOVER + 1,
                            grad_rect,
                            rgba(0, 0, 0, 0.94 * reveal),
                            rgba(0, 0, 0, 0.0),
                            0.0,
                            [0.0, 0.0, 11.0, 11.0],
                            0.0,
                            [0.0; 4],
                            Some(card_rect),
                        );
                        b.push_text(
                            format!("bg-card-name-{kind:?}"),
                            ZIndex::BG_ACCORDION_POPOVER + 1,
                            RectPx::new(
                                card_rect.x + 14.0,
                                card_rect.y + card_rect.height - 50.0 + slide_y,
                                (card_w - 28.0).max(16.0),
                                18.0,
                            ),
                            name,
                            FontRole::Display,
                            13.0,
                            700,
                            [1.0, 1.0, 1.0, reveal],
                            TextAlign::Left,
                            Some(card_rect),
                        );
                        b.push_text(
                            format!("bg-card-desc-{kind:?}"),
                            ZIndex::BG_ACCORDION_POPOVER + 1,
                            RectPx::new(
                                card_rect.x + 14.0,
                                card_rect.y + card_rect.height - 27.6 + slide_y,
                                (card_w - 28.0).max(16.0),
                                16.0,
                            ),
                            desc,
                            FontRole::Ui,
                            11.0,
                            400,
                            rgba(255, 255, 255, 0.82 * reveal),
                            TextAlign::Left,
                            Some(card_rect),
                        );
                    }
                    if is_sel && reveal > 0.01 {
                        let check_badge = RectPx::new(
                            card_rect.x + card_rect.width - 30.0,
                            card_rect.y + 8.0,
                            22.0,
                            22.0,
                        );
                        b.push_solid(
                            format!("bg-check-badge-{kind:?}"),
                            ZIndex::BG_ACCORDION_POPOVER + 1,
                            check_badge,
                            with_alpha(theme.accent, reveal),
                            [11.0; 4],
                            0.0,
                            [0.0; 4],
                            Some(card_rect),
                        );
                        b.push_svg(
                            format!("bg-check-{kind:?}"),
                            ZIndex::BG_ACCORDION_POPOVER + 1,
                            check_svg,
                            SvgVariant::Default,
                            RectPx::new(check_badge.x + 4.0, check_badge.y + 4.0, 14.0, 14.0),
                            [
                                255,
                                255,
                                255,
                                (255.0 * reveal).round().clamp(0.0, 255.0) as u8,
                            ],
                            Some(card_rect),
                        );
                    }
                    b.push_hit(
                        HitTarget::BgAccordionCard(kind),
                        ZIndex::BG_ACCORDION_POPOVER + 1,
                        card_rect,
                        CursorHint::Pointer,
                    );
                }
                if state.anim.bg_accordion_t < 0.999 || state.anim.bg_accordion_closing {
                    let t_acc = if state.anim.bg_accordion_closing {
                        1.0 - MotionProfile::ease_standard(state.anim.bg_accordion_t)
                    } else {
                        MotionProfile::ease_emphasized(state.anim.bg_accordion_t)
                    };
                    let kf = evaluate_keyframe(
                        KeyframeId::BgAccordionPopoverIn,
                        t_acc,
                        state.motion_profile,
                    );
                    b.apply_subtree_anim(
                        acc_cp,
                        pop_rect.x + pop_rect.width,
                        pop_rect.y + pop_rect.height * 0.5,
                        kf,
                        false,
                    );
                    if state.anim.bg_accordion_closing {
                        b.hit_regions.truncate(acc_cp.hit);
                    }
                }
            }
        }
    }
    if state.anim.modal_closing {
        let eased = MotionProfile::ease_standard(state.anim.modal_t);
        let kf = evaluate_keyframe(KeyframeId::ModalSlideOut, eased, state.motion_profile);
        b.apply_subtree_anim(
            modal_panel_cp,
            panel_rect.x + panel_rect.width * 0.5,
            panel_rect.y + panel_rect.height * 0.5,
            kf,
            false,
        );
        b.hit_regions.truncate(pre_modal_hit_cp);
    } else if state.anim.modal_t < 0.999 {
        let eased = MotionProfile::ease_emphasized(state.anim.modal_t);
        let kf = evaluate_keyframe(KeyframeId::ModalSlideIn, eased, state.motion_profile);
        b.apply_subtree_anim(
            modal_panel_cp,
            panel_rect.x + panel_rect.width * 0.5,
            panel_rect.y + panel_rect.height * 0.5,
            kf,
            false,
        );
    }
}

fn eq_preset_label(preset: EqualizerPreset) -> &'static str {
    match preset {
        EqualizerPreset::Flat => "Flat",
        EqualizerPreset::BassBoost => "Bass Boost",
        EqualizerPreset::TrebleBoost => "Treble Boost",
        EqualizerPreset::Vocal => "Vocal",
        EqualizerPreset::Rock => "Rock",
        EqualizerPreset::Pop => "Pop",
        EqualizerPreset::Jazz => "Jazz",
        EqualizerPreset::Classical => "Classical",
        EqualizerPreset::Electronic => "Electronic",
        EqualizerPreset::Custom => "Custom",
    }
}

fn hsv_pure_hue(h_deg: f32) -> [f32; 4] {
    let h = h_deg.rem_euclid(360.0) / 60.0;
    let x = 1.0 - (h.rem_euclid(2.0) - 1.0).abs();
    let (r, g, b) = if h < 1.0 {
        (1.0, x, 0.0)
    } else if h < 2.0 {
        (x, 1.0, 0.0)
    } else if h < 3.0 {
        (0.0, 1.0, x)
    } else if h < 4.0 {
        (0.0, x, 1.0)
    } else if h < 5.0 {
        (x, 0.0, 1.0)
    } else {
        (1.0, 0.0, x)
    };
    [r, g, b, 1.0]
}

fn rgb_to_sv_local(rgb: [f32; 3]) -> (f32, f32) {
    let max = rgb[0].max(rgb[1]).max(rgb[2]).clamp(0.0, 1.0);
    let min = rgb[0].min(rgb[1]).min(rgb[2]).clamp(0.0, 1.0);
    let s = if max < 1e-6 { 0.0 } else { (max - min) / max };
    (s, max)
}

fn build_floating_popovers(
    b: &mut FrameBuilder,
    state: &UiState,
    theme: &ThemeTokens,
    w: f32,
    h: f32,
) {
    if state.active_dropdown.is_none() && state.active_color_picker.is_none() {
        return;
    }

    let z = ZIndex::BG_ACCORDION_POPOVER + 50;
    let is_light = state.theme == ThemeMode::Light;
    fn find_trigger(regions: &[HitRegion], tgt: &HitTarget) -> Option<RectPx> {
        regions
            .iter()
            .rev()
            .find(|r| &r.target == tgt)
            .map(|r| r.rect)
    }

    // Transparent full-screen dismissal backdrop beneath floating popover
    b.push_hit(
        HitTarget::DropdownBackdrop,
        z - 1,
        RectPx::new(0.0, 0.0, w, h),
        CursorHint::Default,
    );

    let pop_bg = match state.theme {
        ThemeMode::Oled => rgba8(0x0a, 0x0a, 0x0e, 248),
        ThemeMode::Light => rgba(255, 255, 255, 0.98),
        ThemeMode::Dark => rgba8(0x16, 0x18, 0x22, 248),
    };
    let pop_br = if is_light {
        rgba(0, 0, 0, 0.14)
    } else {
        rgba(255, 255, 255, 0.14)
    };

    if let Some(kind) = state.active_dropdown {
        let (trigger_target, items, selected_idx): (HitTarget, Vec<String>, usize) = match kind {
            DropdownKind::SettingsTheme => (
                HitTarget::SettingsThemeSelect,
                vec!["Dark".into(), "OLED Black".into(), "Light".into()],
                match state.theme {
                    ThemeMode::Dark => 0,
                    ThemeMode::Oled => 1,
                    ThemeMode::Light => 2,
                },
            ),
            DropdownKind::SettingsMotion => (
                HitTarget::SettingsMotionSelect,
                vec!["Reduced".into(), "Balanced".into(), "Showcase".into()],
                match state.motion_profile {
                    MotionProfile::Reduced => 0,
                    MotionProfile::Balanced => 1,
                    MotionProfile::Showcase => 2,
                },
            ),
            DropdownKind::SettingsSeekStep => (
                HitTarget::SettingsSeekStepSelect,
                vec![
                    "3 seconds".into(),
                    "5 seconds (Default)".into(),
                    "10 seconds".into(),
                ],
                match state.seek_step_secs {
                    3 => 0,
                    10 => 2,
                    _ => 1,
                },
            ),
            DropdownKind::SettingsScreenshotFormat => (
                HitTarget::SettingsScreenshotFormatSelect,
                vec![
                    "JPEG (.jpg)".into(),
                    "PNG (.png)".into(),
                    "WebP (.webp)".into(),
                ],
                match state.screenshot_format.to_ascii_lowercase().as_str() {
                    "png" => 1,
                    "webp" => 2,
                    _ => 0,
                },
            ),
            DropdownKind::SettingsMouseScreenshot => (
                HitTarget::SettingsMouseScreenshotSelect,
                vec![
                    "Mouse 4 (Back Button)".into(),
                    "Mouse 5 (Forward Button)".into(),
                    "Middle Click".into(),
                    "Off".into(),
                ],
                match state.screenshot_mouse_button {
                    4 => 1,
                    1 => 2,
                    -1 => 3,
                    _ => 0,
                },
            ),
            DropdownKind::EqPreset => {
                let presets: Vec<EqualizerPreset> = EqualizerPreset::BUILTIN_PRESETS
                    .iter()
                    .copied()
                    .chain(std::iter::once(EqualizerPreset::Custom))
                    .collect();
                (
                    HitTarget::EqPresetSelect,
                    presets
                        .iter()
                        .map(|p| eq_preset_label(*p).to_string())
                        .collect(),
                    presets
                        .iter()
                        .position(|p| *p == state.equalizer.preset)
                        .unwrap_or(0),
                )
            }
            DropdownKind::YoutubeQuality => {
                let labels: Vec<String> = state
                    .youtube_quality_options
                    .iter()
                    .map(|q| {
                        if q.eq_ignore_ascii_case("auto") {
                            "Auto".to_string()
                        } else if q.eq_ignore_ascii_case("audio") {
                            "Audio".to_string()
                        } else {
                            q.clone()
                        }
                    })
                    .collect();
                let sel = labels
                    .iter()
                    .position(|l| l.eq_ignore_ascii_case(&state.youtube_quality_label))
                    .unwrap_or(0);
                (HitTarget::YoutubeQualityButton, labels, sel)
            }
            DropdownKind::BgGridmotionAspectRatio => (
                HitTarget::BgControlInput("bgGridmotionAspectRatio".to_string()),
                vec![
                    "16:9 (Widescreen)".into(),
                    "1:1 (Square)".into(),
                    "3:4 (Portrait)".into(),
                    "4:3 (Classic)".into(),
                    "Auto".into(),
                ],
                match state.gridmotion_cfg.aspect_ratio {
                    GridAspectRatio::Ratio16x9 => 0,
                    GridAspectRatio::Ratio1x1 => 1,
                    GridAspectRatio::Ratio3x4 => 2,
                    GridAspectRatio::Ratio4x3 => 3,
                    GridAspectRatio::Auto => 4,
                },
            ),
        };

        if !items.is_empty() {
            let trig = find_trigger(&b.hit_regions, &trigger_target)
                .unwrap_or_else(|| RectPx::new((w - 160.0) * 0.5, (h - 140.0) * 0.5, 160.0, 32.0));
            let max_lbl_w = items
                .iter()
                .map(|s| measure_text_width(s, FontRole::Ui, 12.0, 500, 0.0, state.font_config))
                .fold(0.0_f32, f32::max);
            let menu_w = trig.width.max(max_lbl_w + 40.0).min((w - 16.0).max(120.0));
            let row_h = 28.0_f32;
            let menu_h = 8.0 + (items.len() as f32) * row_h;
            let mx = trig.x.clamp(8.0, (w - menu_w - 8.0).max(8.0));
            let below_y = trig.y + trig.height + 4.0;
            let above_y = trig.y - menu_h - 4.0;
            let my = if below_y + menu_h <= h - 8.0 {
                below_y
            } else if above_y >= 8.0 {
                above_y
            } else {
                (h - menu_h - 8.0).max(8.0)
            };
            let menu_rect = RectPx::new(mx, my, menu_w, menu_h);

            b.push_solid(
                "dropdown-menu-shadow",
                z,
                RectPx::new(
                    menu_rect.x - 1.0,
                    menu_rect.y + 4.0,
                    menu_rect.width + 2.0,
                    menu_rect.height + 6.0,
                ),
                rgba(0, 0, 0, 0.36),
                [11.0; 4],
                0.0,
                [0.0; 4],
                None,
            );
            b.push_solid(
                "dropdown-menu-bg",
                z,
                menu_rect,
                pop_bg,
                [10.0; 4],
                1.0,
                pop_br,
                None,
            );
            b.push_hit(HitTarget::PopoverSurface, z, menu_rect, CursorHint::Default);

            for (idx, lbl) in items.iter().enumerate() {
                let r = RectPx::new(
                    menu_rect.x + 4.0,
                    menu_rect.y + 4.0 + (idx as f32) * row_h,
                    menu_w - 8.0,
                    row_h,
                );
                let is_sel = idx == selected_idx;
                let is_hov = b.is_hovered(HitTarget::DropdownOption(idx));
                if is_sel || is_hov {
                    b.push_solid(
                        format!("dropdown-opt-bg-{idx}"),
                        z,
                        r,
                        if is_sel {
                            theme.accent_dim
                        } else if is_light {
                            rgba(0, 0, 0, 0.06)
                        } else {
                            rgba(255, 255, 255, 0.08)
                        },
                        [7.0; 4],
                        0.0,
                        [0.0; 4],
                        Some(menu_rect),
                    );
                }
                b.push_text(
                    format!("dropdown-opt-lbl-{idx}"),
                    z,
                    RectPx::new(r.x + 10.0, r.y + 5.0, r.width - 28.0, 18.0),
                    lbl.clone(),
                    FontRole::Ui,
                    12.0,
                    if is_sel { 600 } else { 500 },
                    if is_sel {
                        theme.accent
                    } else {
                        theme.text_primary
                    },
                    TextAlign::Left,
                    Some(menu_rect),
                );
                if is_sel {
                    b.push_solid(
                        format!("dropdown-opt-dot-{idx}"),
                        z,
                        RectPx::new(r.x + r.width - 14.0, r.y + 11.0, 6.0, 6.0),
                        theme.accent,
                        [3.0; 4],
                        0.0,
                        [0.0; 4],
                        Some(menu_rect),
                    );
                }
                b.push_hit(HitTarget::DropdownOption(idx), z, r, CursorHint::Pointer);
            }
        }
    }

    if let Some(cp_target) = state.active_color_picker.as_ref() {
        let (trigger_target, rgb): (HitTarget, [f32; 3]) = match cp_target {
            ColorPickerTarget::Accent => (
                HitTarget::SettingsAccentPicker,
                parse_hex_rgb(&state.accent_hex).unwrap_or([0.231, 0.510, 0.965]),
            ),
            ColorPickerTarget::SubtitleFontColor => (
                HitTarget::SubtitleFontColorPicker,
                parse_hex_rgb(&state.subtitle_font_color).unwrap_or([1.0, 1.0, 1.0]),
            ),
            ColorPickerTarget::SubtitleBgColor => (
                HitTarget::SubtitleBgColorPicker,
                parse_hex_rgb(&state.subtitle_bg_color).unwrap_or([0.0, 0.0, 0.0]),
            ),
            ColorPickerTarget::BgControl(id) => {
                let c = match id.as_str() {
                    "bgDitherColor" => state.dither_cfg.wave_color,
                    "bgDitherBgColor" => state.dither_cfg.bg_color,
                    "bgParticlesColor" => state
                        .particles_cfg
                        .particle_colors
                        .first()
                        .copied()
                        .unwrap_or([1.0, 1.0, 1.0]),
                    "bgFaultyTint" => state.faulty_cfg.tint,
                    "bgGridmotionGradientColor" => state.gridmotion_cfg.gradient_color,
                    _ => [1.0, 1.0, 1.0],
                };
                (HitTarget::BgControlInput(id.clone()), c)
            }
        };

        let trig = find_trigger(&b.hit_regions, &trigger_target)
            .unwrap_or_else(|| RectPx::new((w - 236.0) * 0.5, (h - 274.0) * 0.5, 28.0, 28.0));
        let pw = 236.0_f32;
        let ph = 276.0_f32;
        let px = (trig.x + trig.width - pw).clamp(8.0, (w - pw - 8.0).max(8.0));
        let below_y = trig.y + trig.height + 6.0;
        let above_y = trig.y - ph - 6.0;
        let py = if below_y + ph <= h - 8.0 {
            below_y
        } else if above_y >= 40.0 {
            above_y
        } else {
            (h - ph - 8.0).max(40.0)
        };
        let pop = RectPx::new(px, py, pw, ph);

        b.push_solid(
            "cp-shadow",
            z,
            RectPx::new(pop.x - 1.0, pop.y + 4.0, pop.width + 2.0, pop.height + 6.0),
            rgba(0, 0, 0, 0.40),
            [13.0; 4],
            0.0,
            [0.0; 4],
            None,
        );
        b.push_solid("cp-bg", z, pop, pop_bg, [12.0; 4], 1.0, pop_br, None);
        b.push_hit(HitTarget::PopoverSurface, z, pop, CursorHint::Default);

        // Header: live preview swatch + hex + Done
        let r8 = (rgb[0].clamp(0.0, 1.0) * 255.0).round() as u8;
        let g8 = (rgb[1].clamp(0.0, 1.0) * 255.0).round() as u8;
        let b8 = (rgb[2].clamp(0.0, 1.0) * 255.0).round() as u8;
        let hex = format!("#{:02X}{:02X}{:02X}", r8, g8, b8);
        b.push_solid(
            "cp-preview",
            z,
            RectPx::new(pop.x + 12.0, pop.y + 11.0, 22.0, 22.0),
            [rgb[0], rgb[1], rgb[2], 1.0],
            [6.0; 4],
            1.0,
            pop_br,
            Some(pop),
        );
        b.push_text(
            "cp-hex",
            z,
            RectPx::new(pop.x + 42.0, pop.y + 13.0, 96.0, 18.0),
            hex,
            FontRole::Mono,
            12.0,
            600,
            theme.text_primary,
            TextAlign::Left,
            Some(pop),
        );
        let done_r = RectPx::new(pop.x + pop.width - 58.0, pop.y + 10.0, 46.0, 24.0);
        b.push_solid(
            "cp-done-bg",
            z,
            done_r,
            if is_light {
                rgba(0, 0, 0, 0.06)
            } else {
                rgba(255, 255, 255, 0.08)
            },
            [6.0; 4],
            1.0,
            pop_br,
            Some(pop),
        );
        b.push_text(
            "cp-done-lbl",
            z,
            RectPx::new(done_r.x, done_r.y + 4.0, done_r.width, 16.0),
            "Done",
            FontRole::Ui,
            11.0,
            600,
            theme.text_primary,
            TextAlign::Center,
            Some(pop),
        );
        b.push_hit(HitTarget::ColorPickerClose, z, done_r, CursorHint::Pointer);

        // 2D SV Spectrum Box
        let spec = RectPx::new(pop.x + 12.0, pop.y + 42.0, pop.width - 24.0, 102.0);
        let pure = hsv_pure_hue(state.color_picker_hue);
        b.push_gradient(
            "cp-sv-horiz",
            z,
            spec,
            [1.0, 1.0, 1.0, 1.0],
            pure,
            90.0,
            [7.0; 4],
            0.0,
            [0.0; 4],
            Some(pop),
        );
        b.push_gradient(
            "cp-sv-vert",
            z,
            spec,
            [0.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
            180.0,
            [7.0; 4],
            1.0,
            pop_br,
            Some(pop),
        );
        let (cur_s, cur_v) = rgb_to_sv_local(rgb);
        let cx = spec.x + cur_s * spec.width;
        let cy = spec.y + (1.0 - cur_v) * spec.height;
        b.push_solid(
            "cp-sv-cursor",
            z,
            RectPx::new(cx - 5.0, cy - 5.0, 10.0, 10.0),
            [rgb[0], rgb[1], rgb[2], 1.0],
            [5.0; 4],
            2.0,
            [1.0; 4],
            Some(spec),
        );
        b.push_hit(HitTarget::ColorPickerSpectrum, z, spec, CursorHint::Pointer);

        // Hue rainbow bar (6 stops)
        let hue_r = RectPx::new(pop.x + 12.0, pop.y + 154.0, pop.width - 24.0, 12.0);
        const HUE_STOPS: [[f32; 4]; 7] = [
            [1.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, 1.0, 1.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            [1.0, 0.0, 1.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
        ];
        let seg_w = hue_r.width / 6.0;
        for i in 0..6 {
            let radii = match i {
                0 => [6.0, 0.0, 0.0, 6.0],
                5 => [0.0, 6.0, 6.0, 0.0],
                _ => [0.0; 4],
            };
            b.push_gradient(
                format!("cp-hue-seg-{i}"),
                z,
                RectPx::new(
                    hue_r.x + (i as f32) * seg_w,
                    hue_r.y,
                    seg_w + 0.5,
                    hue_r.height,
                ),
                HUE_STOPS[i],
                HUE_STOPS[i + 1],
                90.0,
                radii,
                0.0,
                [0.0; 4],
                Some(hue_r),
            );
        }
        let hue_norm = (state.color_picker_hue / 360.0).clamp(0.0, 1.0);
        b.push_solid(
            "cp-hue-thumb",
            z,
            RectPx::new(
                hue_r.x + (hue_r.width - 10.0) * hue_norm,
                hue_r.y - 1.0,
                10.0,
                14.0,
            ),
            [1.0; 4],
            [5.0; 4],
            1.0,
            rgba(0, 0, 0, 0.45),
            Some(pop),
        );
        b.push_hit(
            HitTarget::ColorPickerChannel(0),
            z,
            hue_r,
            CursorHint::Pointer,
        );

        // R, G, B channel sliders
        let ch_specs = [
            (1u8, "R", rgb[0], r8, [0.95, 0.25, 0.25, 1.0]),
            (2u8, "G", rgb[1], g8, [0.22, 0.85, 0.42, 1.0]),
            (3u8, "B", rgb[2], b8, [0.25, 0.55, 0.98, 1.0]),
        ];
        for (idx, (ch_id, ch_lbl, norm, byte_val, ch_col)) in ch_specs.into_iter().enumerate() {
            let ry = pop.y + 174.0 + (idx as f32) * 22.0;
            b.push_text(
                format!("cp-ch-lbl-{ch_id}"),
                z,
                RectPx::new(pop.x + 12.0, ry + 1.0, 14.0, 14.0),
                ch_lbl,
                FontRole::Mono,
                10.5,
                700,
                theme.text_secondary,
                TextAlign::Left,
                Some(pop),
            );
            let tr = RectPx::new(pop.x + 28.0, ry + 1.0, pop.width - 72.0, 14.0);
            b.push_solid(
                format!("cp-ch-track-{ch_id}"),
                z,
                RectPx::new(tr.x, tr.y + 5.0, tr.width, 4.0),
                if is_light {
                    rgba(0, 0, 0, 0.14)
                } else {
                    rgba(255, 255, 255, 0.16)
                },
                [2.0; 4],
                0.0,
                [0.0; 4],
                Some(pop),
            );
            b.push_solid(
                format!("cp-ch-fill-{ch_id}"),
                z,
                RectPx::new(
                    tr.x,
                    tr.y + 5.0,
                    (tr.width * norm.clamp(0.0, 1.0)).max(2.0),
                    4.0,
                ),
                ch_col,
                [2.0; 4],
                0.0,
                [0.0; 4],
                Some(pop),
            );
            b.push_solid(
                format!("cp-ch-thumb-{ch_id}"),
                z,
                RectPx::new(
                    tr.x + (tr.width - 10.0) * norm.clamp(0.0, 1.0),
                    tr.y + 2.0,
                    10.0,
                    10.0,
                ),
                [1.0; 4],
                [5.0; 4],
                1.0,
                rgba(0, 0, 0, 0.45),
                Some(pop),
            );
            b.push_hit(
                HitTarget::ColorPickerChannel(ch_id),
                z,
                tr,
                CursorHint::Pointer,
            );
            b.push_text(
                format!("cp-ch-val-{ch_id}"),
                z,
                RectPx::new(pop.x + pop.width - 40.0, ry + 1.0, 28.0, 14.0),
                format!("{byte_val}"),
                FontRole::Mono,
                10.5,
                500,
                theme.text_secondary,
                TextAlign::Right,
                Some(pop),
            );
        }

        // Preset Swatches row (8 swatches)
        let sw_y = pop.y + 244.0;
        let sw_size = 20.0_f32;
        let sw_gap = ((pop.width - 24.0 - 8.0 * sw_size) / 7.0).max(2.0);
        for (i, swatch_hex) in COLOR_PICKER_SWATCHES.iter().enumerate() {
            let sr = RectPx::new(
                pop.x + 12.0 + (i as f32) * (sw_size + sw_gap),
                sw_y,
                sw_size,
                sw_size,
            );
            let sc = parse_hex_rgb(swatch_hex).unwrap_or([1.0, 1.0, 1.0]);
            let is_cur = (sc[0] - rgb[0]).abs() < 0.03
                && (sc[1] - rgb[1]).abs() < 0.03
                && (sc[2] - rgb[2]).abs() < 0.03;
            b.push_solid(
                format!("cp-swatch-{i}"),
                z,
                sr,
                [sc[0], sc[1], sc[2], 1.0],
                [5.0; 4],
                if is_cur { 2.0 } else { 1.0 },
                if is_cur { theme.accent } else { pop_br },
                Some(pop),
            );
            b.push_hit(
                HitTarget::ColorPickerSwatch(i as u8),
                z,
                sr,
                CursorHint::Pointer,
            );
        }
    }
}

/// Computes the exact maximum vertical scroll offset (in DIP) for the Settings modal body.
pub fn settings_scroll_max(state: &UiState) -> f32 {
    let filtered = state.filtered_shortcuts();
    let mut sc_list_h = 0.0_f32;
    let mut first_cat = true;
    for cat in SHORTCUT_CATEGORIES {
        let count = filtered.iter().filter(|(_, c, _, _)| *c == cat).count();
        if count == 0 {
            continue;
        }
        if !first_cat {
            sc_list_h += 12.0;
        }
        first_cat = false;
        sc_list_h += 20.0 + (count as f32) * 38.0 - 4.0;
    }
    let sc_viewport_h = if filtered.is_empty() {
        54.0
    } else {
        sc_list_h.clamp(80.0, 290.0)
    };
    let sc_card_h = 104.0 + sc_viewport_h;
    let total_settings_h = 14.0
        + 173.0
        + 11.0
        + 251.0
        + 11.0
        + 264.0
        + 14.0
        + sc_card_h
        + 14.0
        + 310.0
        + 14.0
        + 250.0
        + 20.0;
    let body_clip_h = (state.viewport_h * 0.88 - 71.0).max(40.0);
    (total_settings_h - body_clip_h).max(0.0)
}

/// Computes the exact maximum vertical scroll offset (in DIP) for the Background Settings modal body.
pub fn bg_settings_scroll_max(state: &UiState) -> f32 {
    let panel_h = (state.viewport_h - 124.0).clamp(220.0, 640.0);
    let body_clip_h = (panel_h - 99.0).max(40.0);
    let content_h = match state.welcome_background {
        WelcomeBackgroundKind::Dither => 33.0 + 3.0 * 42.0 + 9.0 + 33.0 + 2.0 * 49.0 + 2.0 * 42.0,
        WelcomeBackgroundKind::Particles => 33.0 + 3.0 * 42.0 + 8.0 + 33.0 + 49.0 + 2.0 * 42.0,
        WelcomeBackgroundKind::Faulty => {
            33.0 + 3.0 * 42.0 + 8.0 + 33.0 + 3.0 * 42.0 + 8.0 + 33.0 + 49.0 + 42.0
        }
        WelcomeBackgroundKind::Gridmotion => {
            let thumb_rows = state.gridmotion_custom_pictures.len().div_ceil(4);
            let thumbs_h = if thumb_rows > 0 {
                6.0 + (thumb_rows as f32) * 50.0
            } else {
                0.0
            };
            33.0 + 4.0 * 42.0 + 8.0 + 33.0 + 42.0 + 49.0 + 42.0 + thumbs_h
        }
        WelcomeBackgroundKind::None => 36.0,
    };
    let total_h = 21.0 + 54.0 + 12.0 + 52.0 + content_h + 20.0;
    (total_h - body_clip_h).max(0.0)
}

#[allow(clippy::too_many_arguments)]
fn draw_text_input_box(
    b: &mut FrameBuilder,
    state: &UiState,
    theme: &ThemeTokens,
    id_prefix: &str,
    z: i32,
    rect: RectPx,
    bg: [f32; 4],
    border_idle: [f32; 4],
    radius: f32,
    h_pad: f32,
    font_size: f32,
    placeholder: &str,
    field: FocusedTextField,
    hit_target: HitTarget,
    outer_clip: Option<RectPx>,
) {
    let is_focused = state.focused_text_field == Some(field);
    b.push_solid(
        format!("{id_prefix}-box"),
        z,
        rect,
        bg,
        [radius; 4],
        1.0,
        if is_focused {
            theme.accent
        } else {
            border_idle
        },
        outer_clip,
    );

    let inner_x = rect.x + h_pad;
    let inner_w = (rect.width - h_pad * 2.0).max(16.0);
    let inner_clip_raw = RectPx::new(
        inner_x,
        rect.y + 1.0,
        inner_w,
        (rect.height - 2.0).max(10.0),
    );
    let inner_clip = if let Some(oc) = outer_clip {
        let x0 = inner_clip_raw.x.max(oc.x);
        let y0 = inner_clip_raw.y.max(oc.y);
        let x1 = (inner_clip_raw.x + inner_clip_raw.width).min(oc.x + oc.width);
        let y1 = (inner_clip_raw.y + inner_clip_raw.height).min(oc.y + oc.height);
        if x1 > x0 && y1 > y0 {
            RectPx::new(x0, y0, x1 - x0, y1 - y0)
        } else {
            RectPx::new(x0, y0, 0.0, 0.0)
        }
    } else {
        inner_clip_raw
    };

    let (val, cur) = state.text_field_parts(field);
    let text_y = rect.y + (rect.height - 20.0) * 0.5;
    let mut caret_x = inner_x;

    if val.is_empty() {
        b.push_text(
            format!("{id_prefix}-text"),
            z,
            RectPx::new(inner_x, text_y, inner_w, 20.0),
            placeholder.to_string(),
            FontRole::Ui,
            font_size,
            400,
            theme.text_tertiary,
            TextAlign::Left,
            Some(inner_clip),
        );
    } else {
        let mut cursor = cur.cursor.min(val.len());
        while cursor > 0 && !val.is_char_boundary(cursor) {
            cursor -= 1;
        }
        let text_w = measure_text_width(val, FontRole::Ui, font_size, 400, 0.0, state.font_config);
        let caret_rel_w = measure_text_width(
            &val[..cursor],
            FontRole::Ui,
            font_size,
            400,
            0.0,
            state.font_config,
        );
        let scroll_x = if text_w <= inner_w {
            0.0
        } else {
            cur.scroll_x.clamp(
                (caret_rel_w - (inner_w - 3.0)).max(0.0),
                caret_rel_w.min((text_w - inner_w + 3.0).max(0.0)),
            )
        };

        if is_focused {
            if let Some(mut anc) = cur.anchor {
                anc = anc.min(val.len());
                while anc > 0 && !val.is_char_boundary(anc) {
                    anc -= 1;
                }
                if anc != cursor {
                    let (lo, hi) = if anc < cursor {
                        (anc, cursor)
                    } else {
                        (cursor, anc)
                    };
                    let sel_x0 = measure_text_width(
                        &val[..lo],
                        FontRole::Ui,
                        font_size,
                        400,
                        0.0,
                        state.font_config,
                    );
                    let sel_x1 = measure_text_width(
                        &val[..hi],
                        FontRole::Ui,
                        font_size,
                        400,
                        0.0,
                        state.font_config,
                    );
                    b.push_solid(
                        format!("{id_prefix}-selection"),
                        z,
                        RectPx::new(
                            inner_x + sel_x0 - scroll_x,
                            rect.y + 6.0,
                            (sel_x1 - sel_x0).max(2.0),
                            (rect.height - 12.0).max(12.0),
                        ),
                        [theme.accent[0], theme.accent[1], theme.accent[2], 0.32],
                        [2.0; 4],
                        0.0,
                        [0.0; 4],
                        Some(inner_clip),
                    );
                }
            }
        }

        b.push_text(
            format!("{id_prefix}-text"),
            z,
            RectPx::new(inner_x - scroll_x, text_y, text_w.max(inner_w) + 8.0, 20.0),
            val.to_string(),
            FontRole::Ui,
            font_size,
            400,
            theme.text_primary,
            TextAlign::Left,
            Some(inner_clip),
        );
        caret_x = (inner_x + caret_rel_w - scroll_x).clamp(inner_x, inner_x + inner_w - 1.5);
    }

    if is_focused {
        let caret_h = (rect.height - 16.0).clamp(14.0, 20.0);
        b.push_solid(
            format!("{id_prefix}-caret"),
            z,
            RectPx::new(
                caret_x,
                rect.y + (rect.height - caret_h) * 0.5,
                1.5,
                caret_h,
            ),
            theme.text_primary,
            [0.0; 4],
            0.0,
            [0.0; 4],
            Some(inner_clip),
        );
    }

    b.push_hit(hit_target, z, rect, CursorHint::Text);
}

fn shortcut_suffix(state: &UiState, action_id: &str) -> String {
    let raw = state
        .custom_shortcuts
        .get(action_id)
        .map(|s| s.as_str())
        .unwrap_or_else(|| ShortcutManager::get_default_binding(action_id));
    let compact = format_combo_compact(raw);
    if compact.is_empty() {
        String::new()
    } else {
        format!(" ({compact})")
    }
}

pub fn target_has_tooltip(state: &UiState, target: &HitTarget) -> bool {
    tooltip_text_for_target(state, target).is_some()
}

fn tooltip_text_for_target(state: &UiState, target: &HitTarget) -> Option<String> {
    match target {
        HitTarget::TitlebarMinimize => Some("Minimize".to_string()),
        HitTarget::TitlebarMaximize => Some(match state.window_maximize_state {
            WindowMaximizeState::Restored => "Maximize (F11)".to_string(),
            WindowMaximizeState::Maximized | WindowMaximizeState::Fullscreen => {
                "Restore (F11)".to_string()
            }
        }),
        HitTarget::TitlebarClose => Some("Close".to_string()),
        HitTarget::PlayPause => Some(format!(
            "{}{}",
            if state.playing { "Pause" } else { "Play" },
            shortcut_suffix(state, "toggle-play")
        )),
        HitTarget::PrevTrack => Some(format!(
            "Previous{}",
            shortcut_suffix(state, "previous-track")
        )),
        HitTarget::NextTrack => Some(format!("Next{}", shortcut_suffix(state, "next-track"))),
        HitTarget::MuteToggle => Some(format!("Volume{}", shortcut_suffix(state, "toggle-mute"))),
        HitTarget::BgSettingsToggle => Some(format!(
            "Background Settings{}",
            shortcut_suffix(state, "toggle-bg-settings")
        )),
        HitTarget::SpeedButton => Some("Playback Speed".to_string()),
        HitTarget::EqualizerButton => Some(format!(
            "Equalizer{}",
            shortcut_suffix(state, "toggle-equalizer")
        )),
        HitTarget::AudioTrackButton => Some(format!(
            "Subs / Dubs{}",
            shortcut_suffix(state, "cycle-audio")
        )),
        HitTarget::SubtitlesButton => Some(format!(
            "Subtitles{}",
            shortcut_suffix(state, "toggle-subtitles")
        )),
        HitTarget::PlaylistToggle => Some(format!(
            "Playlist{}",
            shortcut_suffix(state, "toggle-playlist")
        )),
        HitTarget::RepeatToggle => {
            let lbl = match state.repeat {
                RepeatMode::None => "Repeat Off",
                RepeatMode::All => "Repeat All",
                RepeatMode::One => "Repeat One",
            };
            Some(format!("{lbl}{}", shortcut_suffix(state, "toggle-repeat")))
        }
        HitTarget::ShuffleToggle => Some(if state.shuffle {
            "Shuffle On".to_string()
        } else {
            "Shuffle Off".to_string()
        }),
        HitTarget::AbLoopToggle => Some(format!(
            "A-B Loop{}",
            shortcut_suffix(state, "toggle-ab-loop")
        )),
        HitTarget::LockToggle => Some(format!(
            "Lock Controls{}",
            shortcut_suffix(state, "toggle-lock")
        )),
        HitTarget::UnlockControls => Some(format!("Unlock{}", shortcut_suffix(state, "unlock-ui"))),
        HitTarget::YoutubeQualityButton => Some("Quality".to_string()),
        HitTarget::SettingsButton => Some(format!(
            "Settings{}",
            shortcut_suffix(state, "toggle-settings")
        )),
        HitTarget::FullscreenToggle => Some(format!(
            "Fullscreen{}",
            shortcut_suffix(state, "toggle-fullscreen")
        )),
        HitTarget::PlaylistClose => Some("Collapse Playlist".to_string()),
        HitTarget::PlaylistAddFiles => Some("Add Files".to_string()),
        HitTarget::PlaylistShuffle => Some(if state.shuffle {
            "Shuffle On".to_string()
        } else {
            "Shuffle".to_string()
        }),
        HitTarget::PlaylistRepeat => Some(match state.repeat {
            RepeatMode::None => "Repeat Off".to_string(),
            RepeatMode::All => "Repeat All".to_string(),
            RepeatMode::One => "Repeat One".to_string(),
        }),
        HitTarget::PlaylistClear => Some("Clear Playlist".to_string()),
        HitTarget::RecordingStopClip => Some(format!(
            "Stop Recording{}",
            shortcut_suffix(state, "toggle-recording")
        )),
        HitTarget::ChapterMarker(idx) => {
            let t_secs = state.chapter_times_secs.get(*idx).copied().or_else(|| {
                state
                    .chapter_positions_norm
                    .get(*idx)
                    .map(|norm| (*norm as f64) * state.duration_secs.max(0.0))
            });
            t_secs.map(|sec| {
                let title = state
                    .chapter_titles
                    .get(*idx)
                    .filter(|s| !s.trim().is_empty())
                    .cloned()
                    .unwrap_or_else(|| format!("Chapter {}", idx + 1));
                format!("{title} - {}", UiState::format_time(sec))
            })
        }
        _ => None,
    }
}

fn build_focus_ring_and_tooltip(
    b: &mut FrameBuilder,
    state: &UiState,
    theme: &ThemeTokens,
    w: f32,
    h: f32,
) {
    if state.focus_visible {
        if let Some(target) = &state.focused_target {
            if !matches!(
                target,
                HitTarget::ModalClose
                    | HitTarget::ModalBackdrop
                    | HitTarget::DropdownBackdrop
                    | HitTarget::ModalPanelSurface
                    | HitTarget::PopoverSurface
                    | HitTarget::BgAccordionSurface
                    | HitTarget::ControlsBarSurface
                    | HitTarget::PlaylistSurface
                    | HitTarget::WelcomeContentSurface
                    | HitTarget::VolumeContainerSurface
                    | HitTarget::ShortcutScrollArea
                    | HitTarget::PlaylistSearch
                    | HitTarget::NetworkStreamInput
                    | HitTarget::ShortcutSearch
            ) {
                if let Some(reg) = b.hit_regions.iter().rev().find(|r| &r.target == target) {
                    let is_1px_style = matches!(
                        target,
                        HitTarget::SettingsThemeSelect
                            | HitTarget::SettingsMotionSelect
                            | HitTarget::SettingsSeekStepSelect
                            | HitTarget::SettingsScreenshotFormatSelect
                            | HitTarget::SettingsMouseScreenshotSelect
                            | HitTarget::EqPresetSelect
                            | HitTarget::SettingsScreenshotDirPick
                            | HitTarget::SettingsScreenshotDirOpen
                            | HitTarget::SettingsScreenshotDirReset
                            | HitTarget::ShortcutResetAll
                            | HitTarget::ShortcutResetOne(_)
                            | HitTarget::ShortcutRebind(_)
                            | HitTarget::EqBandSlider(_)
                            | HitTarget::SpeedSlider
                            | HitTarget::SubDelaySlider
                            | HitTarget::SubtitleFontSizeSlider
                            | HitTarget::SubtitleBgOpacitySlider
                            | HitTarget::VolumeSlider
                            | HitTarget::ProgressSeek
                    );
                    let (ring_rect, stroke_w, radius) = if is_1px_style {
                        (reg.rect, 1.0, 8.0)
                    } else {
                        (
                            RectPx::new(
                                reg.rect.x - 2.0,
                                reg.rect.y - 2.0,
                                reg.rect.width + 4.0,
                                reg.rect.height + 4.0,
                            ),
                            2.0,
                            10.0,
                        )
                    };
                    b.push_solid(
                        "focus-visible-ring",
                        reg.z_index + 1,
                        ring_rect,
                        [0.0; 4],
                        [radius; 4],
                        stroke_w,
                        theme.accent,
                        None,
                    );
                }
            }
        }
    }

    if state.hover_tooltip_elapsed_secs >= HOVER_TOOLTIP_DELAY_SECS {
        if let Some(target) = &state.hovered_target {
            if let Some(label) = tooltip_text_for_target(state, target) {
                if let Some(reg) = b.hit_regions.iter().rev().find(|r| &r.target == target) {
                    let z = ZIndex::BG_ACCORDION_POPOVER + 30;
                    let text_w =
                        measure_text_width(&label, FontRole::Ui, 11.5, 500, 0.0, state.font_config);
                    let tip_w = (text_w + 16.0).clamp(42.0, 280.0);
                    let tip_h = 24.0;
                    let tx = (reg.rect.x + (reg.rect.width - tip_w) * 0.5)
                        .clamp(8.0, (w - tip_w - 8.0).max(8.0));
                    let above_y = reg.rect.y - tip_h - 6.0;
                    let ty = if above_y >= 8.0 {
                        above_y
                    } else {
                        (reg.rect.y + reg.rect.height + 6.0).min((h - tip_h - 8.0).max(8.0))
                    };
                    let tip_rect = RectPx::new(tx, ty, tip_w, tip_h);
                    let is_light = state.theme == ThemeMode::Light;
                    b.push_solid(
                        "hover-tooltip-bg",
                        z,
                        tip_rect,
                        if is_light {
                            rgba8(0xf4, 0xf6, 0xfa, 245)
                        } else {
                            rgba8(0x14, 0x16, 0x20, 245)
                        },
                        [6.0; 4],
                        1.0,
                        if is_light {
                            rgba(0, 0, 0, 0.14)
                        } else {
                            rgba(255, 255, 255, 0.14)
                        },
                        None,
                    );
                    b.push_text(
                        "hover-tooltip-text",
                        z,
                        RectPx::new(tip_rect.x + 8.0, tip_rect.y + 4.0, tip_w - 16.0, 16.0),
                        label,
                        FontRole::Ui,
                        11.5,
                        500,
                        theme.text_primary,
                        TextAlign::Center,
                        Some(tip_rect),
                    );
                }
            }
        }
    }
}
