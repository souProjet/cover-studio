//! Project storage and the high-level pipeline used by the CLI and the app.

use crate::analysis::key::detect_key;
use crate::analysis::sync::analyze_sync;
use crate::analysis::takes::{best_take, detect_takes};
use crate::analysis::tuning::Scale;
use crate::error::{EngineError, Result};
use crate::events::{Ctx, Stage};
use crate::project::{
    slug, Analysis, ExportKind, ExportRecord, ExportedFile, MediaSummary, Project, ProjectSummary,
    RenderInfo, Sources, Waveform,
};
use crate::render::{render, RenderInputs};
use crate::settings::{ScaleMode, Settings};
use crate::social;
use chrono::Utc;
use cover_dsp::analysis::key::Key;
use cover_dsp::loudness::{measure, normalize};
use cover_dsp::util::apply_fades;
use cover_media::wav::{read_wav, write_wav, WavFormat};
use cover_media::{
    decode_audio, encode_audio, extract_frame, extract_preview, probe, render_video, AudioCodec,
    MediaInfo, Tools, VideoRenderSpec,
};
use cover_ml::gpu_runtime::{self, GpuRuntimeStatus};
use cover_ml::{ensure_model, ModelKind, ModelSpec, PitchTrack, MODELS};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const SR: u32 = cover_ml::mdx::MDX_SAMPLE_RATE;
const SYNC_SR: u32 = 22_050;
const WAVEFORM_POINTS: usize = 2400;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportRequest {
    /// Defaults to `<Music>/Cover Studio/<project>`.
    pub output_dir: Option<PathBuf>,
    pub wav: bool,
    pub audio_formats: Vec<AudioCodec>,
    pub video: bool,
    pub thumbnail: bool,
    pub caption: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelStatus {
    pub spec: ModelSpec,
    pub downloaded: bool,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemInfo {
    pub version: &'static str,
    pub ffmpeg: Option<String>,
    pub ffmpeg_error: Option<String>,
    pub nvenc: bool,
    pub gpu_runtime: GpuRuntimeStatus,
    pub models: Vec<ModelStatus>,
    pub models_dir: PathBuf,
    pub projects_dir: PathBuf,
}

pub struct Engine {
    root: PathBuf,
}

fn file_stamp(h: &mut Sha256, path: &Path) {
    h.update(path.to_string_lossy().as_bytes());
    if let Ok(m) = path.metadata() {
        h.update(m.len().to_le_bytes());
        if let Ok(t) = m.modified() {
            if let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) {
                h.update(d.as_secs().to_le_bytes());
            }
        }
    }
}

fn hash_hex(h: Sha256) -> String {
    hex::encode(&h.finalize()[..12])
}

/// Identifies everything the cached stems depend on.
fn stems_fingerprint(sources: &Sources, settings: &Settings) -> String {
    let mut h = Sha256::new();
    if let Some(p) = sources.master_audio() {
        file_stamp(&mut h, p);
    }
    h.update(settings.separation_model.as_bytes());
    h.update([settings.denoise_separation as u8]);
    hash_hex(h)
}

/// Bump whenever the processing changes what a render sounds like, so that
/// renders cached by an older engine are not reused.
const RENDER_REVISION: u32 = 2;

/// Identifies everything the rendered master depends on.
fn render_fingerprint(project: &Project, range: (f64, f64)) -> String {
    let mut s = project.settings.clone();
    s.video = Settings::default().video;
    s.social = Settings::default().social;
    s.sync_offset_sec = None;
    let mut h = Sha256::new();
    h.update(RENDER_REVISION.to_le_bytes());
    h.update(serde_json::to_vec(&s).unwrap_or_default());
    h.update(range.0.to_le_bytes());
    h.update(range.1.to_le_bytes());
    if let Some(a) = &project.analysis {
        h.update(a.fingerprint.as_bytes());
    }
    hash_hex(h)
}

/// Below this the detected key is a guess, and snapping to its scale could
/// pull notes onto the wrong neighbour; the semitone grid is safer.
const MIN_KEY_CONFIDENCE: f32 = 0.55;

pub fn scale_for(mode: ScaleMode, key: &Key) -> Scale {
    match mode {
        ScaleMode::Auto if key.confidence < MIN_KEY_CONFIDENCE => Scale::chromatic(),
        ScaleMode::Auto => Scale::from_key(key),
        ScaleMode::Chromatic => Scale::chromatic(),
        ScaleMode::Manual { tonic, mode } => Scale::from_key(&Key {
            tonic,
            mode,
            confidence: 1.0,
        }),
    }
}

fn post_title(p: &Project) -> String {
    match p.settings.social.title.trim() {
        "" => p.name.clone(),
        t => t.to_string(),
    }
}

/// Caption text exported with the post.
pub fn caption(p: &Project) -> String {
    let s = &p.settings.social;
    social::caption(
        &post_title(p),
        &s.artist,
        p.settings.instrument,
        s.language,
        &s.extra_hashtags,
    )
}

fn model_spec(id: &str) -> Result<&'static ModelSpec> {
    cover_ml::registry::spec(id).ok_or_else(|| EngineError::Invalid(format!("unknown model {id}")))
}

