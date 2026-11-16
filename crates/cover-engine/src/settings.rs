//! User-facing settings. Presets fill in sensible values; every field stays
//! editable in the expert panel.

use crate::analysis::tuning::TuningParams;
use cover_dsp::analysis::key::Mode;
use cover_media::{Aspect, VideoQuality, VideoStyle};
use cover_ml::Device;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundPreset {
    /// Light touch: keeps the timbre and emotion of the take.
    Natural,
    /// Balanced voice/instrument, moderate tuning. Recommended.
    Balanced,
    /// Tighter tuning and clearer diction.
    Polished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentKind {
    AcousticGuitar,
    ClassicalGuitar,
    ElectricGuitar,
    Piano,
    Ukulele,
    FullBand,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScaleMode {
    /// Key detected from the recording.
    Auto,
    /// Snap to the nearest semitone.
    Chromatic,
    Manual {
        tonic: u8,
        mode: Mode,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TuningSettings {
    pub enabled: bool,
    pub strength: f32,
    pub drift: f32,
    pub scale: ScaleMode,
}

impl TuningSettings {
    pub fn params(&self) -> TuningParams {
        TuningParams {
            strength: self.strength.clamp(0.0, 1.0),
            drift: self.drift.clamp(0.0, 1.0),
            ..TuningParams::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VocalSettings {
    /// 0..1, how much of the processed vocal replaces the raw one.
    pub blend: f32,
    pub presence_db: f32,
    pub air_db: f32,
    pub de_ess: bool,
    /// 0..1 compression amount.
    pub compression: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LoudnessTarget {
    pub lufs: f64,
    pub ceiling_dbtp: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct RangeSettings {
    /// Selected take; `None` means the best detected one.
    pub take: Option<usize>,
    /// Manual bounds in audio time, overriding the take.
    pub start_sec: Option<f64>,
    pub end_sec: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VideoSettings {
    pub aspect: Aspect,
    pub style: VideoStyle,
    pub quality: VideoQuality,
    pub fade_in_sec: f64,
    pub fade_out_sec: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Fr,
    En,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SocialSettings {
    pub title: String,
    pub artist: String,
    pub language: Language,
    /// Thumbnail frame time in audio time; `None` picks one inside the range.
    pub thumbnail_at_sec: Option<f64>,
    pub extra_hashtags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub separation_model: String,
    pub denoise_separation: bool,
    pub device: Device,
    pub preset: SoundPreset,
    pub instrument: InstrumentKind,
    pub tuning: TuningSettings,
    pub vocal: VocalSettings,
    /// 0..1, how much of the processed instrument replaces the raw one.
    pub instrument_blend: f32,
    /// Low shelf at 150 Hz on the mix bus, dB.
    pub warmth_db: f32,
    /// 0..1 small-room ambience.
    pub room: f32,
    pub loudness: LoudnessTarget,
    pub range: RangeSettings,
    /// Manual video/audio offset (video_time - audio_time); overrides detection.
    pub sync_offset_sec: Option<f64>,
    pub video: VideoSettings,
    pub social: SocialSettings,
}

impl Default for Settings {
    fn default() -> Self {
        let mut s = Settings {
            separation_model: "mdx-voc-ft".into(),
            denoise_separation: false,
            device: Device::Auto,
            preset: SoundPreset::Balanced,
            instrument: InstrumentKind::AcousticGuitar,
            tuning: TuningSettings {
                enabled: true,
                strength: 0.8,
                drift: 0.2,
                scale: ScaleMode::Auto,
            },
            vocal: VocalSettings {
                blend: 0.75,
                presence_db: 1.5,
                air_db: 1.0,
                de_ess: true,
                compression: 0.5,
            },
            instrument_blend: 0.6,
            warmth_db: 0.8,
            room: 0.25,
            loudness: LoudnessTarget {
                lufs: -14.0,
                ceiling_dbtp: -1.0,
            },
            range: RangeSettings::default(),
            sync_offset_sec: None,
            video: VideoSettings {
                aspect: Aspect::Vertical,
                style: VideoStyle::Social,
                quality: VideoQuality::High,
                fade_in_sec: 0.4,
                fade_out_sec: 1.2,
            },
            social: SocialSettings {
                title: String::new(),
                artist: String::new(),
                language: Language::Fr,
                thumbnail_at_sec: None,
                extra_hashtags: Vec::new(),
            },
        };
        s.apply_preset(SoundPreset::Balanced);
        s
    }
}

impl Settings {
    /// Overwrites the sound-shaping fields with the preset's values.
    pub fn apply_preset(&mut self, preset: SoundPreset) {
        self.preset = preset;
        match preset {
            SoundPreset::Natural => {
                self.tuning.strength = 0.55;
                self.tuning.drift = 0.0;
                self.vocal = VocalSettings {
                    blend: 0.5,
                    presence_db: 0.8,
                    air_db: 0.5,
                    de_ess: true,
                    compression: 0.3,
                };
                self.instrument_blend = 0.4;
                self.warmth_db = 0.6;
                self.room = 0.15;
            }
            SoundPreset::Balanced => {
                self.tuning.strength = 0.8;
                self.tuning.drift = 0.2;
                self.vocal = VocalSettings {
                    blend: 0.75,
                    presence_db: 1.5,
                    air_db: 1.0,
                    de_ess: true,
                    compression: 0.5,
                };
                self.instrument_blend = 0.6;
                self.warmth_db = 0.8;
                self.room = 0.25;
            }
            SoundPreset::Polished => {
                self.tuning.strength = 0.95;
                self.tuning.drift = 0.45;
                self.vocal = VocalSettings {
                    blend: 0.9,
                    presence_db: 2.5,
                    air_db: 1.8,
                    de_ess: true,
                    compression: 0.7,
                };
                self.instrument_blend = 0.65;
                self.warmth_db = 0.7;
                self.room = 0.3;
            }
        }
    }
}
