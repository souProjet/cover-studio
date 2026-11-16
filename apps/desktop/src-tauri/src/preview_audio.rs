//! Native A/B preview playback. The webview's media stack (GStreamer on Linux)
//! cannot be trusted to play two files in lockstep, so both versions are
//! decoded here and mixed in the output callback with a short crossfade.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use serde::Serialize;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

const FADE_SEC: f32 = 0.03;

/// Two stereo versions of the same audio, interleaved, at the output rate.
struct Track {
    a: Vec<f32>,
    b: Vec<f32>,
    frames: usize,
}

#[derive(Default)]
struct Shared {
    track: Mutex<Option<Arc<Track>>>,
    pos: AtomicUsize,
    playing: AtomicBool,
    side_b: AtomicBool,
    generation: AtomicUsize,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct PreviewState {
    pub time: f64,
    pub playing: bool,
    pub duration: f64,
}

pub struct PreviewAudio {
    shared: Arc<Shared>,
    rate: u32,
    /// Keeps the output thread (and its stream) alive.
    _stop: mpsc::Sender<()>,
}

impl PreviewAudio {
    /// Opens the default output device on a dedicated thread.
    pub fn open() -> Result<Self, String> {
        let shared = Arc::new(Shared::default());
        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, String>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let s = shared.clone();
        std::thread::Builder::new()
            .name("preview-audio".into())
            .spawn(move || match build_stream(s) {
                Ok((stream, rate)) => {
                    let _ = ready_tx.send(Ok(rate));
                    let _ = stop_rx.recv();
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .map_err(|e| e.to_string())?;
        let rate = ready_rx
            .recv()
            .map_err(|_| "audio thread exited".to_string())??;
        Ok(Self {
            shared,
            rate,
            _stop: stop_tx,
        })
    }

    /// Decodes both versions; a newer `load` started meanwhile wins.
    pub fn load(&self, studio: &Path, original: &Path) -> Result<f64, String> {
        let generation = self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.shared.playing.store(false, Ordering::SeqCst);
        self.shared.pos.store(0, Ordering::SeqCst);
        *self.shared.track.lock().unwrap() = None;
        let decode = |p: &Path| {
            cover_engine::media::decode_audio(p, self.rate, Some(2))
                .map(|b| b.interleaved())
                .map_err(|e| e.to_string())
        };
        let a = decode(studio)?;
        let b = decode(original)?;
        let frames = a.len().min(b.len()) / 2;
        let mut track = self.shared.track.lock().unwrap();
        if self.shared.generation.load(Ordering::SeqCst) != generation {
            return Err("superseded by a newer preview".into());
        }
        *track = Some(Arc::new(Track { a, b, frames }));
        Ok(frames as f64 / self.rate as f64)
    }

    pub fn unload(&self) {
        self.shared.playing.store(false, Ordering::SeqCst);
        *self.shared.track.lock().unwrap() = None;
    }

    pub fn play(&self) {
        let frames = self.frames();
        if frames == 0 {
            return;
        }
        if self.shared.pos.load(Ordering::SeqCst) + 1 >= frames {
            self.shared.pos.store(0, Ordering::SeqCst);
        }
        self.shared.playing.store(true, Ordering::SeqCst);
    }

    pub fn pause(&self) {
        self.shared.playing.store(false, Ordering::SeqCst);
    }

    pub fn seek(&self, sec: f64) {
        let frame = (sec.max(0.0) * self.rate as f64) as usize;
        self.shared
            .pos
            .store(frame.min(self.frames()), Ordering::SeqCst);
    }

    pub fn set_original(&self, original: bool) {
        self.shared.side_b.store(original, Ordering::SeqCst);
    }

    pub fn state(&self) -> PreviewState {
        let rate = self.rate as f64;
        PreviewState {
            time: self.shared.pos.load(Ordering::SeqCst) as f64 / rate,
            playing: self.shared.playing.load(Ordering::SeqCst),
            duration: self.frames() as f64 / rate,
        }
    }

    fn frames(&self) -> usize {
        self.shared
            .track
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, |t| t.frames)
    }
}

fn build_stream(shared: Arc<Shared>) -> Result<(cpal::Stream, u32), String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "no audio output device".to_string())?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let rate = config.sample_rate;
    let stream = match format {
        SampleFormat::F32 => output::<f32>(&device, config, shared),
        SampleFormat::I16 => output::<i16>(&device, config, shared),
        SampleFormat::I32 => output::<i32>(&device, config, shared),
        SampleFormat::U16 => output::<u16>(&device, config, shared),
        other => return Err(format!("unsupported output format {other}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, rate))
}

fn output<T>(
    device: &cpal::Device,
    config: StreamConfig,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let step = 1.0 / (FADE_SEC * config.sample_rate as f32);
    let mut gain_b = 0.0f32;
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let silence = T::from_sample(0.0f32);
                let track = shared.track.try_lock().ok().and_then(|t| t.clone());
                let Some(track) = track.filter(|_| shared.playing.load(Ordering::Relaxed)) else {
                    data.fill(silence);
                    return;
                };
                let target = if shared.side_b.load(Ordering::Relaxed) {
                    1.0
                } else {
                    0.0
                };
                let start = shared.pos.load(Ordering::Relaxed);
                let mut pos = start;
                for frame in data.chunks_mut(channels) {
                    if pos >= track.frames {
                        frame.fill(silence);
                        continue;
                    }
                    gain_b += (target - gain_b).clamp(-step, step);
                    let i = pos * 2;
                    let l = track.a[i] * (1.0 - gain_b) + track.b[i] * gain_b;
                    let r = track.a[i + 1] * (1.0 - gain_b) + track.b[i + 1] * gain_b;
                    for (c, s) in frame.iter_mut().enumerate() {
                        *s = T::from_sample(match c {
                            0 => l,
                            1 => r,
                            _ => 0.0,
                        });
                    }
                    if channels == 1 {
                        frame[0] = T::from_sample(0.5 * (l + r));
                    }
                    pos += 1;
                }
                // A seek from the UI wins over the position reached here.
                let moved = shared
                    .pos
                    .compare_exchange(start, pos, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok();
                if moved && pos >= track.frames {
                    shared.playing.store(false, Ordering::Relaxed);
                }
            },
            |e| tracing::warn!("preview audio: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}
