//! EBU R128 loudness measurement and normalisation.

use crate::buffer::AudioBuffer;
use crate::dynamics::limit;
use crate::util::{db_to_gain, gain_to_db};
use crate::{DspError, Result};
use ebur128::{EbuR128, Mode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LoudnessReport {
    pub integrated_lufs: f64,
    pub true_peak_dbtp: f64,
}

pub fn measure(buf: &AudioBuffer) -> Result<LoudnessReport> {
    let ch = buf.num_channels() as u32;
    let mut m = EbuR128::new(ch, buf.sample_rate, Mode::I | Mode::TRUE_PEAK)
        .map_err(|e| DspError::Loudness(e.to_string()))?;
    let planar: Vec<&[f32]> = buf.channels.iter().map(Vec::as_slice).collect();
    m.add_frames_planar_f32(&planar)
        .map_err(|e| DspError::Loudness(e.to_string()))?;
    let lufs = m
        .loudness_global()
        .map_err(|e| DspError::Loudness(e.to_string()))?;
    let mut tp = 0.0f64;
    for c in 0..ch {
        tp = tp.max(
            m.true_peak(c)
                .map_err(|e| DspError::Loudness(e.to_string()))?,
        );
    }
    Ok(LoudnessReport {
        integrated_lufs: lufs,
        true_peak_dbtp: gain_to_db(tp as f32) as f64,
    })
}

/// Brings `buf` to `target_lufs` and keeps true peaks under `ceiling_dbtp`.
///
/// Gain is applied first, then the look-ahead limiter catches peaks. A final
/// measurement trims the result if inter-sample peaks still overshoot.
pub fn normalize(
    buf: &mut AudioBuffer,
    target_lufs: f64,
    ceiling_dbtp: f32,
) -> Result<LoudnessReport> {
    let before = measure(buf)?;
    if before.integrated_lufs.is_finite() {
        let gain_db = (target_lufs - before.integrated_lufs) as f32;
        buf.apply_gain(db_to_gain(gain_db.clamp(-30.0, 30.0)));
    }
    let sr = buf.sample_rate;
    // Leave headroom for inter-sample peaks the sample-domain limiter cannot see.
    limit(&mut buf.channels, sr, ceiling_dbtp - 0.3, 120.0);
    let mut after = measure(buf)?;
    if after.true_peak_dbtp > ceiling_dbtp as f64 {
        let trim = (ceiling_dbtp as f64 - after.true_peak_dbtp) as f32;
        buf.apply_gain(db_to_gain(trim));
        after = measure(buf)?;
    }
    Ok(after)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_to_target() {
        let sr = 48_000u32;
        let tone: Vec<f32> = (0..sr * 5)
            .map(|i| 0.05 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin())
            .collect();
        let mut buf = AudioBuffer::new(sr, vec![tone.clone(), tone]);
        let r = normalize(&mut buf, -14.0, -1.0).unwrap();
        assert!((r.integrated_lufs + 14.0).abs() < 0.5, "{r:?}");
        assert!(r.true_peak_dbtp <= -0.99, "{r:?}");
    }
}
