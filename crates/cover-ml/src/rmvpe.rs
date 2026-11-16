//! RMVPE vocal pitch estimation (log-mel in, 360-bin pitch salience out).

use crate::runtime::{open_session, Device, Provider};
use crate::{MlError, Result};
use cover_dsp::mel::MelBank;
use cover_dsp::resample::resample;
use cover_dsp::stft::stft;
use ort::session::Session;
use ort::value::Tensor;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

const SR: u32 = 16_000;
const N_FFT: usize = 1024;
const HOP: usize = 160;
const N_MELS: usize = 128;
const N_BINS: usize = 360;
/// Frames per inference chunk (multiple of 32) and context kept on each side.
const CHUNK: usize = 3200;
const CONTEXT: usize = 128;

/// Pitch per 10 ms frame. `f0_hz == 0` means unvoiced.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PitchTrack {
    pub hop_sec: f64,
    pub f0_hz: Vec<f32>,
    pub confidence: Vec<f32>,
}

impl PitchTrack {
    pub fn frame_at(&self, sec: f64) -> usize {
        ((sec / self.hop_sec).round().max(0.0) as usize).min(self.f0_hz.len().saturating_sub(1))
    }
}

pub struct Rmvpe {
    session: Session,
    input_name: String,
    bank: MelBank,
    pub provider: Provider,
}

impl Rmvpe {
    pub fn load(path: &Path, device: Device) -> Result<Self> {
        let (session, provider) = open_session(path, device)?;
        let input_name = session
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .ok_or_else(|| MlError::BadOutput("model has no inputs".into()))?;
        Ok(Self {
            session,
            input_name,
            bank: MelBank::new(SR, N_FFT, N_MELS, 30.0, 8000.0, true),
            provider,
        })
    }

    fn log_mel(&self, x16k: &[f32]) -> (Vec<f32>, usize) {
        let spec = stft(x16k, N_FFT, HOP);
        let frames = spec.frames;
        // [N_MELS, frames]
        let mut mel = vec![0.0f32; N_MELS * frames];
        let mut mag = vec![0.0f32; spec.bins];
        let mut row = vec![0.0f32; N_MELS];
        for t in 0..frames {
            for (m, c) in mag.iter_mut().zip(spec.frame(t)) {
                *m = c.norm();
            }
            self.bank.apply(&mag, &mut row);
            for (k, v) in row.iter().enumerate() {
                mel[k * frames + t] = v.max(1e-5).ln();
            }
        }
        (mel, frames)
    }

    fn infer(&mut self, mel: &[f32], frames: usize, start: usize, len: usize) -> Result<Vec<f32>> {
        // Pad to a multiple of 32 frames with reflection, as the U-Net requires.
        let padded_len = len.div_ceil(32) * 32;
        let mut input = vec![0.0f32; N_MELS * padded_len];
        for k in 0..N_MELS {
            for t in 0..padded_len {
                let src = if t < len {
                    start + t
                } else {
                    // reflect around the last frame of the chunk
                    let over = t - len + 1;
                    start + len.saturating_sub(1 + over).min(len - 1)
                };
                input[k * padded_len + t] = mel[k * frames + src.min(frames - 1)];
            }
        }
        let tensor = Tensor::from_array(([1usize, N_MELS, padded_len], input))?;
        let outputs = self
            .session
            .run(ort::inputs![self.input_name.as_str() => tensor])?;
        let (shape, data) = outputs[0].try_extract_tensor::<f32>()?;
        if data.len() != padded_len * N_BINS {
            return Err(MlError::BadOutput(format!("rmvpe output shape {shape:?}")));
        }
        Ok(data[..len * N_BINS].to_vec())
    }

    pub fn track(
        &mut self,
        mono: &[f32],
        sample_rate: u32,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(f32),
    ) -> Result<PitchTrack> {
        let x = resample(mono, sample_rate, SR);
        let (mel, frames) = self.log_mel(&x);
        let mut salience = vec![0.0f32; frames * N_BINS];
        let mut pos = 0;
        while pos < frames {
            if cancel.load(Ordering::Relaxed) {
                return Err(MlError::Cancelled);
            }
            let keep_end = (pos + CHUNK).min(frames);
            let a = pos.saturating_sub(CONTEXT);
            let b = (keep_end + CONTEXT).min(frames);
            let out = self.infer(&mel, frames, a, b - a)?;
            let off = pos - a;
            for t in pos..keep_end {
                let src = (t - pos + off) * N_BINS;
                salience[t * N_BINS..(t + 1) * N_BINS].copy_from_slice(&out[src..src + N_BINS]);
            }
            pos = keep_end;
            progress(pos as f32 / frames as f32);
        }
        let (f0_hz, confidence) = decode(&salience, frames);
        Ok(PitchTrack {
            hop_sec: HOP as f64 / SR as f64,
            f0_hz,
            confidence,
        })
    }
}

/// Local weighted average of cents around the argmax (RMVPE's `to_local_average_cents`).
fn decode(salience: &[f32], frames: usize) -> (Vec<f32>, Vec<f32>) {
    const THRESHOLD: f32 = 0.03;
    let cents = |i: usize| 20.0 * i as f32 + 1997.3794;
    let mut f0 = Vec::with_capacity(frames);
    let mut conf = Vec::with_capacity(frames);
    for t in 0..frames {
        let row = &salience[t * N_BINS..(t + 1) * N_BINS];
        let (center, &peak) = row
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap_or((0, &0.0));
        conf.push(peak);
        if peak <= THRESHOLD {
            f0.push(0.0);
            continue;
        }
        let a = center.saturating_sub(4);
        let b = (center + 5).min(N_BINS);
        let (mut num, mut den) = (0.0f32, 0.0f32);
        for (i, &w) in row.iter().enumerate().take(b).skip(a) {
            num += w * cents(i);
            den += w;
        }
        let c = num / den.max(1e-9);
        f0.push(10.0 * 2f32.powf(c / 1200.0));
    }
    (f0, conf)
}
