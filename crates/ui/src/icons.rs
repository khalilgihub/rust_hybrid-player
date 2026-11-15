//! SVG icon registry and `resvg` rasterizer covering all 64 main UI SVGs
//! and 3 companion auth SVGs.

use std::collections::HashMap;
use std::sync::Arc;

use resvg::{tiny_skia, usvg};

use crate::fonts::{IBM_PLEX_SANS_REGULAR_TTF, IBM_PLEX_SANS_SEMIBOLD_TTF, SORA_BOLD_TTF};
use crate::svg_catalog::SvgId;

/// Variant overrides used by stateful SVG elements (`data-repeat="one"`, `#btnABLoop` badge).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SvgVariant {
    #[default]
    Default,
    RepeatOne,
    AbLoopA,
    AbLoopAb,
    ChevronUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CacheKey {
    id: SvgId,
    variant: SvgVariant,
    width: u32,
    height: u32,
    tint_rgba: [u8; 4],
}

/// Rasterized RGBA8 pixel buffer ready for GPU atlas upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterizedSvg {
    pub width: u32,
    pub height: u32,
    /// Straight/premultiplied RGBA8 pixel bytes (`width * height * 4`).
    pub rgba: Vec<u8>,
}

/// Caches parsed `usvg::Tree` / rasterized `tiny_skia::Pixmap` results for all 67 SVGs.
pub struct SvgRasterizer {
    fontdb: Arc<usvg::fontdb::Database>,
    cache: HashMap<CacheKey, Arc<RasterizedSvg>>,
}

impl Default for SvgRasterizer {
    fn default() -> Self {
        Self::new()
    }
}

impl SvgRasterizer {
    pub fn new() -> Self {
        let mut db = usvg::fontdb::Database::new();
        db.load_font_data(SORA_BOLD_TTF.to_vec());
        db.load_font_data(IBM_PLEX_SANS_REGULAR_TTF.to_vec());
        db.load_font_data(IBM_PLEX_SANS_SEMIBOLD_TTF.to_vec());
        db.set_sans_serif_family("IBM Plex Sans");
        db.set_serif_family("IBM Plex Sans");
        db.set_monospace_family("IBM Plex Sans");
        Self {
            fontdb: Arc::new(db),
            cache: HashMap::new(),
        }
    }

    /// Produces resolved SVG XML with `currentColor` substituted and stateful classes toggled.
    pub fn resolve_svg_markup(id: SvgId, variant: SvgVariant, tint_rgba: [u8; 4]) -> String {
        let hex = format!(
            "#{:02x}{:02x}{:02x}",
            tint_rgba[0], tint_rgba[1], tint_rgba[2]
        );
        let mut xml = id.raw_svg().replace("currentColor", &hex);

        match (id, variant) {
            (SvgId::CtrlRepeat | SvgId::PlaylistRepeat, SvgVariant::RepeatOne) => {
                xml = xml
                    .replace(
                        "class=\"repeat-infinity-path\"",
                        "class=\"repeat-infinity-path\" style=\"display:none;\"",
                    )
                    .replace(
                        "class=\"repeat-one-path\"",
                        "class=\"repeat-one-path\" data-active=\"1\"",
                    )
                    .replace("data-active=\"1\" d=\"M 231.0", "d=\"M 231.0");
                // Strip the trailing style="display:none;" on repeat-one-path cleanly:
                if let Some(pos) = xml.rfind("style=\"display:none;\"") {
                    xml.replace_range(pos..pos + "style=\"display:none;\"".len(), "");
                }
            }
            (SvgId::CtrlAbLoop, SvgVariant::AbLoopA) => {
                xml = xml
                    .replace("style=\"display:none;\"", "")
                    .replace(
                        "font-family=\"system-ui, -apple-system, sans-serif\"",
                        "font-family=\"Sora, IBM Plex Sans, sans-serif\"",
                    )
                    .replace("font-weight=\"900\"", "font-weight=\"700\"")
                    .replace("></text>", ">A</text>");
            }
            (SvgId::CtrlAbLoop, SvgVariant::AbLoopAb) => {
                xml = xml
                    .replace("style=\"display:none;\"", "")
                    .replace(
                        "font-family=\"system-ui, -apple-system, sans-serif\"",
                        "font-family=\"Sora, IBM Plex Sans, sans-serif\"",
                    )
                    .replace("font-weight=\"900\"", "font-weight=\"700\"")
                    .replace("></text>", ">AB</text>");
            }
            (SvgId::BgPickerChevron, SvgVariant::ChevronUp) => {
                xml = xml.replace("6 9 12 15 18 9", "6 15 12 9 18 15");
            }
            _ => {}
        }

        xml
    }

