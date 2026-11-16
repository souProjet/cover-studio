//! Pitch correction curve.
//!
//! The sung melody is split into notes. Each note gets a target (nearest
//! allowed pitch, with hysteresis and chromatic passing-tone detection) and a
//! per-frame correction made of:
//! - a constant note-level offset, which keeps the singer's contour intact;
//! - a share of the slow drift of the vibrato-free centre around the note
//!   level (held notes, flat tails, attack overshoot), gated so that wide
//!   deliberate slides pass through;
//! - an optional reduction of excessive vibrato depth.
//!
//! Corrections glide across short gaps inside a phrase instead of dropping to
//! zero between notes, which would make the pitch wobble at every consonant.

use cover_dsp::analysis::key::Key;
use cover_dsp::util::{hz_to_midi, median, percentile};
use cover_ml::PitchTrack;
use serde::{Deserialize, Serialize};

/// Allowed pitch classes, on a grid that may be offset from A = 440 Hz.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scale {
    pub pitch_classes: Vec<u8>,
    /// Tuning reference of the recording, in semitones from the A440 grid.
    #[serde(default)]
    pub offset: f32,
}

impl Scale {
    pub fn chromatic() -> Self {
        Self {
            pitch_classes: (0..12).collect(),
            offset: 0.0,
        }
    }

    pub fn from_key(key: &Key) -> Self {
        Self {
            pitch_classes: key.scale_pitch_classes(),
            offset: 0.0,
        }
    }

    pub fn with_offset(mut self, semitones: f32) -> Self {
        self.offset = semitones.clamp(-0.5, 0.5);
        self
    }

    /// Nearest grid semitone (any pitch class).
    pub fn semitone(&self, midi: f32) -> f32 {
        (midi - self.offset).round() + self.offset
    }

    /// Whether the semitone nearest to `midi` belongs to the scale.
    pub fn contains(&self, midi: f32) -> bool {
        let pc = ((midi - self.offset).round() as i32).rem_euclid(12) as u8;
        self.pitch_classes.contains(&pc)
    }

