use crate::probe::{probe, HdrTransfer};
use crate::tools::{gpu_encoder_available, has_zscale, run, stderr_tail, Tools};
use crate::{MediaError, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aspect {
    /// 1080x1920 vertical (Reels, Shorts, TikTok), centre crop.
    Vertical,
    /// 1920x1080 landscape, centre crop.
    Landscape,
    /// 1080x1080.
    Square,
    /// Keep the source ratio, at most 1920 px on the long side.
    Source,
}

impl Aspect {
    pub fn target_size(self, src_w: u32, src_h: u32) -> (u32, u32) {
        match self {
            Aspect::Vertical => (1080, 1920),
            Aspect::Landscape => (1920, 1080),
            Aspect::Square => (1080, 1080),
            Aspect::Source => {
                let long = src_w.max(src_h).max(1) as f64;
                let k = (1920.0 / long).min(1.0);
                let even = |v: f64| ((v * k / 2.0).round() as u32 * 2).max(2);
                (even(src_w as f64), even(src_h as f64))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoStyle {
    /// No denoise or grading, just reframe and encode.
    Natural,
    /// Light denoise, gentle contrast and warmth (phone footage).
    Social,
    /// Stronger temporal denoise for dark rooms.
    LowLight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoQuality {
    Draft,
    High,
    Max,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoRenderSpec {
    pub video: PathBuf,
    /// Audio already aligned to the `[start_sec, end_sec]` window of the video.
    pub audio: PathBuf,
    pub out: PathBuf,
    pub start_sec: f64,
    pub end_sec: f64,
    pub aspect: Aspect,
    pub style: VideoStyle,
    pub quality: VideoQuality,
    pub fade_in_sec: f64,
    pub fade_out_sec: f64,
    pub fps: Option<u32>,
}

fn grading(style: VideoStyle) -> &'static str {
    match style {
        VideoStyle::Natural => "",
        // Warmth is a small chroma offset in YUV: an RGB colour balance would
        // double the render time on 4K sources.
        VideoStyle::Social => {
            "hqdn3d=3:2.5:5:4,eq=contrast=1.04:brightness=0.01:saturation=1.05:gamma=0.98,\
             lutyuv=u='clip(val-2,minval,maxval)':v='clip(val+1,minval,maxval)',unsharp=5:5:0.35:5:5:0.0"
        }
        VideoStyle::LowLight => {
            "hqdn3d=5:4:7:5,atadenoise=0.03:0.03:0.05:0.03,\
             eq=contrast=1.05:brightness=0.025:saturation=1.04:gamma=0.97,\
             lutyuv=u='clip(val-2,minval,maxval)':v='clip(val+1,minval,maxval)',unsharp=5:5:0.3:5:5:0.0"
        }
    }
}

/// PQ to BT.709 SDR. Applied after downscaling so it stays cheap on 4K input.
/// HLG is left alone: tonemapping it flattens contrast compared to showing it as-is.
const PQ_TONEMAP: &str = "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,\
tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv";

pub(crate) fn frame_filter(
    aspect: Aspect,
    src_w: u32,
    src_h: u32,
    hdr: Option<HdrTransfer>,
) -> String {
    let (w, h) = aspect.target_size(src_w, src_h);
    let mut f = match aspect {
        Aspect::Source => format!("scale={w}:{h}:flags=lanczos"),
        _ => {
            format!("scale={w}:{h}:force_original_aspect_ratio=increase:flags=lanczos,crop={w}:{h}")
        }
    };
    if hdr == Some(HdrTransfer::Hlg) {
        // HLG keeps its signal but moves to the BT.709 matrix the output is tagged
        // with. Done after downscaling: in `scale` it is 3x slower on 10-bit 4K.
        f.push_str(",format=yuv420p,colormatrix=bt2020:bt709");
    }
    if hdr == Some(HdrTransfer::Pq) && has_zscale() {
        f.push(',');
        f.push_str(PQ_TONEMAP);
    }
    f.push_str(",setsar=1");
    f
}

fn video_encoder_args(quality: VideoQuality) -> Vec<String> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    if gpu_encoder_available() {
        let (preset, cq, rate, max) = match quality {
            VideoQuality::Draft => ("p3", "26", "6M", "9M"),
            VideoQuality::High => ("p5", "20", "10M", "14M"),
            VideoQuality::Max => ("p7", "18", "14M", "20M"),
        };
        let mut a = s(&[
            "-c:v",
            "h264_nvenc",
            "-preset",
            preset,
            "-tune",
            "hq",
            "-profile:v",
            "high",
            "-rc",
            "vbr",
            "-cq",
            cq,
            "-b:v",
            rate,
            "-maxrate",
            max,
            "-bufsize",
            "24M",
            "-spatial_aq",
            "1",
            "-temporal_aq",
            "1",
            "-rc-lookahead",
            "20",
        ]);
        a.extend(s(&["-pix_fmt", "yuv420p"]));
        a
    } else {
        let (preset, crf) = match quality {
            VideoQuality::Draft => ("veryfast", "24"),
            VideoQuality::High => ("medium", "19"),
            VideoQuality::Max => ("slow", "17"),
        };
        s(&[
            "-c:v",
            "libx264",
            "-preset",
            preset,
            "-crf",
            crf,
            "-profile:v",
            "high",
            "-pix_fmt",
            "yuv420p",
        ])
    }
}

/// Renders the final clip. `on_progress` receives values in 0..=1.
pub fn render_video(
    spec: &VideoRenderSpec,
    cancel: &AtomicBool,
    on_progress: impl FnMut(f32),
) -> Result<PathBuf> {
    let tools = Tools::locate()?;
    let info = probe(&spec.video)?;
    let v = info.video.ok_or_else(|| {
        MediaError::Unsupported(format!("{} has no video stream", spec.video.display()))
    })?;
    let duration = (spec.end_sec - spec.start_sec).max(0.1);
    let fps = spec.fps.unwrap_or(if v.fps > 45.0 { 60 } else { 30 });

    let mut vf = frame_filter(spec.aspect, v.width, v.height, v.hdr);
    vf.push_str(",format=yuv420p");
    let g = grading(spec.style);
    if !g.is_empty() {
        vf.push(',');
        vf.push_str(g);
    }
    if spec.fade_in_sec > 0.0 {
        vf.push_str(&format!(",fade=t=in:st=0:d={:.3}", spec.fade_in_sec));
    }
    if spec.fade_out_sec > 0.0 {
        vf.push_str(&format!(
            ",fade=t=out:st={:.3}:d={:.3}",
            (duration - spec.fade_out_sec).max(0.0),
            spec.fade_out_sec
        ));
    }
    // NVENC copies the source colour tags from the frames, not from -color_trc.
    vf.push_str(&format!(
        ",fps={fps},format=yuv420p,setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv"
    ));

    if let Some(parent) = spec.out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = spec.out.with_extension("part.mp4");

    let mut cmd = tools.ffmpeg();
    cmd.args(["-loglevel", "error", "-progress", "pipe:1", "-nostats"]);
    // No `-hwaccel cuda`: downloading decoded 10-bit 4K frames for the CPU
    // filters measured slower than software decoding.
    cmd.arg("-ss")
        .arg(format!("{:.3}", spec.start_sec))
        .arg("-t")
        .arg(format!("{duration:.3}"))
        .arg("-i")
        .arg(&spec.video)
        .arg("-i")
        .arg(&spec.audio)
        .args(["-map", "0:v:0", "-map", "1:a:0", "-vf"])
        .arg(&vf)
        .args(video_encoder_args(spec.quality))
        .args([
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
            "-c:a",
            "aac",
            "-b:a",
            "256k",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-map_metadata",
            "-1",
            "-movflags",
            "+faststart",
            "-shortest",
        ])
        .arg(&tmp);
    tracing::debug!(?cmd, "render_video");
    run_with_progress(cmd, &tmp, &spec.out, duration, cancel, on_progress)?;
    Ok(spec.out.clone())
}

/// Lightweight H.264 copy of a phone video that any webview can play: SDR
/// BT.709 with the export's tone mapping, short side at most 720 px, 30 fps,
/// no audio. Keeps the source framing so the preview can crop to any aspect.
pub fn render_proxy(
    video: &Path,
    out: &Path,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(f32),
) -> Result<PathBuf> {
    let info = probe(video)?;
    let v = info.video.ok_or_else(|| {
        MediaError::Unsupported(format!("{} has no video stream", video.display()))
    })?;
    let duration = info.duration_sec.max(0.1);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // NVDEC + GPU scaling only downloads 720p frames: ~4x faster than
    // software decoding on 4K HEVC. Rotated sources need ffmpeg's autorotate,
    // which runs on CPU frames.
    let nvidia = gpu_encoder_available();
    if nvidia && v.rotation.rem_euclid(360) == 0 {
        match run_proxy(video, out, &v, duration, true, cancel, &mut on_progress) {
            Err(MediaError::Cancelled) => return Err(MediaError::Cancelled),
            Err(e) => tracing::warn!("GPU proxy failed, falling back to CPU: {e}"),
            Ok(()) => return Ok(out.to_path_buf()),
        }
    }
    run_proxy(video, out, &v, duration, false, cancel, &mut on_progress)?;
    Ok(out.to_path_buf())
}

fn run_proxy(
    video: &Path,
    out: &Path,
    v: &crate::probe::VideoStreamInfo,
    duration: f64,
    gpu_decode: bool,
    cancel: &AtomicBool,
    on_progress: &mut impl FnMut(f32),
) -> Result<()> {
    let tools = Tools::locate()?;
    let (w, h) = proxy_size(v.width, v.height);
    let mut vf = if gpu_decode {
        format!("scale_cuda={w}:{h}:format=nv12,hwdownload,format=nv12,format=yuv420p")
    } else {
        format!("scale={w}:{h}:flags=bicubic")
    };
    match v.hdr {
        Some(HdrTransfer::Hlg) => vf.push_str(",format=yuv420p,colormatrix=bt2020:bt709"),
        Some(HdrTransfer::Pq) if has_zscale() => {
            vf.push(',');
            vf.push_str(PQ_TONEMAP);
        }
        _ => {}
    }
    vf.push_str(
        ",setsar=1,fps=30,format=yuv420p,setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv",
    );

    let tmp = out.with_extension("part.mp4");
    let mut cmd = tools.ffmpeg();
    cmd.args(["-loglevel", "error", "-progress", "pipe:1", "-nostats"]);
    if gpu_decode {
        cmd.args(["-hwaccel", "cuda", "-hwaccel_output_format", "cuda"]);
    }
    cmd.arg("-i")
        .arg(video)
        .args(["-map", "0:v:0", "-an", "-vf"])
        .arg(&vf);
    if gpu_encoder_available() {
        cmd.args([
            "-c:v",
            "h264_nvenc",
            "-preset",
            "p2",
            "-rc",
            "vbr",
            "-cq",
            "30",
            "-b:v",
            "2M",
            "-maxrate",
            "3M",
            "-bufsize",
            "6M",
        ]);
    } else {
        cmd.args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "28"]);
    }
    cmd.args([
        "-profile:v",
        "high",
        "-pix_fmt",
        "yuv420p",
        "-g",
        "15",
        "-color_primaries",
        "bt709",
        "-color_trc",
        "bt709",
        "-colorspace",
        "bt709",
        "-map_metadata",
        "-1",
        "-movflags",
        "+faststart",
    ])
    .arg(&tmp);
    tracing::debug!(?cmd, "render_proxy");
    run_with_progress(cmd, &tmp, out, duration, cancel, on_progress)
}

/// Even size with the short side at most 720 px.
fn proxy_size(w: u32, h: u32) -> (u32, u32) {
    let short = w.min(h).max(1) as f64;
    let k = (720.0 / short).min(1.0);
    let even = |v: u32| ((v as f64 * k / 2.0).round() as u32 * 2).max(2);
    (even(w), even(h))
}

/// Runs an ffmpeg command writing to `tmp` with `-progress pipe:1`, then moves
/// the result to `out`.
fn run_with_progress(
    mut cmd: std::process::Command,
    tmp: &Path,
    out: &Path,
    duration: f64,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(f32),
) -> Result<()> {
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let err_reader = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = stderr.read_to_end(&mut s);
        s
    });
    for line in BufReader::new(stdout).lines() {
        let line = line?;
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(tmp);
            return Err(MediaError::Cancelled);
        }
        if let Some(us) = line.strip_prefix("out_time_us=") {
            if let Ok(us) = us.trim().parse::<f64>() {
                on_progress(((us / 1e6) / duration).clamp(0.0, 1.0) as f32);
            }
        }
    }
    let status = child.wait()?;
    let stderr = err_reader.join().unwrap_or_default();
    if !status.success() {
        let _ = std::fs::remove_file(tmp);
        return Err(MediaError::ToolFailed {
            tool: "ffmpeg",
            message: stderr_tail(&stderr),
        });
    }
    std::fs::rename(tmp, out)?;
    on_progress(1.0);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    Aac,
    Mp3,
    Flac,
}

impl AudioCodec {
    pub fn extension(self) -> &'static str {
        match self {
            AudioCodec::Aac => "m4a",
            AudioCodec::Mp3 => "mp3",
            AudioCodec::Flac => "flac",
        }
    }
}

/// Encodes a WAV into a distribution format.
pub fn encode_audio(wav: &Path, out: &Path, codec: AudioCodec) -> Result<()> {
    let tools = Tools::locate()?;
    let mut cmd = tools.ffmpeg();
    cmd.args(["-loglevel", "error", "-i"]).arg(wav);
    match codec {
        AudioCodec::Aac => cmd.args(["-c:a", "aac", "-b:a", "256k", "-movflags", "+faststart"]),
        AudioCodec::Mp3 => cmd.args(["-c:a", "libmp3lame", "-q:a", "0"]),
        AudioCodec::Flac => cmd.args(["-c:a", "flac"]),
    };
    cmd.arg(out);
    run(cmd, "ffmpeg")?;
    Ok(())
}
