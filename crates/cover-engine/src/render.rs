//! Builds the processed master for a time range.
//!
//! The separated stems satisfy `original = vocals + accompaniment` exactly
//! (the accompaniment is computed as the residual), so the mix is the
//! original plus the *difference* each processed stem makes. Blend amounts
//! below 1 therefore fade towards the untouched recording, room sound and all.
//!
//! Pitch correction is the exception: it is always applied in full, because
//! mixing a shifted vocal with the unshifted one (a partial blend) beats and
//! phases like a chorus. Its strength is set on the correction curve instead.

use crate::analysis::tuning::{correction_curve, CorrectionCurve, Scale};
use crate::chain::{process_bus, process_instrument, process_vocal, VocalReport};
use crate::error::Result;
use crate::settings::Settings;
use cover_dsp::analysis::tuning_ref::tuning_reference;
use cover_dsp::filter::{filtfilt, FilterKind};
use cover_dsp::loudness::{normalize, LoudnessReport};
use cover_dsp::pitch_shift::{shift_varying_multi_with, ShiftOptions};
use cover_dsp::reverb::{parallel_room, vocal_plate};
use cover_dsp::util::window_mask;
use cover_dsp::vocal::{plosive_delta, Voicing};
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
    pub vocal: VocalReport,
    /// Tuning reference of the accompaniment, when one was applied.
    pub reference_cents: Option<f32>,
}

const PAD_SEC: f64 = 1.5;

/// Per-sample 0..1 weight that is 1 wherever the curve corrects anything
/// (plus 100 ms), so the stretcher's output is only used where needed and
/// untouched passages stay bit-identical to the stem.
fn correction_gate(curve: &CorrectionCurve, w0: f64, n: usize, sr: u32) -> Vec<f32> {
    let frames = curve.semitones.len();
    let active: Vec<bool> = curve.semitones.iter().map(|v| v.abs() > 0.003).collect();
    let dilate = (0.1 / curve.hop_sec).round() as usize;
    let mut wide = vec![0.0f32; frames];
    for (k, a) in active.iter().enumerate() {
        if *a {
            let (lo, hi) = (k.saturating_sub(dilate), (k + dilate + 1).min(frames));
            wide[lo..hi].iter_mut().for_each(|v| *v = 1.0);
        }
    }
    // ~50 ms crossfades.
    let h = (0.025 / curve.hop_sec).round() as usize;
    let smooth: Vec<f32> = (0..frames)
        .map(|k| {
            let (lo, hi) = (k.saturating_sub(h), (k + h + 1).min(frames));
            wide[lo..hi].iter().sum::<f32>() / (hi - lo) as f32
        })
        .collect();
    let gate = CorrectionCurve {
        hop_sec: curve.hop_sec,
        semitones: smooth,
        notes: Vec::new(),
    };
    (0..n)
        .map(|i| gate.at_sec(w0 + i as f64 / sr as f64))
        .collect()
}

fn delta(a: &[f32], b: &[f32], sr: u32, highpass: f32) -> Vec<f32> {
    let d: Vec<f32> = a.iter().zip(b).map(|(x, y)| x - y).collect();
    filtfilt(
        &d,
        sr,
        FilterKind::Highpass {
            freq: highpass,
            q: 0.707,
        },
    )
}

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

    let voiced: Option<Vec<bool>> = inp.pitch.map(|p| {
        let frames = ((w1 - w0) / p.hop_sec).ceil() as usize;
        (0..frames)
            .map(|k| {
                let i = p.frame_at(w0 + k as f64 * p.hop_sec);
                p.f0_hz[i] > 0.0 && p.confidence[i] > 0.5
            })
            .collect()
    });
    let voicing = voiced.as_deref().map(|v| Voicing {
        hop_sec: inp.pitch.map_or(0.01, |p| p.hop_sec),
        voiced: v,
    });

    // Pitch correction on the vocal stem, towards the instruments' tuning.
    let mut curve = None;
    let mut reference_cents = None;
    let mut v_tuned = v_raw.channels.clone();
    if s.tuning.enabled {
        if let Some(pitch) = inp.pitch {
            let mut scale = inp.scale.clone();
            let acc = inp
                .accompaniment
                .slice_sec(inp.start_sec, inp.end_sec)
                .to_mono();
            if let Some(r) = tuning_reference(&acc, sr).filter(|r| r.confidence > 0.3) {
                scale = scale.with_offset(r.cents / 100.0);
                reference_cents = Some(r.cents);
            }
            let c = correction_curve(
                pitch,
                &scale,
                &s.tuning.params(),
                inp.start_sec,
                inp.end_sec,
            );
            if !c.is_flat() {
                let opts = ShiftOptions {
                    preserve_formants: s.tuning.preserve_formants,
                    ..ShiftOptions::voice()
                };
                let shifted = shift_varying_multi_with(&v_raw.channels, sr, opts, |i| {
                    c.at_sample(i + w0_samples, sr)
                });
                let gate = correction_gate(&c, w0, n, sr);
                for (t, (sh, raw)) in v_tuned.iter_mut().zip(shifted.iter().zip(&v_raw.channels)) {
                    for i in 0..n {
                        t[i] = raw[i] + gate[i] * (sh[i] - raw[i]);
                    }
                }
            }
            curve = Some(c);
        }
    }
    check()?;

    let (v_proc, vocal_report) = process_vocal(&v_tuned, sr, &s.vocal, voicing.as_ref());
    check()?;
    let a_proc = process_instrument(&a_raw.channels, sr, s.instrument);
    let plosives = plosive_delta(&v_raw.channels, sr, s.vocal.cleanup);
    check()?;

    // Changes are confined to the selected range, with soft edges.
    let i0 = ((inp.start_sec - w0) * sr as f64) as usize;
    let i1 = (((inp.end_sec - w0) * sr as f64) as usize).min(n);
    let mask = window_mask(n, i0, i1, (0.15 * sr as f64) as usize);

    let mut mix: Vec<Vec<f32>> = Vec::with_capacity(2);
    for ch in 0..2 {
        // Low-frequency bleed in the vocal stem must not be re-injected.
        let pitch_delta = delta(&v_tuned[ch], &v_raw.channels[ch], sr, 40.0);
        let proc_delta = delta(&v_proc[ch], &v_tuned[ch], sr, 70.0);
        let plosive = plosives.as_ref().map(|p| &p[ch]);
        let out: Vec<f32> = (0..n)
            .map(|i| {
                let a_delta = a_proc[ch][i] - a_raw.channels[ch][i];
                let fix = plosive.map_or(0.0, |p| p[i]);
                orig.channels[ch][i]
                    + mask[i]
                        * (pitch_delta[i]
                            + s.vocal.blend * proc_delta[i]
                            + s.instrument_blend * a_delta
                            + fix)
            })
            .collect();
        mix.push(out);
    }
    if s.vocal.reverb > 0.01 {
        let vocal: Vec<f32> = (0..n)
            .map(|i| 0.5 * (v_proc[0][i] + v_proc[1][i]))
            .collect();
        let wet = vocal_plate(&vocal, sr, 0.5 * s.vocal.reverb.min(1.0));
        for (m, w) in mix.iter_mut().zip(&wet) {
            for i in 0..n {
                m[i] += mask[i] * w[i];
            }
        }
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
        vocal: vocal_report,
        reference_cents,
    })
}
