pub const EQ_BAND_COUNT: usize = 10;
pub const EQ_UI_FREQUENCIES: [f64; EQ_BAND_COUNT] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];
pub const EQ_BAND_LABELS: [&str; EQ_BAND_COUNT] = [
    "31Hz", "62Hz", "125Hz", "250Hz", "500Hz", "1K", "2K", "4K", "8K", "16K",
];

pub const EQ_FFMPEG_FREQUENCIES: [f64; 18] = [
    65.0, 92.0, 131.0, 185.0, 262.0, 370.0, 523.0, 740.0, 1047.0, 1480.0, 2093.0, 2960.0, 4186.0,
    5920.0, 8372.0, 11840.0, 16744.0, 20000.0,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqualizerPreset {
    Flat,
    BassBoost,
    TrebleBoost,
    Vocal,
    Rock,
    Pop,
    Jazz,
    Classical,
    Electronic,
    Custom,
}

impl EqualizerPreset {
    pub const BUILTIN_PRESETS: [EqualizerPreset; 9] = [
        Self::Flat,
        Self::BassBoost,
        Self::TrebleBoost,
        Self::Vocal,
        Self::Rock,
        Self::Pop,
        Self::Jazz,
        Self::Classical,
        Self::Electronic,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::BassBoost => "bass-boost",
            Self::TrebleBoost => "treble-boost",
            Self::Vocal => "vocal",
            Self::Rock => "rock",
            Self::Pop => "pop",
            Self::Jazz => "jazz",
            Self::Classical => "classical",
            Self::Electronic => "electronic",
            Self::Custom => "custom",
        }
    }

    pub fn from_str_name(s: &str) -> Option<Self> {
        match s {
            "flat" => Some(Self::Flat),
            "bass-boost" => Some(Self::BassBoost),
            "treble-boost" => Some(Self::TrebleBoost),
            "vocal" => Some(Self::Vocal),
            "rock" => Some(Self::Rock),
            "pop" => Some(Self::Pop),
            "jazz" => Some(Self::Jazz),
            "classical" => Some(Self::Classical),
            "electronic" => Some(Self::Electronic),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }

    pub fn bands(self) -> Option<[f64; EQ_BAND_COUNT]> {
        match self {
            Self::Flat => Some([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
            Self::BassBoost => Some([8.0, 6.0, 5.0, 3.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
            Self::TrebleBoost => Some([0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 3.0, 5.0, 6.0, 8.0]),
            Self::Vocal => Some([-2.0, -1.0, 0.0, 3.0, 6.0, 6.0, 3.0, 0.0, -1.0, -2.0]),
            Self::Rock => Some([5.0, 4.0, 2.0, 0.0, -1.0, -1.0, 0.0, 2.0, 4.0, 5.0]),
            Self::Pop => Some([-1.0, 2.0, 4.0, 5.0, 3.0, 0.0, -1.0, -1.0, 2.0, 3.0]),
            Self::Jazz => Some([4.0, 3.0, 1.0, 2.0, -1.0, -1.0, 0.0, 1.0, 3.0, 4.0]),
            Self::Classical => Some([5.0, 4.0, 3.0, 2.0, -1.0, -1.0, 0.0, 2.0, 3.0, 5.0]),
            Self::Electronic => Some([6.0, 5.0, 2.0, 0.0, -2.0, 0.0, 1.0, 3.0, 5.0, 6.0]),
            Self::Custom => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EqualizerState {
    pub bands: [f64; EQ_BAND_COUNT],
    pub preset: EqualizerPreset,
}

impl Default for EqualizerState {
    fn default() -> Self {
        Self {
            bands: [0.0; EQ_BAND_COUNT],
            preset: EqualizerPreset::Flat,
        }
    }
}

impl EqualizerState {
    pub fn clamp_gain(gain_db: f64) -> f64 {
        if gain_db.is_finite() {
            gain_db.clamp(-12.0, 12.0)
        } else {
            0.0
        }
    }

    pub fn normalize_bands(input: &[f64]) -> [f64; EQ_BAND_COUNT] {
        let mut bands = [0.0; EQ_BAND_COUNT];
        for (i, dst) in bands.iter_mut().enumerate() {
            *dst = input.get(i).copied().map(Self::clamp_gain).unwrap_or(0.0);
        }
        bands
    }

    pub fn set_band(&mut self, index: usize, gain_db: f64) {
        if index < EQ_BAND_COUNT {
            self.bands[index] = Self::clamp_gain(gain_db);
            self.preset = EqualizerPreset::Custom;
        }
    }

    pub fn apply_preset(&mut self, preset: EqualizerPreset) -> bool {
        if let Some(values) = preset.bands() {
            self.bands = values;
            self.preset = preset;
            true
        } else {
            false
        }
    }

    pub fn mpv_af_value(&self) -> String {
        build_mpv_af_value(&self.bands)
    }
}

/// Generates mpv `lavfi=[superequalizer=v0:v1:...:v17]` filter string matching `equalizer.js:168-205`.
pub fn generate_filter_string(custom_bands: &[f64]) -> String {
    let bands = EqualizerState::normalize_bands(custom_bands);
    let max_boost = bands.iter().copied().fold(0.0_f64, f64::max);
    let attenuation = if max_boost > 0.0 {
        10.0_f64.powf(-max_boost / 20.0)
    } else {
        1.0
    };

    let mut gains = Vec::with_capacity(EQ_FFMPEG_FREQUENCIES.len());
    for &f in &EQ_FFMPEG_FREQUENCIES {
        let db = if f <= EQ_UI_FREQUENCIES[0] {
            bands[0]
        } else if f >= EQ_UI_FREQUENCIES[EQ_BAND_COUNT - 1] {
            bands[EQ_BAND_COUNT - 1]
        } else {
            let mut k = 0usize;
            while k < EQ_BAND_COUNT - 1 && EQ_UI_FREQUENCIES[k + 1] < f {
                k += 1;
            }
            let log_f0 = EQ_UI_FREQUENCIES[k].log2();
            let log_f1 = EQ_UI_FREQUENCIES[k + 1].log2();
            let t = (f.log2() - log_f0) / (log_f1 - log_f0);
            (1.0 - t) * bands[k] + t * bands[k + 1]
        };
        let clamped_db = db.clamp(-12.0, 12.0);
        let mult = (10.0_f64.powf(clamped_db / 20.0) * attenuation).clamp(0.05, 20.0);
        gains.push(format!("{mult:.2}"));
    }

    format!("lavfi=[superequalizer={}]", gains.join(":"))
}

/// Returns empty string `""` when all 10 bands are 0, otherwise `generate_filter_string`.
pub fn build_mpv_af_value(custom_bands: &[f64]) -> String {
    let bands = EqualizerState::normalize_bands(custom_bands);
    if bands.iter().all(|&b| b == 0.0) {
        String::new()
    } else {
        generate_filter_string(&bands)
    }
}
