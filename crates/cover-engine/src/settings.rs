//! User-facing settings. Presets fill in sensible values; every field stays
//! editable in the expert panel.

use crate::analysis::tuning::TuningParams;
use cover_dsp::analysis::key::Mode;
use cover_media::{Aspect, VideoQuality, VideoStyle};
use cover_ml::Device;
use serde::{Deserialize, Deserializer, Serialize};

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

/// Values for fields added after the first release, so older projects load
/// with the balanced preset's behaviour.
mod defaults {
    pub fn retune_speed() -> f32 {
        0.35
    }
    pub fn vibrato() -> f32 {
        0.2
    }
    pub fn yes() -> bool {
        true
    }
    pub const DE_ESS: f32 = 0.55;
    pub fn leveler() -> f32 {
        0.55
    }
    pub fn breath() -> f32 {
        0.4
    }
    pub fn cleanup() -> f32 {
        0.45
    }
    pub fn harshness() -> f32 {
        0.5
    }
    pub fn reverb() -> f32 {
        0.18
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TuningSettings {
    pub enabled: bool,
    /// 0..1 share of each note's pitch error that gets corrected.
    pub strength: f32,
    /// 0..1, steadies slow drift inside held notes.
    pub drift: f32,
    /// 0 (natural) .. 1 (tight): how much attacks and small scoops are
    /// pulled onto the note.
    #[serde(default = "defaults::retune_speed")]
    pub retune_speed: f32,
    /// 0..1, calms vibrato that is wider than normal (normal vibrato is kept).
    #[serde(default = "defaults::vibrato")]
    pub vibrato: f32,
    /// Keeps the vocal timbre while the pitch moves.
    #[serde(default = "defaults::yes")]
    pub preserve_formants: bool,
    pub scale: ScaleMode,
}

impl TuningSettings {
    pub fn params(&self) -> TuningParams {
        TuningParams {
            strength: self.strength.clamp(0.0, 1.0),
            retune_speed: self.retune_speed.clamp(0.0, 1.0),
            drift: self.drift.clamp(0.0, 1.0),
            vibrato: self.vibrato.clamp(0.0, 1.0),
            ..TuningParams::default()
        }
    }
}

/// Accepts the 0..1 amount, or the on/off flag older projects stored.
fn amount_or_flag<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<f32, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value {
        Flag(bool),
        Amount(f32),
    }
    Ok(match Value::deserialize(d)? {
        Value::Flag(true) => defaults::DE_ESS,
        Value::Flag(false) => 0.0,
        Value::Amount(v) => v,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VocalSettings {
    /// 0..1, how much of the processed (EQ, dynamics, clean-up) vocal
    /// replaces the raw one. Pitch correction is always applied in full.
    pub blend: f32,
    pub presence_db: f32,
    pub air_db: f32,
    /// 0..1 de-essing amount.
    #[serde(deserialize_with = "amount_or_flag")]
    pub de_ess: f32,
    /// 0..1 compression amount.
    pub compression: f32,
    /// 0..1, evens out quiet and loud phrases before the compressor.
    #[serde(default = "defaults::leveler")]
    pub leveler: f32,
    /// 0..1, lowers breaths between phrases.
    #[serde(default = "defaults::breath")]
    pub breath: f32,
    /// 0..1 noise, mouth-click and plosive clean-up.
    #[serde(default = "defaults::cleanup")]
    pub cleanup: f32,
    /// 0..1 dynamic control of harsh, boomy and resonant moments.
    #[serde(default = "defaults::harshness")]
    pub harshness: f32,
    /// 0..1 vocal plate reverb.
    #[serde(default = "defaults::reverb")]
    pub reverb: f32,
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
                strength: 0.85,
                drift: 0.35,
                retune_speed: defaults::retune_speed(),
                vibrato: defaults::vibrato(),
                preserve_formants: true,
                scale: ScaleMode::Auto,
            },
            vocal: VocalSettings::preset(SoundPreset::Balanced),
            instrument_blend: 0.6,
            warmth_db: 0.8,
            room: 0.22,
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

impl VocalSettings {
    pub fn preset(preset: SoundPreset) -> Self {
        match preset {
            SoundPreset::Natural => VocalSettings {
                blend: 0.6,
                presence_db: 0.8,
                air_db: 0.5,
                de_ess: 0.4,
                compression: 0.3,
                leveler: 0.35,
                breath: 0.25,
                cleanup: 0.3,
                harshness: 0.3,
                reverb: 0.1,
            },
            SoundPreset::Balanced => VocalSettings {
                blend: 0.8,
                presence_db: 1.5,
                air_db: 1.0,
                de_ess: defaults::DE_ESS,
                compression: 0.5,
                leveler: defaults::leveler(),
                breath: defaults::breath(),
                cleanup: defaults::cleanup(),
                harshness: defaults::harshness(),
                reverb: defaults::reverb(),
            },
            SoundPreset::Polished => VocalSettings {
                blend: 0.95,
                presence_db: 2.3,
                air_db: 1.6,
                de_ess: 0.7,
                compression: 0.65,
                leveler: 0.75,
                breath: 0.55,
                cleanup: 0.6,
                harshness: 0.65,
                reverb: 0.26,
            },
        }
    }
}

impl Settings {
    /// Overwrites the sound-shaping fields with the preset's values.
    pub fn apply_preset(&mut self, preset: SoundPreset) {
        self.preset = preset;
        self.vocal = VocalSettings::preset(preset);
        // (strength, retune speed, drift, vibrato), instrument blend, warmth, room
        let ((strength, retune_speed, drift, vibrato), instrument_blend, warmth_db, room) =
            match preset {
                SoundPreset::Natural => ((0.6, 0.15, 0.15, 0.0), 0.4, 0.6, 0.15),
                SoundPreset::Balanced => ((0.85, 0.35, 0.35, 0.2), 0.6, 0.8, 0.22),
                SoundPreset::Polished => ((0.95, 0.6, 0.6, 0.4), 0.65, 0.7, 0.25),
            };
        self.tuning.strength = strength;
        self.tuning.retune_speed = retune_speed;
        self.tuning.drift = drift;
        self.tuning.vibrato = vibrato;
        self.instrument_blend = instrument_blend;
        self.warmth_db = warmth_db;
        self.room = room;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_projects_load_with_new_defaults() {
        let mut v = serde_json::to_value(Settings::default()).unwrap();
        let vocal = v["vocal"].as_object_mut().unwrap();
        for k in ["leveler", "breath", "cleanup", "harshness", "reverb"] {
            vocal.remove(k);
        }
        vocal.insert("de_ess".into(), serde_json::Value::Bool(true));
        let tuning = v["tuning"].as_object_mut().unwrap();
        for k in ["retune_speed", "vibrato", "preserve_formants"] {
            tuning.remove(k);
        }
        let s: Settings = serde_json::from_value(v).unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn presets_get_progressively_stronger() {
        let mut s = Settings::default();
        let mut prev: Option<Settings> = None;
        for p in [
            SoundPreset::Natural,
            SoundPreset::Balanced,
            SoundPreset::Polished,
        ] {
            s.apply_preset(p);
            if let Some(q) = &prev {
                assert!(s.tuning.strength > q.tuning.strength);
                assert!(s.tuning.retune_speed > q.tuning.retune_speed);
                assert!(s.vocal.leveler > q.vocal.leveler);
                assert!(s.vocal.de_ess > q.vocal.de_ess);
                assert!(s.vocal.cleanup > q.vocal.cleanup);
            }
            prev = Some(s.clone());
        }
    }
}
