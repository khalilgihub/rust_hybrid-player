//! Pure-Rust golden image comparison (MAE, RMSE, 8x8 luminance SSIM, per-pixel
//! threshold, ignore rects, and highlighted diff-PNG generation).

use std::fs;
use std::path::{Path, PathBuf};

use image::{ImageBuffer, RgbaImage};
use serde::{Deserialize, Serialize};

use crate::{QaError, Result};

/// Owned 8-bit RGBA image buffer (`width * height * 4` bytes, row-major).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImageBuffer {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl RgbaImageBuffer {
    pub fn from_raw_rgba(width: u32, height: u32, data: Vec<u8>) -> Result<Self> {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|px| px.checked_mul(4))
            .ok_or(QaError::InvalidBufferLength {
                width,
                height,
                expected: usize::MAX,
                actual: data.len(),
            })?;
        if data.len() != expected {
            return Err(QaError::InvalidBufferLength {
                width,
                height,
                expected,
                actual: data.len(),
            });
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    pub fn load_png(path: &Path) -> Result<Self> {
        let dyn_img = image::open(path)?;
        let rgba = dyn_img.to_rgba8();
        let (width, height) = rgba.dimensions();
        Ok(Self {
            width,
            height,
            data: rgba.into_raw(),
        })
    }

    pub fn save_png(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let img: RgbaImage = ImageBuffer::from_raw(self.width, self.height, self.data.clone())
            .ok_or(QaError::InvalidBufferLength {
                width: self.width,
                height: self.height,
                expected: (self.width as usize) * (self.height as usize) * 4,
                actual: self.data.len(),
            })?;
        img.save(path)?;
        Ok(())
    }

    /// Counts pixels where at least one RGB channel is non-zero (`> 0`).
    pub fn non_zero_rgb_pixels(&self) -> usize {
        self.data
            .chunks_exact(4)
            .filter(|px| px[0] > 0 || px[1] > 0 || px[2] > 0)
            .count()
    }

    /// Resizes an RGBA buffer to `(width, height)` using Lanczos3 filtering (useful for normalizing antialiased DPI-scaled golden captures).
    pub fn resize_lanczos3(&self, width: u32, height: u32) -> Result<Self> {
        self.resize_with_filter(width, height, image::imageops::FilterType::Lanczos3)
    }

    /// Resizes an RGBA buffer to `(width, height)` using nearest-neighbor sampling (for `image-rendering: pixelated` captures like Dither Waves).
    pub fn resize_nearest(&self, width: u32, height: u32) -> Result<Self> {
        self.resize_with_filter(width, height, image::imageops::FilterType::Nearest)
    }

    fn resize_with_filter(
        &self,
        width: u32,
        height: u32,
        filter: image::imageops::FilterType,
    ) -> Result<Self> {
        if self.width == width && self.height == height {
            return Ok(self.clone());
        }
        let img: RgbaImage = ImageBuffer::from_raw(self.width, self.height, self.data.clone())
            .ok_or(QaError::InvalidBufferLength {
                width: self.width,
                height: self.height,
                expected: (self.width as usize) * (self.height as usize) * 4,
                actual: self.data.len(),
            })?;
        let resized = image::imageops::resize(&img, width, height, filter);
        Ok(Self {
            width,
            height,
            data: resized.into_raw(),
        })
    }
}

/// Rectangular exclusion zone (e.g. live clock/FPS counter) ignored during comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgnoreRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl IgnoreRect {
    pub fn contains(&self, px: u32, py: u32) -> bool {
        px >= self.x
            && px < self.x.saturating_add(self.width)
            && py >= self.y
            && py < self.y.saturating_add(self.height)
    }
}

