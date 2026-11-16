//! Aligns an external audio recording with the audio track of a video.
//!
//! Convention used everywhere in the engine: `offset_sec = video_time - audio_time`,
//! i.e. the external audio's t = 0 happens `offset_sec` seconds into the video.

use cover_dsp::analysis::onset::{detect_clap, onset_strength, ONSET_HOP};
use cover_dsp::analysis::rms_envelope;
use cover_dsp::analysis::xcorr::best_lag;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMethod {
    Correlation,
    ClapAndCorrelation,
    Clap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncResult {
    pub offset_sec: f64,
    /// 0..1, how much the two recordings agree at the chosen offset.
    pub confidence: f32,
    /// Raw onset-envelope correlation at the best lag.
    pub correlation: f32,
    pub method: SyncMethod,
    pub clap_video_sec: Option<f64>,
    pub clap_audio_sec: Option<f64>,
}

const FINE_HOP: usize = 32;

fn log_env(x: &[f32], hop: usize) -> Vec<f32> {
    rms_envelope(x, hop)
        .into_iter()
        .map(|v| (1.0 + 200.0 * v).ln())
        .collect()
}

/// Lag in samples such that `audio[n + lag] ≈ video[n]` (positive: audio is late).
fn refine(video: &[f32], audio: &[f32], coarse_lag: isize, sr: u32) -> isize {
    let (v, a) = if coarse_lag >= 0 {
        let s = (coarse_lag as usize).min(audio.len());
        (video, &audio[s..])
    } else {
        let s = ((-coarse_lag) as usize).min(video.len());
        (&video[s..], audio)
    };
    let n = v.len().min(a.len()).min(sr as usize * 120);
    if n < sr as usize {
        return coarse_lag;
    }
    let ev = log_env(&v[..n], FINE_HOP);
    let ea = log_env(&a[..n], FINE_HOP);
    let max = (3 * ONSET_HOP / FINE_HOP).max(4);
    let (fine, _) = best_lag(&ev, &ea, max);
    coarse_lag + fine * FINE_HOP as isize
}

pub fn analyze_sync(video_audio: &[f32], audio: &[f32], sr: u32) -> SyncResult {
    let ov = onset_strength(video_audio, sr);
    let oa = onset_strength(audio, sr);
    let max_lag = ov.len().max(oa.len());
    let (lag_frames, score) = best_lag(&ov, &oa, max_lag);
    let coarse = lag_frames * ONSET_HOP as isize;
    let lag = refine(video_audio, audio, coarse, sr);
    let corr_offset = -(lag as f64) / sr as f64;

    let clap_v = detect_clap(video_audio, sr, 60.0).filter(|c| c.1 > 4.0);
    let clap_a = detect_clap(audio, sr, 60.0).filter(|c| c.1 > 4.0);
    let clap_offset = match (clap_v, clap_a) {
        (Some(v), Some(a)) => Some(v.0 - a.0),
        _ => None,
    };

    let mut result = SyncResult {
        offset_sec: corr_offset,
        confidence: score.clamp(0.0, 1.0),
        correlation: score,
        method: SyncMethod::Correlation,
        clap_video_sec: clap_v.map(|c| c.0),
        clap_audio_sec: clap_a.map(|c| c.0),
    };
    if let Some(co) = clap_offset {
        if (co - corr_offset).abs() < 0.1 {
            result.method = SyncMethod::ClapAndCorrelation;
            result.confidence = (result.confidence + 0.15).min(1.0);
        } else if score < 0.2 {
            // Matching recordings correlate well above 0.5; below 0.2 they are
            // probably different performances, so the clap is only a guess.
            result.offset_sec = co;
            result.method = SyncMethod::Clap;
            result.confidence = 0.15;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_offset_between_two_mics() {
        let sr = 22_050u32;
        // A "performance": decaying plucks at irregular times.
        let mut perf = vec![0.0f32; sr as usize * 30];
        let mut seed = 7u32;
        let mut t = sr as usize;
        while t < perf.len() - sr as usize {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let f = 150.0 + (seed >> 20) as f32 % 400.0;
            for k in 0..sr as usize / 3 {
                perf[t + k] += (-(k as f32) / 3000.0).exp()
                    * (2.0 * std::f32::consts::PI * f * k as f32 / sr as f32).sin();
            }
            t += sr as usize / 4 + (seed >> 16) as usize % (sr as usize / 2);
        }
        // Video starts 2.37 s before the external recorder.
        let offset = 2.37f64;
        let pad = (offset * sr as f64) as usize;
        let mut video = vec![0.0f32; pad];
        video.extend(perf.iter().map(|v| v * 0.3 + 0.001));
        let audio = perf.clone();
        let r = analyze_sync(&video, &audio, sr);
        assert!((r.offset_sec - offset).abs() < 0.003, "{r:?}");
        assert!(r.confidence > 0.5, "{r:?}");
    }
}
