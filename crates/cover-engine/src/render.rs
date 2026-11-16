//! Builds the processed master for a time range.
//!
//! The separated stems satisfy `original = vocals + accompaniment` exactly
//! (the accompaniment is computed as the residual), so the mix is the
//! original plus the *difference* each processed stem makes. Blend amounts
//! below 1 therefore fade towards the untouched recording, room sound and all.

use crate::analysis::tuning::{correction_curve, CorrectionCurve, Scale};
use crate::chain::{process_bus, process_instrument, process_vocal};
use crate::error::Result;
use crate::settings::Settings;
use cover_dsp::filter::{filtfilt, FilterKind};
use cover_dsp::loudness::{normalize, LoudnessReport};
use cover_dsp::pitch_shift::shift_varying_multi;
use cover_dsp::reverb::parallel_room;
use cover_dsp::util::window_mask;
use cover_dsp::AudioBuffer;
use cover_ml::PitchTrack;
use serde::{Deserialize, Serialize};

pub struct RenderInputs<'a> {
    pub original: &'a AudioBuffer,
    pub vocals: &'a AudioBuffer,
    pub accompaniment: &'a AudioBuffer,
    pub pitch: Option<&'a PitchTrack>,
    pub scale: Scale,
    pub settings: &'a Settings,
    pub start_sec: f64,
    pub end_sec: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderStats {
    pub loudness: LoudnessReport,
    pub notes_corrected: usize,
    /// Mean absolute applied correction, in cents.
    pub mean_correction_cents: f32,
}

pub struct RenderOutput {
    /// Stereo master covering exactly `[start_sec, end_sec]`.
    pub master: AudioBuffer,
    pub stats: RenderStats,
    pub curve: Option<CorrectionCurve>,
}

const PAD_SEC: f64 = 1.5;

pub fn render(inp: &RenderInputs, check: &dyn Fn() -> Result<()>) -> Result<RenderOutput> {
    let s = inp.settings;
    let sr = inp.original.sample_rate;
    let total = inp.original.duration_sec();
    let w0 = (inp.start_sec - PAD_SEC).max(0.0);
    let w1 = (inp.end_sec + PAD_SEC).min(total);

    let orig = inp.original.to_stereo().slice_sec(w0, w1);
    let v_raw = inp.vocals.to_stereo().slice_sec(w0, w1);
    let a_raw = inp.accompaniment.to_stereo().slice_sec(w0, w1);
    let n = orig.len().min(v_raw.len()).min(a_raw.len());
    let w0_samples = (w0 * sr as f64).round() as usize;

    // Pitch correction on the vocal stem.
    let mut curve = None;
    let mut v_tuned = v_raw.channels.clone();
    if s.tuning.enabled {
        if let Some(pitch) = inp.pitch {
            let c = correction_curve(
                pitch,
                &inp.scale,
                &s.tuning.params(),
                inp.start_sec,
                inp.end_sec,
            );
            if !c.notes.is_empty() {
                v_tuned =
                    shift_varying_multi(&v_raw.channels, sr, |i| c.at_sample(i + w0_samples, sr));
            }
            curve = Some(c);
        }
    }
    check()?;

    let v_proc = process_vocal(&v_tuned, sr, &s.vocal);
    check()?;
    let a_proc = process_instrument(&a_raw.channels, sr, s.instrument);
    check()?;

    // Changes are confined to the selected range, with soft edges.
    let i0 = ((inp.start_sec - w0) * sr as f64) as usize;
    let i1 = (((inp.end_sec - w0) * sr as f64) as usize).min(n);
    let mask = window_mask(n, i0, i1, (0.15 * sr as f64) as usize);

    let mut mix: Vec<Vec<f32>> = Vec::with_capacity(2);
    for ch in 0..2 {
        let v_delta: Vec<f32> = (0..n)
            .map(|i| v_proc[ch][i] - v_raw.channels[ch][i])
            .collect();
        // Keep low-frequency bleed in the vocal stem from being re-injected.
        let v_delta = filtfilt(
            &v_delta,
            sr,
            FilterKind::Highpass {
                freq: 70.0,
                q: 0.707,
            },
        );
        let out: Vec<f32> = (0..n)
            .map(|i| {
                let a_delta = a_proc[ch][i] - a_raw.channels[ch][i];
                orig.channels[ch][i]
                    + mask[i] * (s.vocal.blend * v_delta[i] + s.instrument_blend * a_delta)
            })
            .collect();
        mix.push(out);
    }
    check()?;

    process_bus(&mut mix, sr, s.warmth_db);
    if s.room > 0.0 {
        let room_mix = 0.02 + 0.08 * s.room.min(1.0);
        parallel_room(&mut mix, sr, 0.22, 0.6, room_mix);
    }
    check()?;

    let window = AudioBuffer::new(sr, mix);
    let mut master = window.slice_sec(inp.start_sec - w0, inp.end_sec - w0);
    drop(window);
    let loudness = normalize(&mut master, s.loudness.lufs, s.loudness.ceiling_dbtp)?;

    let (notes_corrected, mean_correction_cents) = match &curve {
        Some(c) if !c.notes.is_empty() => (
            c.notes.len(),
            c.notes.iter().map(|n| n.shift.abs() * 100.0).sum::<f32>() / c.notes.len() as f32,
        ),
        _ => (0, 0.0),
    };
    Ok(RenderOutput {
        master,
        stats: RenderStats {
            loudness,
            notes_corrected,
            mean_correction_cents,
        },
        curve,
    })
}
