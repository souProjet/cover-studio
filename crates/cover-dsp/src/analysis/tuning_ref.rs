//! Tuning reference of a recording: how far its instruments sit from the
//! A = 440 Hz equal-tempered grid. A home guitar tuned 15 cents flat should
//! pull the vocal correction with it, not fight it.

use crate::stft::stft;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TuningReference {
    /// Offset from the A440 grid, in cents (-50..50).
    pub cents: f32,
    /// 0..1 agreement between spectral peaks; above ~0.3 is reliable.
    pub confidence: f32,
}

/// Magnitude-weighted circular mean of the fractional pitch of strong
/// spectral peaks between 80 Hz and 2 kHz.
pub fn tuning_reference(x: &[f32], sample_rate: u32) -> Option<TuningReference> {
    let n_fft = 16_384usize;
    if x.len() < n_fft * 2 {
        return None;
    }
    let s = stft(x, n_fft, n_fft / 4);
    let bin_hz = sample_rate as f32 / n_fft as f32;
    let (lo, hi) = ((80.0 / bin_hz) as usize, (2000.0 / bin_hz) as usize);
    let (mut sx, mut sy, mut sw) = (0.0f64, 0.0f64, 0.0f64);
    for f in 0..s.frames {
        let mag: Vec<f32> = s.frame(f)[..hi + 2].iter().map(|c| c.norm()).collect();
        let peak = mag[lo..hi].iter().copied().fold(0.0f32, f32::max);
        if peak <= 1e-6 {
            continue;
        }
        let thr = peak * 0.05;
        for k in lo.max(1)..hi {
            let (a, b, c) = (mag[k - 1], mag[k], mag[k + 1]);
            if b < thr || b <= a || b <= c {
                continue;
            }
            let (la, lb, lc) = (a.max(1e-12).ln(), b.ln(), c.max(1e-12).ln());
            let denom = la - 2.0 * lb + lc;
            let d = if denom.abs() > 1e-9 {
                0.5 * (la - lc) / denom
            } else {
                0.0
            };
            let hz = (k as f32 + d) * bin_hz;
            let midi = 69.0 + 12.0 * (hz / 440.0).log2();
            let phase = 2.0 * std::f64::consts::PI * midi as f64;
            sx += b as f64 * phase.cos();
            sy += b as f64 * phase.sin();
            sw += b as f64;
        }
    }
    if sw <= 0.0 {
        return None;
    }
    Some(TuningReference {
        cents: (sy.atan2(sx) / (2.0 * std::f64::consts::PI) * 100.0) as f32,
        confidence: ((sx * sx + sy * sy).sqrt() / sw) as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn finds_a_flat_guitar() {
        let sr = 44_100u32;
        // Chords of harmonic tones tuned 18 cents flat.
        let detune = 2f32.powf(-0.18 / 12.0);
        let mut x = vec![0.0f32; sr as usize * 4];
        for (seg, chord) in [[110.0, 164.81, 220.0], [146.83, 220.0, 293.66]]
            .iter()
            .enumerate()
        {
            for i in 0..sr as usize * 2 {
                let t = i as f32 / sr as f32;
                x[seg * sr as usize * 2 + i] = chord
                    .iter()
                    .map(|f| {
                        (1..6)
                            .map(|h| (2.0 * PI * f * detune * h as f32 * t).sin() / h as f32)
                            .sum::<f32>()
                    })
                    .sum::<f32>()
                    * 0.1;
            }
        }
        let r = tuning_reference(&x, sr).unwrap();
        assert!((r.cents + 18.0).abs() < 3.0, "{r:?}");
        assert!(r.confidence > 0.3, "{r:?}");
    }
}
