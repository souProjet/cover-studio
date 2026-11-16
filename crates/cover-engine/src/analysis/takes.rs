//! Finds the separate takes in a session recording and ranks them.

use super::tuning::Scale;
use cover_dsp::analysis::activity::{active_regions, ActivityParams};
use cover_dsp::util::hz_to_midi;
use cover_ml::PitchTrack;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Take {
    pub index: usize,
    pub start_sec: f64,
    pub end_sec: f64,
    /// First sustained singing inside the take, if vocals were found.
    pub vocal_start_sec: Option<f64>,
    /// Mean absolute deviation of stable sung notes from the scale, in cents.
    pub tuning_cents: Option<f32>,
    /// Fraction of samples at or near full scale.
    pub clipping: f32,
    /// 0..1, higher is better.
    pub score: f32,
}

impl Take {
    pub fn duration(&self) -> f64 {
        self.end_sec - self.start_sec
    }
}

fn clipping_ratio(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    x.iter().filter(|v| v.abs() > 0.989).count() as f32 / x.len() as f32
}

fn tuning_and_start(
    pitch: &PitchTrack,
    scale: &Scale,
    start: f64,
    end: f64,
) -> (Option<f32>, Option<f64>) {
    let a = pitch.frame_at(start);
    let b = pitch.frame_at(end).max(a);
    let mut devs = Vec::new();
    let mut run = 0usize;
    let mut vocal_start = None;
    let min_run = (0.3 / pitch.hop_sec) as usize;
    for i in a..b {
        let voiced = pitch.f0_hz[i] > 0.0 && pitch.confidence[i] > 0.5;
        if voiced {
            run += 1;
            if run == min_run && vocal_start.is_none() {
                vocal_start = Some((i + 1 - min_run) as f64 * pitch.hop_sec);
            }
        } else {
            run = 0;
            continue;
        }
        if i == 0 || i + 1 >= pitch.f0_hz.len() {
            continue;
        }
        let (p, c, n) = (pitch.f0_hz[i - 1], pitch.f0_hz[i], pitch.f0_hz[i + 1]);
        if p <= 0.0 || n <= 0.0 {
            continue;
        }
        // Only steady frames: glides and vibrato peaks would penalise good singers.
        if (hz_to_midi(n) - hz_to_midi(p)).abs() > 0.15 {
            continue;
        }
        let m = hz_to_midi(c);
        devs.push(((m - scale.nearest(m)) * 100.0).abs());
    }
    let tuning = (devs.len() > 50).then(|| devs.iter().sum::<f32>() / devs.len() as f32);
    (tuning, vocal_start)
}

/// `mix` is the full recording; `pitch` comes from the separated vocals.
pub fn detect_takes(mix: &[f32], sr: u32, pitch: Option<&PitchTrack>, scale: &Scale) -> Vec<Take> {
    let duration = mix.len() as f64 / sr as f64;
    let params = ActivityParams {
        min_gap_sec: 3.0,
        min_region_sec: (duration * 0.15).clamp(8.0, 30.0),
        ..ActivityParams::default()
    };
    let mut regions = active_regions(mix, sr, &params);
    if regions.is_empty() {
        regions.push(cover_dsp::analysis::activity::Region {
            start_sec: 0.0,
            end_sec: duration,
        });
    }
    // Small margins keep the attack of the first note and the ring-out of the last.
    let longest = regions
        .iter()
        .map(|r| r.duration())
        .fold(0.0, f64::max)
        .max(1e-6);
    let n = regions.len();
    let mut takes: Vec<Take> = regions
        .into_iter()
        .enumerate()
        .map(|(index, r)| {
            let start_sec = (r.start_sec - 0.4).max(0.0);
            let end_sec = (r.end_sec + 1.2).min(duration);
            let i0 = (start_sec * sr as f64) as usize;
            let i1 = ((end_sec * sr as f64) as usize).min(mix.len());
            let clipping = clipping_ratio(&mix[i0..i1]);
            let (tuning_cents, vocal_start_sec) = pitch
                .map(|p| tuning_and_start(p, scale, start_sec, end_sec))
                .unwrap_or((None, None));
            let completeness = (r.duration() / longest) as f32;
            let tuning_score = tuning_cents.map_or(0.5, |c| (1.0 - c / 35.0).clamp(0.0, 1.0));
            let recency = if n > 1 {
                index as f32 / (n - 1) as f32
            } else {
                1.0
            };
            let score = 0.5 * completeness
                + 0.3 * tuning_score
                + 0.1 * (1.0 - (clipping * 1000.0).min(1.0))
                + 0.1 * recency;
            Take {
                index,
                start_sec,
                end_sec,
                vocal_start_sec,
                tuning_cents,
                clipping,
                score,
            }
        })
        .collect();
    takes.sort_by_key(|t| t.index);
    takes
}

pub fn best_take(takes: &[Take]) -> Option<&Take> {
    takes.iter().max_by(|a, b| a.score.total_cmp(&b.score))
}
