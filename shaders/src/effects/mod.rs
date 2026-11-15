pub mod dither_waves;
pub mod faulty_terminal;
pub mod grid_motion;
pub mod particles;

pub use dither_waves::DitherWavesEffect;
pub use faulty_terminal::FaultyTerminalEffect;
pub use grid_motion::{
    decode_and_resize_image, decode_image_file_async, default_gradients_rgb, GridMotionEffect,
    CARD_TEXTS, DEFAULT_GRADIENT_HEXES, MAX_IMAGE_DIM,
};
pub use particles::ParticlesEffect;

use crate::types::{
    DitherWavesConfig, EffectInstanceConfig, EffectLifecycle, FaultyTerminalConfig,
    GridMotionConfig, ParticlesConfig, QualityTier, WelcomeBackgroundKind,
};

/// Bundle of all 4 Welcome Screen shader effects, supporting full-bleed or `400x240 @ 30fps` preview.
pub struct WelcomeEffectsBundle {
    pub dither: DitherWavesEffect,
    pub particles: ParticlesEffect,
    pub faulty: FaultyTerminalEffect,
    pub grid_motion: GridMotionEffect,
    pub quality: QualityTier,
}

impl WelcomeEffectsBundle {
    /// Creates all 4 effects targeting `format` with full-bleed `medium` preset defaults (`settings.js:87`).
    pub fn new_full_bleed(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        instance: EffectInstanceConfig,
    ) -> Self {
        Self {
            dither: DitherWavesEffect::new(
                device,
                format,
                DitherWavesConfig::for_quality(QualityTier::Medium),
                instance,
            ),
            particles: ParticlesEffect::new(
                device,
                format,
                ParticlesConfig::for_quality(QualityTier::Medium),
                instance,
            ),
            faulty: FaultyTerminalEffect::new(
                device,
                format,
                FaultyTerminalConfig::for_quality(QualityTier::Medium),
                instance,
            ),
            grid_motion: GridMotionEffect::new(
                device,
                queue,
                format,
                GridMotionConfig::for_quality(QualityTier::Medium),
                instance,
            ),
            quality: QualityTier::Medium,
        }
    }

    /// Updates `quality` preset tier (controls `DitherWaves` DPR scaling parity with `ditherWaves.js`).
    pub fn set_quality(&mut self, quality: QualityTier) {
        self.quality = quality;
    }

    /// Updates `instance.dpr` across all 4 effects when window DPI scale changes.
    pub fn set_dpr(&mut self, dpr: f32) {
        if !dpr.is_finite() || dpr <= 0.0 {
            return;
        }
        self.dither.instance.dpr = dpr;
        self.particles.instance.dpr = dpr;
        self.faulty.instance.dpr = dpr;
        self.grid_motion.instance.dpr = dpr;
    }

    /// Pushes live configuration updates into all 4 effects, rebuilding particle buffers only when needed.
    pub fn apply_configs(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dither: DitherWavesConfig,
        particles: ParticlesConfig,
        faulty: FaultyTerminalConfig,
        grid_motion: GridMotionConfig,
    ) {
        self.dither.config = dither;
        self.particles.apply_config(device, particles);
        self.faulty.config = faulty;
        self.grid_motion.apply_config(queue, grid_motion);
    }

    /// Creates all 4 effects configured for the `400x240 @ 30fps` accordion preview gallery (`settings.js:2484-2534`).
    pub fn new_accordion_previews(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        let preview = EffectInstanceConfig::preview_400x240();
        Self {
            dither: DitherWavesEffect::new(
                device,
                format,
                DitherWavesConfig::preview_preset(),
                preview,
            ),
            particles: ParticlesEffect::new(
                device,
                format,
                ParticlesConfig::preview_preset(),
                preview,
            ),
            faulty: FaultyTerminalEffect::new(
                device,
                format,
                FaultyTerminalConfig::preview_preset(),
                preview,
            ),
            grid_motion: GridMotionEffect::new(
                device,
                queue,
                format,
                GridMotionConfig::preview_preset(),
                preview,
            ),
            quality: QualityTier::Medium,
        }
    }