    /// Nearest allowed midi note to a fractional midi value.
    pub fn nearest(&self, midi: f32) -> f32 {
        let m = midi - self.offset;
        let base = (m / 12.0).floor() * 12.0;
        let mut best = m.round();
        let mut best_d = f32::INFINITY;
        for oct in [-12.0, 0.0, 12.0] {
            for pc in &self.pitch_classes {
                let cand = base + oct + *pc as f32;
                let d = (cand - m).abs();
                if d < best_d {
                    best_d = d;
                    best = cand;
                }
            }
        }
        best + self.offset
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TuningParams {
    /// 0..1 fraction of the error that gets corrected.
    pub strength: f32,
    /// 0 (natural, note-level) .. 1 (tight): how much of attacks, overshoots
    /// and small scoops gets pulled onto the target.
    pub retune_speed: f32,
    /// 0..1, pulls slow drift inside held notes towards the target.
    pub drift: f32,
    /// 0..1, reduces vibrato deeper than [`VIBRATO_EXCESS_ST`]. Normal
    /// vibrato is never touched.
    pub vibrato: f32,
    /// Note-level errors smaller than this (semitones) are left alone.
    pub min_shift: f32,
    /// Never shift further than this (semitones).
    pub max_shift: f32,
}

impl Default for TuningParams {
    fn default() -> Self {
        Self {
            strength: 0.8,
            retune_speed: 0.35,
            drift: 0.3,
            vibrato: 0.0,
            min_shift: 0.06,
            max_shift: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub start_sec: f64,
    pub end_sec: f64,
    pub sung_midi: f32,
    pub target_midi: f32,
    /// Mean applied correction over the note, in semitones.
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
        self.at_sec(sample as f64 / sample_rate as f64)
    }

    pub fn at_sec(&self, t: f64) -> f32 {
        let pos = t / self.hop_sec;
        let i = pos as usize;
        if i + 1 >= self.semitones.len() {
            return self.semitones.last().copied().unwrap_or(0.0);
        }
        let frac = (pos - i as f64) as f32;
        self.semitones[i] * (1.0 - frac) + self.semitones[i + 1] * frac
    }

    pub fn is_flat(&self) -> bool {
        self.semitones.iter().all(|v| v.abs() < 1e-3)
    }
}

/// Vibrato deeper than this (semitones, peak amplitude) counts as excessive.
pub const VIBRATO_EXCESS_ST: f32 = 0.4;
const VOICED_CONF: f32 = 0.5;
const MIN_NOTE_SEC: f64 = 0.06;
/// Sustained jump that starts a new note.
const SPLIT_ST: f32 = 0.75;
/// Adjacent segments closer than this are one note (vibrato, wobble).
const MERGE_ST: f32 = 0.45;
/// Gaps shorter than this inside a phrase are bridged.
const BRIDGE_SEC: f64 = 0.25;
const RAMP_SEC: f64 = 0.06;
/// Longest half-cycle of a slow (3.8 Hz) vibrato, in 10 ms frames.
const HALF_VIBRATO_FRAMES: usize = 13;

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    if e1 <= e0 {
        return if x >= e1 { 1.0 } else { 0.0 };
    }
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// Centred moving average with a shrinking window at the edges.
fn boxcar(x: &[f32], window: usize) -> Vec<f32> {
    let h = window.max(1) / 2;
    let mut prefix = Vec::with_capacity(x.len() + 1);
    prefix.push(0.0f64);
    for v in x {
        prefix.push(prefix.last().unwrap() + *v as f64);
    }
    (0..x.len())
        .map(|i| {
            let a = i.saturating_sub(h);
            let b = (i + h + 1).min(x.len());
            ((prefix[b] - prefix[a]) / (b - a) as f64) as f32
        })
        .collect()
}

/// Vibrato-free centre: two passes of a moving average one vibrato period
/// long, which puts a null right on the vibrato rate.
fn centre_line(x: &[f32], window: usize) -> Vec<f32> {
    boxcar(&boxcar(x, window), window)
}

/// Midi pitch per frame with octave jumps folded back and tiny dropouts
/// filled. Unvoiced frames are `None`.
fn clean_track(pitch: &PitchTrack) -> Vec<Option<f32>> {
    let n = pitch.f0_hz.len();
    let mut m: Vec<Option<f32>> = (0..n)
        .map(|i| {
            let f = pitch.f0_hz[i];
            let c = pitch.confidence.get(i).copied().unwrap_or(0.0);
            (f > 60.0 && f < 1600.0 && c > VOICED_CONF).then(|| hz_to_midi(f))
        })
        .collect();

    // Short runs an octave away from their surroundings are tracking errors.
    let reference: Vec<Option<f32>> = (0..n)
        .map(|i| {
            m[i]?;
            let a = i.saturating_sub(25);
            let b = (i + 26).min(n);
            let v: Vec<f32> = m[a..b].iter().flatten().copied().collect();
            Some(median(&v))
        })
        .collect();
    let mut i = 0;
    while i < n {
        let off = match (m[i], reference[i]) {
            (Some(v), Some(r)) => (v - r).abs() > 7.0,
            _ => false,
        };
        if !off {
            i += 1;
            continue;
        }
        let s = i;
        while i < n && matches!((m[i], reference[i]), (Some(v), Some(r)) if (v - r).abs() > 7.0) {
            i += 1;
        }
        for k in s..i {
            let (v, r) = (m[k].unwrap(), reference[k].unwrap());
            let folded = v - 12.0 * ((v - r) / 12.0).round();
            m[k] = (i - s < 15 && (folded - r).abs() < 3.0).then_some(folded);
        }
    }

    // Fill dropouts of up to 3 frames inside a voiced run.
    let mut i = 0;
    while i < n {
        if m[i].is_some() {
            i += 1;
            continue;
        }
        let s = i;
        while i < n && m[i].is_none() {
            i += 1;
        }
        if s > 0 && i < n && i - s <= 3 {
            let (a, b) = (m[s - 1].unwrap(), m[i].unwrap());
            for (j, slot) in m[s..i].iter_mut().enumerate() {
                let t = (j + 1) as f32 / (i - s + 1) as f32;
                *slot = Some(a + (b - a) * t);
            }
        }
    }

    // Light median over voiced neighbours to remove single-frame glitches.
    (0..n)
        .map(|i| {
            m[i]?;
            let a = i.saturating_sub(2);
            let b = (i + 3).min(n);
            let v: Vec<f32> = m[a..b].iter().flatten().copied().collect();
            Some(median(&v))
        })
        .collect()
}

/// Splits voiced runs into notes at sustained pitch changes.
fn segment_notes(m: &[Option<f32>], min_frames: usize) -> Vec<(usize, usize)> {
    const W: usize = 5;
    let n = m.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if m[i].is_none() {
            i += 1;
            continue;
        }
        let rs = i;
        while i < n && m[i].is_some() {
            i += 1;
        }
        let re = i;
        let v: Vec<f32> = m[rs..re].iter().map(|x| x.unwrap()).collect();
        let len = v.len();
        let mut d = vec![0.0f32; len];
        if len > 2 * W {
            for (k, slot) in d.iter_mut().enumerate().take(len - W).skip(W) {
                let after: f32 = v[k..k + W].iter().sum::<f32>() / W as f32;
                let before: f32 = v[k - W..k].iter().sum::<f32>() / W as f32;
                *slot = after - before;
            }
        }
        let mut bounds = vec![0usize];
        for k in W..len.saturating_sub(W) {
            let dk = d[k].abs();
            if dk < SPLIT_ST {
                continue;
            }
            let lo = k.saturating_sub(W);
            let hi = (k + W + 1).min(len);
            // Local maximum; the first of equal neighbours wins.
            if (lo..hi).all(|j| j == k || d[j].abs() < dk || (j > k && d[j].abs() <= dk)) {
                bounds.push(k);
            }
        }
        bounds.push(len);
        let mut segs: Vec<(usize, usize)> = bounds.windows(2).map(|w| (w[0], w[1])).collect();
        // Merge neighbours that sit on the same pitch.
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for s in segs.drain(..) {
            if let Some(last) = merged.last_mut() {
                let a = median(&v[last.0..last.1]);
                let b = median(&v[s.0..s.1]);
                if (a - b).abs() < MERGE_ST {
                    last.1 = s.1;
                    continue;
                }
            }
            merged.push(s);
        }
        // A short excursion that comes back (A-B-A) is one half-cycle of a
        // wide vibrato, not a new note: snapping it would turn the vibrato
        // into a trill.
        let mean = |r: (usize, usize)| v[r.0..r.1].iter().sum::<f32>() / (r.1 - r.0) as f32;
        let mut k = 0;
        while k + 2 < merged.len() {
            let (a, b, c) = (merged[k], merged[k + 1], merged[k + 2]);
            if b.1 - b.0 <= HALF_VIBRATO_FRAMES && (mean(a) - mean(c)).abs() < MERGE_ST {
                merged[k].1 = c.1;
                merged.drain(k + 1..k + 3);
            } else {
                k += 1;
            }
        }
        for (s, e) in merged {
            if e - s >= min_frames {
                out.push((rs + s, rs + e));
            }
        }
    }
    out
}

#[derive(Debug, Clone, Copy)]
struct Vibrato {
    rate_hz: f32,
}

/// Periodic 3.8–8.5 Hz modulation of a detrended pitch residual.
fn detect_vibrato(residual: &[f32], hop: f64) -> Option<Vibrato> {
    if (residual.len() as f64) * hop < 0.3 {
        return None;
    }
    let lo = ((1.0 / (8.5 * hop)).round() as usize).max(2);
    let hi = (1.0 / (3.8 * hop)).round() as usize;
    if hi + 4 >= residual.len() {
        return None;
    }
    let r = |lag: usize| -> f32 {
        let (mut num, mut ea, mut eb) = (0.0f64, 0.0f64, 0.0f64);
        for i in 0..residual.len() - lag {
            let (a, b) = (residual[i] as f64, residual[i + lag] as f64);
            num += a * b;
            ea += a * a;
            eb += b * b;
        }
        (num / (ea * eb).sqrt().max(1e-12)) as f32
    };
    let scores: Vec<f32> = (lo - 1..=hi + 1).map(r).collect();
    let mut best = None;
    for k in 1..scores.len() - 1 {
        if scores[k] > 0.35 && scores[k] >= scores[k - 1] && scores[k] >= scores[k + 1] {
            match best {
                Some((_, s)) if s >= scores[k] => {}
                _ => best = Some((k, scores[k])),
            }
        }
    }
    let (k, _) = best?;
    let lag = (lo - 1 + k) as f64;
    Some(Vibrato {
        rate_hz: (1.0 / (lag * hop)) as f32,
    })
}

/// Chooses the pitch the singer was aiming at. Returns the target and a
/// 0..1 confidence that lowers the correction when the choice is ambiguous.
fn choose_target(sung: f32, scale: &Scale, prev: Option<f32>, dur_sec: f64) -> (f32, f32) {
    let semitone = scale.semitone(sung);
    let d_semi = (sung - semitone).abs();
    let mut target = if scale.contains(semitone) {
        semitone
    } else {
        let s = scale.nearest(sung);
        let d_scale = (sung - s).abs();
        // Clearly centred on an out-of-key note, or held long and close:
        // a deliberate chromatic note rather than a miss.
        let deliberate = d_semi < 0.25 || (d_semi < 0.35 && dur_sec > 0.35) || d_scale > 0.7;
        if deliberate {
            semitone
        } else {
            s
        }
    };
    // Repeated notes keep their target when the singer wavers in between.
    if let Some(p) = prev {
        let dp = (sung - p).abs();
        if dp < 0.6 && dp < (sung - target).abs() + 0.15 {
            target = p;
        }
    }
    let err = (target - sung).abs();
    let confidence = 1.0 - 0.5 * smoothstep(0.35, 0.6, err);
    (target, confidence)
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
    let frames = |sec: f64| ((sec / hop).round() as usize).max(1);
    let a = ((start_sec.max(0.0) / hop) as usize).min(n);
    let b = ((end_sec / hop).ceil() as usize).min(n).max(a);

    let strength = params.strength.clamp(0.0, 1.0);
    let speed = params.retune_speed.clamp(0.0, 1.0);
    let drift = params.drift.clamp(0.0, 1.0);
    let core_flat = drift;
    let attack_flat = drift.max(speed);
    let tail_flat = drift.max(0.55 + 0.4 * speed);
    // Deviations beyond this are expressive slides and pass untouched.
    let gate_hi = 1.0 + 0.6 * speed;

    let midi = clean_track(pitch);
    let mut curve = vec![0.0f32; n];
    let mut placed: Vec<(usize, usize)> = Vec::new();
    let mut targets: Vec<(f32, f32)> = Vec::new();
    let mut prev: Option<(f32, usize)> = None;

    for (s, e) in segment_notes(&midi[a..b], frames(MIN_NOTE_SEC)) {
        let (s, e) = (s + a, e + a);
        let len = e - s;
        let m: Vec<f32> = midi[s..e].iter().map(|v| v.unwrap_or(0.0)).collect();

        let rough = centre_line(&m, frames(0.12) | 1);
        let residual: Vec<f32> = m.iter().zip(&rough).map(|(x, c)| x - c).collect();
        let vibrato = detect_vibrato(&residual, hop);
        let period = vibrato
            .map(|v| ((1.0 / (v.rate_hz as f64 * hop)).round() as usize).max(3))
            .unwrap_or(frames(0.12) | 1);
        let centre = centre_line(&m, period);

        let att = frames(lerp(0.15, 0.06, speed) as f64).min(len / 3);
        let rel = frames(0.15).min(len / 3);
        let core = &centre[att..(len - rel).max(att + 1)];
        let sung = median(core);
        // A glide, not a held note: let it through, bridged by its neighbours.
        if percentile(core, 90.0) - percentile(core, 10.0) > 1.6 {
            continue;
        }
        let prev_target = prev
            .filter(|(_, pe)| (s - pe) as f64 * hop < 0.35)
            .map(|(t, _)| t);
        let (target, t_conf) = choose_target(sung, scale, prev_target, len as f64 * hop);
        prev = Some((target, e));

        let mean_conf = pitch.confidence[s..e].iter().sum::<f32>() / len as f32;
        let k = strength * t_conf * ((mean_conf - 0.45) / 0.2).clamp(0.0, 1.0);
        let err = target - sung;
        let err = err * smoothstep(params.min_shift * 0.5, params.min_shift * 1.5, err.abs());

        let depth = {
            let core_res: Vec<f32> = m[att..(len - rel).max(att + 1)]
                .iter()
                .zip(core)
                .map(|(x, c)| x - c)
                .collect();
            std::f32::consts::SQRT_2 * cover_dsp::util::rms(&core_res)
        };
        let vib_cut = match vibrato {
            Some(_) if params.vibrato > 0.0 && depth > VIBRATO_EXCESS_ST => {
                let kept = VIBRATO_EXCESS_ST
                    + (depth - VIBRATO_EXCESS_ST) * (1.0 - params.vibrato.clamp(0.0, 1.0));
                1.0 - kept / depth
            }
            _ => 0.0,
        };

        for j in 0..len {
            let w_att = if att > 0 {
                1.0 - smoothstep(0.0, att as f32, j as f32)
            } else {
                0.0
            };
            let w_tail = if rel > 0 {
                1.0 - smoothstep(0.0, rel as f32, (len - 1 - j) as f32)
            } else {
                0.0
            };
            let flat =
                (core_flat + (attack_flat - core_flat) * w_att + (tail_flat - core_flat) * w_tail)
                    .clamp(0.0, 1.0);
            let dev = sung - centre[j];
            let gate = 1.0 - smoothstep(0.35, gate_hi, dev.abs());
            let mut d = k * (err + flat * gate * dev);
            if vib_cut > 0.0 {
                let edge = 1.0 - w_att.max(w_tail);
                d -= vib_cut * edge * (m[j] - centre[j]);
            }
            curve[s + j] = d.clamp(-params.max_shift, params.max_shift);
        }
        placed.push((s, e));
        targets.push((sung, target));
    }

    bridge(
        &mut curve,
        &placed,
        frames(BRIDGE_SEC),
        frames(RAMP_SEC),
        a,
        b,
    );

    // ~30 ms zero-phase smoothing against zipper noise; vibrato edits survive.
    let curve = boxcar(&curve, 3);

    let notes = placed
        .iter()
        .zip(&targets)
        .filter_map(|(&(s, e), &(sung, target))| {
            let shift = curve[s..e].iter().sum::<f32>() / (e - s) as f32;
            let peak = curve[s..e].iter().fold(0.0f32, |m, v| m.max(v.abs()));
            (peak >= params.min_shift * 0.5).then_some(Note {
                start_sec: s as f64 * hop,
                end_sec: e as f64 * hop,
                sung_midi: sung,
                target_midi: target,
                shift,
            })
        })
        .collect();
    CorrectionCurve {
        hop_sec: hop,
        semitones: curve,
        notes,
    }
}

/// Objective intonation measures of a sung pitch track, used to compare
/// renders: note-level error, drift inside notes and vibrato depth.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct IntonationReport {
    pub notes: usize,
    /// Duration-weighted mean distance of each note's centre to its target.
    pub mean_error_cents: f32,
    /// Share of note time whose centre is within 15 / 30 cents of the target.
    pub within_15: f32,
    pub within_30: f32,
    /// Mean spread (std) of the vibrato-free centre inside notes.
    pub drift_cents: f32,
    pub vibrato_notes: usize,
    /// Mean vibrato depth (peak) over notes with vibrato.
    pub vibrato_depth_cents: f32,
}

pub fn intonation_report(
    pitch: &PitchTrack,
    scale: &Scale,
    start_sec: f64,
    end_sec: f64,
) -> IntonationReport {
    let n = pitch.f0_hz.len();
    let hop = pitch.hop_sec;
    let frames = |sec: f64| ((sec / hop).round() as usize).max(1);
    let a = ((start_sec.max(0.0) / hop) as usize).min(n);
    let b = ((end_sec / hop).ceil() as usize).min(n).max(a);
    let midi = clean_track(pitch);
    let mut r = IntonationReport::default();
    let (mut total, mut err_sum, mut in15, mut in30, mut drift_sum) = (0.0f32, 0.0, 0.0, 0.0, 0.0);
    let mut depth_sum = 0.0f32;
    for (s, e) in segment_notes(&midi[a..b], frames(0.12)) {
        let m: Vec<f32> = midi[s + a..e + a]
            .iter()
            .map(|v| v.unwrap_or(0.0))
            .collect();
        let len = m.len();
        let rough = centre_line(&m, frames(0.12) | 1);
        let residual: Vec<f32> = m.iter().zip(&rough).map(|(x, c)| x - c).collect();
        let vibrato = detect_vibrato(&residual, hop);
        let period = vibrato
            .map(|v| ((1.0 / (v.rate_hz as f64 * hop)).round() as usize).max(3))
            .unwrap_or(frames(0.12) | 1);
        let centre = centre_line(&m, period);
        let edge = frames(0.06).min(len / 4);
        let core = &centre[edge..len - edge];
        if core.is_empty() || percentile(core, 90.0) - percentile(core, 10.0) > 1.6 {
            continue;
        }
        let sung = median(core);
        let (target, _) = choose_target(sung, scale, None, len as f64 * hop);
        let err = (sung - target).abs() * 100.0;
        let w = len as f32;
        r.notes += 1;
        total += w;
        err_sum += err * w;
        if err <= 15.0 {
            in15 += w;
        }
        if err <= 30.0 {
            in30 += w;
        }
        drift_sum += cover_dsp::util::std_dev(core) * 100.0 * w;
        if vibrato.is_some() {
            let res: Vec<f32> = m[edge..len - edge]
                .iter()
                .zip(core)
                .map(|(x, c)| x - c)
                .collect();
            r.vibrato_notes += 1;
            depth_sum += std::f32::consts::SQRT_2 * cover_dsp::util::rms(&res) * 100.0;
        }
    }
    if total > 0.0 {
        r.mean_error_cents = err_sum / total;
        r.within_15 = in15 / total;
        r.within_30 = in30 / total;
        r.drift_cents = drift_sum / total;
    }
    if r.vibrato_notes > 0 {
        r.vibrato_depth_cents = depth_sum / r.vibrato_notes as f32;
    }
    r
}

/// Glides between notes separated by short gaps and ramps to zero around
/// phrase boundaries, without touching frames inside notes.
fn bridge(
    curve: &mut [f32],
    placed: &[(usize, usize)],
    max_gap: usize,
    ramp: usize,
    lo: usize,
    hi: usize,
) {
    for (i, &(s, e)) in placed.iter().enumerate() {
        let gap_before = (i > 0).then(|| s - placed[i - 1].1);
        match gap_before {
            Some(g) if g <= max_gap => {
                let pe = placed[i - 1].1;
                let (va, vb) = (curve[pe - 1], curve[s]);
                for (j, slot) in curve[pe..s].iter_mut().enumerate() {
                    let t = (j + 1) as f32 / (g + 1) as f32;
                    *slot = va + (vb - va) * t;
                }
            }
            _ => {
                let r = ramp.min(gap_before.map_or(ramp, |g| g / 2)).min(s - lo);
                for k in s - r..s {
                    curve[k] += curve[s] * (k + 1 + r - s) as f32 / (r + 1) as f32;
                }
            }
        }
        let gap_after = placed.get(i + 1).map(|n| n.0 - e);
        if !matches!(gap_after, Some(g) if g <= max_gap) {
            let r = ramp.min(gap_after.map_or(ramp, |g| g / 2)).min(hi - e);
            for k in e..e + r {
                curve[k] += curve[e - 1] * (e + r - k) as f32 / (r + 1) as f32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cover_dsp::util::{midi_to_hz, std_dev};
    use std::f32::consts::PI;

    const HOP: f64 = 0.01;

    fn track(midi: &[Option<f32>]) -> PitchTrack {
        PitchTrack {
            hop_sec: HOP,
            f0_hz: midi.iter().map(|m| m.map_or(0.0, midi_to_hz)).collect(),
            confidence: midi
                .iter()
                .map(|m| if m.is_some() { 0.9 } else { 0.1 })
                .collect(),
        }
    }

    /// Silence, then a sung contour `f(t)` for `dur` seconds, then silence.
    fn sung(dur: f32, f: impl Fn(f32) -> f32) -> Vec<Option<f32>> {
        let pad = 30;
        let len = (dur / HOP as f32) as usize;
        let mut v = vec![None; pad];
        v.extend((0..len).map(|i| Some(f(i as f32 * HOP as f32))));
        v.extend(vec![None; pad]);
        v
    }

    fn run(midi: &[Option<f32>], scale: &Scale, p: TuningParams) -> CorrectionCurve {
        correction_curve(&track(midi), scale, &p, 0.0, midi.len() as f64 * HOP)
    }

    fn corrected(midi: &[Option<f32>], c: &CorrectionCurve) -> Vec<f32> {
        midi.iter()
            .zip(&c.semitones)
            .filter_map(|(m, d)| m.map(|m| m + d))
            .collect()
    }

    fn vibrato_depth(x: &[f32], hop: f64, rate: f32) -> f32 {
        let period = (1.0 / (rate as f64 * hop)).round() as usize;
        let c = centre_line(x, period);
        let r: Vec<f32> = x.iter().zip(&c).map(|(a, b)| a - b).collect();
        let core = &r[period..r.len() - period];
        std::f32::consts::SQRT_2 * cover_dsp::util::rms(core)
    }

    fn c_major() -> Scale {
        Scale {
            pitch_classes: vec![0, 2, 4, 5, 7, 9, 11],
            offset: 0.0,
        }
    }

    #[test]
    fn nearest_in_c_major() {
        let s = c_major();
        assert_eq!(s.nearest(60.3), 60.0);
        assert_eq!(s.nearest(61.4), 62.0);
        assert_eq!(s.nearest(70.6), 71.0);
        assert!(s.contains(59.8) && !s.contains(61.1));
    }

    #[test]
    fn targets_follow_the_tuning_reference() {
        // Guitar tuned 20 cents flat: a note sung at that pitch is in tune.
        let s = Scale::chromatic().with_offset(-0.2);
        assert!((s.nearest(68.82) - 68.8).abs() < 1e-4);
        let midi = sung(0.6, |_| 68.8);
        let c = run(&midi, &s, TuningParams::default());
        let peak = c.semitones.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak < 0.02, "{peak}");
    }

    #[test]
    fn flat_note_is_raised() {
        let midi = sung(1.0, |t| 69.0 - 0.3 + 0.1 * (2.0 * PI * 5.5 * t).sin());
        let c = run(
            &midi,
            &Scale::chromatic(),
            TuningParams {
                strength: 1.0,
                drift: 0.0,
                ..Default::default()
            },
        );
        assert_eq!(c.notes.len(), 1);
        assert!((c.notes[0].shift - 0.3).abs() < 0.05, "{:?}", c.notes);
        assert!((c.semitones[80] - 0.3).abs() < 0.05);
        assert!(c.semitones[0].abs() < 1e-4, "silence before the phrase");
        assert!(c.semitones[29].abs() < 0.3, "ramped in before the onset");
    }

    #[test]
    fn vibrato_is_preserved_around_the_corrected_centre() {
        let rate = 5.5;
        let midi = sung(2.0, |t| 69.0 - 0.35 + 0.35 * (2.0 * PI * rate * t).sin());
        let c = run(
            &midi,
            &Scale::chromatic(),
            TuningParams {
                strength: 1.0,
                retune_speed: 1.0,
                drift: 1.0,
                ..Default::default()
            },
        );
        let before: Vec<f32> = midi.iter().flatten().copied().collect();
        let after = corrected(&midi, &c);
        let ratio = vibrato_depth(&after, HOP, rate) / vibrato_depth(&before, HOP, rate);
        assert!(ratio > 0.85 && ratio < 1.1, "vibrato depth ratio {ratio}");
        let centre = median(&after[30..after.len() - 30]);
        assert!((centre - 69.0).abs() < 0.04, "centre {centre}");
    }

    #[test]
    fn excessive_vibrato_is_tamed_when_asked() {
        let rate = 6.0;
        let midi = sung(2.0, |t| 69.0 + 0.9 * (2.0 * PI * rate * t).sin());
        let before: Vec<f32> = midi.iter().flatten().copied().collect();
        let keep = run(&midi, &Scale::chromatic(), TuningParams::default());
        let tame = run(
            &midi,
            &Scale::chromatic(),
            TuningParams {
                vibrato: 1.0,
                ..Default::default()
            },
        );
        let d0 = vibrato_depth(&before, HOP, rate);
        let d_keep = vibrato_depth(&corrected(&midi, &keep), HOP, rate);
        let d_tame = vibrato_depth(&corrected(&midi, &tame), HOP, rate);
        assert!((d_keep / d0 - 1.0).abs() < 0.1, "kept {d_keep} vs {d0}");
        assert!(d_tame < 0.7 * d0 && d_tame > 0.3, "tamed {d_tame} vs {d0}");
    }

    #[test]
    fn slow_drift_inside_a_held_note_is_flattened() {
        // Held A4 sliding 45 cents flat over 1.5 s, with light vibrato.
        let midi = sung(1.5, |t| 69.0 - 0.3 * t + 0.15 * (2.0 * PI * 5.0 * t).sin());
        let before: Vec<f32> = midi.iter().flatten().copied().collect();
        let c = run(
            &midi,
            &Scale::chromatic(),
            TuningParams {
                strength: 1.0,
                drift: 1.0,
                ..Default::default()
            },
        );
        let after = corrected(&midi, &c);
        let spread = |x: &[f32]| {
            let c = centre_line(x, 20);
            std_dev(&c[20..c.len() - 20])
        };
        let (s0, s1) = (spread(&before), spread(&after));
        assert!(s1 < 0.35 * s0, "drift spread {s0} -> {s1}");
    }

    #[test]
    fn flat_tail_is_lifted_by_default() {
        // In-tune note whose last 200 ms sag 40 cents.
        let midi = sung(1.0, |t| {
            if t < 0.8 {
                69.0
            } else {
                69.0 - 0.4 * (t - 0.8) / 0.2
            }
        });
        let c = run(&midi, &Scale::chromatic(), TuningParams::default());
        let last = 30 + 95;
        let before = 69.0 - midi[last].unwrap();
        let after = 69.0 - (midi[last].unwrap() + c.semitones[last]);
        assert!(after < 0.65 * before, "tail error {before} -> {after}");
    }

    #[test]
    fn wide_scoop_into_a_note_is_kept() {
        // 2-semitone scoop from below over 150 ms, then a note 30 cents sharp.
        let midi = sung(1.0, |t| {
            if t < 0.15 {
                67.3 + 2.0 * t / 0.15
            } else {
                69.3
            }
        });
        let c = run(
            &midi,
            &Scale::chromatic(),
            TuningParams {
                strength: 1.0,
                ..Default::default()
            },
        );
        // At the bottom of the scoop the correction is just the note offset.
        let k = 30 + 2;
        assert!((c.semitones[k] + 0.3).abs() < 0.15, "{}", c.semitones[k]);
        let held = 30 + 60;
        assert!((c.semitones[held] + 0.3).abs() < 0.05);
    }

    #[test]
    fn unvoiced_and_low_confidence_frames_are_left_alone() {
        let mut t = track(&sung(1.0, |_| 69.4));
        t.confidence.iter_mut().for_each(|c| *c = 0.3);
        let c = correction_curve(&t, &Scale::chromatic(), &TuningParams::default(), 0.0, 2.0);
        assert!(c.is_flat());
        assert!(c.notes.is_empty());
    }

    #[test]
    fn octave_glitches_do_not_cause_big_shifts() {
        let mut midi = sung(1.0, |_| 69.2);
        for k in [50, 51, 52, 90, 91] {
            midi[k] = midi[k].map(|m| m + 12.0);
        }
        let c = run(&midi, &Scale::chromatic(), TuningParams::default());
        let peak = c.semitones.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak < 0.3, "peak {peak}");
        assert_eq!(c.notes.len(), 1);
        assert!((c.notes[0].target_midi - 69.0).abs() < 1e-3);
    }

    #[test]
    fn chromatic_passing_tone_is_not_snapped_into_the_key() {
        // C#4 sung 8 cents sharp in C major: deliberate, stays a C#.
        let midi = sung(0.5, |_| 61.08);
        let c = run(&midi, &c_major(), TuningParams::default());
        assert_eq!(c.notes.len(), 1);
        assert_eq!(c.notes[0].target_midi, 61.0);
        // A clearly missed note between scale degrees goes to the key.
        let midi = sung(0.3, |_| 60.6);
        let c = run(&midi, &c_major(), TuningParams::default());
        assert_eq!(c.notes[0].target_midi, 60.0);
    }

    #[test]
    fn consecutive_notes_glide_without_jumps() {
        // Three notes with consonant gaps, each off by a different amount.
        let mut midi: Vec<Option<f32>> = vec![None; 20];
        for (pitch, gap) in [(64.25, 8), (67.3, 6), (69.15, 0)] {
            midi.extend((0..40).map(|i| Some(pitch + 0.05 * (i as f32 * 0.6).sin())));
            midi.extend(vec![None; gap]);
        }
        midi.extend(vec![None; 20]);
        let c = run(&midi, &c_major(), TuningParams::default());
        assert_eq!(c.notes.len(), 3, "{:?}", c.notes);
        let max_step = c
            .semitones
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(max_step < 0.12, "max per-frame step {max_step}");
        // Gaps are bridged, not dropped to zero.
        let gap = 20 + 40 + 4;
        assert!(c.semitones[gap].abs() > 0.05);
    }

    #[test]
    fn legato_note_change_splits_notes() {
        // Two notes joined by an 80 ms legato slide.
        let midi = sung(1.0, |t| {
            if t < 0.45 {
                64.2
            } else if t < 0.53 {
                64.2 + 3.0 * (t - 0.45) / 0.08
            } else {
                67.3
            }
        });
        let c = run(&midi, &c_major(), TuningParams::default());
        assert_eq!(c.notes.len(), 2, "{:?}", c.notes);
        assert_eq!(c.notes[0].target_midi, 64.0);
        assert_eq!(c.notes[1].target_midi, 67.0);
    }

    #[test]
    fn in_tune_singing_is_untouched() {
        let midi = sung(1.0, |t| 69.02 + 0.2 * (2.0 * PI * 5.5 * t).sin());
        let c = run(
            &midi,
            &Scale::chromatic(),
            TuningParams {
                drift: 0.0,
                retune_speed: 0.0,
                ..Default::default()
            },
        );
        let peak = c.semitones.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak < 0.04, "peak {peak}");
    }
}
