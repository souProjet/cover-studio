//! Automatic clean-up for a separated lead vocal: level riding, adaptive
//! de-essing, dynamic EQ, breath control, mouth-click repair and plosive
//! detection. Every processor is conservative by design and scaled by a
//! 0..1 `amount`; multi-channel input is always processed stereo-linked.

use crate::filter::{Biquad, FilterKind};
use crate::util::{db_to_gain, percentile};

/// Voiced/unvoiced flags per pitch frame, aligned with the start of the buffer.
#[derive(Debug, Clone, Copy)]
pub struct Voicing<'a> {
    pub hop_sec: f64,
    pub voiced: &'a [bool],
}

impl Voicing<'_> {
    pub fn at_sec(&self, t: f64) -> bool {
        let i = (t / self.hop_sec).round();
        i >= 0.0 && self.voiced.get(i as usize).copied().unwrap_or(false)
    }
}

fn mono(channels: &[Vec<f32>]) -> Vec<f32> {
    let n = channels.iter().map(Vec::len).min().unwrap_or(0);
    let k = channels.len().max(1) as f32;
    (0..n)
        .map(|i| channels.iter().map(|c| c[i]).sum::<f32>() / k)
        .collect()
}

fn filtered(x: &[f32], sr: u32, chain: &[FilterKind]) -> Vec<f32> {
    let mut y = x.to_vec();
    for k in chain {
        Biquad::new(*k, sr).process(&mut y);
    }
    y
}

/// Zero-phase version of [`filtered`]: band splits built from it add back
/// up exactly, without the phase shift that would blunt a cut.
fn zero_phase(x: &[f32], sr: u32, chain: &[FilterKind]) -> Vec<f32> {
    let mut y = x.to_vec();
    for k in chain {
        y = crate::filter::filtfilt(&y, sr, *k);
    }
    y
}

fn coef(ms: f32, sr: u32) -> f32 {
    if ms <= 0.0 {
        0.0
    } else {
        (-1.0 / (ms * 0.001 * sr as f32)).exp()
    }
}

/// Mean-square envelope with separate attack and release.
fn power_envelope(x: &[f32], sr: u32, attack_ms: f32, release_ms: f32) -> Vec<f32> {
    let (a, r) = (coef(attack_ms, sr), coef(release_ms, sr));
    let mut e = 0.0f32;
    x.iter()
        .map(|v| {
            let p = v * v;
            let c = if p > e { a } else { r };
            e = p + c * (e - p);
            e
        })
        .collect()
}

fn power_db(p: f32) -> f32 {
    10.0 * p.max(1e-12).log10()
}

/// Mean-square level in dB per hop, over a centred window.
fn frame_levels_db(x: &[f32], hop: usize, window: usize) -> Vec<f32> {
    let n = x.len();
    let mut prefix = Vec::with_capacity(n + 1);
    prefix.push(0.0f64);
    for v in x {
        prefix.push(prefix.last().unwrap() + (*v as f64) * (*v as f64));
    }
    let frames = n.div_ceil(hop.max(1));
    (0..frames)
        .map(|k| {
            let c = k * hop + hop / 2;
            let a = c.saturating_sub(window / 2).min(n);
            let b = (c + window / 2).min(n);
            if b <= a {
                return -120.0;
            }
            power_db(((prefix[b] - prefix[a]) / (b - a) as f64) as f32)
        })
        .collect()
}

/// Per-sample linear interpolation of per-hop values (value k sits at the
/// centre of hop k).
fn per_sample(values: &[f32], hop: usize, n: usize) -> Vec<f32> {
    if values.is_empty() {
        return vec![0.0; n];
    }
    (0..n)
        .map(|i| {
            let pos = (i as f32 - hop as f32 / 2.0) / hop as f32;
            if pos <= 0.0 {
                return values[0];
            }
            let k = pos as usize;
            if k + 1 >= values.len() {
                return *values.last().unwrap();
            }
            let f = pos - k as f32;
            values[k] * (1.0 - f) + values[k + 1] * f
        })
        .collect()
}

/// Forward-backward one-pole smoothing (zero phase).
fn smooth_zero_phase(x: &mut [f32], frames: f32) {
    if frames <= 0.0 || x.is_empty() {
        return;
    }
    let c = (-1.0 / frames).exp();
    let mut s = x[0];
    for v in x.iter_mut() {
        s = *v + c * (s - *v);
        *v = s;
    }
    let mut s = *x.last().unwrap();
    for v in x.iter_mut().rev() {
        s = *v + c * (s - *v);
        *v = s;
    }
}

