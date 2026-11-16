use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use cover_engine::media::{Aspect, AudioCodec, VideoQuality, VideoStyle};
use cover_engine::ml::Device;
use cover_engine::settings::{InstrumentKind, Language, ScaleMode, SoundPreset};
use cover_engine::{Ctx, Engine, Event, ExportRequest, Project, Sources};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

#[derive(Parser)]
#[command(
    name = "cover-studio",
    version,
    about = "Mix, tune, sync and export home-recorded music covers"
)]
struct Cli {
    /// Projects folder (default: platform data dir, or $COVER_STUDIO_PROJECTS).
    #[arg(long, global = true)]
    projects: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a project, analyse it and export everything in one go.
    Process {
        /// Audio or video file. Use --video/--audio for a separate recording.
        input: Option<PathBuf>,
        #[arg(long)]
        audio: Option<PathBuf>,
        #[arg(long)]
        video: Option<PathBuf>,
        #[arg(long)]
        name: Option<String>,
        #[command(flatten)]
        settings: SettingsArgs,
        #[command(flatten)]
        export: ExportArgs,
    },
    /// List projects.
    Projects,
    /// Show the detected toolchain, GPU support and models.
    Info,
    /// Run separation, pitch tracking, take detection and sync.
    Analyze { id: String },
    /// Render the processed preview with updated settings.
    Render {
        id: String,
        #[command(flatten)]
        settings: SettingsArgs,
    },
    /// Export audio, video, thumbnail and caption.
    Export {
        id: String,
        #[command(flatten)]
        settings: SettingsArgs,
        #[command(flatten)]
        export: ExportArgs,
    },
    /// Delete a project and its cache.
    Delete { id: String },
    /// Download models (all when no id is given).
    Models { id: Option<String> },
    /// Download the CUDA runtime used for GPU inference (~1.3 GB).
    Gpu,
}

#[derive(Clone, Copy, ValueEnum)]
enum PresetArg {
    Natural,
    Balanced,
    Polished,
}

#[derive(Clone, Copy, ValueEnum)]
enum InstrumentArg {
    Acoustic,
    Classical,
    Electric,
    Piano,
    Ukulele,
    Band,
    Other,
}

#[derive(Clone, Copy, ValueEnum)]
enum AspectArg {
    Vertical,
    Landscape,
    Square,
    Source,
}

#[derive(Clone, Copy, ValueEnum)]
enum StyleArg {
    Natural,
    Social,
    LowLight,
}

#[derive(Clone, Copy, ValueEnum)]
enum QualityArg {
    Draft,
    High,
    Max,
}

#[derive(Clone, Copy, ValueEnum)]
enum DeviceArg {
    Auto,
    Cuda,
    Cpu,
}

#[derive(Clone, Copy, ValueEnum)]
enum FormatArg {
    Wav,
    Mp3,
    M4a,
    Flac,
}

#[derive(Clone, Copy, ValueEnum)]
enum LangArg {
    Fr,
    En,
}

