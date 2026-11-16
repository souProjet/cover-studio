//! Compressor, look-ahead limiter and de-esser. Multi-channel processing is
//! always stereo-linked so the image does not wander.

use crate::filter::{Biquad, FilterKind};
use crate::util::{db_to_gain, gain_to_db};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

fn time_coef(ms: f32, sample_rate: u32) -> f32 {
    if ms <= 0.0 {
        0.0
    } else {
        (-1.0 / (ms * 0.001 * sample_rate as f32)).exp()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CompressorSettings {
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub knee_db: f32,
    pub makeup_db: f32,
}

impl Default for CompressorSettings {
    fn default() -> Self {
        Self {
            threshold_db: -18.0,
            ratio: 2.0,
            attack_ms: 15.0,
            release_ms: 180.0,
            knee_db: 6.0,
            makeup_db: 0.0,
        }
    }
}

impl CompressorSettings {
    fn gain_reduction_db(&self, level_db: f32) -> f32 {
        let over = level_db - self.threshold_db;
        let slope = 1.0 - 1.0 / self.ratio.max(1.0);
        let k = self.knee_db.max(0.0);
        if k > 0.0 && over.abs() <= k / 2.0 {
            -slope * (over + k / 2.0).powi(2) / (2.0 * k)
        } else if over > 0.0 {
            -slope * over
        } else {
            0.0
        }
    }
}

/// Feed-forward compressor with a peak detector and smoothed gain.
pub fn compress(channels: &mut [Vec<f32>], sample_rate: u32, s: &CompressorSettings) {
    let n = channels.iter().map(Vec::len).min().unwrap_or(0);
    let att = time_coef(s.attack_ms, sample_rate);
    let rel = time_coef(s.release_ms, sample_rate);
    let makeup = s.makeup_db;
    let mut gr = 0.0f32;
    for i in 0..n {
        let peak = channels.iter().fold(0.0f32, |m, c| m.max(c[i].abs()));
        let target = s.gain_reduction_db(gain_to_db(peak));
        let coef = if target < gr { att } else { rel };
        gr = target + coef * (gr - target);
        let g = db_to_gain(gr + makeup);
        for c in channels.iter_mut() {
            c[i] *= g;
        }
    }
}

pub fn compress_mono(x: &mut Vec<f32>, sample_rate: u32, s: &CompressorSettings) {
    compress(std::slice::from_mut(x), sample_rate, s);
}

/// Look-ahead brickwall limiter.
///
/// Gain is computed with a sliding-minimum hold followed by a boxcar of the
/// same length, which guarantees the delayed signal never exceeds the ceiling
/// while keeping the gain curve smooth.
pub fn limit(channels: &mut [Vec<f32>], sample_rate: u32, ceiling_db: f32, release_ms: f32) {
    let n = channels.iter().map(Vec::len).min().unwrap_or(0);
    if n == 0 {
        return;
    }
    let ceiling = db_to_gain(ceiling_db);
    let look = ((0.005 * sample_rate as f32) as usize).max(1);
    let rel = time_coef(release_ms, sample_rate);

    let raw: Vec<f32> = (0..n)
        .map(|i| {
            let p = channels.iter().fold(0.0f32, |m, c| m.max(c[i].abs()));
            if p > ceiling {
                ceiling / p
            } else {
                1.0
            }
        })
        .collect();

    // Sliding minimum over `look` samples (monotonic deque), then release smoothing.
    let mut hold = vec![1.0f32; n + look];
    let mut dq: VecDeque<usize> = VecDeque::new();
    let mut released = 1.0f32;
    for k in 0..n + look {
        let v = if k < n { raw[k] } else { 1.0 };
        while dq.back().is_some_and(|&j| {
            let vj = if j < n { raw[j] } else { 1.0 };
            vj >= v
        }) {
            dq.pop_back();
        }
        dq.push_back(k);
        while dq.front().is_some_and(|&j| j + look <= k) {
            dq.pop_front();
        }
        let j = *dq.front().unwrap();
        let m = if j < n { raw[j] } else { 1.0 };
        released = if m < released {
            m
        } else {
            m + rel * (released - m)
        };
        hold[k] = released.min(m);
    }

    // Boxcar average of length `look`, output delayed by `look - 1`.
    let mut smooth = vec![1.0f32; n + look];
    let mut acc = 0.0f64;
    for k in 0..n + look {
        acc += hold[k] as f64;
        if k >= look {
            acc -= hold[k - look] as f64;
        }
        let count = (k + 1).min(look) as f64;
        smooth[k] = (acc / count) as f32;
    }
    let delay = look - 1;
    for c in channels.iter_mut() {
        for i in 0..n {
            c[i] = (c[i] * smooth[i + delay]).clamp(-ceiling, ceiling);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DeEsserSettings {
    pub freq: f32,
    pub threshold_db: f32,
    pub max_reduction_db: f32,
}

impl Default for DeEsserSettings {
    fn default() -> Self {
        Self {
            freq: 5500.0,
            threshold_db: -28.0,
            max_reduction_db: 8.0,
        }
    }
}

/// Split-band de-esser: only the high band is attenuated when sibilance is detected.
pub fn de_ess(x: &mut [f32], sample_rate: u32, s: &DeEsserSettings) {
    let mut split = Biquad::new(
        FilterKind::Highpass {
            freq: s.freq,
            q: 0.707,
        },
        sample_rate,
    );
    let att = time_coef(1.5, sample_rate);
    let rel = time_coef(60.0, sample_rate);
    let mut env = 0.0f32;
    for v in x.iter_mut() {
        let high = split.tick(*v);
        let low = *v - high;
        let a = high.abs();
        let c = if a > env { att } else { rel };
        env = a + c * (env - a);
        let over = gain_to_db(env) - s.threshold_db;
        let red = if over > 0.0 {
            (over * 0.6).min(s.max_reduction_db)
        } else {
            0.0
        };
        *v = low + high * db_to_gain(-red);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_respects_ceiling() {
        let sr = 48_000;
        let mut ch = vec![(0..sr)
            .map(|i| 1.8 * (i as f32 * 0.05).sin())
            .collect::<Vec<f32>>()];
        limit(&mut ch, sr as u32, -1.0, 100.0);
        let peak = ch[0].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak <= db_to_gain(-1.0) + 1e-6, "{peak}");
        assert!(peak > 0.8);
    }

    #[test]
    fn compressor_reduces_loud_signal() {
        let sr = 48_000u32;
        let mut ch = vec![vec![0.9f32; sr as usize]];
        compress(
            &mut ch,
            sr,
            &CompressorSettings {
                threshold_db: -20.0,
                ratio: 4.0,
                knee_db: 0.0,
                ..Default::default()
            },
        );
        let out_db = gain_to_db(*ch[0].last().unwrap());
        let expected = -20.0 + (gain_to_db(0.9) + 20.0) / 4.0;
        assert!((out_db - expected).abs() < 0.2, "{out_db} vs {expected}");
    }
}
