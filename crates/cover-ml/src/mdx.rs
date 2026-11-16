//! MDX-Net (Ultimate Vocal Remover) separation.
//!
//! The network takes a `[batch, 4, dim_f, dim_t]` spectrogram where the four
//! channels are `L.re, L.im, R.re, R.im`, and returns the spectrogram of its
//! primary stem. The other stem is `mix - primary`.

use crate::runtime::{open_session, Device, Provider};
use crate::{MlError, Result};
use cover_dsp::stft::{istft, stft, Spectrogram};
use cover_dsp::AudioBuffer;
use ort::session::Session;
use ort::value::Tensor;
use realfft::num_complex::Complex32;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

pub const MDX_SAMPLE_RATE: u32 = 44_100;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MdxParams {
    pub n_fft: usize,
    pub hop: usize,
    pub dim_f: usize,
    pub dim_t_log2: u32,
    pub compensate: f32,
    pub primary_is_vocals: bool,
}

impl MdxParams {
    fn dim_t(&self) -> usize {
        1 << self.dim_t_log2
    }

    fn chunk_size(&self) -> usize {
        self.hop * (self.dim_t() - 1)
    }
}

/// Stereo vocal and accompaniment stems at 44.1 kHz.
#[derive(Debug, Clone)]
pub struct Separation {
    pub vocals: AudioBuffer,
    pub accompaniment: AudioBuffer,
}

pub struct MdxSeparator {
    session: Session,
    params: MdxParams,
    input_name: String,
    pub provider: Provider,
}

impl MdxSeparator {
    pub fn load(path: &Path, params: MdxParams, device: Device) -> Result<Self> {
        let (session, provider) = open_session(path, device)?;
        let input_name = session
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .ok_or_else(|| MlError::BadOutput("model has no inputs".into()))?;
        Ok(Self {
            session,
            params,
            input_name,
            provider,
        })
    }

    /// Runs the network on one chunk of `chunk_size` stereo samples.
    fn run_chunk(&mut self, left: &[f32], right: &[f32], denoise: bool) -> Result<[Vec<f32>; 2]> {
        let p = self.params;
        let dim_t = p.dim_t();
        let specs = [stft(left, p.n_fft, p.hop), stft(right, p.n_fft, p.hop)];
        debug_assert_eq!(specs[0].frames, dim_t);

        // [1, 4, dim_f, dim_t], channel-major then frequency then time.
        let plane = p.dim_f * dim_t;
        let mut input = vec![0.0f32; 4 * plane];
        for (ch, spec) in specs.iter().enumerate() {
            for t in 0..dim_t {
                let frame = spec.frame(t);
                for f in 0..p.dim_f {
                    let c = frame[f];
                    input[(2 * ch) * plane + f * dim_t + t] = c.re;
                    input[(2 * ch + 1) * plane + f * dim_t + t] = c.im;
                }
            }
        }

        let mut output = self.infer(&input, dim_t)?;
        if denoise {
            // Average with the sign-inverted pass to cancel the network's own noise floor.
            let neg: Vec<f32> = input.iter().map(|v| -v).collect();
            let out_neg = self.infer(&neg, dim_t)?;
            for (o, n) in output.iter_mut().zip(out_neg) {
                *o = 0.5 * (*o - n);
            }
        }

        let bins = p.n_fft / 2 + 1;
        let mut result: [Vec<f32>; 2] = [Vec::new(), Vec::new()];
        for (ch, r) in result.iter_mut().enumerate() {
            let mut spec = Spectrogram {
                n_fft: p.n_fft,
                hop: p.hop,
                frames: dim_t,
                bins,
                data: vec![Complex32::new(0.0, 0.0); dim_t * bins],
            };
            for t in 0..dim_t {
                let frame = spec.frame_mut(t);
                for (f, bin) in frame.iter_mut().enumerate().take(p.dim_f) {
                    *bin = Complex32::new(
                        output[(2 * ch) * plane + f * dim_t + t],
                        output[(2 * ch + 1) * plane + f * dim_t + t],
                    );
                }
            }
            *r = istft(&spec, left.len());
        }
        Ok(result)
    }

    fn infer(&mut self, input: &[f32], dim_t: usize) -> Result<Vec<f32>> {
        let shape = [1usize, 4, self.params.dim_f, dim_t];
        let tensor = Tensor::from_array((shape, input.to_vec()))?;
        let outputs = self
            .session
            .run(ort::inputs![self.input_name.as_str() => tensor])?;
        let (out_shape, data) = outputs[0].try_extract_tensor::<f32>()?;
        if data.len() != input.len() {
            return Err(MlError::BadOutput(format!(
                "expected {} values, got shape {out_shape:?}",
                input.len()
            )));
        }
        Ok(data.to_vec())
    }

    /// Separates a 44.1 kHz recording (mono input is treated as dual mono).
    pub fn separate(
        &mut self,
        mix: &AudioBuffer,
        denoise: bool,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(f32),
    ) -> Result<Separation> {
        assert_eq!(
            mix.sample_rate, MDX_SAMPLE_RATE,
            "MDX models expect 44.1 kHz"
        );
        let stereo = mix.to_stereo();
        let n = stereo.len();
        let p = self.params;
        let chunk = p.chunk_size();
        let trim = p.n_fft / 2;
        let gen = chunk - 2 * trim;
        let pad = gen - n % gen;

        let padded: Vec<Vec<f32>> = stereo
            .channels
            .iter()
            .map(|c| {
                let mut v = vec![0.0f32; trim];
                v.extend_from_slice(c);
                v.resize(trim + n + pad + trim, 0.0);
                v
            })
            .collect();

        let total_steps = (n + pad) / gen;
        let mut primary = [Vec::with_capacity(n + pad), Vec::with_capacity(n + pad)];
        for step in 0..total_steps {
            if cancel.load(Ordering::Relaxed) {
                return Err(MlError::Cancelled);
            }
            let s = step * gen;
            let [l, r] =
                self.run_chunk(&padded[0][s..s + chunk], &padded[1][s..s + chunk], denoise)?;
            primary[0].extend_from_slice(&l[trim..chunk - trim]);
            primary[1].extend_from_slice(&r[trim..chunk - trim]);
            progress((step + 1) as f32 / total_steps as f32);
        }

        let mut prim_ch = Vec::with_capacity(2);
        let mut other_ch = Vec::with_capacity(2);
        for (ch, src) in primary.into_iter().zip(&stereo.channels) {
            let prim: Vec<f32> = ch[..n].iter().map(|v| v * p.compensate).collect();
            let other: Vec<f32> = src.iter().zip(&prim).map(|(m, v)| m - v).collect();
            prim_ch.push(prim);
            other_ch.push(other);
        }
        let prim = AudioBuffer::new(MDX_SAMPLE_RATE, prim_ch);
        let other = AudioBuffer::new(MDX_SAMPLE_RATE, other_ch);
        Ok(if p.primary_is_vocals {
            Separation {
                vocals: prim,
                accompaniment: other,
            }
        } else {
            Separation {
                vocals: other,
                accompaniment: prim,
            }
        })
    }
}