#[derive(Args, Default)]
struct SettingsArgs {
    #[arg(long, value_enum)]
    preset: Option<PresetArg>,
    #[arg(long, value_enum)]
    instrument: Option<InstrumentArg>,
    /// Pitch correction strength 0..1 (0 disables tuning).
    #[arg(long)]
    tuning: Option<f32>,
    /// Retune speed 0..1: 0 keeps attacks and scoops natural, 1 is tight.
    #[arg(long)]
    retune_speed: Option<f32>,
    /// Steady slow drift inside held notes, 0..1.
    #[arg(long)]
    drift: Option<f32>,
    /// Calm vibrato wider than normal, 0..1 (normal vibrato is kept).
    #[arg(long)]
    vibrato: Option<f32>,
    /// Shift the pitch without keeping the vocal timbre (formants).
    #[arg(long)]
    no_formants: bool,
    /// Snap to semitones instead of the detected key.
    #[arg(long)]
    chromatic: bool,
    /// Even out quiet and loud vocal phrases, 0..1.
    #[arg(long)]
    leveler: Option<f32>,
    /// Lower breaths between phrases, 0..1.
    #[arg(long)]
    breath: Option<f32>,
    /// De-essing amount, 0..1.
    #[arg(long)]
    de_ess: Option<f32>,
    /// Vocal noise, mouth-click and plosive clean-up, 0..1.
    #[arg(long)]
    cleanup: Option<f32>,
    /// Tame harsh, boomy and resonant moments, 0..1.
    #[arg(long)]
    harshness: Option<f32>,
    /// Vocal plate reverb, 0..1.
    #[arg(long)]
    vocal_reverb: Option<f32>,
    /// Take number to export (default: best detected take).
    #[arg(long)]
    take: Option<usize>,
    /// Range start in seconds of the audio recording.
    #[arg(long)]
    start: Option<f64>,
    #[arg(long)]
    end: Option<f64>,
    /// Video offset in seconds (video time minus audio time), overrides detection.
    #[arg(long, allow_hyphen_values = true)]
    offset: Option<f64>,
    #[arg(long)]
    lufs: Option<f64>,
    #[arg(long, value_enum)]
    device: Option<DeviceArg>,
    /// Second separation pass that removes model noise (twice slower).
    #[arg(long)]
    denoise: bool,
    #[arg(long, value_enum)]
    aspect: Option<AspectArg>,
    #[arg(long, value_enum)]
    style: Option<StyleArg>,
    #[arg(long, value_enum)]
    quality: Option<QualityArg>,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    artist: Option<String>,
    #[arg(long, value_enum)]
    lang: Option<LangArg>,
}

#[derive(Args)]
struct ExportArgs {
    /// Output folder (default: Music/Cover Studio/<project>).
    #[arg(long, short)]
    out: Option<PathBuf>,
    /// Audio formats to write.
    #[arg(long, value_enum, value_delimiter = ',', default_value = "wav,m4a")]
    formats: Vec<FormatArg>,
    #[arg(long)]
    no_video: bool,
    #[arg(long)]
    no_social: bool,
}

