//! RBJ "Audio EQ Cookbook" biquads and a serialisable EQ chain.

use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FilterKind {
    Lowpass { freq: f32, q: f32 },
    Highpass { freq: f32, q: f32 },
    Bandpass { freq: f32, q: f32 },
    Peak { freq: f32, q: f32, gain_db: f32 },
    LowShelf { freq: f32, q: f32, gain_db: f32 },
    HighShelf { freq: f32, q: f32, gain_db: f32 },
}

#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    pub fn new(kind: FilterKind, sample_rate: u32) -> Self {
        let fs = sample_rate as f64;
        let (freq, q, gain_db) = match kind {
            FilterKind::Lowpass { freq, q }
            | FilterKind::Highpass { freq, q }
            | FilterKind::Bandpass { freq, q } => (freq, q, 0.0),
            FilterKind::Peak { freq, q, gain_db }
            | FilterKind::LowShelf { freq, q, gain_db }
            | FilterKind::HighShelf { freq, q, gain_db } => (freq, q, gain_db),
        };
        let f = (freq as f64).clamp(1.0, fs * 0.49);
        let q = (q as f64).max(0.05);
        let a = 10f64.powf(gain_db as f64 / 40.0);
        let w0 = 2.0 * PI * f / fs;
        let (sw, cw) = w0.sin_cos();
        let alpha = sw / (2.0 * q);

        let (b0, b1, b2, a0, a1, a2) = match kind {
            FilterKind::Lowpass { .. } => (
                (1.0 - cw) / 2.0,
                1.0 - cw,
                (1.0 - cw) / 2.0,
                1.0 + alpha,
                -2.0 * cw,
                1.0 - alpha,
            ),
            FilterKind::Highpass { .. } => (
                (1.0 + cw) / 2.0,
                -(1.0 + cw),
                (1.0 + cw) / 2.0,
                1.0 + alpha,
                -2.0 * cw,
                1.0 - alpha,
            ),
            FilterKind::Bandpass { .. } => {
                (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cw, 1.0 - alpha)
            }
            FilterKind::Peak { .. } => (
                1.0 + alpha * a,
                -2.0 * cw,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cw,
                1.0 - alpha / a,
            ),
            FilterKind::LowShelf { .. } => {
                let k = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cw + k),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cw),
                    a * ((a + 1.0) - (a - 1.0) * cw - k),
                    (a + 1.0) + (a - 1.0) * cw + k,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cw),
                    (a + 1.0) + (a - 1.0) * cw - k,
                )
            }
            FilterKind::HighShelf { .. } => {
                let k = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cw + k),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cw),
                    a * ((a + 1.0) + (a - 1.0) * cw - k),
                    (a + 1.0) - (a - 1.0) * cw + k,
                    2.0 * ((a - 1.0) - (a + 1.0) * cw),
                    (a + 1.0) - (a - 1.0) * cw - k,
                )
            }
        };
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let x = x as f64;
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y as f32
    }

    pub fn process(&mut self, x: &mut [f32]) {
        for s in x.iter_mut() {
            *s = self.tick(*s);
        }
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// Applies a chain of filters to a mono signal and returns the result.
pub fn apply_chain(x: &[f32], sample_rate: u32, chain: &[FilterKind]) -> Vec<f32> {
    let mut out = x.to_vec();
    for kind in chain {
        Biquad::new(*kind, sample_rate).process(&mut out);
    }
    out
}

/// Zero-phase filtering (forward then backward), like `scipy.signal.filtfilt`.
pub fn filtfilt(x: &[f32], sample_rate: u32, kind: FilterKind) -> Vec<f32> {
    let mut y = x.to_vec();
    Biquad::new(kind, sample_rate).process(&mut y);
    y.reverse();
    Biquad::new(kind, sample_rate).process(&mut y);
    y.reverse();
    y
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, sr: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sr as f32).sin())
            .collect()
    }

    fn rms_tail(x: &[f32]) -> f32 {
        crate::util::rms(&x[x.len() / 2..])
    }

    #[test]
    fn highpass_kills_lows_keeps_highs() {
        let sr = 48_000;
        let hp = FilterKind::Highpass {
            freq: 1000.0,
            q: 0.707,
        };
        let low = apply_chain(&sine(50.0, sr, sr as usize), sr, &[hp]);
        let high = apply_chain(&sine(8000.0, sr, sr as usize), sr, &[hp]);
        assert!(rms_tail(&low) < 0.01);
        assert!((rms_tail(&high) - 0.707).abs() < 0.02);
    }

    #[test]
    fn peak_boost_matches_gain() {
        let sr = 48_000;
        let pk = FilterKind::Peak {
            freq: 1000.0,
            q: 1.0,
            gain_db: 6.0,
        };
        let y = apply_chain(&sine(1000.0, sr, sr as usize), sr, &[pk]);
        let gain_db = crate::util::gain_to_db(rms_tail(&y) / 0.70710677);
        assert!((gain_db - 6.0).abs() < 0.1, "{gain_db}");
    }

    #[test]
    fn shelves_reach_gain() {
        let sr = 48_000;
        let ls = FilterKind::LowShelf {
            freq: 200.0,
            q: 0.707,
            gain_db: -4.0,
        };
        let y = apply_chain(&sine(30.0, sr, sr as usize), sr, &[ls]);
        let gain_db = crate::util::gain_to_db(rms_tail(&y) / 0.70710677);
        assert!((gain_db + 4.0).abs() < 0.3, "{gain_db}");
    }
}
