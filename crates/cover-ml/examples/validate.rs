//! Sanity checks for the ML pipeline against reference material.
//!
//! cargo run -p cover-ml --release --example validate -- <mix.wav> [reference_vocals.wav]

use cover_dsp::util::rms;
use cover_ml::registry::spec;
use cover_ml::{model_path, Device, MdxSeparator, ModelKind, Rmvpe};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

fn harmonic_tone(f0: f32, sr: u32, secs: f32) -> Vec<f32> {
    (0..(sr as f32 * secs) as usize)
        .map(|i| {
            let t = i as f32 / sr as f32;
            (1..8)
                .map(|h| (2.0 * std::f32::consts::PI * f0 * h as f32 * t).sin() / h as f32)
                .sum::<f32>()
                * 0.2
        })
        .collect()
}

fn pearson(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let (a, b) = (&a[..n], &b[..n]);
    let ma = a.iter().sum::<f32>() / n as f32;
    let mb = b.iter().sum::<f32>() / n as f32;
    let (mut num, mut da, mut db) = (0.0f64, 0.0f64, 0.0f64);
    for i in 0..n {
        let x = (a[i] - ma) as f64;
        let y = (b[i] - mb) as f64;
        num += x * y;
        da += x * x;
        db += y * y;
    }
    (num / (da.sqrt() * db.sqrt()).max(1e-12)) as f32
}

fn main() {
    let device = match std::env::var("DEVICE").as_deref() {
        Ok("cpu") => Device::Cpu,
        Ok("cuda") => Device::Cuda,
        _ => Device::Auto,
    };
    let cancel = AtomicBool::new(false);

    let rm_spec = spec("rmvpe").unwrap();
    let t = Instant::now();
    let mut rmvpe = Rmvpe::load(&model_path(rm_spec), device).expect("load rmvpe");
    println!(
        "rmvpe loaded on {} in {:?}",
        rmvpe.provider.label(),
        t.elapsed()
    );
    for f0 in [110.0f32, 220.0, 330.0, 523.25] {
        let x = harmonic_tone(f0, 44_100, 2.0);
        let track = rmvpe.track(&x, 44_100, &cancel, &mut |_| {}).unwrap();
        let voiced: Vec<f32> = track.f0_hz.iter().copied().filter(|v| *v > 0.0).collect();
        let med = cover_dsp::util::median(&voiced);
        println!(
            "  tone {f0:>7.2} Hz -> median {med:>7.2} Hz ({} / {} frames voiced)",
            voiced.len(),
            track.f0_hz.len()
        );
    }

    let Some(mix_path) = std::env::args().nth(1).map(PathBuf::from) else {
        return;
    };
    let reference = std::env::args().nth(2).map(PathBuf::from);
    let mix = cover_media::decode_audio(&mix_path, 44_100, Some(2)).expect("decode mix");
    let mix = mix.slice_sec(0.0, mix.duration_sec().min(60.0));
    println!("mix: {:.1}s", mix.duration_sec());

    let s = spec("mdx-voc-ft").unwrap();
    let ModelKind::Mdx { params } = s.kind else {
        unreachable!()
    };
    let t = Instant::now();
    let mut sep = MdxSeparator::load(&model_path(s), params, device).expect("load mdx");
    println!(
        "mdx loaded on {} in {:?}",
        sep.provider.label(),
        t.elapsed()
    );
    let t = Instant::now();
    let out = sep
        .separate(&mix, false, &cancel, &mut |_| {})
        .expect("separate");
    println!("separated in {:?}", t.elapsed());
    let v = out.vocals.to_mono();
    let a = out.accompaniment.to_mono();
    println!(
        "  rms mix {:.4} vocals {:.4} accomp {:.4}",
        rms(&mix.to_mono()),
        rms(&v),
        rms(&a)
    );

    if let Some(r) = reference {
        let refv = cover_media::decode_audio(&r, 44_100, Some(1)).expect("decode reference");
        let refv = &refv.channels[0];
        println!(
            "  correlation with reference vocals: {:.3}",
            pearson(&v, refv)
        );
        let env = |x: &[f32]| cover_dsp::analysis::rms_envelope(x, 4410);
        println!(
            "  envelope correlation: {:.3}",
            pearson(&env(&v), &env(refv))
        );
    }
    let out_dir = std::env::temp_dir().join("cover-ml-validate");
    std::fs::create_dir_all(&out_dir).unwrap();
    cover_media::wav::write_wav(
        &out_dir.join("vocals.wav"),
        &out.vocals,
        cover_media::wav::WavFormat::Pcm16,
    )
    .unwrap();
    cover_media::wav::write_wav(
        &out_dir.join("accompaniment.wav"),
        &out.accompaniment,
        cover_media::wav::WavFormat::Pcm16,
    )
    .unwrap();
    println!("  stems written to {}", out_dir.display());

    let t = Instant::now();
    let track = rmvpe.track(&v, 44_100, &cancel, &mut |_| {}).unwrap();
    let voiced = track.f0_hz.iter().filter(|f| **f > 0.0).count();
    println!(
        "  vocal pitch tracked in {:?}: {voiced}/{} frames voiced",
        t.elapsed(),
        track.f0_hz.len()
    );
}
