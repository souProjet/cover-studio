//! STFT / iSTFT with the same conventions as `torch.stft(center=True,
//! pad_mode="reflect", window=hann_window(periodic=True))`. ONNX separation
//! and pitch models are exported without their spectral front-end, so the
//! framing here has to match PyTorch exactly.

use realfft::num_complex::Complex32;
use realfft::RealFftPlanner;
use std::f32::consts::PI;

pub fn hann_periodic(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / n as f32).cos())
        .collect()
}

/// Complex spectrogram stored frame-major: `data[frame * bins + bin]`.
#[derive(Debug, Clone)]
pub struct Spectrogram {
    pub n_fft: usize,
    pub hop: usize,
    pub frames: usize,
    pub bins: usize,
    pub data: Vec<Complex32>,
}

impl Spectrogram {
    pub fn frame(&self, f: usize) -> &[Complex32] {
        &self.data[f * self.bins..(f + 1) * self.bins]
    }

    pub fn frame_mut(&mut self, f: usize) -> &mut [Complex32] {
        &mut self.data[f * self.bins..(f + 1) * self.bins]
    }

    pub fn magnitude(&self) -> Vec<f32> {
        self.data.iter().map(|c| c.norm()).collect()
    }
}

fn reflect_pad(x: &[f32], pad: usize) -> Vec<f32> {
    let n = x.len();
    let mut out = Vec::with_capacity(n + 2 * pad);
    if n <= pad {
        out.resize(pad, 0.0);
        out.extend_from_slice(x);
        out.resize(n + 2 * pad, 0.0);
        return out;
    }
    for i in (1..=pad).rev() {
        out.push(x[i]);
    }
    out.extend_from_slice(x);
    for i in 0..pad {
        out.push(x[n - 2 - i]);
    }
    out
}

pub fn stft(x: &[f32], n_fft: usize, hop: usize) -> Spectrogram {
    let window = hann_periodic(n_fft);
    let padded = reflect_pad(x, n_fft / 2);
    let frames = if padded.len() >= n_fft {
        1 + (padded.len() - n_fft) / hop
    } else {
        0
    };
    let bins = n_fft / 2 + 1;
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n_fft);
    let mut input = fft.make_input_vec();
    let mut output = fft.make_output_vec();
    let mut scratch = fft.make_scratch_vec();
    let mut data = Vec::with_capacity(frames * bins);
    for f in 0..frames {
        let start = f * hop;
        for (i, v) in input.iter_mut().enumerate() {
            *v = padded[start + i] * window[i];
        }
        fft.process_with_scratch(&mut input, &mut output, &mut scratch)
            .expect("fft sizes are fixed by the planner");
        data.extend_from_slice(&output);
    }
    Spectrogram {
        n_fft,
        hop,
        frames,
        bins,
        data,
    }
}

/// Inverse STFT (windowed overlap-add, normalised by the summed squared window).
pub fn istft(spec: &Spectrogram, length: usize) -> Vec<f32> {
    let n_fft = spec.n_fft;
    let hop = spec.hop;
    let window = hann_periodic(n_fft);
    let total = n_fft + hop * spec.frames.saturating_sub(1);
    let mut out = vec![0.0f32; total];
    let mut wsum = vec![0.0f32; total];
    let mut planner = RealFftPlanner::<f32>::new();
    let ifft = planner.plan_fft_inverse(n_fft);
    let mut input = ifft.make_input_vec();
    let mut output = ifft.make_output_vec();
    let mut scratch = ifft.make_scratch_vec();
    let scale = 1.0 / n_fft as f32;
    for f in 0..spec.frames {
        input.copy_from_slice(spec.frame(f));
        input[0].im = 0.0;
        if let Some(last) = input.last_mut() {
            last.im = 0.0;
        }
        ifft.process_with_scratch(&mut input, &mut output, &mut scratch)
            .expect("fft sizes are fixed by the planner");
        let start = f * hop;
        for i in 0..n_fft {
            out[start + i] += output[i] * scale * window[i];
            wsum[start + i] += window[i] * window[i];
        }
    }
    let pad = n_fft / 2;
    (0..length)
        .map(|i| {
            let j = i + pad;
            if j < total && wsum[j] > 1e-11 {
                out[j] / wsum[j]
            } else {
                0.0
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_is_identity() {
        let x: Vec<f32> = (0..10_000)
            .map(|i| ((i as f32) * 0.013).sin() + 0.3 * ((i as f32) * 0.31).cos())
            .collect();
        let s = stft(&x, 1024, 256);
        assert_eq!(s.frames, 1 + x.len() / 256);
        let y = istft(&s, x.len());
        let err = x
            .iter()
            .zip(&y)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(err < 1e-4, "{err}");
    }
}