impl Engine {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Engine { root })
    }

    /// `$COVER_STUDIO_PROJECTS`, else the platform data dir.
    pub fn default_root() -> PathBuf {
        if let Some(p) = std::env::var_os("COVER_STUDIO_PROJECTS") {
            return PathBuf::from(p);
        }
        directories::ProjectDirs::from("", "", "cover-studio")
            .map(|d| d.data_dir().join("projects"))
            .unwrap_or_else(|| PathBuf::from("cover-studio-projects"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    fn cache_dir(&self, id: &str) -> PathBuf {
        self.dir(id).join("cache")
    }

    fn renders_dir(&self, id: &str) -> PathBuf {
        self.dir(id).join("renders")
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectSummary>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let file = entry.path().join("project.json");
            if !file.is_file() {
                continue;
            }
            match std::fs::read(&file)
                .map_err(EngineError::from)
                .and_then(|b| serde_json::from_slice::<Project>(&b).map_err(EngineError::from))
            {
                Ok(p) => out.push(p.summary()),
                Err(e) => tracing::warn!("skipping {}: {e}", file.display()),
            }
        }
        out.sort_by_key(|p| std::cmp::Reverse(p.updated_at));
        Ok(out)
    }

    pub fn create_project(&self, name: Option<String>, sources: Sources) -> Result<Project> {
        let Some(master) = sources.master_audio() else {
            return Err(EngineError::Invalid(
                "a project needs an audio or video file".into(),
            ));
        };
        let mut media = MediaSummary::default();
        if let Some(a) = &sources.audio {
            let info = probe(a)?;
            if info.audio.is_none() {
                return Err(EngineError::Invalid(format!(
                    "{} has no audio stream",
                    a.display()
                )));
            }
            media.audio = Some(info);
        }
        if let Some(v) = &sources.video {
            let info = probe(v)?;
            if info.video.is_none() {
                return Err(EngineError::Invalid(format!(
                    "{} has no video stream",
                    v.display()
                )));
            }
            if sources.audio.is_none() && info.audio.is_none() {
                return Err(EngineError::Invalid(format!(
                    "{} has no audio stream",
                    v.display()
                )));
            }
            media.video = Some(info);
        }
        let name = name
            .filter(|n| !n.trim().is_empty())
            .or_else(|| master.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "Cover".into());
        let now = Utc::now();
        let mut settings = Settings::default();
        settings.social.title = name.clone();
        let mut project = Project {
            id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
            name,
            created_at: now,
            updated_at: now,
            sources,
            media,
            analysis: None,
            settings,
            last_render: None,
            exports: Vec::new(),
        };
        std::fs::create_dir_all(self.dir(&project.id))?;
        self.save(&mut project)?;
        Ok(project)
    }

    pub fn load(&self, id: &str) -> Result<Project> {
        let file = self.dir(id).join("project.json");
        if !file.is_file() {
            return Err(EngineError::ProjectNotFound(id.into()));
        }
        Ok(serde_json::from_slice(&std::fs::read(file)?)?)
    }

    pub fn save(&self, project: &mut Project) -> Result<()> {
        project.updated_at = Utc::now();
        let dir = self.dir(&project.id);
        std::fs::create_dir_all(&dir)?;
        let tmp = dir.join("project.json.part");
        std::fs::write(&tmp, serde_json::to_vec_pretty(project)?)?;
        std::fs::rename(tmp, dir.join("project.json"))?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let dir = self.dir(id);
        if !dir.join("project.json").is_file() {
            return Err(EngineError::ProjectNotFound(id.into()));
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<Project> {
        let mut p = self.load(id)?;
        p.name = name.trim().to_string();
        self.save(&mut p)?;
        Ok(p)
    }

    pub fn update_settings(&self, id: &str, settings: Settings) -> Result<Project> {
        let mut p = self.load(id)?;
        p.settings = settings;
        self.save(&mut p)?;
        Ok(p)
    }

    pub fn download_model(&self, id: &str, ctx: &Ctx) -> Result<PathBuf> {
        let spec = model_spec(id)?;
        self.ensure_model(spec, ctx)
    }

    fn ensure_model(&self, spec: &ModelSpec, ctx: &Ctx) -> Result<PathBuf> {
        if cover_ml::registry::is_downloaded(spec) {
            return Ok(cover_ml::model_path(spec));
        }
        ctx.stage(Stage::Download, format!("Downloading {}", spec.name));
        let path = ensure_model(spec, ctx.cancel, &mut |done, total| {
            ctx.progress(Stage::Download, done as f32 / total.max(1) as f32)
        })?;
        Ok(path)
    }

    pub fn install_gpu_runtime(&self, ctx: &Ctx) -> Result<PathBuf> {
        ctx.stage(Stage::Download, "Downloading the CUDA runtime");
        let mut last_pkg = String::new();
        let dir = gpu_runtime::install(ctx.cancel, &mut |done, total, pkg| {
            if pkg != last_pkg {
                ctx.log(format!("Downloading {pkg}"));
                last_pkg = pkg.to_string();
            }
            ctx.progress(Stage::Download, done as f32 / total.max(1) as f32)
        })?;
        Ok(dir)
    }

    pub fn system_info(&self) -> SystemInfo {
        let (ffmpeg, ffmpeg_error, nvenc) = match Tools::locate() {
            Ok(t) => (t.version(), None, cover_media::gpu_encoder_available()),
            Err(e) => (None, Some(e.to_string()), false),
        };
        SystemInfo {
            version: env!("CARGO_PKG_VERSION"),
            ffmpeg,
            ffmpeg_error,
            nvenc,
            gpu_runtime: gpu_runtime::status(),
            models: MODELS
                .iter()
                .map(|m| ModelStatus {
                    spec: *m,
                    downloaded: cover_ml::registry::is_downloaded(m),
                    path: cover_ml::model_path(m),
                })
                .collect(),
            models_dir: cover_ml::models_dir(),
            projects_dir: self.root.clone(),
        }
    }

    /// Separation and pitch tracking, reused while the fingerprint matches.
    fn ensure_stems(&self, p: &Project, ctx: &Ctx) -> Result<(String, String)> {
        let fp = stems_fingerprint(&p.sources, &p.settings);
        let cache = self.cache_dir(&p.id);
        let stamp = cache.join("fingerprint");
        let files = [
            "original.wav",
            "vocals.wav",
            "accompaniment.wav",
            "pitch.json",
        ];
        if std::fs::read_to_string(&stamp).ok().as_deref() == Some(fp.as_str())
            && files.iter().all(|f| cache.join(f).is_file())
        {
            ctx.log("Reusing cached stems");
            let provider =
                std::fs::read_to_string(cache.join("provider")).unwrap_or_else(|_| "cache".into());
            return Ok((fp, provider));
        }
        std::fs::create_dir_all(&cache)?;
        std::fs::remove_file(&stamp).ok();

        let sep_spec = model_spec(&p.settings.separation_model)?;
        let ModelKind::Mdx { params } = sep_spec.kind else {
            return Err(EngineError::Invalid(format!(
                "{} is not a separation model",
                sep_spec.id
            )));
        };
        let sep_path = self.ensure_model(sep_spec, ctx)?;
        let rmvpe_path = self.ensure_model(model_spec("rmvpe")?, ctx)?;
        ctx.check()?;

        ctx.stage(Stage::Prepare, "Decoding audio");
        let master = p.sources.master_audio().expect("checked at creation");
        let original = decode_audio(master, SR, Some(2))?;
        ctx.check()?;

        ctx.stage(
            Stage::Separate,
            format!("Separating voice and instrument ({})", sep_spec.name),
        );
        let mut sep = cover_ml::MdxSeparator::load(&sep_path, params, p.settings.device)?;
        let provider = sep.provider.label();
        ctx.log(format!("Separation running on {provider}"));
        let stems = sep.separate(
            &original,
            p.settings.denoise_separation,
            ctx.cancel,
            &mut |f| ctx.progress(Stage::Separate, f),
        )?;
        drop(sep);

        ctx.stage(Stage::Pitch, "Tracking the sung melody");
        let mut rmvpe = cover_ml::Rmvpe::load(&rmvpe_path, p.settings.device)?;
        let pitch = rmvpe.track(&stems.vocals.to_mono(), SR, ctx.cancel, &mut |f| {
            ctx.progress(Stage::Pitch, f)
        })?;
        drop(rmvpe);

        write_wav(&cache.join("original.wav"), &original, WavFormat::Float32)?;
        write_wav(&cache.join("vocals.wav"), &stems.vocals, WavFormat::Float32)?;
        write_wav(
            &cache.join("accompaniment.wav"),
            &stems.accompaniment,
            WavFormat::Float32,
        )?;
        std::fs::write(cache.join("pitch.json"), serde_json::to_vec(&pitch)?)?;
        std::fs::write(cache.join("provider"), &provider)?;
        std::fs::write(&stamp, &fp)?;
        Ok((fp, provider))
    }

    fn load_pitch(&self, id: &str) -> Result<PitchTrack> {
        Ok(serde_json::from_slice(&std::fs::read(
            self.cache_dir(id).join("pitch.json"),
        )?)?)
    }

    pub fn analyze(&self, id: &str, ctx: &Ctx) -> Result<Project> {
        let mut p = self.load(id)?;
        let (fingerprint, provider) = self.ensure_stems(&p, ctx)?;
        let cache = self.cache_dir(id);

        ctx.stage(Stage::Analyze, "Finding takes and key");
        let original = read_wav(&cache.join("original.wav"))?;
        let vocals = read_wav(&cache.join("vocals.wav"))?;
        let accompaniment = read_wav(&cache.join("accompaniment.wav"))?;
        let pitch = self.load_pitch(id)?;
        let mono = original.to_mono();
        let key = detect_key(&accompaniment.to_mono(), SR, Some(&pitch));
        ctx.progress(Stage::Analyze, 0.4);
        let scale = scale_for(p.settings.tuning.scale, &key);
        let takes = detect_takes(&mono, SR, Some(&pitch), &scale);
        let best = best_take(&takes).map(|t| t.index);
        ctx.progress(Stage::Analyze, 0.7);
        let original_loudness = measure(&original)?;
        let waveform = Waveform::from_samples(&mono, SR, WAVEFORM_POINTS);
        let vocal_waveform = Waveform::from_samples(&vocals.to_mono(), SR, WAVEFORM_POINTS);
        drop((vocals, accompaniment));
        ctx.progress(Stage::Analyze, 1.0);

        let sync = if p.sources.needs_sync() {
            ctx.stage(Stage::Sync, "Synchronising audio and video");
            let video = p.sources.video.as_ref().expect("needs_sync");
            let v = decode_audio(video, SYNC_SR, Some(1))?.to_mono();
            ctx.progress(Stage::Sync, 0.4);
            let a = cover_dsp::resample::resample(&mono, SR, SYNC_SR);
            let r = analyze_sync(&v, &a, SYNC_SR);
            ctx.log(format!(
                "Video offset {:+.3} s (confidence {:.2}, {:?})",
                r.offset_sec, r.confidence, r.method
            ));
            ctx.progress(Stage::Sync, 1.0);
            Some(r)
        } else {
            None
        };

        let duration_sec = original.duration_sec();
        p.analysis = Some(Analysis {
            fingerprint,
            duration_sec,
            sample_rate: SR,
            sync,
            takes,
            best_take: best,
            key,
            original_loudness,
            waveform,
            vocal_waveform,
            provider,
        });
        // A new analysis invalidates take indices chosen earlier.
        if p.settings
            .range
            .take
            .is_some_and(|t| t >= p.analysis.as_ref().map_or(0, |a| a.takes.len()))
        {
            p.settings.range.take = None;
        }
        p.last_render = None;
        self.save(&mut p)?;
        Ok(p)
    }

    /// Renders the selected range with the current settings, plus a
    /// loudness-matched copy of the untouched recording for A/B listening.
    pub fn render_preview(&self, id: &str, ctx: &Ctx) -> Result<Project> {
        let mut p = self.load(id)?;
        let analysis = p
            .analysis
            .clone()
            .ok_or_else(|| EngineError::Invalid("analyse the project first".into()))?;
        let range = p
            .effective_range()
            .ok_or_else(|| EngineError::Invalid("the selected range is empty".into()))?;
        let fp = render_fingerprint(&p, range);
        let renders = self.renders_dir(id);
        if let Some(r) = &p.last_render {
            if r.fingerprint == fp && r.master.is_file() && r.processed.is_file() {
                return Ok(p);
            }
        }
        let cache = self.cache_dir(id);
        if std::fs::read_to_string(cache.join("fingerprint"))
            .ok()
            .as_deref()
            != Some(analysis.fingerprint.as_str())
        {
            return Err(EngineError::Invalid(
                "sources or separation settings changed, analyse again".into(),
            ));
        }

        ctx.stage(Stage::Render, "Processing voice and instrument");
        let original = read_wav(&cache.join("original.wav"))?;
        let vocals = read_wav(&cache.join("vocals.wav"))?;
        let accompaniment = read_wav(&cache.join("accompaniment.wav"))?;
        let pitch = self.load_pitch(id)?;
        ctx.progress(Stage::Render, 0.1);
        let scale = scale_for(p.settings.tuning.scale, &analysis.key);
        let out = render(
            &RenderInputs {
                original: &original,
                vocals: &vocals,
                accompaniment: &accompaniment,
                pitch: Some(&pitch),
                scale,
                settings: &p.settings,
                start_sec: range.0,
                end_sec: range.1,
            },
            &|| ctx.check(),
        )?;
        drop((vocals, accompaniment));
        ctx.progress(Stage::Render, 0.85);

        std::fs::create_dir_all(&renders)?;
        let master_path = renders.join("master.wav");
        let processed = renders.join("preview.wav");
        let original_path = renders.join("original.wav");
        write_wav(&master_path, &out.master, WavFormat::Float32)?;
        write_wav(&processed, &out.master, WavFormat::Pcm16)?;
        let mut orig_range = original.to_stereo().slice_sec(range.0, range.1);
        normalize(
            &mut orig_range,
            p.settings.loudness.lufs,
            p.settings.loudness.ceiling_dbtp,
        )?;
        write_wav(&original_path, &orig_range, WavFormat::Pcm16)?;
        ctx.progress(Stage::Render, 1.0);
        ctx.log(format!(
            "Rendered {:.1} s at {:.1} LUFS, {} notes corrected (mean {:.0} cents)",
            range.1 - range.0,
            out.stats.loudness.integrated_lufs,
            out.stats.notes_corrected,
            out.stats.mean_correction_cents
        ));
        if let Some(c) = out.reference_cents {
            ctx.log(format!("Tuning reference {c:+.1} cents from A440"));
        }
        let v = out.vocal;
        ctx.log(format!(
            "Voice: levelling {:+.1}..{:+.1} dB, {} breath(s) lowered, {} click(s) repaired, \
             sibilance at {:.0} Hz, noise floor {:.0} dB below{}",
            v.leveler_min_db,
            v.leveler_max_db,
            v.breaths,
            v.clicks,
            v.sibilance_hz,
            v.snr_db,
            if v.denoised { " (denoised)" } else { "" }
        ));

        p.last_render = Some(RenderInfo {
            at: Utc::now(),
            fingerprint: fp,
            start_sec: range.0,
            end_sec: range.1,
            master: master_path,
            processed,
            original: original_path,
            waveform: Waveform::from_samples(&out.master.to_mono(), SR, WAVEFORM_POINTS),
            stats: out.stats,
            notes: out.curve.map(|c| c.notes).unwrap_or_default(),
        });
        self.save(&mut p)?;
        Ok(p)
    }

    /// Video frame at a master-audio time, for checking sync and picking thumbnails.
    pub fn video_frame(&self, id: &str, audio_sec: f64, max_width: u32) -> Result<PathBuf> {
        let p = self.load(id)?;
        let video = p
            .sources
            .video
            .as_ref()
            .ok_or_else(|| EngineError::Invalid("the project has no video".into()))?;
        let t = (audio_sec + p.sync_offset()).max(0.0);
        let dir = self.cache_dir(id).join("frames");
        std::fs::create_dir_all(&dir)?;
        let out = dir.join(format!("{}-{max_width}.jpg", (t * 1000.0).round() as i64));
        if !out.is_file() {
            extract_preview(video, t, max_width, &out)?;
        }
        Ok(out)
    }

    pub fn default_output_dir(&self, p: &Project) -> PathBuf {
        let base = directories::UserDirs::new()
            .and_then(|d| {
                d.audio_dir()
                    .map(Path::to_path_buf)
                    .or_else(|| Some(d.home_dir().to_path_buf()))
            })
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("Cover Studio").join(slug(&p.name))
    }

    pub fn export(&self, id: &str, req: &ExportRequest, ctx: &Ctx) -> Result<ExportRecord> {
        let mut p = self.render_preview(id, ctx)?;
        let r = p
            .last_render
            .clone()
            .expect("render_preview sets last_render");
        let dir = req
            .output_dir
            .clone()
            .unwrap_or_else(|| self.default_output_dir(&p));
        std::fs::create_dir_all(&dir)?;
        let social = &p.settings.social;
        let title = post_title(&p);
        let base = slug(&title);
        let vs = p.settings.video;
        let mut files = Vec::new();

        ctx.stage(Stage::Export, "Writing audio");
        let mut master = read_wav(&r.master)?;
        let fade_in = master.sec_to_frame(vs.fade_in_sec.min(0.05));
        let fade_out = master.sec_to_frame(vs.fade_out_sec);
        for c in master.channels.iter_mut() {
            apply_fades(c, fade_in, fade_out);
        }
        let wav_path = dir.join(format!("{base}.wav"));
        let need_wav = req.wav || !req.audio_formats.is_empty();
        if need_wav {
            write_wav(&wav_path, &master, WavFormat::Pcm24)?;
            for codec in &req.audio_formats {
                ctx.check()?;
                let out = dir.join(format!("{base}.{}", codec.extension()));
                encode_audio(&wav_path, &out, *codec)?;
                files.push(ExportedFile {
                    kind: ExportKind::Audio,
                    path: out,
                });
            }
            if req.wav {
                files.push(ExportedFile {
                    kind: ExportKind::Wav,
                    path: wav_path.clone(),
                });
            } else {
                std::fs::remove_file(&wav_path).ok();
            }
        }

        let video_info: Option<MediaInfo> = p.media.video.clone();
        let offset = p.sync_offset();
        if req.video {
            match (&p.sources.video, &video_info) {
                (Some(video), Some(info)) => {
                    ctx.stage(Stage::Video, "Rendering the video");
                    let v_start = (r.start_sec + offset).max(0.0);
                    let v_end = (r.end_sec + offset).min(info.duration_sec);
                    if v_end - v_start < 0.5 {
                        return Err(EngineError::Invalid(
                            "the selected range falls outside the video, check the sync offset"
                                .into(),
                        ));
                    }
                    let a0 = master.sec_to_frame(v_start - offset - r.start_sec);
                    let a1 = master.sec_to_frame(v_end - offset - r.start_sec);
                    let mut aligned = read_wav(&r.master)?.slice_frames(a0, a1);
                    let fi = aligned.sec_to_frame(vs.fade_in_sec.min(0.05));
                    let fo = aligned.sec_to_frame(vs.fade_out_sec);
                    for c in aligned.channels.iter_mut() {
                        apply_fades(c, fi, fo);
                    }
                    let aligned_path = self.renders_dir(id).join("video_audio.wav");
                    write_wav(&aligned_path, &aligned, WavFormat::Float32)?;
                    let out = dir.join(format!("{base}.mp4"));
                    render_video(
                        &VideoRenderSpec {
                            video: video.clone(),
                            audio: aligned_path,
                            out: out.clone(),
                            start_sec: v_start,
                            end_sec: v_end,
                            aspect: vs.aspect,
                            style: vs.style,
                            quality: vs.quality,
                            fade_in_sec: vs.fade_in_sec,
                            fade_out_sec: vs.fade_out_sec,
                            fps: None,
                        },
                        ctx.cancel,
                        |f| ctx.progress(Stage::Video, f),
                    )?;
                    files.push(ExportedFile {
                        kind: ExportKind::Video,
                        path: out,
                    });
                }
                _ => ctx.log("No video in this project, skipping the video export"),
            }
        }

        if req.thumbnail {
            ctx.stage(Stage::Social, "Creating the thumbnail");
            let t_audio = social
                .thumbnail_at_sec
                .unwrap_or(r.start_sec + 0.35 * (r.end_sec - r.start_sec));
            let img = match (&p.sources.video, &video_info) {
                (Some(video), Some(info)) => {
                    let t = (t_audio + offset).clamp(0.0, (info.duration_sec - 0.1).max(0.0));
                    let frame = self.renders_dir(id).join("thumb_frame.png");
                    extract_frame(video, t, vs.aspect, &frame)?;
                    image::open(&frame)?.to_rgb8()
                }
                _ => {
                    let (w, h) = vs.aspect.target_size(1080, 1920);
                    social::gradient_background(w, h)
                }
            };
            let subtitle = match social.artist.trim() {
                "" => "Cover".to_string(),
                a => format!("{a} - cover"),
            };
            let out = dir.join(format!("{base}.jpg"));
            social::compose_thumbnail(img, &title, &subtitle, &out)?;
            files.push(ExportedFile {
                kind: ExportKind::Thumbnail,
                path: out,
            });
        }

        if req.caption {
            let text = caption(&p);
            let out = dir.join(format!("{base}.txt"));
            std::fs::write(&out, text)?;
            files.push(ExportedFile {
                kind: ExportKind::Caption,
                path: out,
            });
        }

        let record = ExportRecord {
            at: Utc::now(),
            dir,
            files,
        };
        p.exports.push(record.clone());
        self.save(&mut p)?;
        ctx.log(format!(
            "Exported {} file(s) to {}",
            record.files.len(),
            record.dir.display()
        ));
        Ok(record)
    }
}
