//! cargo run -p cover-engine --release --example sync_probe -- <video> <audio>...

use cover_engine::analysis::sync::analyze_sync;
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1).map(PathBuf::from);
    let video = args.next().expect("video path");
    let sr = 22_050;
    let v = cover_media::decode_audio(&video, sr, Some(1)).expect("decode video audio");
    for audio in args {
        let a = cover_media::decode_audio(&audio, sr, Some(1)).expect("decode audio");
        let t = Instant::now();
        let r = analyze_sync(&v.channels[0], &a.channels[0], sr);
        println!(
            "{:<28} offset {:>8.3}s  conf {:.2} corr {:.3} {:?}  ({:?})",
            audio.file_name().unwrap().to_string_lossy(),
            r.offset_sec,
            r.confidence,
            r.correlation,
            r.method,
            t.elapsed()
        );
    }
}
