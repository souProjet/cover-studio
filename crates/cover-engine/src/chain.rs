//! Processing chains for the vocal stem, the instrument stem and the mix bus.

use crate::settings::{InstrumentKind, VocalSettings};
use cover_dsp::analysis::rms_envelope;
use cover_dsp::dynamics::{compress, CompressorSettings};
use cover_dsp::filter::{Biquad, FilterKind};
use cover_dsp::spectral::{spectral_clean, SpectralCleanSettings};
use cover_dsp::util::{gain_to_db, percentile, rms};
use cover_dsp::vocal::{
    breath_control, de_ess_adaptive, declick, dynamic_eq, level_ride, Voicing, VOCAL_DYNAMIC_BANDS,
};
use serde::{Deserialize, Serialize};

fn eq(channels: &mut [Vec<f32>], sr: u32, chain: &[FilterKind]) {
    for ch in channels.iter_mut() {
        for k in chain {
            Biquad::new(*k, sr).process(ch);
        }
    }
}

/// Level of the loud passages (90th percentile of 50 ms RMS), in dBFS.
fn active_level_db(channels: &[Vec<f32>], sr: u32) -> f32 {
    let mono: Vec<f32> = (0..channels[0].len())
        .map(|i| channels.iter().map(|c| c[i]).sum::<f32>() / channels.len() as f32)
        .collect();
    let env: Vec<f32> = rms_envelope(&mono, (sr / 20) as usize)
        .into_iter()
        .filter(|v| *v > 1e-4)
        .map(gain_to_db)
        .collect();
    if env.is_empty() {
        -60.0
    } else {
        percentile(&env, 90.0)
    }
}

fn match_level(processed: &mut [Vec<f32>], reference: &[Vec<f32>]) {
    let r_ref: f32 = reference.iter().map(|c| rms(c)).sum();
    let r_p: f32 = processed.iter().map(|c| rms(c)).sum();
    if r_p > 1e-9 {
        let g = r_ref / r_p;
        for c in processed.iter_mut() {
            c.iter_mut().for_each(|s| *s *= g);
        }
    }
}

/// What the automatic vocal clean-up found and did, for the render log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct VocalReport {
    pub snr_db: f32,
    pub denoised: bool,
    pub clicks: usize,
    pub breaths: usize,
    pub leveler_min_db: f32,
    pub leveler_max_db: f32,
    pub sibilance_hz: f32,
}

/// Clean-up, levelling, EQ, de-essing and compression for a (possibly
/// pitch-corrected) vocal stem. `voicing` comes from the pitch track and
/// lets breath control and the leveler tell singing from breaths and noise.
/// Output level matches the input so blends do not change the balance.
pub fn process_vocal(
    input: &[Vec<f32>],
    sr: u32,
    s: &VocalSettings,
    voicing: Option<&Voicing>,
) -> (Vec<Vec<f32>>, VocalReport) {
    let mut report = VocalReport::default();
    let (mut out, spec) = spectral_clean(
        input,
        sr,
        &SpectralCleanSettings {
            denoise: s.cleanup,
            resonance: s.harshness,
        },
    );
    report.snr_db = spec.snr_db;
    report.denoised = spec.denoised;
    report.clicks = declick(&mut out, sr, s.cleanup);
    if let Some(v) = voicing {
        report.breaths = breath_control(&mut out, sr, s.breath, v);
    }
    (report.leveler_min_db, report.leveler_max_db) = level_ride(&mut out, sr, s.leveler, voicing);
    eq(
        &mut out,
        sr,
        &[
            FilterKind::Highpass {
                freq: 80.0,
                q: 0.707,
            },
            FilterKind::Peak {
                freq: 260.0,
                q: 0.9,
                gain_db: -1.5,
            },
            FilterKind::Peak {
                freq: 3000.0,
                q: 0.9,
                gain_db: s.presence_db,
            },
            FilterKind::HighShelf {
                freq: 10_000.0,
                q: 0.6,
                gain_db: s.air_db,
            },
        ],
    );
    dynamic_eq(&mut out, sr, &VOCAL_DYNAMIC_BANDS, s.harshness);
    report.sibilance_hz = de_ess_adaptive(&mut out, sr, s.de_ess);
    let amount = s.compression.clamp(0.0, 1.0);
    if amount > 0.01 {
        let level = active_level_db(&out, sr);
        compress(
            &mut out,
            sr,
            &CompressorSettings {
                threshold_db: level - 4.0 - 8.0 * amount,
                ratio: 1.6 + 2.4 * amount,
                attack_ms: 12.0 - 5.0 * amount,
                release_ms: 160.0,
                knee_db: 6.0,
                makeup_db: 0.0,
            },
        );
    }
    match_level(&mut out, input);
    (out, report)
}