fn apply_gain(channels: &mut [Vec<f32>], gain: &[f32]) {
    for c in channels.iter_mut() {
        for (s, g) in c.iter_mut().zip(gain) {
            *s *= g;
        }
    }
}

/// Phrase-level gain riding ahead of the compressor: quiet lines come up,
/// loud lines go down, so the compressor only has to catch syllables.
/// Returns the applied gain range in dB (min, max).
pub fn level_ride(
    channels: &mut [Vec<f32>],
    sr: u32,
    amount: f32,
    voicing: Option<&Voicing>,
) -> (f32, f32) {
    let amount = amount.clamp(0.0, 1.0);
    let n = channels.iter().map(Vec::len).min().unwrap_or(0);
    let hop = (sr / 100) as usize;
    if amount < 0.01 || n < hop * 50 {
        return (0.0, 0.0);
    }
    let det = filtered(
        &mono(channels),
        sr,
        &[FilterKind::Highpass {
            freq: 100.0,
            q: 0.707,
        }],
    );
    let level = frame_levels_db(&det, hop, hop * 5);
    let frames = level.len();
    let loud = percentile(&level, 95.0);
    let floor = (loud - 30.0).max(-75.0);
    let active: Vec<bool> = (0..frames)
        .map(|k| {
            let voiced = voicing.is_none_or(|v| {
                let t = k as f64 * hop as f64 / sr as f64;
                v.at_sec(t) || v.at_sec(t - 0.05) || v.at_sec(t + 0.05)
            });
            level[k] > floor && voiced
        })
        .collect();
    if active.iter().filter(|a| **a).count() < 50 {
        return (0.0, 0.0);
    }

    // Phrase level: power average of the active frames within ±150 ms.
    let w = 15usize;
    let mut sum_p = vec![0.0f64; frames + 1];
    let mut sum_n = vec![0usize; frames + 1];
    for k in 0..frames {
        let (p, c) = if active[k] {
            (10f64.powf(level[k] as f64 / 10.0), 1)
        } else {
            (0.0, 0)
        };
        sum_p[k + 1] = sum_p[k] + p;
        sum_n[k + 1] = sum_n[k] + c;
    }
    let phrase: Vec<Option<f32>> = (0..frames)
        .map(|k| {
            if !active[k] {
                return None;
            }
            let (a, b) = (k.saturating_sub(w), (k + w + 1).min(frames));
            let cnt = sum_n[b] - sum_n[a];
            Some(power_db(((sum_p[b] - sum_p[a]) / cnt.max(1) as f64) as f32))
        })
        .collect();
    let levels: Vec<f32> = phrase.iter().flatten().copied().collect();
    let target = percentile(&levels, 55.0);
    let range = 3.0 + 6.0 * amount;

    let mut gain: Vec<Option<f32>> = phrase
        .iter()
        .map(|p| p.map(|l| ((target - l) * amount).clamp(-range, range)))
        .collect();
    // Gaps: carry the boost through short ones (consonants), never boost
    // long ones (breaths, noise between phrases).
    let mut k = 0;
    while k < frames {
        if gain[k].is_some() {
            k += 1;
            continue;
        }
        let s = k;
        while k < frames && gain[k].is_none() {
            k += 1;
        }
        let left = (s > 0).then(|| gain[s - 1].unwrap());
        let right = (k < frames).then(|| gain[k].unwrap());
        let short = k - s <= 30;
        for (j, slot) in gain.iter_mut().enumerate().take(k).skip(s) {
            let v = match (left, right) {
                (Some(a), Some(b)) => a + (b - a) * (j - s + 1) as f32 / (k - s + 1) as f32,
                (Some(a), None) => a,
                (None, Some(b)) => b,
                (None, None) => 0.0,
            };
            *slot = Some(if short { v } else { v.min(0.0) });
        }
    }
    let mut g: Vec<f32> = gain.into_iter().map(|v| v.unwrap_or(0.0)).collect();
    smooth_zero_phase(&mut g, 8.0);
    let lo = g.iter().copied().fold(f32::INFINITY, f32::min);
    let hi = g.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let lin: Vec<f32> = g.iter().map(|d| db_to_gain(*d)).collect();
    apply_gain(channels, &per_sample(&lin, hop, n));
    (lo, hi)
}

