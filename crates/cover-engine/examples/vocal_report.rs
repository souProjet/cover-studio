//! Objective before/after comparison of renders of one project.
//!
//! cargo run --release -p cover-engine --example vocal_report -- \
//!     <projects dir> <project id> <label>=<file.wav> [<label>=<file.wav> ...]
//!
//! Each file must cover the project's last rendered range. The sung pitch is
//! tracked again with RMVPE on every file, then intonation (note error,
//! drift, vibrato), loudness and click counts are printed side by side.

use cover_dsp::analysis::tuning_ref::tuning_reference;
use cover_dsp::loudness::measure;
use cover_dsp::util::median;
use cover_engine::analysis::tuning::intonation_report;
use cover_engine::media::wav::read_wav;
use cover_engine::ml::{model_path, registry, Device, Rmvpe};
use cover_engine::{scale_for, Engine};
use std::sync::atomic::AtomicBool;

/// Isolated spikes in the second difference: clicks, pops, zipper noise.
fn clicks_per_minute(x: &[f32], sr: u32) -> f32 {
    let d2: Vec<f32> = (2..x.len())
        .map(|i| x[i] - 2.0 * x[i - 1] + x[i - 2])
        .collect();
    let block = (sr / 200) as usize;
    let rms: Vec<f32> = d2.chunks(block).map(cover_dsp::util::rms).collect();
    let local: Vec<f32> = (0..rms.len())
        .map(|b| median(&rms[b.saturating_sub(6)..(b + 7).min(rms.len())]))
        .collect();
    let mut count = 0;
    let mut last = 0usize;
    for (i, v) in d2.iter().enumerate() {
        if v.abs() > 10.0 * local[i / block] && v.abs() > 1e-3 && i > last + block {
            count += 1;
            last = i;
        }
    }
    count as f32 / (x.len() as f32 / sr as f32 / 60.0)
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() >= 3,
        "usage: vocal_report <projects dir> <project id> <label>=<file.wav>..."
    );
    let engine = Engine::new(&args[0])?;
    let project = engine.load(&args[1])?;
    let analysis = project.analysis.as_ref().expect("analysed project");
    let render = project.last_render.as_ref().expect("rendered project");
    let mut scale = scale_for(project.settings.tuning.scale, &analysis.key);
    let acc = read_wav(
        &engine
            .root()
            .join(&project.id)
            .join("cache/accompaniment.wav"),
    )?
    .slice_sec(render.start_sec, render.end_sec)
    .to_mono();
    if let Some(r) = tuning_reference(&acc, analysis.sample_rate).filter(|r| r.confidence > 0.3) {
        scale = scale.with_offset(r.cents / 100.0);
        println!("tuning reference {:+.1} cents", r.cents);
    }
    println!(
        "key {} (confidence {:.2}), scale {:?}",
        analysis.key.name(),
        analysis.key.confidence,
        scale.pitch_classes
    );
    let spec = registry::spec("rmvpe").expect("rmvpe in registry");
    let mut rmvpe = Rmvpe::load(&model_path(spec), Device::Auto)?;
    let cancel = AtomicBool::new(false);
    println!(
        "{:<14} {:>5} {:>9} {:>6} {:>6} {:>9} {:>9} {:>7} {:>7} {:>8}",
        "file",
        "notes",
        "err(c)",
        "<=15c",
        "<=30c",
        "drift(c)",
        "vib(c)",
        "LUFS",
        "dBTP",
        "clicks/m"
    );
    for arg in &args[2..] {
        let (label, path) = arg.split_once('=').unwrap_or((arg.as_str(), arg.as_str()));
        let audio = read_wav(std::path::Path::new(path))?;
        let mono = audio.to_mono();
        let pitch = rmvpe.track(&mono, audio.sample_rate, &cancel, &mut |_| {})?;
        let r = intonation_report(&pitch, &scale, 0.0, audio.duration_sec());
        let loud = measure(&audio.to_stereo())?;
        println!(
            "{:<14} {:>5} {:>9.1} {:>5.0}% {:>5.0}% {:>9.1} {:>9.1} {:>7.1} {:>7.1} {:>8.1}",
            label,
            r.notes,
            r.mean_error_cents,
            r.within_15 * 100.0,
            r.within_30 * 100.0,
            r.drift_cents,
            r.vibrato_depth_cents,
            loud.integrated_lufs,
            loud.true_peak_dbtp,
            clicks_per_minute(&mono, audio.sample_rate),
        );
    }
    Ok(())
}