impl SettingsArgs {
    fn apply(&self, p: &mut Project) {
        let s = &mut p.settings;
        if let Some(v) = self.preset {
            s.apply_preset(match v {
                PresetArg::Natural => SoundPreset::Natural,
                PresetArg::Balanced => SoundPreset::Balanced,
                PresetArg::Polished => SoundPreset::Polished,
            });
        }
        if let Some(v) = self.instrument {
            s.instrument = match v {
                InstrumentArg::Acoustic => InstrumentKind::AcousticGuitar,
                InstrumentArg::Classical => InstrumentKind::ClassicalGuitar,
                InstrumentArg::Electric => InstrumentKind::ElectricGuitar,
                InstrumentArg::Piano => InstrumentKind::Piano,
                InstrumentArg::Ukulele => InstrumentKind::Ukulele,
                InstrumentArg::Band => InstrumentKind::FullBand,
                InstrumentArg::Other => InstrumentKind::Other,
            };
        }
        if let Some(v) = self.tuning {
            s.tuning.enabled = v > 0.0;
            s.tuning.strength = v.clamp(0.0, 1.0);
        }
        let unit = |v: f32| v.clamp(0.0, 1.0);
        if let Some(v) = self.retune_speed {
            s.tuning.retune_speed = unit(v);
        }
        if let Some(v) = self.drift {
            s.tuning.drift = unit(v);
        }
        if let Some(v) = self.vibrato {
            s.tuning.vibrato = unit(v);
        }
        if self.no_formants {
            s.tuning.preserve_formants = false;
        }
        if self.chromatic {
            s.tuning.scale = ScaleMode::Chromatic;
        }
        for (arg, field) in [
            (self.leveler, &mut s.vocal.leveler),
            (self.breath, &mut s.vocal.breath),
            (self.de_ess, &mut s.vocal.de_ess),
            (self.cleanup, &mut s.vocal.cleanup),
            (self.harshness, &mut s.vocal.harshness),
            (self.vocal_reverb, &mut s.vocal.reverb),
        ] {
            if let Some(v) = arg {
                *field = unit(v);
            }
        }
        if self.take.is_some() {
            s.range.take = self.take.map(|t| t.saturating_sub(1));
        }
        if self.start.is_some() {
            s.range.start_sec = self.start;
        }
        if self.end.is_some() {
            s.range.end_sec = self.end;
        }
        if self.offset.is_some() {
            s.sync_offset_sec = self.offset;
        }
        if let Some(v) = self.lufs {
            s.loudness.lufs = v;
        }
        if let Some(v) = self.device {
            s.device = match v {
                DeviceArg::Auto => Device::Auto,
                DeviceArg::Cuda => Device::Cuda,
                DeviceArg::Cpu => Device::Cpu,
            };
        }
        if self.denoise {
            s.denoise_separation = true;
        }
        if let Some(v) = self.aspect {
            s.video.aspect = match v {
                AspectArg::Vertical => Aspect::Vertical,
                AspectArg::Landscape => Aspect::Landscape,
                AspectArg::Square => Aspect::Square,
                AspectArg::Source => Aspect::Source,
            };
        }
        if let Some(v) = self.style {
            s.video.style = match v {
                StyleArg::Natural => VideoStyle::Natural,
                StyleArg::Social => VideoStyle::Social,
                StyleArg::LowLight => VideoStyle::LowLight,
            };
        }
        if let Some(v) = self.quality {
            s.video.quality = match v {
                QualityArg::Draft => VideoQuality::Draft,
                QualityArg::High => VideoQuality::High,
                QualityArg::Max => VideoQuality::Max,
            };
        }
        if let Some(v) = &self.title {
            s.social.title = v.clone();
        }
        if let Some(v) = &self.artist {
            s.social.artist = v.clone();
        }
        if let Some(v) = self.lang {
            s.social.language = match v {
                LangArg::Fr => Language::Fr,
                LangArg::En => Language::En,
            };
        }
    }
}

impl ExportArgs {
    fn request(&self) -> ExportRequest {
        ExportRequest {
            output_dir: self.out.clone(),
            wav: self.formats.iter().any(|f| matches!(f, FormatArg::Wav)),
            audio_formats: self
                .formats
                .iter()
                .filter_map(|f| match f {
                    FormatArg::Wav => None,
                    FormatArg::Mp3 => Some(AudioCodec::Mp3),
                    FormatArg::M4a => Some(AudioCodec::Aac),
                    FormatArg::Flac => Some(AudioCodec::Flac),
                })
                .collect(),
            video: !self.no_video,
            thumbnail: !self.no_social,
            caption: !self.no_social,
        }
    }
}

/// Prints stage headers and a single updating progress line on stderr.
fn printer() -> impl Fn(Event) + Sync {
    let last = Mutex::new(-1i32);
    move |e| {
        let mut err = std::io::stderr().lock();
        match e {
            Event::Stage { label, .. } => {
                *last.lock().unwrap() = -1;
                let _ = writeln!(err, "\r\x1b[K> {label}");
            }
            Event::Progress { fraction, .. } => {
                let pct = (fraction * 100.0) as i32;
                let mut l = last.lock().unwrap();
                if pct != *l {
                    *l = pct;
                    let bar = "#".repeat((pct / 4) as usize);
                    let _ = write!(err, "\r\x1b[K  [{bar:<25}] {pct:3}%");
                    if pct >= 100 {
                        let _ = writeln!(err);
                    }
                }
            }
            Event::Log { message } => {
                let _ = writeln!(err, "\r\x1b[K  {message}");
            }
        }
    }
}

