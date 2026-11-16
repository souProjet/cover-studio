use crate::Result;
use cover_dsp::AudioBuffer;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WavFormat {
    Pcm16,
    Pcm24,
    Float32,
}

pub fn write_wav(path: &Path, buf: &AudioBuffer, format: WavFormat) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let (bits, sample_format) = match format {
        WavFormat::Pcm16 => (16, hound::SampleFormat::Int),
        WavFormat::Pcm24 => (24, hound::SampleFormat::Int),
        WavFormat::Float32 => (32, hound::SampleFormat::Float),
    };
    let spec = hound::WavSpec {
        channels: buf.num_channels() as u16,
        sample_rate: buf.sample_rate,
        bits_per_sample: bits,
        sample_format,
    };
    let tmp = path.with_extension("wav.part");
    {
        let mut w = hound::WavWriter::create(&tmp, spec)?;
        let scale = match format {
            WavFormat::Pcm16 => i16::MAX as f32,
            WavFormat::Pcm24 => 8_388_607.0,
            WavFormat::Float32 => 1.0,
        };
        for s in buf.interleaved() {
            let s = s.clamp(-1.0, 1.0);
            match format {
                WavFormat::Float32 => w.write_sample(s)?,
                _ => w.write_sample((s * scale).round() as i32)?,
            }
        }
        w.finalize()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Fast path for WAV files we wrote ourselves (cache, stems).
pub fn read_wav(path: &Path) -> Result<AudioBuffer> {
    let mut r = hound::WavReader::open(path)?;
    let spec = r.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().collect::<std::result::Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<std::result::Result<_, _>>()?
        }
    };
    Ok(AudioBuffer::from_interleaved(
        spec.sample_rate,
        spec.channels as usize,
        &samples,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_24bit() {
        let dir = std::env::temp_dir().join("cover-media-wav-test");
        let p = dir.join("t.wav");
        let b = AudioBuffer::new(44_100, vec![vec![0.5, -0.25, 0.0], vec![0.1, 0.2, -0.9]]);
        write_wav(&p, &b, WavFormat::Pcm24).unwrap();
        let r = read_wav(&p).unwrap();
        for (a, c) in b.channels.iter().flatten().zip(r.channels.iter().flatten()) {
            assert!((a - c).abs() < 1e-6);
        }
        std::fs::remove_dir_all(dir).ok();
    }
}
