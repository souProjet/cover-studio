//! Prints the active regions of a recording for several detection settings.
//! Usage: cargo run --release -p cover-engine --example regions -- <file>

use cover_dsp::analysis::activity::{active_regions, ActivityParams};

fn main() {
    let path = std::env::args().nth(1).expect("file");
    let sr = 22_050;
    let x = cover_media::decode_audio(path.as_ref(), sr, Some(1))
        .unwrap()
        .to_mono();
    for smooth in [0.05, 0.5, 1.0] {
        for floor in [30.0, 35.0] {
            let p = ActivityParams {
                smooth_sec: smooth,
                floor_db: floor,
                min_gap_sec: 3.0,
                min_region_sec: 0.0,
                ..ActivityParams::default()
            };
            let r: Vec<String> = active_regions(&x, sr, &p)
                .iter()
                .map(|r| format!("{:.0}-{:.0}", r.start_sec, r.end_sec))
                .collect();
            println!("smooth {smooth} floor {floor}: {}", r.join(" "));
        }
    }
}
