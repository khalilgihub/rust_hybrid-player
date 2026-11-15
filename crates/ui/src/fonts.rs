//! Embedded brand & UI fonts (`Sora Local`, `IBM Plex Sans Local`) and
//! `cosmic-text` shaping, measurement, and glyph rasterization.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap};

use crate::tokens::FontConfig;

pub const SORA_REGULAR_TTF: &[u8] =
    include_bytes!("../../../hybrid-player/assets/fonts/Sora-Regular.ttf");
pub const SORA_SEMIBOLD_TTF: &[u8] =
    include_bytes!("../../../hybrid-player/assets/fonts/Sora-SemiBold.ttf");
pub const SORA_BOLD_TTF: &[u8] =
    include_bytes!("../../../hybrid-player/assets/fonts/Sora-Bold.ttf");

pub const IBM_PLEX_SANS_REGULAR_TTF: &[u8] =
    include_bytes!("../../../hybrid-player/assets/fonts/IBMPlexSans-Regular.ttf");
pub const IBM_PLEX_SANS_MEDIUM_TTF: &[u8] =
    include_bytes!("../../../hybrid-player/assets/fonts/IBMPlexSans-Medium.ttf");
pub const IBM_PLEX_SANS_SEMIBOLD_TTF: &[u8] =
    include_bytes!("../../../hybrid-player/assets/fonts/IBMPlexSans-SemiBold.ttf");

/// All 6 embedded TrueType font buffers (`themes.css:6-51`).
pub const BUNDLED_FONT_DATA: [(&str, &[u8]); 6] = [
    ("Sora-Regular.ttf", SORA_REGULAR_TTF),
    ("Sora-SemiBold.ttf", SORA_SEMIBOLD_TTF),
    ("Sora-Bold.ttf", SORA_BOLD_TTF),
    ("IBMPlexSans-Regular.ttf", IBM_PLEX_SANS_REGULAR_TTF),
    ("IBMPlexSans-Medium.ttf", IBM_PLEX_SANS_MEDIUM_TTF),
    ("IBMPlexSans-SemiBold.ttf", IBM_PLEX_SANS_SEMIBOLD_TTF),
];