/// Configurable thresholds for a pixel-diff pass/fail verdict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PixelDiffConfig {
    /// Max per-channel absolute difference `[0..=255]` tolerated without counting as an outlier pixel.
    pub channel_tolerance: u8,
    /// Maximum tolerated Mean Absolute Error across RGBA channels `[0.0..=255.0]`.
    pub max_mae: f64,
    /// Maximum tolerated Root Mean Squared Error across RGBA channels `[0.0..=255.0]`.
    pub max_rmse: f64,
    /// Minimum tolerated structural similarity (`SSIM`, `[-1.0..=1.0]`, typically `>= 0.95`).
    pub min_ssim: f64,
    /// Maximum ratio of outlier pixels over compared pixels `[0.0..=1.0]`.
    pub max_outlier_ratio: f64,
    /// Optional rectangular regions masked out of metrics and diff highlighting.
    pub ignore_rects: Vec<IgnoreRect>,
}

impl Default for PixelDiffConfig {
    fn default() -> Self {
        Self {
            channel_tolerance: 4,
            max_mae: 3.0,
            max_rmse: 8.0,
            min_ssim: 0.95,
            max_outlier_ratio: 0.02,
            ignore_rects: Vec::new(),
        }
    }
}

impl PixelDiffConfig {
    /// Strict tolerance preset for deterministic UI/SDF/shader headless captures.
    pub fn strict() -> Self {
        Self {
            channel_tolerance: 2,
            max_mae: 1.0,
            max_rmse: 3.0,
            min_ssim: 0.99,
            max_outlier_ratio: 0.005,
            ignore_rects: Vec::new(),
        }
    }

