//! Small Freeverb-style room, used at low mix levels to glue stems back together.

const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;

struct Comb {
    buf: Vec<f32>,
    idx: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            idx: 0,
            store: 0.0,
        }
    }

    #[inline]
    fn tick(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.idx];
        self.store = out * (1.0 - damp) + self.store * damp;
        self.buf[self.idx] = x + self.store * feedback;
        self.idx = (self.idx + 1) % self.buf.len();
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    idx: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            idx: 0,
        }
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let b = self.buf[self.idx];
        let out = b - x;
        self.buf[self.idx] = x + b * 0.5;
        self.idx = (self.idx + 1) % self.buf.len();
        out
    }
}

/// Returns a stereo wet signal for a mono input.
pub fn room(x: &[f32], sample_rate: u32, room_size: f32, damping: f32) -> [Vec<f32>; 2] {
    let scale = sample_rate as f32 / 44_100.0;
    let sz = |n: usize| ((n as f32) * scale) as usize;
    let feedback = room_size.clamp(0.0, 1.0) * 0.28 + 0.7;
    let damp = damping.clamp(0.0, 1.0) * 0.4;

    let mut out = [vec![0.0f32; x.len()], vec![0.0f32; x.len()]];
    for (side, spread) in [0usize, STEREO_SPREAD].into_iter().enumerate() {
        let mut combs: Vec<Comb> = COMBS.iter().map(|&n| Comb::new(sz(n + spread))).collect();
        let mut aps: Vec<Allpass> = ALLPASSES
            .iter()
            .map(|&n| Allpass::new(sz(n + spread)))
            .collect();
        for (o, &s) in out[side].iter_mut().zip(x) {
            let input = s * 0.015;
            let mut acc = 0.0f32;
            for c in &mut combs {
                acc += c.tick(input, feedback, damp);
            }
            for a in &mut aps {
                acc = a.tick(acc);
            }
            *o = acc;
        }
    }
    out
}

/// Wet-only vocal plate: 30 ms pre-delay, a band-limited send (no mud, no
/// sibilance splash) and a ~1.6 s tail. The wet RMS is `level` times the
/// RMS of `x`.
pub fn vocal_plate(x: &[f32], sample_rate: u32, level: f32) -> [Vec<f32>; 2] {
    use crate::filter::{Biquad, FilterKind};
    let n = x.len();
    let pre = (0.03 * sample_rate as f32) as usize;
    let mut send = vec![0.0f32; n];
    if n > pre {
        send[pre..].copy_from_slice(&x[..n - pre]);
    }
    for k in [
        FilterKind::Highpass {
            freq: 300.0,
            q: 0.707,
        },
        FilterKind::Lowpass {
            freq: 6500.0,
            q: 0.707,
        },
    ] {
        Biquad::new(k, sample_rate).process(&mut send);
    }
    let mut wet = room(&send, sample_rate, 0.82, 0.35);
    let rd = crate::util::rms(x);
    let rw = 0.5 * (crate::util::rms(&wet[0]) + crate::util::rms(&wet[1]));
    if rw > 1e-9 {
        let g = level.max(0.0) * rd / rw;
        for w in wet.iter_mut() {
            w.iter_mut().for_each(|s| *s *= g);
        }
    }
    wet
}

/// Parallel room on a stereo signal: `dry * (1 - mix) + wet * mix`.
pub fn parallel_room(
    channels: &mut [Vec<f32>],
    sample_rate: u32,
    room_size: f32,
    damping: f32,
    mix: f32,
) {
    let mix = mix.clamp(0.0, 1.0);
    if mix <= 1e-4 || channels.is_empty() {
        return;
    }
    let n = channels[0].len();
    let mono: Vec<f32> = (0..n)
        .map(|i| channels.iter().map(|c| c[i]).sum::<f32>() / channels.len() as f32)
        .collect();
    let wet = room(&mono, sample_rate, room_size, damping);
    // Normalise the wet level so `mix` behaves like a perceptual send amount.
    let norm = {
        let rd = crate::util::rms(&mono).max(1e-9);
        let rw = crate::util::rms(&wet[0]).max(1e-9);
        rd / rw
    };
    for (ci, c) in channels.iter_mut().enumerate() {
        let w = &wet[ci.min(1)];
        for (s, wv) in c.iter_mut().zip(w) {
            *s = *s * (1.0 - mix) + wv * norm * mix;
        }
    }
}
