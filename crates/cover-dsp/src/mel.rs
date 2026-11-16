//! Mel filterbank compatible with `librosa.filters.mel` (Slaney normalisation).

fn hz_to_mel(f: f64, htk: bool) -> f64 {
    if htk {
        return 2595.0 * (1.0 + f / 700.0).log10();
    }
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1000.0;
    let min_log_mel = min_log_hz / f_sp;
    let logstep = 6.4f64.ln() / 27.0;
    if f >= min_log_hz {
        min_log_mel + (f / min_log_hz).ln() / logstep
    } else {
        f / f_sp
    }
}

fn mel_to_hz(m: f64, htk: bool) -> f64 {
    if htk {
        return 700.0 * (10f64.powf(m / 2595.0) - 1.0);
    }
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1000.0;
    let min_log_mel = min_log_hz / f_sp;
    let logstep = 6.4f64.ln() / 27.0;
    if m >= min_log_mel {
        min_log_hz * (logstep * (m - min_log_mel)).exp()
    } else {
        f_sp * m
    }
}

/// Filterbank stored row-major: `weights[mel * bins + bin]`.
#[derive(Debug, Clone)]
pub struct MelBank {
    pub n_mels: usize,
    pub bins: usize,
    pub weights: Vec<f32>,
}

impl MelBank {
    pub fn new(sr: u32, n_fft: usize, n_mels: usize, fmin: f32, fmax: f32, htk: bool) -> Self {
        let bins = n_fft / 2 + 1;
        let fftfreqs: Vec<f64> = (0..bins)
            .map(|i| i as f64 * sr as f64 / n_fft as f64)
            .collect();
        let mmin = hz_to_mel(fmin as f64, htk);
        let mmax = hz_to_mel(fmax as f64, htk);
        let mel_f: Vec<f64> = (0..n_mels + 2)
            .map(|i| mel_to_hz(mmin + (mmax - mmin) * i as f64 / (n_mels + 1) as f64, htk))
            .collect();
        let mut weights = vec![0.0f32; n_mels * bins];
        for m in 0..n_mels {
            let (lo, ce, hi) = (mel_f[m], mel_f[m + 1], mel_f[m + 2]);
            let enorm = 2.0 / (hi - lo);
            for (b, &f) in fftfreqs.iter().enumerate() {
                let lower = (f - lo) / (ce - lo);
                let upper = (hi - f) / (hi - ce);
                let w = lower.min(upper).max(0.0);
                weights[m * bins + b] = (w * enorm) as f32;
            }
        }
        Self {
            n_mels,
            bins,
            weights,
        }
    }

    /// Applies the bank to one magnitude (or power) frame.
    pub fn apply(&self, frame: &[f32], out: &mut [f32]) {
        for (m, o) in out.iter_mut().enumerate().take(self.n_mels) {
            let row = &self.weights[m * self.bins..(m + 1) * self.bins];
            *o = row.iter().zip(frame).map(|(w, x)| w * x).sum();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slaney_mel_scale_is_continuous_at_1khz() {
        let a = hz_to_mel(999.999, false);
        let b = hz_to_mel(1000.0, false);
        assert!((a - b).abs() < 1e-3);
        assert!((mel_to_hz(hz_to_mel(3000.0, false), false) - 3000.0).abs() < 1e-6);
    }

    #[test]
    fn bank_shape() {
        let bank = MelBank::new(16_000, 1024, 128, 30.0, 8000.0, true);
        assert_eq!(bank.weights.len(), 128 * 513);
        assert!(bank.weights.iter().all(|w| *w >= 0.0));
    }
}
