use crate::tools::{run, Tools};
use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoStreamInfo {
    pub codec: String,
    /// Display size, after applying the rotation metadata.
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub rotation: i32,
    pub pix_fmt: String,
    pub hdr: Option<HdrTransfer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HdrTransfer {
    /// Hybrid log-gamma (most phones). Designed to look right on SDR displays as-is.
    Hlg,
    /// PQ / HDR10. Looks dark and flat without tonemapping.
    Pq,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioStreamInfo {
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub duration_sec: f64,
    pub video: Option<VideoStreamInfo>,
    pub audio: Option<AudioStreamInfo>,
}

#[derive(Deserialize)]
struct RawProbe {
    #[serde(default)]
    streams: Vec<RawStream>,
    format: Option<RawFormat>,
}

#[derive(Deserialize)]
struct RawFormat {
    duration: Option<String>,
}

#[derive(Deserialize)]
struct RawStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    pix_fmt: Option<String>,
    color_transfer: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    duration: Option<String>,
    #[serde(default)]
    side_data_list: Vec<serde_json::Value>,
    #[serde(default)]
    tags: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    disposition: serde_json::Map<String, serde_json::Value>,
}

fn parse_rate(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/')?;
    let n: f64 = n.parse().ok()?;
    let d: f64 = d.parse().ok()?;
    (d > 0.0 && n > 0.0).then_some(n / d)
}

fn rotation(s: &RawStream) -> i32 {
    for sd in &s.side_data_list {
        if let Some(r) = sd.get("rotation").and_then(|v| v.as_f64()) {
            return r as i32;
        }
    }
    s.tags
        .get("rotate")
        .and_then(|v| v.as_str())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

pub fn probe(path: &Path) -> Result<MediaInfo> {
    let tools = Tools::locate()?;
    let mut cmd = tools.ffprobe();
    cmd.args([
        "-v",
        "error",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
    ])
    .arg(path);
    let out = run(cmd, "ffprobe")?;
    let raw: RawProbe = serde_json::from_slice(&out.stdout)?;

    let mut duration = raw
        .format
        .and_then(|f| f.duration)
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(0.0);

    let mut video = None;
    let mut audio = None;
    for s in &raw.streams {
        match s.codec_type.as_deref() {
            Some("video") if video.is_none() => {
                // Cover art in audio files is exposed as a single-frame video stream.
                let attached = s
                    .disposition
                    .get("attached_pic")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
                    == 1;
                if attached {
                    continue;
                }
                let rot = rotation(s);
                let (mut w, mut h) = (s.width.unwrap_or(0), s.height.unwrap_or(0));
                if rot.rem_euclid(180) == 90 {
                    std::mem::swap(&mut w, &mut h);
                }
                let fps = s
                    .avg_frame_rate
                    .as_deref()
                    .and_then(parse_rate)
                    .or_else(|| s.r_frame_rate.as_deref().and_then(parse_rate))
                    .unwrap_or(30.0);
                let hdr = match s.color_transfer.as_deref() {
                    Some("arib-std-b67") => Some(HdrTransfer::Hlg),
                    Some("smpte2084") => Some(HdrTransfer::Pq),
                    _ => None,
                };
                video = Some(VideoStreamInfo {
                    codec: s.codec_name.clone().unwrap_or_default(),
                    width: w,
                    height: h,
                    fps,
                    rotation: rot,
                    pix_fmt: s.pix_fmt.clone().unwrap_or_default(),
                    hdr,
                });
            }
            Some("audio") if audio.is_none() => {
                audio = Some(AudioStreamInfo {
                    codec: s.codec_name.clone().unwrap_or_default(),
                    sample_rate: s
                        .sample_rate
                        .as_deref()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(44_100),
                    channels: s.channels.unwrap_or(2),
                });
                if duration <= 0.0 {
                    duration = s
                        .duration
                        .as_deref()
                        .and_then(|d| d.parse().ok())
                        .unwrap_or(0.0);
                }
            }
            _ => {}
        }
    }
    Ok(MediaInfo {
        duration_sec: duration,
        video,
        audio,
    })
}
