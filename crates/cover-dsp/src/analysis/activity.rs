//! Splits a recording into active regions separated by silence. Used to find
//! the individual takes in a long session recording.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub start_sec: f64,
    pub end_sec: f64,
}

impl Region {
    pub fn duration(&self) -> f64 {
        self.end_sec - self.start_sec
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ActivityParams {
    /// Analysis window in seconds.
    pub window_sec: f64,
    /// Regions quieter than this many dB under the loud level are silence.
    pub floor_db: f32,
    /// Gaps shorter than this are bridged (pauses between phrases).
    pub min_gap_sec: f64,
    /// Regions shorter than this are dropped.
    pub min_region_sec: f64,
    /// Median smoothing of the level, so handling noise between takes does
    /// not bridge them.
    pub smooth_sec: f64,
}

impl Default for ActivityParams {
    fn default() -> Self {
        Self {
            window_sec: 0.05,
            floor_db: 30.0,
            min_gap_sec: 2.5,
            min_region_sec: 8.0,
            smooth_sec: 0.5,
        }
    }
}

pub fn active_regions(x: &[f32], sample_rate: u32, p: &ActivityParams) -> Vec<Region> {
    let hop = ((p.window_sec * sample_rate as f64) as usize).max(1);
    let env = super::rms_envelope(x, hop);
    if env.is_empty() {
        return Vec::new();
    }
    let db: Vec<f32> = env.iter().map(|v| crate::util::gain_to_db(*v)).collect();
    let db = crate::util::median_filter(&db, (p.smooth_sec / p.window_sec).round() as usize);
    let loud = crate::util::percentile(&db, 95.0);
    let noise = crate::util::percentile(&db, 10.0);
    let thr = (loud - p.floor_db).max(noise + 6.0);
    let active: Vec<bool> = db.iter().map(|v| *v > thr).collect();

    let hop_sec = hop as f64 / sample_rate as f64;
    let mut regions: Vec<Region> = Vec::new();
    let mut i = 0;
    while i < active.len() {
        if !active[i] {
            i += 1;
            continue;
        }
        let s = i;
        while i < active.len() && active[i] {
            i += 1;
        }
        let r = Region {
            start_sec: s as f64 * hop_sec,
            end_sec: i as f64 * hop_sec,
        };
        match regions.last_mut() {
            Some(last) if r.start_sec - last.end_sec < p.min_gap_sec => last.end_sec = r.end_sec,
            _ => regions.push(r),
        }
    }
    regions.retain(|r| r.duration() >= p.min_region_sec);
    regions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_two_takes() {
        let sr = 8_000u32;
        let mut x = vec![0.0005f32; sr as usize * 60];
        for range in [(5usize, 20usize), (30, 55)] {
            let (a, b) = (range.0 * sr as usize, range.1 * sr as usize);
            for (i, v) in x.iter_mut().enumerate().take(b).skip(a) {
                *v = 0.3 * ((i as f32) * 0.2).sin();
            }
        }
        let r = active_regions(&x, sr, &ActivityParams::default());
        assert_eq!(r.len(), 2, "{r:?}");
        assert!((r[0].start_sec - 5.0).abs() < 0.1);
        assert!((r[1].end_sec - 55.0).abs() < 0.1);
    }
}
