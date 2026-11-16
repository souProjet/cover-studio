//! Pitch shifting with Signalsmith Stretch (MIT). Length is always preserved,
//! so timing and lip-sync stay intact.

use signalsmith_stretch::Stretch;

const BLOCK: usize = 256;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ShiftOptions {
    /// Keeps the spectral envelope (vocal timbre) in place while the pitch
    /// moves, so corrected notes do not sound thinner or darker.
    pub preserve_formants: bool,
    /// Above this frequency the shift becomes less tonal, which keeps
    /// breathiness and consonants natural on voices.
    pub tonality_limit_hz: Option<f32>,
}

impl ShiftOptions {
    pub fn voice() -> Self {
        Self {
            preserve_formants: true,
            tonality_limit_hz: Some(8000.0),
        }
    }
}

/// Shifts planar audio by a time-varying amount, keeping channels phase-coherent.
///
/// `semitones_at(sample_index)` is queried once per block of 256 samples and
/// should already be smooth; the stretcher interpolates within its own frames.
pub fn shift_varying_multi(
    channels: &[Vec<f32>],
    sample_rate: u32,
    semitones_at: impl Fn(usize) -> f32,
) -> Vec<Vec<f32>> {
    shift_varying_multi_with(channels, sample_rate, ShiftOptions::default(), semitones_at)
}

/// Like [`shift_varying_multi`], with formant and tonality options.
pub fn shift_varying_multi_with(
    channels: &[Vec<f32>],
    sample_rate: u32,
    opts: ShiftOptions,
    semitones_at: impl Fn(usize) -> f32,
) -> Vec<Vec<f32>> {
    let ch = channels.len();
    let n = channels.iter().map(Vec::len).min().unwrap_or(0);
    if ch == 0 || n == 0 {
        return channels.to_vec();
    }
    let mut st = Stretch::preset_default(ch as u32, sample_rate);
    // The binding's `set_formant_base` actually sets a formant shift (and
    // clears compensation), so the base frequency is left on auto-detect.
    if opts.preserve_formants {
        st.set_formant_factor(1.0, true);
    }
    let tonality = opts
        .tonality_limit_hz
        .map(|hz| (hz / sample_rate as f32).clamp(0.01, 0.5));
    let in_lat = st.input_latency();
    let latency = in_lat + st.output_latency();

    let total_in = n + latency;
    let mut out = Vec::with_capacity((total_in + BLOCK) * ch);
    let mut block_in = vec![0.0f32; BLOCK * ch];
    let mut block_out = vec![0.0f32; BLOCK * ch];
    let mut pos = 0usize;
    while pos < total_in {
        let len = BLOCK.min(total_in - pos);
        for k in 0..len {
            for (c, src) in channels.iter().enumerate() {
                block_in[k * ch + c] = src.get(pos + k).copied().unwrap_or(0.0);
            }
        }
        let center = pos.saturating_sub(in_lat).min(n - 1);
        st.set_transpose_factor_semitones(semitones_at(center), tonality);
        st.process(&block_in[..len * ch], &mut block_out[..len * ch]);
        out.extend_from_slice(&block_out[..len * ch]);
        pos += len;
    }
    let mut result = vec![Vec::with_capacity(n); ch];
    for frame in out.chunks_exact(ch).skip(latency).take(n) {
        for (c, s) in frame.iter().enumerate() {
            result[c].push(*s);
        }
    }
    for r in &mut result {
        r.resize(n, 0.0);
    }
    result
}

/// Mono convenience wrapper around [`shift_varying_multi`].
pub fn shift_varying(x: &[f32], sample_rate: u32, semitones_at: impl Fn(usize) -> f32) -> Vec<f32> {
    if x.is_empty() {
        return Vec::new();
    }
    shift_varying_multi(&[x.to_vec()], sample_rate, semitones_at)
        .pop()
        .unwrap_or_default()
}

