//! Chroma features and musical key estimation (Krumhansl-Schmuckler profiles).

use crate::stft::stft;
use serde::{Deserialize, Serialize};

pub const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

const MAJOR_PROFILE: [f32; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];
const MINOR_PROFILE: [f32; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

const MAJOR_SCALE: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
const MINOR_SCALE: [u8; 7] = [0, 2, 3, 5, 7, 8, 10];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Major,
    Minor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Key {
    /// Pitch class of the tonic, 0 = C.
    pub tonic: u8,
    pub mode: Mode,
    /// Correlation of the best profile (higher is more certain).
    pub confidence: f32,
}

impl Key {
    pub fn name(&self) -> String {
        format!(
            "{} {}",
            NOTE_NAMES[self.tonic as usize % 12],
            match self.mode {
                Mode::Major => "major",
                Mode::Minor => "minor",
            }
        )
    }

    /// Pitch classes (0 = C) belonging to the key's diatonic scale.
    pub fn scale_pitch_classes(&self) -> Vec<u8> {
        let steps = match self.mode {
            Mode::Major => MAJOR_SCALE,
            Mode::Minor => MINOR_SCALE,
        };
        steps.iter().map(|s| (s + self.tonic) % 12).collect()
    }
}

/// Mean chroma vector (12 bins, C first) weighted by magnitude.
pub fn mean_chroma(x: &[f32], sample_rate: u32) -> [f32; 12] {
    let n_fft = 8192;
    let hop = 4096;
    let mut chroma = [0.0f32; 12];
    if x.len() < n_fft {
        return chroma;
    }
    let spec = stft(x, n_fft, hop);
    let bin_hz = sample_rate as f32 / n_fft as f32;
    let lo = (55.0 / bin_hz).ceil() as usize;
    let hi = ((4200.0 / bin_hz) as usize).min(spec.bins - 1);
    let pcs: Vec<usize> = (0..spec.bins)
        .map(|b| {
            let hz = (b as f32 * bin_hz).max(1.0);
            let midi = crate::util::hz_to_midi(hz).round() as i32;
            midi.rem_euclid(12) as usize
        })
        .collect();
    for f in 0..spec.frames {
        let frame = spec.frame(f);
        let mut local = [0.0f32; 12];
        for b in lo..=hi {
            // Log-compressed magnitude keeps loud bass notes from dominating.
            local[pcs[b]] += (1.0 + 100.0 * frame[b].norm()).ln();
        }
        let s: f32 = local.iter().sum::<f32>().max(1e-9);
        for (c, l) in chroma.iter_mut().zip(local) {
            *c += l / s;
        }
    }
    let s: f32 = chroma.iter().sum::<f32>().max(1e-9);
    chroma.iter_mut().for_each(|c| *c /= s);
    chroma
}

fn pearson(a: &[f32; 12], b: &[f32; 12]) -> f32 {
    let ma = a.iter().sum::<f32>() / 12.0;
    let mb = b.iter().sum::<f32>() / 12.0;
    let mut num = 0.0;
    let mut da = 0.0;
    let mut db = 0.0;
    for i in 0..12 {
        num += (a[i] - ma) * (b[i] - mb);
        da += (a[i] - ma).powi(2);
        db += (b[i] - mb).powi(2);
    }
    num / (da.sqrt() * db.sqrt()).max(1e-9)
}

pub fn key_from_chroma(chroma: &[f32; 12]) -> Key {
    let mut best = Key {
        tonic: 0,
        mode: Mode::Major,
        confidence: f32::NEG_INFINITY,
    };
    for tonic in 0..12u8 {
        for (mode, profile) in [(Mode::Major, MAJOR_PROFILE), (Mode::Minor, MINOR_PROFILE)] {
            let mut rotated = [0.0f32; 12];
            for i in 0..12 {
                rotated[(i + tonic as usize) % 12] = profile[i];
            }
            let r = pearson(chroma, &rotated);
            if r > best.confidence {
                best = Key {
                    tonic,
                    mode,
                    confidence: r,
                };
            }
        }
    }
    best
}

pub fn detect_key(x: &[f32], sample_rate: u32) -> Key {
    key_from_chroma(&mean_chroma(x, sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_a_minor_triad_progression() {
        let sr = 22_050u32;
        // Am - F - C - G (vi-IV-I-V of C major / i-VI-III-VII of A minor), with A emphasised.
        let chords: [&[f32]; 4] = [
            &[220.0, 261.63, 329.63, 110.0],
            &[174.61, 220.0, 261.63],
            &[261.63, 329.63, 392.0],
            &[196.0, 246.94, 293.66],
        ];
        let mut x = Vec::new();
        for _ in 0..2 {
            for c in chords {
                for i in 0..sr as usize {
                    let t = i as f32 / sr as f32;
                    x.push(
                        c.iter()
                            .map(|f| (2.0 * std::f32::consts::PI * f * t).sin())
                            .sum::<f32>()
                            * 0.2,
                    );
                }
            }
        }
        let key = detect_key(&x, sr);
        let pcs = key.scale_pitch_classes();
        // Either C major or A minor share the same scale; both are acceptable.
        assert!(
            pcs.contains(&0) && pcs.contains(&9) && !pcs.contains(&1),
            "{}",
            key.name()
        );
    }
}