    /// Parses an SVG from the catalog into a `usvg::Tree`.
    pub fn parse_tree(
        &self,
        id: SvgId,
        variant: SvgVariant,
        tint_rgba: [u8; 4],
    ) -> Result<usvg::Tree, usvg::Error> {
        let xml = Self::resolve_svg_markup(id, variant, tint_rgba);
        let opts = usvg::Options {
            fontdb: Arc::clone(&self.fontdb),
            ..Default::default()
        };
        usvg::Tree::from_str(&xml, &opts)
    }

    /// Rasterizes `id` at exact pixel dimensions `(width, height)` with caching.
    pub fn rasterize(
        &mut self,
        id: SvgId,
        variant: SvgVariant,
        width: u32,
        height: u32,
        tint_rgba: [u8; 4],
    ) -> Result<Arc<RasterizedSvg>, usvg::Error> {
        let width = width.max(1);
        let height = height.max(1);
        let key = CacheKey {
            id,
            variant,
            width,
            height,
            tint_rgba,
        };
        if let Some(hit) = self.cache.get(&key) {
            return Ok(Arc::clone(hit));
        }

        let tree = self.parse_tree(id, variant, tint_rgba)?;
        let size = tree.size();

        let rgba = if id == SvgId::BrandMarkTitlebar && width <= 64 && height <= 64 {
            // Supersample 4x to avoid tiny_skia under-filling subpixel dots in the 508x400 viewBox
            let scale = 4u32;
            let ss_w = width * scale;
            let ss_h = height * scale;
            let sx = ss_w as f32 / size.width().max(1.0);
            let sy = ss_h as f32 / size.height().max(1.0);
            let transform = tiny_skia::Transform::from_scale(sx, sy);
            let mut ss_pix = tiny_skia::Pixmap::new(ss_w, ss_h).expect("non-zero pixmap size");
            resvg::render(&tree, transform, &mut ss_pix.as_mut());
            let ss_data = ss_pix.data();

            let mut out = vec![0u8; (width * height * 4) as usize];
            for y in 0..height {
                for x in 0..width {
                    let mut sum_a = 0u32;
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let idx = (((y * scale + dy) * ss_w + (x * scale + dx)) * 4) as usize;
                            sum_a += ss_data[idx + 3] as u32;
                        }
                    }
                    let a_norm = (sum_a as f32) / ((scale * scale * 255) as f32);
                    // Boost subpixel dot coverage so the 20x20 brand mark stays crisp & vivid
                    let a_boost = if a_norm > 0.01 {
                        a_norm.powf(0.55).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let final_a = a_boost * (tint_rgba[3] as f32 / 255.0);
                    let dst = ((y * width + x) * 4) as usize;
                    out[dst] = (tint_rgba[0] as f32 * final_a).round() as u8;
                    out[dst + 1] = (tint_rgba[1] as f32 * final_a).round() as u8;
                    out[dst + 2] = (tint_rgba[2] as f32 * final_a).round() as u8;
                    out[dst + 3] = (final_a * 255.0).round() as u8;
                }
            }
            out
        } else {
            let sx = width as f32 / size.width().max(1.0);
            let sy = height as f32 / size.height().max(1.0);
            let transform = tiny_skia::Transform::from_scale(sx, sy);

            let mut pixmap = tiny_skia::Pixmap::new(width, height).expect("non-zero pixmap size");
            resvg::render(&tree, transform, &mut pixmap.as_mut());

            // Apply uniform alpha scaling if tint_rgba[3] < 255
            let mut rgba = pixmap.take();
            if tint_rgba[3] < 255 {
                let factor = tint_rgba[3] as u16;
                for chunk in rgba.chunks_exact_mut(4) {
                    chunk[0] = ((chunk[0] as u16 * factor) / 255) as u8;
                    chunk[1] = ((chunk[1] as u16 * factor) / 255) as u8;
                    chunk[2] = ((chunk[2] as u16 * factor) / 255) as u8;
                    chunk[3] = ((chunk[3] as u16 * factor) / 255) as u8;
                }
            }
            rgba
        };

        let out = Arc::new(RasterizedSvg {
            width,
            height,
            rgba,
        });
        self.cache.insert(key, Arc::clone(&out));
        Ok(out)
    }

    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }
}