/// `out[i] = max(x[i..=i + ahead])`.
fn sliding_max_ahead(x: &[f32], ahead: usize) -> Vec<f32> {
    let n = x.len();
    let mut out = vec![0.0f32; n];
    let mut dq: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for j in (0..n).rev() {
        while dq.back().is_some_and(|&k| x[k] <= x[j]) {
            dq.pop_back();
        }
        dq.push_back(j);
        while dq.front().is_some_and(|&k| k > j + ahead) {
            dq.pop_front();
        }
        out[j] = x[*dq.front().unwrap()];
    }
    out
}

/// Finds the singer's sibilance band (peak of the average spectrum of
/// sibilant frames between 4 and 10 kHz).
fn sibilance_freq(x: &[f32], sr: u32) -> f32 {
    let n_fft = 1024usize;
    if x.len() < n_fft * 4 {
        return 6500.0;
    }
    let s = crate::stft::stft(x, n_fft, n_fft / 2);
    let bin_hz = sr as f32 / n_fft as f32;
    let b = |hz: f32| ((hz / bin_hz) as usize).min(s.bins - 1);
    let mut acc = vec![0.0f64; s.bins];
    let mut count = 0usize;
    let totals: Vec<f32> = (0..s.frames)
        .map(|f| s.frame(f)[b(150.0)..].iter().map(|c| c.norm_sqr()).sum())
        .collect();
    let gate = percentile(&totals, 50.0);
    for (f, total) in totals.iter().enumerate() {
        let fr = s.frame(f);
        let hf: f32 = fr[b(4000.0)..b(11_000.0)]
            .iter()
            .map(|c| c.norm_sqr())
            .sum();
        if *total > gate && hf > 0.45 * total {
            count += 1;
            for (a, c) in acc.iter_mut().zip(fr) {
                *a += c.norm() as f64;
            }
        }
    }
    if count < 10 {
        return 6500.0;
    }
    let (lo, hi) = (b(4500.0), b(9500.0));
    let smooth = |k: usize| acc[k - 3..=k + 3].iter().sum::<f64>();
    let best = (lo..hi)
        .max_by(|p, q| smooth(*p).total_cmp(&smooth(*q)))
        .unwrap_or(lo);
    best as f32 * bin_hz
}