/// Constant shift, length preserved.
pub fn shift(x: &[f32], sample_rate: u32, semitones: f32) -> Vec<f32> {
    if semitones.abs() < 1e-4 {
        return x.to_vec();
    }
    shift_varying(x, sample_rate, |_| semitones)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zero_crossings_hz(x: &[f32], sr: u32) -> f32 {
        let c = x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        c as f32 * sr as f32 / x.len() as f32
    }

    #[test]
    fn shifts_up_an_octave() {
        let sr = 44_100u32;
        let x: Vec<f32> = (0..sr as usize * 2)
            .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / sr as f32).sin())
            .collect();
        let y = shift(&x, sr, 12.0);
        assert_eq!(y.len(), x.len());
        let mid = &y[sr as usize / 2..sr as usize * 3 / 2];
        let f = zero_crossings_hz(mid, sr);
        assert!((f - 440.0).abs() < 15.0, "{f}");
    }

    #[test]
    fn keeps_alignment() {
        let sr = 44_100u32;
        // Noise bursts with sharp envelopes make any latency error obvious.
        let mut seed = 1u32;
        let x: Vec<f32> = (0..sr as usize * 2)
            .map(|i| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
                if (i / 4410) % 3 == 0 {
                    noise
                } else {
                    0.0
                }
            })
            .collect();
        let y = shift_varying(&x, sr, |_| 0.3);
        let ex = crate::analysis::rms_envelope(&x, 64);
        let ey = crate::analysis::rms_envelope(&y, 64);
        let (lag, score) = crate::analysis::xcorr::best_lag(&ex, &ey, 100);
        assert!(lag.abs() <= 1, "lag {lag} blocks of 64 samples");
        assert!(score > 0.8, "{score}");
    }

    /// Sawtooth through two formant resonances: a crude sung vowel.
    fn vowel(sr: u32, f0: f32) -> Vec<f32> {
        use crate::filter::{Biquad, FilterKind};
        let saw: Vec<f32> = (0..sr as usize * 2)
            .map(|i| ((i as f32 * f0 / sr as f32) % 1.0) - 0.5)
            .collect();
        let mut a = saw.clone();
        Biquad::new(
            FilterKind::Bandpass {
                freq: 700.0,
                q: 2.0,
            },
            sr,
        )
        .process(&mut a);
        let mut b = saw;
        Biquad::new(
            FilterKind::Bandpass {
                freq: 2200.0,
                q: 2.5,
            },
            sr,
        )
        .process(&mut b);
        a.iter().zip(&b).map(|(p, q)| p + 0.5 * q).collect()
    }

    /// Long-term spectral envelope in dB, smoothed over ~270 Hz.
    fn envelope(x: &[f32]) -> Vec<f32> {
        let s = crate::stft::stft(x, 4096, 1024);
        let mut avg = vec![0.0f64; s.bins];
        for f in 4..s.frames - 4 {
            for (b, c) in s.frame(f).iter().enumerate() {
                avg[b] += c.norm_sqr() as f64;
            }
        }
        (0..s.bins)
            .map(|b| {
                let (lo, hi) = (b.saturating_sub(25), (b + 26).min(s.bins));
                let m = avg[lo..hi].iter().sum::<f64>() / (hi - lo) as f64;
                (10.0 * (m + 1e-20).log10()) as f32
            })
            .collect()
    }

    /// RMS difference of the envelope shapes between 300 Hz and 4 kHz, in dB.
    fn envelope_error(x: &[f32], y: &[f32], sr: u32) -> f32 {
        let (ex, ey) = (envelope(x), envelope(y));
        let bin = |hz: f32| (hz * 4096.0 / sr as f32) as usize;
        let d: Vec<f32> = (bin(300.0)..bin(4000.0)).map(|b| ey[b] - ex[b]).collect();
        let m = d.iter().sum::<f32>() / d.len() as f32;
        (d.iter().map(|v| (v - m) * (v - m)).sum::<f32>() / d.len() as f32).sqrt()
    }

    #[test]
    fn formants_stay_closer_when_preserved() {
        let sr = 44_100u32;
        for f0 in [110.0, 220.0] {
            let x = vowel(sr, f0);
            for semi in [-3.0, 3.0] {
                let plain = shift_varying_multi(std::slice::from_ref(&x), sr, |_| semi)
                    .pop()
                    .unwrap();
                let kept = shift_varying_multi_with(
                    std::slice::from_ref(&x),
                    sr,
                    ShiftOptions::voice(),
                    |_| semi,
                )
                .pop()
                .unwrap();
                let (ep, ek) = (
                    envelope_error(&x, &plain, sr),
                    envelope_error(&x, &kept, sr),
                );
                assert!(ek < 0.85 * ep, "f0 {f0} shift {semi}: {ep} dB -> {ek} dB");
            }
        }
    }

    #[test]
    fn stereo_channels_stay_identical_for_dual_mono() {
        let sr = 44_100u32;
        let x: Vec<f32> = (0..sr as usize)
            .map(|i| 0.3 * (2.0 * std::f32::consts::PI * 196.0 * i as f32 / sr as f32).sin())
            .collect();
        let y = shift_varying_multi(&[x.clone(), x], sr, |_| 0.5);
        let diff = y[0]
            .iter()
            .zip(&y[1])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(diff < 1e-3, "{diff}");
    }
}
