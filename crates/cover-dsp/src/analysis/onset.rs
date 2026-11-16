//! Spectral-flux onset strength on a log-mel spectrogram (similar to
//! `librosa.onset.onset_strength`).

use super::find_peaks;
use crate::mel::MelBank;
use crate::stft::stft;
use crate::util::percentile;

pub const ONSET_N_FFT: usize = 2048;
pub const ONSET_HOP: usize = 512;

/// Onset strength envelope, one value per `ONSET_HOP` samples.
pub fn onset_strength(x: &[f32], sample_rate: u32) -> Vec<f32> {
    if x.len() < ONSET_N_FFT {
        return Vec::new();
    }
    let spec = stft(x, ONSET_N_FFT, ONSET_HOP);
    let bank = MelBank::new(
        sample_rate,
        ONSET_N_FFT,
        128,
        0.0,
        sample_rate as f32 / 2.0,
        false,
    );
    let mut prev = vec![0.0f32; 128];
    let mut cur = vec![0.0f32; 128];
    let mut power = vec![0.0f32; spec.bins];
    let mut out = Vec::with_capacity(spec.frames);
    for f in 0..spec.frames {
        for (p, c) in power.iter_mut().zip(spec.frame(f)) {
            *p = c.norm_sqr();
        }
        bank.apply(&power, &mut cur);
        for v in cur.iter_mut() {
            // power_to_db with ref=1, top_db=80 is approximated by a floor.
            *v = 10.0 * v.max(1e-10).log10();
        }
        let flux = if f == 0 {
            0.0
        } else {
            cur.iter()
                .zip(&prev)
                .map(|(c, p)| (c - p).max(0.0))
                .sum::<f32>()
                / 128.0
        };
        out.push(flux);
        std::mem::swap(&mut prev, &mut cur);
    }
    out
}

/// Onset times in seconds.
pub fn detect_onsets(x: &[f32], sample_rate: u32, sensitivity: f32) -> Vec<f64> {
    let env = onset_strength(x, sample_rate);
    if env.is_empty() {
        return Vec::new();
    }
    let thr = percentile(&env, (90.0 - 30.0 * sensitivity.clamp(0.0, 1.0)).max(40.0));
    let frame_sec = ONSET_HOP as f64 / sample_rate as f64;
    find_peaks(&env, thr, 3)
        .into_iter()
        .map(|i| i as f64 * frame_sec)
        .collect()
}

/// Strongest transient (e.g. a hand clap) within the first `max_sec` seconds.
///
/// Returns `(time_sec, prominence)` where prominence compares the peak to the
/// median onset strength; a real clap is usually well above 4.
pub fn detect_clap(x: &[f32], sample_rate: u32, max_sec: f64) -> Option<(f64, f32)> {
    let n = ((max_sec * sample_rate as f64) as usize).min(x.len());
    let env = onset_strength(&x[..n], sample_rate);
    if env.len() < 8 {
        return None;
    }
    let (best, &peak) = env.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
    let med = crate::util::median(&env).max(1e-6);
    let frame_sec = ONSET_HOP as f64 / sample_rate as f64;
    Some((best as f64 * frame_sec, peak / med))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_click() {
        let sr = 22_050u32;
        let mut x: Vec<f32> = (0..sr * 4)
            .map(|i| 0.01 * ((i as f32) * 0.37).sin())
            .collect();
        let at = (sr as f32 * 2.5) as usize;
        for (k, v) in x.iter_mut().skip(at).take(400).enumerate() {
            *v += 0.9 * (-(k as f32) / 60.0).exp() * if k % 2 == 0 { 1.0 } else { -1.0 };
        }
        let (t, prom) = detect_clap(&x, sr, 10.0).unwrap();
        assert!((t - 2.5).abs() < 0.05, "{t}");
        assert!(prom > 4.0);
    }
}