/// Split-band de-esser that listens for sibilance relative to the rest of
/// the voice, so loud and quiet passages are treated alike. Returns the
/// detected sibilance frequency.
pub fn de_ess_adaptive(channels: &mut [Vec<f32>], sr: u32, amount: f32) -> f32 {
    let amount = amount.clamp(0.0, 1.0);
    let n = channels.iter().map(Vec::len).min().unwrap_or(0);
    if amount < 0.01 || n == 0 {
        return 0.0;
    }
    let m = mono(channels);
    let f_s = sibilance_freq(&m, sr).min(sr as f32 * 0.4);
    let side = filtered(&m, sr, &[FilterKind::Bandpass { freq: f_s, q: 1.0 }]);
    let full = filtered(
        &m,
        sr,
        &[FilterKind::Highpass {
            freq: 150.0,
            q: 0.707,
        }],
    );
    let es = power_envelope(&side, sr, 1.0, 40.0);
    let ef = power_envelope(&full, sr, 1.0, 40.0);
    let level: Vec<f32> = ef.iter().step_by(64).map(|p| power_db(*p)).collect();
    let gate = percentile(&level, 90.0) - 30.0;
    let thr = -7.0 - 7.0 * amount;
    let slope = 0.6 + 0.6 * amount;
    let max_red = 3.0 + 9.0 * amount;
    let mut gr: Vec<f32> = (0..n)
        .map(|i| {
            let s_db = power_db(es[i]);
            if s_db < gate {
                return 0.0;
            }
            ((s_db - power_db(ef[i]) - thr) * slope).clamp(0.0, max_red)
        })
        .collect();
    // 1.5 ms look-ahead, instant attack, 40 ms release.
    let ahead = sliding_max_ahead(&gr, (0.0015 * sr as f32) as usize);
    let rel = coef(40.0, sr);
    let mut env = 0.0f32;
    for i in 0..n {
        let ahead = ahead[i];
        env = if ahead > env {
            ahead
        } else {
            ahead + rel * (env - ahead)
        };
        gr[i] = env;
    }
    let split = FilterKind::Highpass {
        freq: f_s * 0.65,
        q: 0.707,
    };
    for c in channels.iter_mut() {
        let high = zero_phase(c, sr, &[split]);
        for i in 0..n {
            c[i] += (db_to_gain(-gr[i]) - 1.0) * high[i];
        }
    }
    f_s
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicBand {
    pub freq: f32,
    pub q: f32,
    pub max_cut_db: f32,
}

/// Mud, honk and harshness bands used by [`dynamic_eq`] on vocals.
pub const VOCAL_DYNAMIC_BANDS: [DynamicBand; 3] = [
    DynamicBand {
        freq: 300.0,
        q: 1.0,
        max_cut_db: 4.0,
    },
    DynamicBand {
        freq: 2800.0,
        q: 1.3,
        max_cut_db: 5.0,
    },
    DynamicBand {
        freq: 4300.0,
        q: 1.6,
        max_cut_db: 4.0,
    },
];

/// Dynamic EQ: each band is cut only when it rises above its own usual
/// share of the voice (a harsh note, a boomy word), never statically.
pub fn dynamic_eq(channels: &mut [Vec<f32>], sr: u32, bands: &[DynamicBand], amount: f32) {
    let amount = amount.clamp(0.0, 1.0);
    let n = channels.iter().map(Vec::len).min().unwrap_or(0);
    if amount < 0.01 || n < sr as usize / 2 {
        return;
    }
    let m = mono(channels);
    let ef = power_envelope(&m, sr, 5.0, 120.0);
    let full_db: Vec<f32> = ef.iter().step_by(64).map(|p| power_db(*p)).collect();
    let active_gate = percentile(&full_db, 90.0) - 25.0;
    let thr = 4.0 - 2.0 * amount;
    let slope = 0.5 + 0.3 * amount;
    for band in bands {
        let kind = FilterKind::Bandpass {
            freq: band.freq.min(sr as f32 * 0.45),
            q: band.q,
        };
        let eb = power_envelope(&filtered(&m, sr, &[kind]), sr, 5.0, 120.0);
        let rel: Vec<f32> = eb
            .iter()
            .zip(&ef)
            .step_by(64)
            .zip(&full_db)
            .filter(|(_, l)| **l > active_gate)
            .map(|((b, f), _)| power_db(*b) - power_db(*f))
            .collect();
        if rel.len() < 100 {
            continue;
        }
        let typical = percentile(&rel, 50.0);
        let max_cut = band.max_cut_db * amount;
        let att = coef(10.0, sr);
        let relc = coef(150.0, sr);
        let mut env = 0.0f32;
        let gains: Vec<f32> = (0..n)
            .map(|i| {
                let excess = power_db(eb[i]) - power_db(ef[i]) - typical - thr;
                let target = if power_db(ef[i]) > active_gate {
                    (excess * slope).clamp(0.0, max_cut)
                } else {
                    0.0
                };
                let c = if target > env { att } else { relc };
                env = target + c * (env - target);
                db_to_gain(-env) - 1.0
            })
            .collect();
        for c in channels.iter_mut() {
            let bp = filtered(c, sr, &[kind]);
            for i in 0..n {
                c[i] += gains[i] * bp[i];
            }
        }
    }
}

/// Lowers breaths between phrases (never removes them). Needs the pitch
/// track's voicing so consonants inside words are left alone. Returns the
/// number of breaths treated.
pub fn breath_control(channels: &mut [Vec<f32>], sr: u32, amount: f32, voicing: &Voicing) -> usize {
    let amount = amount.clamp(0.0, 1.0);
    let n = channels.iter().map(Vec::len).min().unwrap_or(0);
    let hop = (sr / 100) as usize;
    if amount < 0.01 || n < hop * 20 {
        return 0;
    }
    let m = filtered(
        &mono(channels),
        sr,
        &[FilterKind::Highpass {
            freq: 100.0,
            q: 0.707,
        }],
    );
    let hf = filtered(
        &m,
        sr,
        &[
            FilterKind::Highpass {
                freq: 4500.0,
                q: 0.707,
            },
            FilterKind::Highpass {
                freq: 4500.0,
                q: 0.707,
            },
        ],
    );
    let level = frame_levels_db(&m, hop, hop * 3);
    let hf_level = frame_levels_db(&hf, hop, hop * 3);
    let frames = level.len();
    let voiced: Vec<bool> = (0..frames)
        .map(|k| voicing.at_sec((k * hop + hop / 2) as f64 / sr as f64))
        .collect();
    let voiced_levels: Vec<f32> = (0..frames)
        .filter(|k| voiced[*k])
        .map(|k| level[k])
        .collect();
    if voiced_levels.len() < 50 {
        return 0;
    }
    let phrase = percentile(&voiced_levels, 90.0);
    let noise = percentile(&level, 10.0);
    let is_breath = |k: usize| {
        level[k] > noise + 6.0 && level[k] < phrase - 10.0 && hf_level[k] - level[k] < -3.0
    };

    let cut = db_to_gain(-(3.0 + 9.0 * amount));
    let mut gain = vec![1.0f32; frames];
    let mut count = 0;
    let mut k = 0;
    while k < frames {
        if voiced[k] {
            k += 1;
            continue;
        }
        let rs = k;
        while k < frames && !voiced[k] {
            k += 1;
        }
        let re = k;
        if re - rs < 15 {
            continue;
        }
        // Keep clear of the end of the previous word and of the next onset.
        let lo = if rs > 0 { rs + 3 } else { rs };
        let hi = if re < frames {
            re.saturating_sub(5)
        } else {
            re
        };
        let mut j = lo;
        while j < hi {
            if !is_breath(j) {
                j += 1;
                continue;
            }
            let s = j;
            let mut holes = 0;
            while j < hi && holes <= 2 {
                if is_breath(j) {
                    holes = 0;
                } else {
                    holes += 1;
                }
                j += 1;
            }
            let e = j - holes;
            if e - s >= 12 {
                count += 1;
                for slot in gain.iter_mut().take(e).skip(s) {
                    *slot = cut;
                }
            }
        }
    }
    if count == 0 {
        return 0;
    }
    // 25 ms fades.
    let mut g = gain;
    smooth_zero_phase(&mut g, 1.5);
    apply_gain(channels, &per_sample(&g, hop, n));
    count
}

/// Repairs short mouth clicks: isolated high-frequency spikes far above the
/// local noise level, at most 2 ms long. Returns the number repaired.
pub fn declick(channels: &mut [Vec<f32>], sr: u32, amount: f32) -> usize {
    let amount = amount.clamp(0.0, 1.0);
    let n = channels.iter().map(Vec::len).min().unwrap_or(0);
    if amount < 0.01 || n < sr as usize / 10 {
        return 0;
    }
    let hp = [
        FilterKind::Highpass {
            freq: 2500.0,
            q: 0.707,
        },
        FilterKind::Highpass {
            freq: 2500.0,
            q: 0.707,
        },
    ];
    let h = filtered(&mono(channels), sr, &hp);
    // Robust local level: median of 2.5 ms block RMS over ±10 ms.
    let block = ((0.0025 * sr as f32) as usize).max(8);
    let rms: Vec<f32> = h.chunks(block).map(crate::util::rms).collect();
    let local: Vec<f32> = (0..rms.len())
        .map(|b| {
            let (lo, hi) = (b.saturating_sub(4), (b + 5).min(rms.len()));
            crate::util::median(&rms[lo..hi])
        })
        .collect();
    let floor = percentile(&rms, 90.0) * 0.05;
    let crest = 9.0 - 3.0 * amount;
    let max_len = (0.002 * sr as f32) as usize;
    let merge = (0.001 * sr as f32) as usize;

    let mut events: Vec<(usize, usize)> = Vec::new();
    for (i, v) in h.iter().enumerate() {
        let r = local[i / block].max(1e-9);
        if v.abs() > crest * r && v.abs() > floor {
            match events.last_mut() {
                Some(e) if i - e.1 <= merge => e.1 = i,
                _ => events.push((i, i)),
            }
        }
    }
    // A mouth click stands alone; a consonant burst ("t", "k") is followed
    // by frication of comparable level and must keep its attack.
    let (gap, reach) = ((0.002 * sr as f32) as usize, (0.012 * sr as f32) as usize);
    events.retain(|&(a, b)| {
        let peak = h[a..=b].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let around = h[(b + gap).min(n)..(b + reach).min(n)]
            .iter()
            .chain(&h[a.saturating_sub(reach)..a.saturating_sub(gap)])
            .fold(0.0f32, |m, v| m.max(v.abs()));
        b - a <= max_len && peak > 2.5 * around
    });
    if events.is_empty() {
        return 0;
    }
    let pad = (0.001 * sr as f32) as usize;
    let mut dip = vec![0.0f32; n];
    for &(a, b) in &events {
        let (s, e) = (a.saturating_sub(2 * pad), (b + 2 * pad).min(n - 1));
        for (i, slot) in dip.iter_mut().enumerate().take(e + 1).skip(s) {
            let w = if i < a.saturating_sub(pad) {
                (i - s) as f32 / pad as f32
            } else if i > b + pad {
                (e - i) as f32 / pad as f32
            } else {
                1.0
            };
            let w = 0.5 - 0.5 * (std::f32::consts::PI * w.clamp(0.0, 1.0)).cos();
            *slot = slot.max(0.9 * w);
        }
    }
    for c in channels.iter_mut() {
        let hc = zero_phase(c, sr, &hp[..1]);
        for i in 0..n {
            c[i] -= dip[i] * hc[i];
        }
    }
    events.len()
}

/// Low-frequency correction that tames plosive pops ("p", "b"), returned as
/// a signal to add to the mix. Kept separate because the regular vocal delta
/// is high-passed and would never reach the pop's energy.
///
/// A pop is a sudden burst below 80 Hz that rivals the rest of the voice;
/// steady low notes and word onsets of low voices do not qualify because
/// their fundamental sits above the band.
pub fn plosive_delta(channels: &[Vec<f32>], sr: u32, amount: f32) -> Option<Vec<Vec<f32>>> {
    let amount = amount.clamp(0.0, 1.0);
    let n = channels.iter().map(Vec::len).min().unwrap_or(0);
    if amount < 0.01 || n < sr as usize / 2 {
        return None;
    }
    let lp = [
        FilterKind::Lowpass {
            freq: 80.0,
            q: 0.54,
        },
        FilterKind::Lowpass {
            freq: 80.0,
            q: 1.31,
        },
    ];
    let m = mono(channels);
    let low = filtered(&m, sr, &lp);
    let rest = filtered(
        &m,
        sr,
        &[
            FilterKind::Highpass {
                freq: 80.0,
                q: 0.54,
            },
            FilterKind::Highpass {
                freq: 80.0,
                q: 1.31,
            },
        ],
    );
    let fast = power_envelope(&low, sr, 1.0, 30.0);
    let slow = power_envelope(&low, sr, 300.0, 300.0);
    let er = power_envelope(&rest, sr, 2.0, 40.0);
    let rest_db: Vec<f32> = er.iter().step_by(64).map(|p| power_db(*p)).collect();
    let gate = percentile(&rest_db, 90.0) - 30.0;
    let max = 12.0 * amount;
    let rel = coef(40.0, sr);
    let max_len = (0.15 * sr as f32) as usize;
    let mut since: Option<usize> = None;
    let mut count = 0;
    let target: Vec<f32> = (0..n)
        .map(|i| {
            let l = power_db(fast[i]);
            let over_rest = l - power_db(er[i]);
            let sudden = l - power_db(slow[i]) > 8.0;
            if since.is_none() && l > gate && sudden && over_rest > -4.0 {
                since = Some(i);
                count += 1;
            }
            if let Some(s) = since {
                if i - s > max_len || over_rest < -8.0 {
                    since = None;
                }
            }
            if since.is_some() {
                ((over_rest + 8.0) * 1.2).clamp(0.0, max)
            } else {
                0.0
            }
        })
        .collect();
    // The detector lags the pop's first half-cycle: look 12 ms ahead.
    let ahead = sliding_max_ahead(&target, (0.012 * sr as f32) as usize);
    let mut env = 0.0f32;
    let gr: Vec<f32> = ahead
        .iter()
        .map(|&t| {
            env = if t > env { t } else { t + rel * (env - t) };
            db_to_gain(-env) - 1.0
        })
        .collect();
    if count == 0 {
        return None;
    }
    Some(
        channels
            .iter()
            .map(|c| {
                let lc = zero_phase(c, sr, &lp);
                lc.iter().zip(&gr).map(|(v, g)| v * g).collect()
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::rms;
    use std::f32::consts::PI;

    const SR: u32 = 44_100;

    fn noise(n: usize, seed: &mut u32) -> Vec<f32> {
        (0..n)
            .map(|_| {
                *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (*seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect()
    }

    /// Harmonic "vowel" at f0 with a falling spectrum.
    fn vowel(n: usize, f0: f32, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f32 / SR as f32;
                (1..12)
                    .map(|h| (2.0 * PI * f0 * h as f32 * t).sin() / h as f32)
                    .sum::<f32>()
                    * amp
            })
            .collect()
    }

    #[test]
    fn leveler_evens_out_phrases() {
        let sec = SR as usize;
        let mut x = vowel(2 * sec, 220.0, 0.05);
        x.extend(vowel(2 * sec, 247.0, 0.4));
        let mut ch = vec![x];
        let (lo, hi) = level_ride(&mut ch, SR, 0.8, None);
        let q = rms(&ch[0][sec / 2..3 * sec / 2]);
        let l = rms(&ch[0][5 * sec / 2..7 * sec / 2]);
        let before = 20.0 * (0.4f32 / 0.05).log10();
        let after = 20.0 * (l / q).log10();
        assert!(after < before - 6.0, "spread {before} dB -> {after} dB");
        assert!(lo < 0.0 && hi > 0.0);
    }

    #[test]
    fn de_esser_reduces_sibilance_not_vowels() {
        let sec = SR as usize;
        let mut seed = 7;
        let v = vowel(sec, 200.0, 0.3);
        let s: Vec<f32> = filtered(
            &noise(sec / 4, &mut seed),
            SR,
            &[FilterKind::Bandpass {
                freq: 7000.0,
                q: 1.0,
            }],
        )
        .iter()
        .map(|x| x * 1.5)
        .collect();
        let mut x = v.clone();
        x.extend(&s);
        x.extend(&v);
        let mut ch = vec![x.clone()];
        let f = de_ess_adaptive(&mut ch, SR, 0.6);
        assert!((5000.0..9000.0).contains(&f), "{f}");
        let sib = |y: &[f32]| rms(&y[sec + 1000..sec + sec / 4 - 1000]);
        let vow = |y: &[f32]| rms(&y[sec / 4..3 * sec / 4]);
        assert!(sib(&ch[0]) < 0.6 * sib(&x), "sibilance not reduced");
        assert!((vow(&ch[0]) / vow(&x) - 1.0).abs() < 0.03, "vowel changed");
    }

    #[test]
    fn dynamic_eq_only_cuts_when_a_band_jumps_out() {
        let sec = SR as usize;
        let mut seed = 3;
        let base = vowel(3 * sec, 180.0, 0.2);
        // A harsh 2.8 kHz resonance appears only in the last second.
        let mut harsh = filtered(
            &noise(3 * sec, &mut seed),
            SR,
            &[FilterKind::Bandpass {
                freq: 2800.0,
                q: 4.0,
            }],
        );
        harsh[..2 * sec].iter_mut().for_each(|v| *v = 0.0);
        let x: Vec<f32> = base.iter().zip(&harsh).map(|(a, b)| a + 2.0 * b).collect();
        let mut ch = vec![x.clone()];
        dynamic_eq(&mut ch, SR, &VOCAL_DYNAMIC_BANDS, 0.8);
        let band = |y: &[f32], a: usize, b: usize| {
            rms(&filtered(
                &y[a..b],
                SR,
                &[FilterKind::Bandpass {
                    freq: 2800.0,
                    q: 4.0,
                }],
            ))
        };
        let calm = band(&ch[0], sec / 2, 3 * sec / 2) / band(&x, sec / 2, 3 * sec / 2);
        let loud = band(&ch[0], 5 * sec / 2, 3 * sec) / band(&x, 5 * sec / 2, 3 * sec);
        assert!(calm > 0.93, "calm passage cut by {calm}");
        assert!(loud < 0.8, "harsh passage only scaled by {loud}");
    }

    #[test]
    fn breaths_between_phrases_are_lowered() {
        let sec = SR as usize;
        let mut seed = 11;
        let phrase = vowel(sec, 220.0, 0.3);
        let breath: Vec<f32> = filtered(
            &noise(sec / 2, &mut seed),
            SR,
            &[FilterKind::Bandpass {
                freq: 1500.0,
                q: 0.7,
            }],
        )
        .iter()
        .map(|v| v * 0.08)
        .collect();
        let mut x = phrase.clone();
        x.extend(&breath);
        x.extend(&phrase);
        let silence: Vec<f32> = noise(sec / 2, &mut seed).iter().map(|v| v * 1e-4).collect();
        x.extend(&silence);
        let voiced: Vec<bool> = (0..300)
            .map(|k| !(100..150).contains(&k) && k < 250)
            .collect();
        let v = Voicing {
            hop_sec: 0.01,
            voiced: &voiced,
        };
        let mut ch = vec![x.clone()];
        assert_eq!(breath_control(&mut ch, SR, 0.5, &v), 1);
        let mid = |y: &[f32]| rms(&y[sec + sec / 8..sec + 3 * sec / 8]);
        let ratio = mid(&ch[0]) / mid(&x);
        assert!(ratio < 0.6 && ratio > 0.2, "breath scaled by {ratio}");
        let p = |y: &[f32]| rms(&y[sec / 4..3 * sec / 4]);
        assert!((p(&ch[0]) / p(&x) - 1.0).abs() < 0.01);
    }

    #[test]
    fn mouth_clicks_are_repaired_and_vowels_kept() {
        let sec = SR as usize;
        let mut x = vowel(sec, 200.0, 0.2);
        let mut seed = 5;
        let hiss = noise(sec, &mut seed);
        for (a, b) in x.iter_mut().zip(&hiss) {
            *a += 0.002 * b;
        }
        let clean = x.clone();
        for at in [11_000usize, 30_000] {
            x[at] += 0.5;
            x[at + 1] -= 0.4;
        }
        let mut ch = vec![x.clone()];
        assert_eq!(declick(&mut ch, SR, 0.6), 2);
        let err = |y: &[f32]| {
            rms(&y[10_900..11_100]
                .iter()
                .zip(&clean[10_900..11_100])
                .map(|(a, b)| a - b)
                .collect::<Vec<_>>())
        };
        assert!(
            err(&ch[0]) < 0.5 * err(&x),
            "{} vs {}",
            err(&ch[0]),
            err(&x)
        );
        let mut plain = vec![clean.clone()];
        assert_eq!(
            declick(&mut plain, SR, 0.6),
            0,
            "no false positives on a vowel"
        );
        // A "t": sharp burst followed by frication keeps its attack.
        let mut t = clean.clone();
        t[20_000] += 0.5;
        t[20_001] -= 0.4;
        let fric = noise(SR as usize / 20, &mut seed);
        for (i, v) in fric.iter().enumerate() {
            t[20_003 + i] += 0.9 * v * (-(i as f32) / 900.0).exp();
        }
        let mut ch = vec![t];
        assert_eq!(declick(&mut ch, SR, 0.6), 0, "consonant burst left alone");
    }

    #[test]
    fn plosive_pop_is_detected_but_low_voices_are_not() {
        let sec = SR as usize;
        let mut x = vowel(sec, 250.0, 0.2);
        let pop: Vec<f32> = (0..sec / 20)
            .map(|i| {
                let t = i as f32 / SR as f32;
                0.6 * (2.0 * PI * 45.0 * t).sin() * (-t * 40.0).exp()
            })
            .collect();
        for (i, p) in pop.iter().enumerate() {
            x[sec / 2 + i] += p;
        }
        let d = plosive_delta(&[x.clone()], SR, 0.6).expect("pop found");
        let fixed: Vec<f32> = x.iter().zip(&d[0]).map(|(a, b)| a + b).collect();
        let w = sec / 2..sec / 2 + sec / 20;
        let lows = |y: &[f32]| {
            rms(&filtered(
                y,
                SR,
                &[FilterKind::Lowpass {
                    freq: 80.0,
                    q: 0.707,
                }],
            )[w.clone()])
        };
        let ratio = lows(&fixed) / lows(&x);
        assert!(ratio < 0.65, "pop low band scaled by {ratio}");
        // A low male voice (98 Hz) starting from silence has a strong
        // fundamental but no pop.
        let mut low = vec![0.0f32; sec / 4];
        low.extend(vowel(sec, 98.0, 0.3));
        let d = plosive_delta(&[low.clone()], SR, 0.6);
        let moved = d.map_or(0.0, |d| rms(&d[0]) / rms(&low));
        assert!(moved < 0.05, "low voice altered by {moved}");
    }
}
