//! End-to-end checks against a real ffmpeg. Skipped when ffmpeg is missing.

use cover_media::wav::{write_wav, WavFormat};
use cover_media::*;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::AtomicBool;

fn workdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cover-media-it-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn make_clip(dir: &std::path::Path) -> Option<PathBuf> {
    let tools = Tools::locate().ok()?;
    let p = dir.join("src.mp4");
    let ok = Command::new(&tools.ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x360:r=30:d=3",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=3",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ])
        .arg(&p)
        .status()
        .ok()?
        .success();
    ok.then_some(p)
}

#[test]
fn probe_decode_render() {
    let dir = workdir("render");
    let Some(src) = make_clip(&dir) else {
        eprintln!("ffmpeg unavailable, skipping");
        return;
    };

    let info = probe(&src).unwrap();
    assert!((info.duration_sec - 3.0).abs() < 0.2, "{info:?}");
    let v = info.video.unwrap();
    assert_eq!((v.width, v.height), (640, 360));
    assert!(v.hdr.is_none());

    let audio = decode_audio(&src, 44_100, Some(1)).unwrap();
    assert_eq!(audio.num_channels(), 1);
    assert!((audio.duration_sec() - 3.0).abs() < 0.2);

    let wav = dir.join("a.wav");
    write_wav(
        &wav,
        &audio.slice_sec(0.5, 2.5).to_stereo(),
        WavFormat::Pcm24,
    )
    .unwrap();
    let out = dir.join("out.mp4");
    let mut last = 0.0;
    render_video(
        &VideoRenderSpec {
            video: src,
            audio: wav,
            out: out.clone(),
            start_sec: 0.5,
            end_sec: 2.5,
            aspect: Aspect::Vertical,
            style: VideoStyle::Social,
            quality: VideoQuality::Draft,
            fade_in_sec: 0.2,
            fade_out_sec: 0.3,
            fps: None,
        },
        &AtomicBool::new(false),
        |p| last = p,
    )
    .unwrap();
    assert_eq!(last, 1.0);
    let o = probe(&out).unwrap();
    let ov = o.video.unwrap();
    assert_eq!((ov.width, ov.height), (1080, 1920));
    assert!((o.duration_sec - 2.0).abs() < 0.2, "{}", o.duration_sec);

    let jpg = dir.join("frame.jpg");
    extract_frame(&out, 1.0, Aspect::Square, &jpg).unwrap();
    assert!(jpg.metadata().unwrap().len() > 1000);
    std::fs::remove_dir_all(dir).ok();
}
