//! Song key from the accompaniment's harmony and the sung melody.

use cover_dsp::analysis::key::{key_from_chroma, mean_chroma, Key};
use cover_dsp::util::hz_to_midi;
use cover_ml::PitchTrack;

/// Duration-weighted pitch-class histogram of the voiced frames.
fn melody_chroma(pitch: &PitchTrack) -> [f32; 12] {
    let mut h = [0.0f32; 12];
    for (f, c) in pitch.f0_hz.iter().zip(&pitch.confidence) {
        if *f > 0.0 && *c > 0.5 {
            let pc = (hz_to_midi(*f).round() as i32).rem_euclid(12) as usize;
            h[pc] += c;
        }
    }
    let s: f32 = h.iter().sum::<f32>().max(1e-9);
    h.iter_mut().for_each(|v| *v /= s);
    h
}

pub fn detect_key(accompaniment: &[f32], sr: u32, pitch: Option<&PitchTrack>) -> Key {
    let harmony = mean_chroma(accompaniment, sr);
    let combined = match pitch {
        Some(p) => {
            let m = melody_chroma(p);
            let mut c = [0.0f32; 12];
            for i in 0..12 {
                c[i] = 0.6 * harmony[i] + 0.4 * m[i];
            }
            c
        }
        None => harmony,
    };
    key_from_chroma(&combined)
}