fn instrument_eq(kind: InstrumentKind) -> Vec<FilterKind> {
    use FilterKind::*;
    match kind {
        InstrumentKind::AcousticGuitar => vec![
            Highpass {
                freq: 70.0,
                q: 0.707,
            },
            Peak {
                freq: 220.0,
                q: 1.0,
                gain_db: -1.8,
            },
            Peak {
                freq: 3500.0,
                q: 0.8,
                gain_db: 1.0,
            },
            HighShelf {
                freq: 9000.0,
                q: 0.6,
                gain_db: 0.8,
            },
        ],
        InstrumentKind::ClassicalGuitar => vec![
            Highpass {
                freq: 62.0,
                q: 0.707,
            },
            LowShelf {
                freq: 120.0,
                q: 0.6,
                gain_db: 1.0,
            },
            Peak {
                freq: 275.0,
                q: 1.1,
                gain_db: -2.0,
            },
            Peak {
                freq: 3600.0,
                q: 0.85,
                gain_db: 0.9,
            },
            HighShelf {
                freq: 7200.0,
                q: 0.55,
                gain_db: -0.8,
            },
        ],
        InstrumentKind::ElectricGuitar => vec![
            Highpass {
                freq: 85.0,
                q: 0.707,
            },
            Peak {
                freq: 400.0,
                q: 0.9,
                gain_db: -1.2,
            },
            Peak {
                freq: 2200.0,
                q: 0.8,
                gain_db: 1.0,
            },
            Lowpass {
                freq: 11_000.0,
                q: 0.707,
            },
        ],
        InstrumentKind::Piano => vec![
            Highpass {
                freq: 35.0,
                q: 0.707,
            },
            Peak {
                freq: 300.0,
                q: 0.8,
                gain_db: -1.5,
            },
            Peak {
                freq: 2500.0,
                q: 0.7,
                gain_db: 0.8,
            },
            HighShelf {
                freq: 8000.0,
                q: 0.6,
                gain_db: 1.0,
            },
        ],
        InstrumentKind::Ukulele => vec![
            Highpass {
                freq: 150.0,
                q: 0.707,
            },
            Peak {
                freq: 400.0,
                q: 1.0,
                gain_db: -1.5,
            },
            Peak {
                freq: 3000.0,
                q: 0.8,
                gain_db: 1.0,
            },
        ],
        InstrumentKind::FullBand | InstrumentKind::Other => vec![
            Highpass {
                freq: 30.0,
                q: 0.707,
            },
            Peak {
                freq: 300.0,
                q: 0.7,
                gain_db: -1.0,
            },
            HighShelf {
                freq: 9000.0,
                q: 0.6,
                gain_db: 0.6,
            },
        ],
    }
}

/// Clean-up EQ and gentle glue compression for the accompaniment stem.
pub fn process_instrument(input: &[Vec<f32>], sr: u32, kind: InstrumentKind) -> Vec<Vec<f32>> {
    let mut out = input.to_vec();
    eq(&mut out, sr, &instrument_eq(kind));
    let level = active_level_db(&out, sr);
    compress(
        &mut out,
        sr,
        &CompressorSettings {
            threshold_db: level - 6.0,
            ratio: 1.6,
            attack_ms: 18.0,
            release_ms: 200.0,
            knee_db: 6.0,
            makeup_db: 0.0,
        },
    );
    match_level(&mut out, input);
    out
}

/// Bus processing before mastering: glue compression, warmth, gentle top trim.
pub fn process_bus(channels: &mut [Vec<f32>], sr: u32, warmth_db: f32) {
    let level = active_level_db(channels, sr);
    compress(
        channels,
        sr,
        &CompressorSettings {
            threshold_db: level - 4.0,
            ratio: 1.4,
            attack_ms: 30.0,
            release_ms: 220.0,
            knee_db: 8.0,
            makeup_db: 0.0,
        },
    );
    eq(
        channels,
        sr,
        &[
            FilterKind::Highpass {
                freq: 30.0,
                q: 0.707,
            },
            FilterKind::LowShelf {
                freq: 150.0,
                q: 0.6,
                gain_db: warmth_db,
            },
            FilterKind::Peak {
                freq: 6200.0,
                q: 1.2,
                gain_db: -0.6,
            },
        ],
    );
}