    /// Font-rasterization / subpixel-AA tolerant preset comparing Electron vs Rust (`parley`/`cosmic-text`).
    pub fn ui_parity() -> Self {
        Self {
            channel_tolerance: 8,
            max_mae: 6.0,
            max_rmse: 18.0,
            min_ssim: 0.90,
            max_outlier_ratio: 0.06,
            ignore_rects: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PixelDiffReport {
    pub width: u32,
    pub height: u32,
    pub total_pixels: usize,
    pub compared_pixels: usize,
    pub outlier_pixels: usize,
    pub outlier_ratio: f64,
    pub max_channel_delta: u8,
    pub mae: f64,
    pub rmse: f64,
    pub ssim: f64,
    pub passed: bool,
}

/// Computes `MAE`, `RMSE`, `8x8` luminance `SSIM`, outlier ratio, and optional
/// magenta-highlighted diff PNG buffer (`expected` vs `actual`).
pub fn compare_rgba(
    expected: &RgbaImageBuffer,
    actual: &RgbaImageBuffer,
    config: &PixelDiffConfig,
    generate_diff: bool,
) -> Result<(PixelDiffReport, Option<RgbaImageBuffer>)> {
    if expected.width != actual.width || expected.height != actual.height {
        return Err(QaError::DimensionMismatch {
            expected_w: expected.width,
            expected_h: expected.height,
            actual_w: actual.width,
            actual_h: actual.height,
        });
    }

    let w = expected.width;
    let h = expected.height;
    let total_pixels = (w as usize) * (h as usize);

    let mut compared_pixels = 0usize;
    let mut outlier_pixels = 0usize;
    let mut max_channel_delta = 0u8;
    let mut sum_abs_err = 0u64;
    let mut sum_sq_err = 0u64;

    let mut diff_bytes = if generate_diff {
        vec![0u8; total_pixels * 4]
    } else {
        Vec::new()
    };

    for y in 0..h {
        for x in 0..w {
            let idx = ((y as usize) * (w as usize) + (x as usize)) * 4;
            let e = &expected.data[idx..idx + 4];
            let a = &actual.data[idx..idx + 4];

            if config.ignore_rects.iter().any(|r| r.contains(x, y)) {
                if generate_diff {
                    diff_bytes[idx] = 48;
                    diff_bytes[idx + 1] = 48;
                    diff_bytes[idx + 2] = 64;
                    diff_bytes[idx + 3] = 255;
                }
                continue;
            }

            compared_pixels += 1;
            let dr = e[0].abs_diff(a[0]);
            let dg = e[1].abs_diff(a[1]);
            let db = e[2].abs_diff(a[2]);
            let da = e[3].abs_diff(a[3]);
            let px_max = dr.max(dg).max(db).max(da);
            max_channel_delta = max_channel_delta.max(px_max);

            sum_abs_err += (dr as u64) + (dg as u64) + (db as u64) + (da as u64);
            sum_sq_err += (dr as u64) * (dr as u64)
                + (dg as u64) * (dg as u64)
                + (db as u64) * (db as u64)
                + (da as u64) * (da as u64);

            let is_outlier = px_max > config.channel_tolerance;
            if is_outlier {
                outlier_pixels += 1;
            }

            if generate_diff {
                if is_outlier {
                    // Highlight mismatching pixel in vivid magenta/red scaled by error magnitude.
                    let boost = px_max.saturating_add(96);
                    diff_bytes[idx] = boost;
                    diff_bytes[idx + 1] = 24;
                    diff_bytes[idx + 2] = 160;
                    diff_bytes[idx + 3] = 255;
                } else {
                    // Dimmed grayscale expected context so layout remains visible in diff output.
                    let luma = ((e[0] as u16 * 77 + e[1] as u16 * 150 + e[2] as u16 * 29) >> 8) as u8;
                    let dim = luma / 3;
                    diff_bytes[idx] = dim;
                    diff_bytes[idx + 1] = dim;
                    diff_bytes[idx + 2] = dim;
                    diff_bytes[idx + 3] = 255;
                }
            }
        }
    }

    let channel_samples = (compared_pixels.max(1) * 4) as f64;
    let mae = if compared_pixels == 0 {
        0.0
    } else {
        (sum_abs_err as f64) / channel_samples
    };
    let rmse = if compared_pixels == 0 {
        0.0
    } else {
        ((sum_sq_err as f64) / channel_samples).sqrt()
    };
    let outlier_ratio = if compared_pixels == 0 {
        0.0
    } else {
        (outlier_pixels as f64) / (compared_pixels as f64)
    };
    let ssim = compute_luminance_ssim(expected, actual, &config.ignore_rects);

    let passed = mae <= config.max_mae
        && rmse <= config.max_rmse
        && ssim >= config.min_ssim
        && outlier_ratio <= config.max_outlier_ratio;

    let report = PixelDiffReport {
        width: w,
        height: h,
        total_pixels,
        compared_pixels,
        outlier_pixels,
        outlier_ratio,
        max_channel_delta,
        mae,
        rmse,
        ssim,
        passed,
    };

    let diff_img = if generate_diff {
        Some(RgbaImageBuffer::from_raw_rgba(w, h, diff_bytes)?)
    } else {
        None
    };

    Ok((report, diff_img))
}

/// Computes mean structural similarity (SSIM) on luminance (`Y = 0.299R + 0.587G + 0.114B`)
/// over `8x8` pixel blocks (with partial edge blocks included).
fn compute_luminance_ssim(
    expected: &RgbaImageBuffer,
    actual: &RgbaImageBuffer,
    ignore_rects: &[IgnoreRect],
) -> f64 {
    let w = expected.width;
    let h = expected.height;
    if w == 0 || h == 0 {
        return 1.0;
    }

    const C1: f64 = (0.01 * 255.0) * (0.01 * 255.0);
    const C2: f64 = (0.03 * 255.0) * (0.03 * 255.0);

    let luma = |px: &[u8]| -> f64 {
        0.299 * (px[0] as f64) + 0.587 * (px[1] as f64) + 0.114 * (px[2] as f64)
    };

    let mut ssim_sum = 0.0;
    let mut block_count = 0usize;

    let mut by = 0u32;
    while by < h {
        let bh = (h - by).min(8);
        let mut bx = 0u32;
        while bx < w {
            let bw = (w - bx).min(8);

            let mut n = 0usize;
            let mut sum_x = 0.0;
            let mut sum_y = 0.0;
            let mut sum_x2 = 0.0;
            let mut sum_y2 = 0.0;
            let mut sum_xy = 0.0;

            for dy in 0..bh {
                let py = by + dy;
                for dx in 0..bw {
                    let px = bx + dx;
                    if ignore_rects.iter().any(|r| r.contains(px, py)) {
                        continue;
                    }
                    let idx = ((py as usize) * (w as usize) + (px as usize)) * 4;
                    let lx = luma(&expected.data[idx..idx + 4]);
                    let ly = luma(&actual.data[idx..idx + 4]);
                    n += 1;
                    sum_x += lx;
                    sum_y += ly;
                    sum_x2 += lx * lx;
                    sum_y2 += ly * ly;
                    sum_xy += lx * ly;
                }
            }

            if n > 0 {
                let nf = n as f64;
                let mu_x = sum_x / nf;
                let mu_y = sum_y / nf;
                let var_x = (sum_x2 / nf - mu_x * mu_x).max(0.0);
                let var_y = (sum_y2 / nf - mu_y * mu_y).max(0.0);
                let cov_xy = sum_xy / nf - mu_x * mu_y;

                let num = (2.0 * mu_x * mu_y + C1) * (2.0 * cov_xy + C2);
                let den = (mu_x * mu_x + mu_y * mu_y + C1) * (var_x + var_y + C2);
                ssim_sum += if den > 0.0 { num / den } else { 1.0 };
                block_count += 1;
            }

            bx += 8;
        }
        by += 8;
    }

    if block_count == 0 {
        1.0
    } else {
        (ssim_sum / (block_count as f64)).clamp(-1.0, 1.0)
    }
}

/// Convenience helper loading two PNG files, running `compare_rgba`, and optionally saving a diff PNG.
pub fn compare_png_files(
    expected_png: &Path,
    actual_png: &Path,
    diff_out_png: Option<&Path>,
    config: &PixelDiffConfig,
) -> Result<PixelDiffReport> {
    let expected = RgbaImageBuffer::load_png(expected_png)?;
    let actual = RgbaImageBuffer::load_png(actual_png)?;
    let (report, diff_img) = compare_rgba(&expected, &actual, config, diff_out_png.is_some())?;
    if let (Some(out_path), Some(img)) = (diff_out_png, diff_img) {
        img.save_png(out_path)?;
    }
    Ok(report)
}

/// Creates a solid color `RgbaImageBuffer` (useful for tests & synthetic baselines).
pub fn solid_rgba(width: u32, height: u32, color: [u8; 4]) -> RgbaImageBuffer {
    let total = (width as usize) * (height as usize) * 4;
    let mut data = Vec::with_capacity(total);
    for _ in 0..(width as usize) * (height as usize) {
        data.extend_from_slice(&color);
    }
    RgbaImageBuffer {
        width,
        height,
        data,
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoldenCaptureEntry {
    pub file: String,
    pub bytes: u64,
    pub inner_width: u32,
    pub inner_height: u32,
    pub theme: String,
    pub bg: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoldenCanvasDiag {
    pub exists: bool,
    pub width: u32,
    pub height: u32,
    pub non_zero_pixels: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticlesDiagnostics {
    pub canvas_info: GoldenCanvasDiag,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoldenCaptureManifest {
    pub captures: Vec<GoldenCaptureEntry>,
    #[serde(default)]
    pub errors: Vec<serde_json::Value>,
    pub particles_diagnostics: Option<ParticlesDiagnostics>,
}

impl GoldenCaptureManifest {
    pub fn load_from_golden_dir(golden_dir: &Path) -> Result<Self> {
        let manifest_path = golden_dir.join("capture-manifest.json");
        let text = fs::read_to_string(&manifest_path)?;
        Ok(serde_json::from_str(&text)?)
    }

    /// Returns `true` when the captured golden file is known to be blank due to
    /// Electron WebGL capture limitation (`shader-particles-1280x720.png`,
    /// `particlesDiagnostics.canvasInfo.nonZeroPixels == 0`).
    pub fn is_known_blank_capture(&self, entry: &GoldenCaptureEntry) -> bool {
        entry.file == "shader-particles-1280x720.png"
            && self
                .particles_diagnostics
                .as_ref()
                .map(|d| d.canvas_info.non_zero_pixels == 0)
                .unwrap_or(false)
    }

    pub fn resolve_png_path(golden_dir: &Path, entry: &GoldenCaptureEntry) -> PathBuf {
        golden_dir.join(&entry.file)
    }
}
