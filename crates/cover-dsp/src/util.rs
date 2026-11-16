//! Small numeric helpers shared across the crate.

pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub fn gain_to_db(gain: f32) -> f32 {
    20.0 * gain.abs().max(1e-12).log10()
}

pub fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>() / x.len() as f64).sqrt() as f32
}

/// Scales `signal` so its RMS matches `reference`.
pub fn match_rms(reference: &[f32], signal: &[f32]) -> Vec<f32> {
    let r_ref = rms(reference);
    let r_sig = rms(signal);
    if r_sig < 1e-9 {
        return signal.to_vec();
    }
    let g = r_ref / r_sig;
    signal.iter().map(|s| s * g).collect()
}

/// A 0..1 mask that is 1 on `[i0, i1)` with linear fades of `fade` samples on both edges.
pub fn window_mask(len: usize, i0: usize, i1: usize, fade: usize) -> Vec<f32> {
    let mut w = vec![0.0f32; len];
    let i1 = i1.min(len);
    if i1 <= i0 {
        return w;
    }
    for v in &mut w[i0..i1] {
        *v = 1.0;
    }
    let m = fade.min((i1 - i0) / 2);
    for k in 0..m {
        let r = k as f32 / m as f32;
        w[i0 + k] = r;
        w[i1 - 1 - k] = r;
    }
    w
}

/// Linear fade in/out applied in place.
pub fn apply_fades(x: &mut [f32], fade_in: usize, fade_out: usize) {
    let n = x.len();
    let fi = fade_in.min(n);
    for (k, s) in x.iter_mut().take(fi).enumerate() {
        *s *= k as f32 / fi as f32;
    }
    let fo = fade_out.min(n);
    for k in 0..fo {
        x[n - 1 - k] *= k as f32 / fo as f32;
    }
}

/// Midi note number (fractional) from a frequency in Hz.
pub fn hz_to_midi(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

pub fn midi_to_hz(midi: f32) -> f32 {
    440.0 * 2f32.powf((midi - 69.0) / 12.0)
}

/// Median of a slice (copy is sorted, input untouched). Returns 0 on empty input.
pub fn median(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    let mut v = x.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    let m = v.len() / 2;
    if v.len().is_multiple_of(2) {
        0.5 * (v[m - 1] + v[m])
    } else {
        v[m]
    }
}

pub fn mean(x: &[f32]) -> f32 {
    if x.is_empty() {
        0.0
    } else {
        x.iter().sum::<f32>() / x.len() as f32
    }
}

pub fn std_dev(x: &[f32]) -> f32 {
    if x.len() < 2 {
        return 0.0;
    }
    let m = mean(x);
    (x.iter().map(|v| (v - m) * (v - m)).sum::<f32>() / x.len() as f32).sqrt()
}

/// Sliding median filter with an odd kernel (edges use a shrunken window).
pub fn median_filter(x: &[f32], kernel: usize) -> Vec<f32> {
    let k = kernel.max(1) | 1;
    let h = k / 2;
    (0..x.len())
        .map(|i| {
            let a = i.saturating_sub(h);
            let b = (i + h + 1).min(x.len());
            median(&x[a..b])
        })
        .collect()
}

/// Percentile in 0..=100 using nearest rank.
pub fn percentile(x: &[f32], p: f32) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    let mut v = x.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    let idx = ((p / 100.0).clamp(0.0, 1.0) * (v.len() - 1) as f32).round() as usize;
    v[idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_roundtrip() {
        assert!((gain_to_db(db_to_gain(-6.0)) + 6.0).abs() < 1e-4);
    }

    #[test]
    fn midi_roundtrip() {
        assert!((hz_to_midi(440.0) - 69.0).abs() < 1e-5);
        assert!((midi_to_hz(60.0) - 261.6256).abs() < 1e-2);
    }

    #[test]
    fn mask_has_fades() {
        let w = window_mask(10, 2, 8, 2);
        assert_eq!(w[0], 0.0);
        assert_eq!(w[2], 0.0);
        assert!(w[3] > 0.0 && w[3] < 1.0);
        assert_eq!(w[5], 1.0);
        assert_eq!(w[9], 0.0);
    }

    #[test]
    fn median_works() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
    }
}