/// Semantic font role matching `--font-display`, `--font-ui`, `--font-mono`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontRole {
    Display,
    Ui,
    Mono,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextMetrics {
    pub width: f32,
    pub height: f32,
    pub line_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RasterizedGlyph {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub is_color: bool,
    pub data: Vec<u8>,
}

/// Pure-Rust text measurement & rasterization engine backed by `cosmic-text`.
pub struct TextEngine {
    font_system: FontSystem,
    swash_cache: SwashCache,
    bundled_face_count: usize,
    has_consolas: bool,
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEngine {
    /// Creates a `FontSystem` with all 6 bundled TTFs loaded into its font database.
    pub fn new() -> Self {
        let mut font_system = FontSystem::new();
        let before = font_system.db().faces().count();
        for (_name, bytes) in BUNDLED_FONT_DATA {
            font_system.db_mut().load_font_data(bytes.to_vec());
        }
        let after = font_system.db().faces().count();
        let has_consolas = font_system.db().faces().any(|face| {
            face.families
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("Consolas"))
        });
        Self {
            font_system,
            swash_cache: SwashCache::new(),
            bundled_face_count: after.saturating_sub(before),
            has_consolas,
        }
    }

    pub fn bundled_face_count(&self) -> usize {
        self.bundled_face_count
    }

    pub fn has_family(&self, family: &str) -> bool {
        let target = family.to_ascii_lowercase();
        self.font_system.db().faces().any(|face| {
            face.families
                .iter()
                .any(|(name, _)| name.to_ascii_lowercase() == target)
        })
    }

    fn resolve_attrs(
        &self,
        role: FontRole,
        weight: u16,
        cfg: FontConfig,
        letter_spacing_em: f32,
    ) -> Attrs<'static> {
        let family = match role {
            FontRole::Display => {
                if cfg.brand_font_enabled {
                    Family::Name("Sora")
                } else {
                    Family::Name("Segoe UI")
                }
            }
            FontRole::Ui => {
                if cfg.brand_font_enabled {
                    Family::Name("IBM Plex Sans")
                } else {
                    Family::Name("Segoe UI")
                }
            }
            FontRole::Mono => {
                if self.has_consolas {
                    Family::Name("Consolas")
                } else {
                    Family::Monospace
                }
            }
        };
        let attrs = Attrs::new().family(family).weight(Weight(weight));
        if letter_spacing_em.abs() > 1e-4 {
            attrs.letter_spacing(letter_spacing_em)
        } else {
            attrs
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn measure_single_line_raw(
        &mut self,
        text: &str,
        font_size: f32,
        line_height: f32,
        role: FontRole,
        weight: u16,
        cfg: FontConfig,
        letter_spacing_px: f32,
    ) -> TextMetrics {
        let metrics = Metrics::new(font_size.max(1.0), line_height.max(1.0));
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let attrs = self.resolve_attrs(role, weight, cfg, letter_spacing_px / font_size.max(1.0));
        let mut borrowed = buffer.borrow_with(&mut self.font_system);
        borrowed.set_wrap(Wrap::None);
        borrowed.set_size(None, None);
        borrowed.set_text(text, &attrs, Shaping::Advanced, None);
        borrowed.shape_until_scroll(true);

        let mut width: f32 = 0.0;
        let mut lines = 0usize;
        for run in borrowed.layout_runs() {
            width = width.max(run.line_w);
            lines += 1;
        }
        let lines = lines.max(1);
        TextMetrics {
            width,
            height: (lines as f32) * line_height.max(1.0),
            line_count: lines,
        }
    }

    /// Truncates a single line of text with `…` (matching CSS `text-overflow: ellipsis`)
    /// so the rendered width does not exceed `max_width`.
    #[allow(clippy::too_many_arguments)]
    pub fn truncate_with_ellipsis(
        &mut self,
        text: &str,
        font_size: f32,
        line_height: f32,
        max_width: f32,
        role: FontRole,
        weight: u16,
        cfg: FontConfig,
        letter_spacing_px: f32,
    ) -> String {
        let clean: std::borrow::Cow<'_, str> = if text.contains(['\n', '\r']) {
            std::borrow::Cow::Owned(text.replace("\r\n", " ").replace(['\n', '\r'], " "))
        } else {
            std::borrow::Cow::Borrowed(text)
        };
        if clean.is_empty() || max_width <= 0.5 {
            return String::new();
        }
        let full = self.measure_single_line_raw(
            clean.as_ref(),
            font_size,
            line_height,
            role,
            weight,
            cfg,
            letter_spacing_px,
        );
        if full.width <= max_width + 0.5 {
            return clean.into_owned();
        }

        let boundaries: Vec<usize> = clean
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(clean.len()))
            .collect();
        let mut lo = 0usize;
        let mut hi = boundaries.len().saturating_sub(1);
        let mut best = 0usize;
        let mut candidate = String::with_capacity(clean.len() + 4);

        while lo <= hi {
            let mid = lo + (hi - lo) / 2;
            let end = boundaries[mid];
            candidate.clear();
            candidate.push_str(&clean[..end]);
            candidate.push('\u{2026}');
            let m = self.measure_single_line_raw(
                &candidate,
                font_size,
                line_height,
                role,
                weight,
                cfg,
                letter_spacing_px,
            );
            if m.width <= max_width + 0.25 {
                best = mid;
                lo = mid + 1;
            } else if mid == 0 {
                break;
            } else {
                hi = mid - 1;
            }
        }

        let mut out = String::with_capacity(boundaries[best] + 4);
        out.push_str(&clean[..boundaries[best]]);
        out.push('\u{2026}');
        out
    }

    /// Measures shaped text bounds for Taffy leaf sizing.
    #[allow(clippy::too_many_arguments)]
    pub fn measure(
        &mut self,
        text: &str,
        font_size: f32,
        line_height: f32,
        max_width: Option<f32>,
        role: FontRole,
        weight: u16,
        cfg: FontConfig,
    ) -> TextMetrics {
        self.measure_with_spacing(
            text,
            font_size,
            line_height,
            max_width,
            role,
            weight,
            cfg,
            0.0,
        )
    }

    /// Measures shaped text bounds with explicit CSS `letter-spacing` in pixels.
    #[allow(clippy::too_many_arguments)]
    pub fn measure_with_spacing(
        &mut self,
        text: &str,
        font_size: f32,
        line_height: f32,
        max_width: Option<f32>,
        role: FontRole,
        weight: u16,
        cfg: FontConfig,
        letter_spacing_px: f32,
    ) -> TextMetrics {
        self.measure_with_options(
            text,
            font_size,
            line_height,
            max_width,
            role,
            weight,
            cfg,
            letter_spacing_px,
            max_width.is_some(),
        )
    }

    /// Measures shaped text bounds with explicit wrapping vs single-line ellipsis mode.
    #[allow(clippy::too_many_arguments)]
    pub fn measure_with_options(
        &mut self,
        text: &str,
        font_size: f32,
        line_height: f32,
        max_width: Option<f32>,
        role: FontRole,
        weight: u16,
        cfg: FontConfig,
        letter_spacing_px: f32,
        allow_wrap: bool,
    ) -> TextMetrics {
        if !allow_wrap {
            if let Some(mw) = max_width {
                let truncated = self.truncate_with_ellipsis(
                    text,
                    font_size,
                    line_height,
                    mw,
                    role,
                    weight,
                    cfg,
                    letter_spacing_px,
                );
                return self.measure_single_line_raw(
                    &truncated,
                    font_size,
                    line_height,
                    role,
                    weight,
                    cfg,
                    letter_spacing_px,
                );
            }
            return self.measure_single_line_raw(
                text,
                font_size,
                line_height,
                role,
                weight,
                cfg,
                letter_spacing_px,
            );
        }

        let metrics = Metrics::new(font_size.max(1.0), line_height.max(1.0));
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let attrs = self.resolve_attrs(role, weight, cfg, letter_spacing_px / font_size.max(1.0));
        let mut borrowed = buffer.borrow_with(&mut self.font_system);
        borrowed.set_wrap(if max_width.is_some() {
            Wrap::WordOrGlyph
        } else {
            Wrap::None
        });
        borrowed.set_size(max_width, None);
        borrowed.set_text(text, &attrs, Shaping::Advanced, None);
        borrowed.shape_until_scroll(true);

        let mut width: f32 = 0.0;
        let mut lines = 0usize;
        for run in borrowed.layout_runs() {
            width = width.max(run.line_w);
            lines += 1;
        }
        let lines = lines.max(1);
        TextMetrics {
            width,
            height: (lines as f32) * line_height.max(1.0),
            line_count: lines,
        }
    }

    /// Shapes and rasterizes all glyphs for a text run into CPU glyph quads.
    #[allow(clippy::too_many_arguments)]
    pub fn rasterize(
        &mut self,
        text: &str,
        font_size: f32,
        line_height: f32,
        max_width: Option<f32>,
        role: FontRole,
        weight: u16,
        color_rgba: [u8; 4],
        cfg: FontConfig,
    ) -> Vec<RasterizedGlyph> {
        self.rasterize_with_spacing(
            text,
            font_size,
            line_height,
            max_width,
            role,
            weight,
            color_rgba,
            cfg,
            0.0,
        )
    }

    /// Shapes and rasterizes glyphs with explicit CSS `letter-spacing` in pixels.
    #[allow(clippy::too_many_arguments)]
    pub fn rasterize_with_spacing(
        &mut self,
        text: &str,
        font_size: f32,
        line_height: f32,
        max_width: Option<f32>,
        role: FontRole,
        weight: u16,
        color_rgba: [u8; 4],
        cfg: FontConfig,
        letter_spacing_px: f32,
    ) -> Vec<RasterizedGlyph> {
        self.rasterize_with_options(
            text,
            font_size,
            line_height,
            max_width,
            role,
            weight,
            color_rgba,
            cfg,
            letter_spacing_px,
            max_width.is_some(),
        )
    }

    /// Shapes and rasterizes glyphs with explicit wrapping vs single-line ellipsis mode.
    #[allow(clippy::too_many_arguments)]
    pub fn rasterize_with_options(
        &mut self,
        text: &str,
        font_size: f32,
        line_height: f32,
        max_width: Option<f32>,
        role: FontRole,
        weight: u16,
        color_rgba: [u8; 4],
        cfg: FontConfig,
        letter_spacing_px: f32,
        allow_wrap: bool,
    ) -> Vec<RasterizedGlyph> {
        let effective_text: std::borrow::Cow<'_, str> = if !allow_wrap {
            if let Some(mw) = max_width {
                std::borrow::Cow::Owned(self.truncate_with_ellipsis(
                    text,
                    font_size,
                    line_height,
                    mw,
                    role,
                    weight,
                    cfg,
                    letter_spacing_px,
                ))
            } else {
                std::borrow::Cow::Borrowed(text)
            }
        } else {
            std::borrow::Cow::Borrowed(text)
        };

        let metrics = Metrics::new(font_size.max(1.0), line_height.max(1.0));
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let attrs = self.resolve_attrs(role, weight, cfg, letter_spacing_px / font_size.max(1.0));
        {
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            borrowed.set_wrap(if allow_wrap && max_width.is_some() {
                Wrap::WordOrGlyph
            } else {
                Wrap::None
            });
            borrowed.set_size(if allow_wrap { max_width } else { None }, None);
            borrowed.set_text(effective_text.as_ref(), &attrs, Shaping::Advanced, None);
            borrowed.shape_until_scroll(true);
        }

        // Perceptual gamma/stem-darkening curve matching DirectWrite / Chromium anti-aliasing
        let luma = (0.2126 * (color_rgba[0] as f32)
            + 0.7152 * (color_rgba[1] as f32)
            + 0.0722 * (color_rgba[2] as f32))
            / 255.0;
        let gamma = if luma >= 0.5 {
            if weight >= 700 && role == FontRole::Ui {
                0.72
            } else {
                0.84
            }
        } else if weight >= 700 && role == FontRole::Ui {
            0.80
        } else {
            0.90
        };

        let mut glyphs = Vec::new();
        for run in buffer.layout_runs() {
            let line_y = run.line_y.round() as i32;
            for glyph in run.glyphs.iter() {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                if let Some(img) = self
                    .swash_cache
                    .get_image_uncached(&mut self.font_system, physical.cache_key)
                {
                    if img.placement.width > 0 && img.placement.height > 0 {
                        let is_color = matches!(img.content, cosmic_text::SwashContent::Color);
                        let data = if is_color {
                            img.data
                        } else {
                            // Convert single-channel alpha mask into tinted RGBA8 with gamma-matched stroke weight
                            let mut rgba = Vec::with_capacity(img.data.len() * 4);
                            for &alpha in &img.data {
                                let shaped = if alpha == 0 || alpha == 255 {
                                    alpha as f32
                                } else {
                                    ((alpha as f32 / 255.0).powf(gamma)) * 255.0
                                };
                                let out_a = ((shaped * (color_rgba[3] as f32 / 255.0))
                                    .round()
                                    .clamp(0.0, 255.0))
                                    as u8;
                                rgba.extend_from_slice(&[
                                    color_rgba[0],
                                    color_rgba[1],
                                    color_rgba[2],
                                    out_a,
                                ]);
                            }
                            rgba
                        };
                        glyphs.push(RasterizedGlyph {
                            x: physical.x + img.placement.left,
                            y: line_y + physical.y - img.placement.top,
                            width: img.placement.width,
                            height: img.placement.height,
                            is_color,
                            data,
                        });
                    }
                }
            }
        }
        glyphs
    }
}
