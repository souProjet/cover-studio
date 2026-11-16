//! FFT cross-correlation for aligning two recordings of the same performance.

use realfft::RealFftPlanner;

fn normalize(x: &[f32]) -> Vec<f32> {
    let m = crate::util::mean(x);
    let s = crate::util::std_dev(x).max(1e-8);
    x.iter().map(|v| (v - m) / s).collect()
}

/// Lag (in samples of the input sequences) that best aligns `other` onto
/// `reference`, searched in `[-max_lag, max_lag]`.
///
/// A positive lag means `other` is late: `other[n + lag] ≈ reference[n]`.
/// The score is the correlation at the best lag, scaled by how much of the
/// shorter sequence overlaps (in -1..=1).
pub fn best_lag(reference: &[f32], other: &[f32], max_lag: usize) -> (isize, f32) {
    if reference.is_empty() || other.is_empty() {
        return (0, 0.0);
    }
    let a = normalize(reference);
    let b = normalize(other);
    let n = (a.len() + b.len()).next_power_of_two();
    let mut planner = RealFftPlanner::<f32>::new();
    let fwd = planner.plan_fft_forward(n);
    let inv = planner.plan_fft_inverse(n);

    let mut ia = vec![0.0f32; n];
    ia[..a.len()].copy_from_slice(&a);
    let mut ib = vec![0.0f32; n];
    ib[..b.len()].copy_from_slice(&b);
    let mut fa = fwd.make_output_vec();
    let mut fb = fwd.make_output_vec();
    fwd.process(&mut ia, &mut fa).expect("fft size");
    fwd.process(&mut ib, &mut fb).expect("fft size");
    // corr[lag] = sum_n a[n] * b[n + lag]  ->  conj(A) * B
    let mut prod: Vec<_> = fa.iter().zip(&fb).map(|(x, y)| x.conj() * y).collect();
    prod[0].im = 0.0;
    if let Some(l) = prod.last_mut() {
        l.im = 0.0;
    }
    let mut corr = vec![0.0f32; n];
    inv.process(&mut prod, &mut corr).expect("fft size");

    let max_lag = max_lag.min(n / 2 - 1) as isize;
    // Biased normalisation: lags with little overlap cannot win on noise alone.
    let norm = a.len().min(b.len()) as f32;
    let mut best = (0isize, f32::NEG_INFINITY);
    for lag in -max_lag..=max_lag {
        let idx = if lag >= 0 {
            lag as usize
        } else {
            (n as isize + lag) as usize
        };
        let overlap = if lag >= 0 {
            a.len().min(b.len().saturating_sub(lag as usize))
        } else {
            b.len().min(a.len().saturating_sub((-lag) as usize))
        };
        if overlap < 8 {
            continue;
        }
        let v = corr[idx] / n as f32 / norm;
        if v > best.1 {
            best = (lag, v);
        }
    }
    (best.0, best.1.clamp(-1.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_known_shift() {
        let x: Vec<f32> = (0..4000)
            .map(|i| {
                ((i as f32 * 0.071).sin() * (i as f32 * 0.0031).cos()).powi(3)
                    + ((i * 7919) % 13) as f32 * 0.01
            })
            .collect();
        let shift = 37usize;
        let mut late = vec![0.0f32; shift];
        late.extend_from_slice(&x);
        let (lag, score) = best_lag(&x, &late, 200);
        assert_eq!(lag, shift as isize);
        assert!(score > 0.9, "{score}");
        let (lag2, _) = best_lag(&late, &x, 200);
        assert_eq!(lag2, -(shift as isize));
    }
}
