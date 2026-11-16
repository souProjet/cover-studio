//! Note-level pitch correction curve.
//!
//! Each sung note is shifted by a constant amount towards the nearest allowed
//! pitch, with ramps at the attack and release. Vibrato, scoops and the
//! timing of note changes are preserved because nothing inside a note is
//! flattened unless `drift` asks for it.

use cover_dsp::analysis::key::Key;
use cover_dsp::util::{hz_to_midi, median, median_filter, std_dev};
use cover_ml::PitchTrack;
use serde::{Deserialize, Serialize};

/// Allowed pitch classes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scale {
    pub pitch_classes: Vec<u8>,
}

impl Scale {
    pub fn chromatic() -> Self {
        Self {
            pitch_classes: (0..12).collect(),
        }
    }

    pub fn from_key(key: &Key) -> Self {
        Self {
            pitch_classes: key.scale_pitch_classes(),
        }
    }

    /// Nearest allowed midi note to a fractional midi value.
    pub fn nearest(&self, midi: f32) -> f32 {
        let base = (midi / 12.0).floor() * 12.0;
        let mut best = midi.round();
        let mut best_d = f32::INFINITY;
        for oct in [-12.0, 0.0, 12.0] {
            for pc in &self.pitch_classes {
                let cand = base + oct + *pc as f32;
                let d = (cand - midi).abs();
                if d < best_d {
                    best_d = d;
                    best = cand;
                }
            }
        }
        best
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TuningParams {
    /// 0..1 fraction of the error that gets corrected.
    pub strength: f32,
    /// 0..1, also pulls slow drift inside long notes towards the target.
    pub drift: f32,
    /// Corrections smaller than this (semitones) are left alone.
    pub min_shift: f32,
    /// Never shift further than this (semitones).
    pub max_shift: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
}

impl Default for TuningParams {
    fn default() -> Self {
        Self {
            strength: 0.8,
            drift: 0.2,
            min_shift: 0.06,
            max_shift: 0.7,
            attack_ms: 60.0,
            release_ms: 50.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub start_sec: f64,
    pub end_sec: f64,
    pub sung_midi: f32,
    pub target_midi: f32,
    pub shift: f32,
}

/// Per-frame correction in semitones (same frame rate as the pitch track).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrectionCurve {
    pub hop_sec: f64,
    pub semitones: Vec<f32>,
    pub notes: Vec<Note>,
}

impl CorrectionCurve {
    pub fn at_sample(&self, sample: usize, sample_rate: u32) -> f32 {
        let t = sample as f64 / sample_rate as f64;
        let i = (t / self.hop_sec) as usize;
        if i + 1 >= self.semitones.len() {
            return self.semitones.last().copied().unwrap_or(0.0);
        }
        let frac = (t / self.hop_sec - i as f64) as f32;
        self.semitones[i] * (1.0 - frac) + self.semitones[i + 1] * frac
    }
}

fn segment_notes(midi: &[f32], voiced: &[bool], min_frames: usize) -> Vec<(usize, usize)> {
    let mut notes = Vec::new();
    let mut i = 0;
    while i < midi.len() {
        if !voiced[i] {
            i += 1;
            continue;
        }
        let s = i;
        i += 1;
        while i < midi.len() && voiced[i] {
            // A sustained jump of more than ~0.7 st starts a new note.
            let jump = (midi[i] - midi[i - 1]).abs() > 0.7;
            let holds =
                i + 2 < midi.len() && voiced[i + 2] && (midi[i + 2] - midi[i - 1]).abs() > 0.7;
            if jump && holds {
                break;
            }
            i += 1;
        }
        if i - s >= min_frames {
            notes.push((s, i));
        }
    }
    notes
}

/// Builds the correction curve for frames between `start_sec` and `end_sec`.
pub fn correction_curve(
    pitch: &PitchTrack,
    scale: &Scale,
    params: &TuningParams,
    start_sec: f64,
    end_sec: f64,
) -> CorrectionCurve {
    let n = pitch.f0_hz.len();
    let hop = pitch.hop_sec;
    let voiced: Vec<bool> = pitch
        .f0_hz
        .iter()
        .zip(&pitch.confidence)
        .map(|(f, c)| *f > 60.0 && *c > 0.5)
        .collect();
    let raw: Vec<f32> = pitch
        .f0_hz
        .iter()
        .map(|f| if *f > 0.0 { hz_to_midi(*f) } else { 0.0 })
        .collect();
    let midi = median_filter(&raw, 5);
    let slow = median_filter(&raw, 15);

    let a = ((start_sec / hop) as usize).min(n);
    let b = ((end_sec / hop).ceil() as usize).min(n);
    let mut curve = vec![0.0f32; n];
    let mut notes = Vec::new();
    let min_frames = (0.06 / hop).ceil() as usize;
    let att = ((params.attack_ms as f64 / 1000.0) / hop).round() as usize;
    let rel = ((params.release_ms as f64 / 1000.0) / hop).round() as usize;

    for (s, e) in segment_notes(&midi[a..b], &voiced[a..b], min_frames) {
        let (s, e) = (s + a, e + a);
        let len = e - s;
        let core_a = s + att.min(len / 3);
        let core_b = e - rel.min(len / 3);
        let core = &midi[core_a..core_b.max(core_a + 1)];
        let sung = median(core);
        if std_dev(core) > 0.9 {
            continue;
        }
        let mut target = scale.nearest(sung);
        // A note far from the scale is probably a deliberate chromatic note.
        if (target - sung).abs() > 0.6 {
            target = sung.round();
        }
        let err = target - sung;
        if err.abs() < params.min_shift {
            continue;
        }
        let shift = (params.strength * err).clamp(-params.max_shift, params.max_shift);
        notes.push(Note {
            start_sec: s as f64 * hop,
            end_sec: e as f64 * hop,
            sung_midi: sung,
            target_midi: target,
            shift,
        });
        for (k, slot) in curve.iter_mut().enumerate().take(e).skip(s) {
            let ramp_in = if att > 0 {
                ((k - s) as f32 / att as f32).min(1.0)
            } else {
                1.0
            };
            let ramp_out = if rel > 0 {
                ((e - k) as f32 / rel as f32).min(1.0)
            } else {
                1.0
            };
            let drift = if params.drift > 0.0 && slow[k] > 0.0 {
                params.drift * params.strength * (sung - slow[k]).clamp(-0.5, 0.5)
            } else {
                0.0
            };
            *slot = (shift + drift).clamp(-params.max_shift, params.max_shift) * ramp_in * ramp_out;
        }
    }
    // Light smoothing (~30 ms) to avoid zipper noise at ramp corners.
    let k = 3usize;
    let smoothed: Vec<f32> = (0..n)
        .map(|i| {
            let lo = i.saturating_sub(k);
            let hi = (i + k + 1).min(n);
            curve[lo..hi].iter().sum::<f32>() / (hi - lo) as f32
        })
        .collect();
    CorrectionCurve {
        hop_sec: hop,
        semitones: smoothed,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_in_c_major() {
        let s = Scale {
            pitch_classes: vec![0, 2, 4, 5, 7, 9, 11],
        };
        assert_eq!(s.nearest(60.3), 60.0);
        assert_eq!(s.nearest(61.4), 62.0);
        assert_eq!(s.nearest(70.6), 71.0);
    }

    #[test]
    fn flat_note_is_raised() {
        let hop = 0.01;
        // 1 s of A4 sung 30 cents flat, with vibrato.
        let f0: Vec<f32> = (0..100)
            .map(|i| {
                let m = 69.0 - 0.3 + 0.1 * (i as f32 * 0.35).sin();
                cover_dsp::util::midi_to_hz(m)
            })
            .collect();
        let track = PitchTrack {
            hop_sec: hop,
            confidence: vec![0.9; f0.len()],
            f0_hz: f0,
        };
        let c = correction_curve(
            &track,
            &Scale::chromatic(),
            &TuningParams {
                strength: 1.0,
                drift: 0.0,
                ..Default::default()
            },
            0.0,
            1.0,
        );
        assert_eq!(c.notes.len(), 1);
        assert!((c.notes[0].shift - 0.3).abs() < 0.05, "{:?}", c.notes);
        assert!((c.semitones[50] - 0.3).abs() < 0.05);
        assert!(c.semitones[0].abs() < 0.1, "attack is ramped");
    }
}
