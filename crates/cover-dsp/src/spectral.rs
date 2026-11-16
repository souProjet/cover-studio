//! Spectral clean-up for a vocal stem: a gentle noise gate driven by a
//! noise profile learnt from the quiet frames, and a resonance suppressor
//! that only cuts regions of the spectral envelope rising above the
//! singer's usual tonal balance. Gains are linked across channels.

use crate::stft::hann_periodic;
use crate::util::percentile;
use realfft::num_complex::Complex32;
use realfft::RealFftPlanner;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpectralCleanSettings {
    /// 0..1 noise reduction (floor at about -4 to -12 dB).
    pub denoise: f32,
    /// 0..1 resonance suppression between 250 Hz and 10 kHz.
    pub resonance: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SpectralCleanReport {
    /// Active level over the noise floor, in dB.
    pub snr_db: f32,
    pub denoised: bool,
}

struct Framer {
    n_fft: usize,
    hop: usize,
    window: Vec<f32>,
    fft: std::sync::Arc<dyn realfft::RealToComplex<f32>>,
    ifft: std::sync::Arc<dyn realfft::ComplexToReal<f32>>,
}

impl Framer {
    fn new(n_fft: usize) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        Self {
            n_fft,
            hop: n_fft / 4,
            window: hann_periodic(n_fft),
            fft: planner.plan_fft_forward(n_fft),
            ifft: planner.plan_fft_inverse(n_fft),
        }
    }

    fn frames(&self, n: usize) -> usize {
        n.div_ceil(self.hop) + 1
    }

    /// Frame `f` is centred on sample `f * hop`.
    fn analyse(&self, x: &[f32], f: usize, out: &mut [Complex32]) {
        let start = (f * self.hop) as isize - (self.n_fft / 2) as isize;
        let mut buf: Vec<f32> = (0..self.n_fft)
            .map(|i| {
                let j = start + i as isize;
                if j >= 0 && (j as usize) < x.len() {
                    x[j as usize] * self.window[i]
                } else {
                    0.0
                }
            })
            .collect();
        self.fft
            .process(&mut buf, out)
            .expect("fft sizes are fixed by the planner");
    }
}

fn bins_power(spec: &[Vec<Complex32>]) -> Vec<f32> {
    let bins = spec[0].len();
    let k = spec.len() as f32;
    (0..bins)
        .map(|b| spec.iter().map(|s| s[b].norm_sqr()).sum::<f32>() / k)
        .collect()
}

/// dB spectrum smoothed with a half-width that grows with frequency.
fn smooth_db(power: &[f32], half_width: impl Fn(usize) -> usize) -> Vec<f32> {
    let n = power.len();
    let mut prefix = Vec::with_capacity(n + 1);
    prefix.push(0.0f64);
    for p in power {
        prefix.push(prefix.last().unwrap() + *p as f64);
    }
    (0..n)
        .map(|b| {
            let h = half_width(b);
            let (lo, hi) = (b.saturating_sub(h), (b + h + 1).min(n));
            let m = (prefix[hi] - prefix[lo]) / (hi - lo) as f64;
            (10.0 * (m + 1e-20).log10()) as f32
        })
        .collect()
}