    /// Propagates window focus, visibility, playback, and motion-pause state to all 4 effects.
    pub fn set_lifecycle_flags(
        &mut self,
        window_focused: bool,
        window_visible: bool,
        video_playing: bool,
        motion_paused: bool,
    ) {
        for lc in [
            &mut self.dither.lifecycle,
            &mut self.particles.lifecycle,
            &mut self.faulty.lifecycle,
            &mut self.grid_motion.lifecycle,
        ] {
            lc.window_focused = window_focused;
            lc.window_visible = window_visible;
            lc.video_playing = video_playing;
            lc.motion_paused = motion_paused;
        }
    }

    /// Returns the lifecycle handle of the specified background effect.
    pub fn lifecycle(&self, kind: WelcomeBackgroundKind) -> Option<&EffectLifecycle> {
        match kind {
            WelcomeBackgroundKind::None => None,
            WelcomeBackgroundKind::Dither => Some(&self.dither.lifecycle),
            WelcomeBackgroundKind::Particles => Some(&self.particles.lifecycle),
            WelcomeBackgroundKind::Faulty => Some(&self.faulty.lifecycle),
            WelcomeBackgroundKind::Gridmotion => Some(&self.grid_motion.lifecycle),
        }
    }

    /// Advances the active effect simulation by `dt_secs`.
    pub fn update(&mut self, kind: WelcomeBackgroundKind, dt_secs: f32) {
        match kind {
            WelcomeBackgroundKind::None => {}
            WelcomeBackgroundKind::Dither => self.dither.update(dt_secs),
            WelcomeBackgroundKind::Particles => self.particles.update(dt_secs),
            WelcomeBackgroundKind::Faulty => self.faulty.update(dt_secs),
            WelcomeBackgroundKind::Gridmotion => self.grid_motion.update(dt_secs),
        }
    }

    /// Encodes the selected welcome background into `target_view` (`target_w x target_h`).
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        kind: WelcomeBackgroundKind,
        target_view: &wgpu::TextureView,
        target_w: u32,
        target_h: u32,
    ) {
        match kind {
            WelcomeBackgroundKind::None => {}
            WelcomeBackgroundKind::Dither => {
                // `ditherWaves.js` renders to an `effect_dpr = tier.cap_dpr(device_dpr)` canvas and
                // CSS-stretches `image-rendering: pixelated` to physical size.
                self.dither.effective_dpr = if self.dither.instance.is_preview {
                    None
                } else {
                    Some(self.quality.cap_dpr(self.dither.instance.dpr))
                };
                self.dither
                    .encode(queue, encoder, target_view, target_w, target_h);
            }
            WelcomeBackgroundKind::Particles => {
                // `particles.js:125-130, 159`: renders at `pixelRatio = tier.cap_particles_dpr(device_dpr)`.
                self.particles.effective_dpr = if self.particles.instance.is_preview {
                    None
                } else {
                    Some(self.quality.cap_particles_dpr(self.particles.instance.dpr))
                };
                self.particles
                    .encode(device, queue, encoder, target_view, target_w, target_h);
            }
            WelcomeBackgroundKind::Faulty => {
                // `faultyTerminal.js:368-373, 495-496`: renders at `effect_dpr = tier.cap_dpr(device_dpr)`
                // and CSS-stretches bilinearly to full viewport dimensions.
                self.faulty.effective_dpr = if self.faulty.instance.is_preview {
                    None
                } else {
                    Some(self.quality.cap_dpr(self.faulty.instance.dpr))
                };
                self.faulty
                    .encode(device, queue, encoder, target_view, target_w, target_h);
            }
            WelcomeBackgroundKind::Gridmotion => {
                self.grid_motion
                    .encode(device, queue, encoder, target_view, target_w, target_h);
            }
        }
    }
}
