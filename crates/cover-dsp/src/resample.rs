//! Windowed-sinc resampler for arbitrary ratios (offline, whole-buffer).

use std::f64::consts::PI;

const HALF_TAPS: usize = 24;

fn blackman(t: f64) -> f64 {
    // t in [-1, 1]
    if t.abs() >= 1.0 {
        return 0.0;
    }
    let x = (t + 1.0) * 0.5;
    0.42 - 0.5 * (2.0 * PI * x).cos() + 0.08 * (4.0 * PI * x).cos()
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

const TABLE_RES: usize = 512;

pub fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || x.is_empty() {
        return x.to_vec();
    }
    let ratio = to as f64 / from as f64;
    let cutoff = ratio.min(1.0) * 0.97;
    let half = (HALF_TAPS as f64 / cutoff).ceil() as isize;

    // Kernel sampled on |d| in [0, half] at TABLE_RES points per input sample.
    let table_len = half as usize * TABLE_RES + 2;
    let table: Vec<f64> = (0..table_len)
        .map(|j| {
            let d = j as f64 / TABLE_RES as f64;
            cutoff * sinc(cutoff * d) * blackman(d / half as f64)
        })
        .collect();
    let kernel = |d: f64| -> f64 {
        let p = d.abs() * TABLE_RES as f64;
        let j = p as usize;
        if j + 1 >= table_len {
            return 0.0;
        }
        let f = p - j as f64;
        table[j] * (1.0 - f) + table[j + 1] * f
    };

    let out_len = (x.len() as f64 * ratio).round() as usize;
    let n = x.len() as isize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let t = i as f64 / ratio;
        let center = t.floor() as isize;
        let mut acc = 0.0f64;
        let mut wsum = 0.0f64;
        for k in (center - half + 1)..=(center + half) {
            let w = kernel(t - k as f64);
            wsum += w;
            if (0..n).contains(&k) {
                acc += x[k as usize] as f64 * w;
            }
        }
        out.push(if wsum.abs() > 1e-12 {
            (acc / wsum) as f32
        } else {
            0.0
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_low_tone() {
        let from = 44_100u32;
        let to = 16_000u32;
        let x: Vec<f32> = (0..from)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / from as f32).sin())
            .collect();
        let y = resample(&x, from, to);
        assert_eq!(y.len(), to as usize);
        let expected: Vec<f32> = (0..to)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / to as f32).sin())
            .collect();
        let err = y[200..15_800]
            .iter()
            .zip(&expected[200..15_800])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(err < 0.01, "{err}");
    }
}
