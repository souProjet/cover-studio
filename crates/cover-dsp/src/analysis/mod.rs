//! Signal analysis: envelopes, onsets, cross-correlation, key detection and
//! activity segmentation.

pub mod activity;
pub mod key;
pub mod onset;
pub mod xcorr;

/// RMS envelope with non-overlapping windows of `hop` samples.
pub fn rms_envelope(x: &[f32], hop: usize) -> Vec<f32> {
    x.chunks(hop.max(1)).map(crate::util::rms).collect()
}

/// Simple local-maximum peak picker.
///
/// Returns indices of samples that are the maximum within `distance` on each
/// side and are above `height`.
pub fn find_peaks(x: &[f32], height: f32, distance: usize) -> Vec<usize> {
    let mut peaks = Vec::new();
    let d = distance.max(1);
    let mut i = 0;
    while i < x.len() {
        let v = x[i];
        if v >= height {
            let a = i.saturating_sub(d);
            let b = (i + d + 1).min(x.len());
            if x[a..b].iter().all(|&o| o <= v) {
                peaks.push(i);
                i += d;
                continue;
            }
        }
        i += 1;
    }
    peaks
}
