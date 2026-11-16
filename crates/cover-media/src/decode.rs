use crate::tools::{run, Tools};
use crate::{MediaError, Result};
use cover_dsp::AudioBuffer;
use std::path::Path;

/// Decodes the first audio stream of any file ffmpeg can read.
///
/// `channels` and `sample_rate` force a conversion; `None` keeps the source
/// layout (mono stays mono, so a single-mic recording is not duplicated).
pub fn decode_audio(path: &Path, sample_rate: u32, channels: Option<u32>) -> Result<AudioBuffer> {
    let tools = Tools::locate()?;
    let ch = match channels {
        Some(c) => c,
        None => crate::probe::probe(path)?
            .audio
            .ok_or_else(|| {
                MediaError::Unsupported(format!("{} has no audio stream", path.display()))
            })?
            .channels
            .clamp(1, 2),
    };
    let mut cmd = tools.ffmpeg();
    cmd.args(["-loglevel", "error", "-i"])
        .arg(path)
        .args(["-vn", "-map", "0:a:0", "-ac"])
        .arg(ch.to_string())
        .arg("-ar")
        .arg(sample_rate.to_string())
        .args(["-f", "f32le", "-acodec", "pcm_f32le", "-"]);
    let out = run(cmd, "ffmpeg")?;
    let samples: Vec<f32> = out
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    Ok(AudioBuffer::from_interleaved(
        sample_rate,
        ch as usize,
        &samples,
    ))
}
