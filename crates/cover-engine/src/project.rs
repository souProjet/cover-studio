//! Projects are folders holding a `project.json` plus cached intermediates.

use crate::analysis::sync::SyncResult;
use crate::analysis::takes::Take;
use crate::analysis::tuning::Note;
use crate::render::RenderStats;
use crate::settings::Settings;
use chrono::{DateTime, Utc};
use cover_dsp::analysis::key::Key;
use cover_dsp::loudness::LoudnessReport;
use cover_media::MediaInfo;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Sources {
    /// Separate audio recording (external mic, DAW bounce...).
    pub audio: Option<PathBuf>,
    /// Phone or camera video.
    pub video: Option<PathBuf>,
}

impl Sources {
    /// The recording whose sound gets processed.
    pub fn master_audio(&self) -> Option<&PathBuf> {
        self.audio.as_ref().or(self.video.as_ref())
    }

    pub fn needs_sync(&self) -> bool {
        self.audio.is_some() && self.video.is_some()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaSummary {
    pub audio: Option<MediaInfo>,
    pub video: Option<MediaInfo>,
}

/// Peak envelope for drawing waveforms without decoding audio in the UI.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Waveform {
    pub duration_sec: f64,
    pub peaks: Vec<f32>,
}

impl Waveform {
    pub fn from_samples(x: &[f32], sample_rate: u32, points: usize) -> Self {
        let points = points.max(1);
        let chunk = x.len().div_ceil(points).max(1);
        Waveform {
            duration_sec: x.len() as f64 / sample_rate as f64,
            peaks: x
                .chunks(chunk)
                .map(|c| c.iter().fold(0.0f32, |m, v| m.max(v.abs())))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    /// Identifies sources + separation settings; a mismatch triggers re-analysis.
    pub fingerprint: String,
    pub duration_sec: f64,
    pub sample_rate: u32,
    pub sync: Option<SyncResult>,
    pub takes: Vec<Take>,
    pub best_take: Option<usize>,
    pub key: Key,
    pub original_loudness: LoudnessReport,
    pub waveform: Waveform,
    pub vocal_waveform: Waveform,
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderInfo {
    pub at: DateTime<Utc>,
    /// Settings + range the render was made with.
    pub fingerprint: String,
    pub start_sec: f64,
    pub end_sec: f64,
    /// Float master used for exports.
    pub master: PathBuf,
    /// Processed preview (16-bit WAV, playable in the webview).
    pub processed: PathBuf,
    /// Same range of the untouched recording, loudness-matched for fair A/B.
    pub original: PathBuf,
    pub stats: RenderStats,
    pub notes: Vec<Note>,
    pub waveform: Waveform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportKind {
    Wav,
    Audio,
    Video,
    Thumbnail,
    Caption,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportedFile {
    pub kind: ExportKind,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportRecord {
    pub at: DateTime<Utc>,
    pub dir: PathBuf,
    pub files: Vec<ExportedFile>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub sources: Sources,
    pub media: MediaSummary,
    pub analysis: Option<Analysis>,
    pub settings: Settings,
    pub last_render: Option<RenderInfo>,
    pub exports: Vec<ExportRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub updated_at: DateTime<Utc>,
    pub has_video: bool,
    pub analyzed: bool,
    pub duration_sec: Option<f64>,
    pub key: Option<Key>,
}

impl Project {
    pub fn summary(&self) -> ProjectSummary {
        ProjectSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            updated_at: self.updated_at,
            has_video: self.sources.video.is_some(),
            analyzed: self.analysis.is_some(),
            duration_sec: self.analysis.as_ref().map(|a| a.duration_sec),
            key: self.analysis.as_ref().map(|a| a.key.clone()),
        }
    }

    /// Range to process, in master-audio time.
    pub fn effective_range(&self) -> Option<(f64, f64)> {
        let a = self.analysis.as_ref()?;
        let r = &self.settings.range;
        let take = r
            .take
            .and_then(|i| a.takes.iter().find(|t| t.index == i))
            .or_else(|| {
                a.best_take
                    .and_then(|i| a.takes.iter().find(|t| t.index == i))
            });
        let (mut s, mut e) = take.map_or((0.0, a.duration_sec), |t| (t.start_sec, t.end_sec));
        if let Some(v) = r.start_sec {
            s = v;
        }
        if let Some(v) = r.end_sec {
            e = v;
        }
        let s = s.clamp(0.0, a.duration_sec);
        let e = e.clamp(s, a.duration_sec);
        (e - s > 0.5).then_some((s, e))
    }

    /// `video_time - audio_time`.
    pub fn sync_offset(&self) -> f64 {
        if let Some(o) = self.settings.sync_offset_sec {
            return o;
        }
        self.analysis
            .as_ref()
            .and_then(|a| a.sync.as_ref())
            .map_or(0.0, |s| s.offset_sec)
    }
}

pub fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s = s
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() {
        "cover".into()
    } else {
        s
    }
}