fn print_analysis(p: &Project) {
    let Some(a) = &p.analysis else { return };
    println!("Project {} ({})", p.name, p.id);
    println!("  duration   {:.1} s", a.duration_sec);
    println!(
        "  key        {} (confidence {:.2})",
        a.key.name(),
        a.key.confidence
    );
    println!(
        "  loudness   {:.1} LUFS, {:.1} dBTP",
        a.original_loudness.integrated_lufs, a.original_loudness.true_peak_dbtp
    );
    println!("  inference  {}", a.provider);
    if let Some(s) = &a.sync {
        println!(
            "  sync       {:+.3} s, confidence {:.2} ({:?})",
            s.offset_sec, s.confidence, s.method
        );
    }
    for t in &a.takes {
        let best = if Some(t.index) == a.best_take {
            "  <- best"
        } else {
            ""
        };
        println!(
            "  take {:>2}    {:>6.1} - {:>6.1} s  score {:.2}  tuning {}{best}",
            t.index + 1,
            t.start_sec,
            t.end_sec,
            t.score,
            t.tuning_cents
                .map_or("-".into(), |c| format!("{c:.0} cents")),
        );
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,ort=error".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let engine = Engine::new(cli.projects.unwrap_or_else(Engine::default_root))?;
    let cancel = AtomicBool::new(false);
    let emit = printer();
    let ctx = Ctx {
        cancel: &cancel,
        emit: &emit,
    };

    match cli.command {
        Command::Process {
            input,
            audio,
            video,
            name,
            settings,
            export,
        } => {
            let sources = match (input, audio, video) {
                (Some(i), None, None) => {
                    let info = cover_engine::media::probe(&i)?;
                    if info.video.is_some() {
                        Sources {
                            audio: None,
                            video: Some(i),
                        }
                    } else {
                        Sources {
                            audio: Some(i),
                            video: None,
                        }
                    }
                }
                (Some(i), None, Some(v)) => Sources {
                    audio: Some(i),
                    video: Some(v),
                },
                (None, a, v) if a.is_some() || v.is_some() => Sources { audio: a, video: v },
                _ => bail!("give an input file, or --audio and/or --video"),
            };
            let mut p = engine.create_project(name, sources)?;
            settings.apply(&mut p);
            engine.save(&mut p)?;
            let p = engine.analyze(&p.id, &ctx)?;
            print_analysis(&p);
            let rec = engine.export(&p.id, &export.request(), &ctx)?;
            for f in rec.files {
                println!("{}", f.path.display());
            }
        }
        Command::Projects => {
            for p in engine.list_projects()? {
                println!(
                    "{}  {:<32} {:>7} {}{}",
                    p.id,
                    p.name,
                    p.duration_sec.map_or("-".into(), |d| format!("{d:.0}s")),
                    p.key.map(|k| k.name()).unwrap_or_default(),
                    if p.has_video { "  [video]" } else { "" }
                );
            }
        }
        Command::Info => println!("{}", serde_json::to_string_pretty(&engine.system_info())?),
        Command::Analyze { id } => print_analysis(&engine.analyze(&id, &ctx)?),
        Command::Render { id, settings } => {
            let mut p = engine.load(&id)?;
            settings.apply(&mut p);
            engine.save(&mut p)?;
            let p = engine.render_preview(&id, &ctx)?;
            let r = p.last_render.context("no render")?;
            println!("{}", r.processed.display());
            println!("{}", r.original.display());
        }
        Command::Export {
            id,
            settings,
            export,
        } => {
            let mut p = engine.load(&id)?;
            settings.apply(&mut p);
            engine.save(&mut p)?;
            for f in engine.export(&id, &export.request(), &ctx)?.files {
                println!("{}", f.path.display());
            }
        }
        Command::Delete { id } => engine.delete(&id)?,
        Command::Models { id } => match id {
            Some(id) => println!("{}", engine.download_model(&id, &ctx)?.display()),
            None => {
                for m in cover_engine::ml::MODELS {
                    println!("{}", engine.download_model(m.id, &ctx)?.display());
                }
            }
        },
        Command::Gpu => println!("{}", engine.install_gpu_runtime(&ctx)?.display()),
    }
    Ok(())
}