pub fn spectral_clean(
    channels: &[Vec<f32>],
    sr: u32,
    s: &SpectralCleanSettings,
) -> (Vec<Vec<f32>>, SpectralCleanReport) {
    let denoise = s.denoise.clamp(0.0, 1.0);
    let resonance = s.resonance.clamp(0.0, 1.0);
    let n = channels.iter().map(Vec::len).min().unwrap_or(0);
    let mut report = SpectralCleanReport::default();
    if channels.is_empty() || n < sr as usize / 2 || (denoise < 0.01 && resonance < 0.01) {
        return (channels.to_vec(), report);
    }
    let n_fft = if sr > 32_000 { 2048 } else { 1024 };
    let fr = Framer::new(n_fft);
    let frames = fr.frames(n);
    let bins = n_fft / 2 + 1;
    let bin_hz = sr as f32 / n_fft as f32;
    let mono: Vec<f32> = (0..n)
        .map(|i| channels.iter().map(|c| c[i]).sum::<f32>() / channels.len() as f32)
        .collect();

    // Frame energies from the time domain decide which frames are quiet.
    let energy: Vec<f32> = (0..frames)
        .map(|f| {
            let c = f * fr.hop;
            let (a, b) = (c.saturating_sub(n_fft / 2).min(n), (c + n_fft / 2).min(n));
            mono[a..b].iter().map(|v| v * v).sum::<f32>() / (b - a).max(1) as f32
        })
        .collect();
    let quiet_gate = percentile(&energy, 20.0);
    let active_gate = percentile(&energy, 50.0);
    let loud = percentile(&energy, 90.0);

    // Noise profile and typical envelope shape.
    let h1 = ((150.0 / bin_hz).round() as usize).max(1);
    let h2 = |b: usize| (2 * h1).max((0.35 * b as f32) as usize);
    let mut noise = vec![0.0f64; bins];
    let mut quiet = 0usize;
    let mut shape_sum = vec![0.0f64; bins];
    let mut active = 0usize;
    let mut spec = vec![Complex32::default(); bins];
    for (f, &e) in energy.iter().enumerate() {
        let is_quiet = e <= quiet_gate && e > 0.0;
        let is_active = e >= active_gate;
        if !is_quiet && !is_active {
            continue;
        }
        fr.analyse(&mono, f, &mut spec);
        let p: Vec<f32> = spec.iter().map(|c| c.norm_sqr()).collect();
        if is_quiet {
            quiet += 1;
            for (acc, v) in noise.iter_mut().zip(&p) {
                *acc += *v as f64;
            }
        }
        if is_active && resonance > 0.0 {
            active += 1;
            let l1 = smooth_db(&p, |_| h1);
            let l2 = smooth_db(&p, h2);
            for b in 0..bins {
                shape_sum[b] += (l1[b] - l2[b]) as f64;
            }
        }
    }
    let noise: Vec<f32> = noise
        .iter()
        .map(|v| (*v / quiet.max(1) as f64) as f32)
        .collect();
    let typical: Vec<f32> = shape_sum
        .iter()
        .map(|v| (*v / active.max(1) as f64) as f32)
        .collect();
    let noise_energy = energy.iter().filter(|e| **e <= quiet_gate).sum::<f32>()
        / energy.iter().filter(|e| **e <= quiet_gate).count().max(1) as f32;
    report.snr_db = 10.0 * (loud / noise_energy.max(1e-12)).log10();
    // Only denoise when there is audible noise to remove.
    let do_denoise = denoise > 0.01 && quiet > 10 && report.snr_db < 55.0;
    report.denoised = do_denoise;
    let do_resonance = resonance > 0.01 && active > 10;
    if !do_denoise && !do_resonance {
        return (channels.to_vec(), report);
    }

    let floor = crate::util::db_to_gain(-(4.0 + 8.0 * denoise));
    let over = 1.0 + 1.5 * denoise;
    let thr = 4.0 - 1.5 * resonance;
    let ratio = 0.4 + 0.4 * resonance;
    let max_cut = 1.5 + 5.0 * resonance;
    let (r_lo, r_hi) = (
        (250.0 / bin_hz) as usize,
        ((10_000.0 / bin_hz) as usize).min(bins),
    );

    let mut out = vec![vec![0.0f32; n + n_fft]; channels.len()];
    let mut wsum = vec![0.0f32; n + n_fft];
    let mut specs = vec![vec![Complex32::default(); bins]; channels.len()];
    let mut prev_p = vec![0.0f32; bins];
    let mut prev_g = vec![1.0f32; bins];
    let mut prev_cut = vec![0.0f32; bins];
    let mut time = vec![0.0f32; n_fft];
    let scale = 1.0 / n_fft as f32;
    for (f, &e) in energy.iter().enumerate() {
        for (c, ch) in channels.iter().enumerate() {
            fr.analyse(ch, f, &mut specs[c]);
        }
        let p = bins_power(&specs);
        let mut gain = vec![1.0f32; bins];
        if do_denoise {
            for b in 0..bins {
                let lo = b.saturating_sub(1);
                let hi = (b + 2).min(bins);
                let local = p[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
                let ps = 0.6 * local + 0.4 * prev_p[b];
                let g = (1.0 - over * noise[b] / ps.max(1e-20))
                    .max(0.0)
                    .sqrt()
                    .max(floor);
                // Open instantly, close smoothly: avoids musical noise.
                let g = if g > prev_g[b] {
                    g
                } else {
                    prev_g[b] + 0.5 * (g - prev_g[b])
                };
                prev_g[b] = g;
                gain[b] = g;
            }
            prev_p.copy_from_slice(&p);
        }
        if do_resonance && e >= active_gate {
            let l1 = smooth_db(&p, |_| h1);
            let l2 = smooth_db(&p, h2);
            for b in r_lo..r_hi {
                let excess = l1[b] - l2[b] - typical[b] - thr;
                let cut = (excess * ratio).clamp(0.0, max_cut);
                let cut = if cut > prev_cut[b] {
                    cut
                } else {
                    prev_cut[b] + 0.4 * (cut - prev_cut[b])
                };
                prev_cut[b] = cut;
                gain[b] *= crate::util::db_to_gain(-cut);
            }
        } else {
            prev_cut.iter_mut().for_each(|c| *c *= 0.6);
            for b in r_lo..r_hi {
                gain[b] *= crate::util::db_to_gain(-prev_cut[b]);
            }
        }
        let start = (f * fr.hop) as isize - (n_fft / 2) as isize;
        for (c, sp) in specs.iter_mut().enumerate() {
            for (v, g) in sp.iter_mut().zip(&gain) {
                *v *= *g;
            }
            sp[0].im = 0.0;
            sp[bins - 1].im = 0.0;
            fr.ifft
                .process(sp, &mut time)
                .expect("fft sizes are fixed by the planner");
            for i in 0..n_fft {
                let j = start + i as isize + (n_fft / 2) as isize;
                if j >= 0 && (j as usize) < out[c].len() {
                    out[c][j as usize] += time[i] * scale * fr.window[i];
                    if c == 0 {
                        wsum[j as usize] += fr.window[i] * fr.window[i];
                    }
                }
            }
        }
    }
    let result = out
        .iter()
        .map(|o| {
            (0..n)
                .map(|i| {
                    let j = i + n_fft / 2;
                    if wsum[j] > 1e-6 {
                        o[j] / wsum[j]
                    } else {
                        0.0
                    }
                })
                .collect()
        })
        .collect();
    (result, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::rms;
    use std::f32::consts::PI;

    const SR: u32 = 44_100;

    fn noise(n: usize, seed: &mut u32) -> Vec<f32> {
        (0..n)
            .map(|_| {
                *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (*seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect()
    }

    fn vowel(n: usize, f0: f32) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f32 / SR as f32;
                (1..15)
                    .map(|h| (2.0 * PI * f0 * h as f32 * t).sin() / h as f32)
                    .sum::<f32>()
                    * 0.2
            })
            .collect()
    }

    #[test]
    fn transparent_when_disabled_or_clean() {
        let x = vowel(SR as usize * 2, 220.0);
        let (y, _) = spectral_clean(
            std::slice::from_ref(&x),
            SR,
            &SpectralCleanSettings {
                denoise: 0.0,
                resonance: 0.0,
            },
        );
        assert_eq!(y[0], x);
        // Resonance suppression alone on a steady vowel changes almost nothing.
        let (y, _) = spectral_clean(
            std::slice::from_ref(&x),
            SR,
            &SpectralCleanSettings {
                denoise: 0.0,
                resonance: 0.6,
            },
        );
        let err: Vec<f32> = x.iter().zip(&y[0]).map(|(a, b)| a - b).collect();
        assert!(rms(&err) < 0.05 * rms(&x), "{}", rms(&err) / rms(&x));
    }

    #[test]
    fn hiss_between_phrases_is_reduced_and_voice_kept() {
        let sec = SR as usize;
        let mut seed = 9;
        let hiss: Vec<f32> = noise(3 * sec, &mut seed).iter().map(|v| v * 0.01).collect();
        let mut x = vec![0.0f32; 3 * sec];
        let v = vowel(sec, 220.0);
        x[sec..2 * sec].copy_from_slice(&v);
        for (a, b) in x.iter_mut().zip(&hiss) {
            *a += b;
        }
        let (y, rep) = spectral_clean(
            &[x.clone()],
            SR,
            &SpectralCleanSettings {
                denoise: 0.6,
                resonance: 0.0,
            },
        );
        assert!(rep.denoised);
        let gap = |s: &[f32]| rms(&s[sec / 4..3 * sec / 4]);
        assert!(
            gap(&y[0]) < 0.5 * gap(&x),
            "hiss {} -> {}",
            gap(&x),
            gap(&y[0])
        );
        let voice = |s: &[f32]| rms(&s[sec + sec / 4..sec + 3 * sec / 4]);
        assert!((voice(&y[0]) / voice(&x) - 1.0).abs() < 0.05);
    }

    #[test]
    fn a_resonance_that_appears_gets_cut() {
        let sec = SR as usize;
        let mut x = vowel(4 * sec, 200.0);
        // Whistling 3.4 kHz resonance during the last second only.
        for (i, v) in x.iter_mut().enumerate().skip(3 * sec) {
            let t = i as f32 / SR as f32;
            *v += 0.08 * (2.0 * PI * 3400.0 * t).sin();
        }
        let (y, _) = spectral_clean(
            &[x.clone()],
            SR,
            &SpectralCleanSettings {
                denoise: 0.0,
                resonance: 0.8,
            },
        );
        let tone = |s: &[f32]| {
            let seg = &s[3 * sec + sec / 4..3 * sec + 3 * sec / 4];
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, v) in seg.iter().enumerate() {
                let ph = 2.0 * PI * 3400.0 * i as f32 / SR as f32;
                re += v * ph.cos();
                im += v * ph.sin();
            }
            (re * re + im * im).sqrt()
        };
        assert!(tone(&y[0]) < 0.7 * tone(&x));
    }
}
