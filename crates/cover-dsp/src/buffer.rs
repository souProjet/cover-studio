use serde::{Deserialize, Serialize};

/// Planar multi-channel audio at a fixed sample rate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioBuffer {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
}

impl AudioBuffer {
    pub fn new(sample_rate: u32, channels: Vec<Vec<f32>>) -> Self {
        let len = channels.iter().map(Vec::len).min().unwrap_or(0);
        let channels = channels
            .into_iter()
            .map(|mut c| {
                c.truncate(len);
                c
            })
            .collect();
        Self {
            sample_rate,
            channels,
        }
    }

    pub fn from_mono(sample_rate: u32, mono: Vec<f32>) -> Self {
        Self {
            sample_rate,
            channels: vec![mono],
        }
    }

    pub fn silence(sample_rate: u32, channels: usize, frames: usize) -> Self {
        Self {
            sample_rate,
            channels: vec![vec![0.0; frames]; channels],
        }
    }

    pub fn num_channels(&self) -> usize {
        self.channels.len()
    }

    pub fn len(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn duration_sec(&self) -> f64 {
        self.len() as f64 / self.sample_rate as f64
    }

    pub fn sec_to_frame(&self, sec: f64) -> usize {
        ((sec.max(0.0) * self.sample_rate as f64).round() as usize).min(self.len())
    }

    /// Average of all channels.
    pub fn to_mono(&self) -> Vec<f32> {
        let n = self.len();
        let c = self.num_channels().max(1) as f32;
        let mut out = vec![0.0f32; n];
        for ch in &self.channels {
            for (o, s) in out.iter_mut().zip(ch) {
                *o += *s;
            }
        }
        for o in &mut out {
            *o /= c;
        }
        out
    }

    /// Returns a stereo copy (mono is duplicated, >2 channels are downmixed).
    pub fn to_stereo(&self) -> AudioBuffer {
        match self.num_channels() {
            0 => AudioBuffer::silence(self.sample_rate, 2, 0),
            1 => AudioBuffer::new(
                self.sample_rate,
                vec![self.channels[0].clone(), self.channels[0].clone()],
            ),
            2 => self.clone(),
            _ => {
                let m = self.to_mono();
                AudioBuffer::new(self.sample_rate, vec![m.clone(), m])
            }
        }
    }

    pub fn slice_frames(&self, start: usize, end: usize) -> AudioBuffer {
        let n = self.len();
        let s = start.min(n);
        let e = end.clamp(s, n);
        AudioBuffer {
            sample_rate: self.sample_rate,
            channels: self.channels.iter().map(|c| c[s..e].to_vec()).collect(),
        }
    }

    pub fn slice_sec(&self, start: f64, end: f64) -> AudioBuffer {
        self.slice_frames(self.sec_to_frame(start), self.sec_to_frame(end))
    }

    pub fn peak(&self) -> f32 {
        self.channels
            .iter()
            .flat_map(|c| c.iter())
            .fold(0.0f32, |m, s| m.max(s.abs()))
    }

    pub fn apply_gain(&mut self, gain: f32) {
        for ch in &mut self.channels {
            for s in ch.iter_mut() {
                *s *= gain;
            }
        }
    }

    /// Interleaved samples, as expected by most encoders.
    pub fn interleaved(&self) -> Vec<f32> {
        let n = self.len();
        let c = self.num_channels();
        let mut out = Vec::with_capacity(n * c);
        for i in 0..n {
            for ch in &self.channels {
                out.push(ch[i]);
            }
        }
        out
    }

    pub fn from_interleaved(sample_rate: u32, channels: usize, data: &[f32]) -> AudioBuffer {
        let channels = channels.max(1);
        let frames = data.len() / channels;
        let mut planar = vec![Vec::with_capacity(frames); channels];
        for frame in data.chunks_exact(channels) {
            for (ch, s) in planar.iter_mut().zip(frame) {
                ch.push(*s);
            }
        }
        AudioBuffer::new(sample_rate, planar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interleave_roundtrip() {
        let b = AudioBuffer::new(48_000, vec![vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0]]);
        let i = b.interleaved();
        assert_eq!(i, vec![1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
        assert_eq!(AudioBuffer::from_interleaved(48_000, 2, &i), b);
    }

    #[test]
    fn mono_is_average() {
        let b = AudioBuffer::new(8, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
        assert_eq!(b.to_mono(), vec![0.5, 0.5]);
    }
}
