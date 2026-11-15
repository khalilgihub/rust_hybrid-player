//! Pure-Rust GPU UI layout, text rendering, SVG icons, widgets, and accessibility.

pub mod a11y;
pub mod fonts;
pub mod icons;
pub mod layout;
pub mod motion;
pub mod primitives;
pub mod state;
pub mod svg_catalog;
pub mod tokens;

use fonts::TextEngine;
use icons::SvgRasterizer;
use layout::{build_ui_frame, hit_test_frame};
use primitives::{HitRegion, UiFrameOutput};
use state::UiState;

/// Top-level UI coordinator holding cached `cosmic-text` font system and `resvg` SVG rasterizer.
pub struct HybridUi {
    pub text_engine: TextEngine,
    pub svg_rasterizer: SvgRasterizer,
}

impl Default for HybridUi {
    fn default() -> Self {
        Self::new()
    }
}

impl HybridUi {
    pub fn new() -> Self {
        Self {
            text_engine: TextEngine::new(),
            svg_rasterizer: SvgRasterizer::new(),
        }
    }

    pub fn build_frame(&self, state: &UiState) -> UiFrameOutput {
        build_ui_frame(state)
    }

    pub fn hit_test<'a>(&self, frame: &'a UiFrameOutput, x: f32, y: f32) -> Option<&'a HitRegion> {
        hit_test_frame(frame, x, y)
    }
}
