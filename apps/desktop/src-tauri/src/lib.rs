//! Tauri shell: exposes the engine as commands and relays progress events.

use cover_engine::media::MediaInfo;
use cover_engine::project::ExportRecord;
use cover_engine::{
    Ctx, Engine, EngineError, Event, ExportRequest, Project, ProjectSummary, Settings, Sources,
    Stage, SystemInfo,
};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    engine: Arc<Engine>,
    jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

#[derive(Debug, Serialize)]
struct CommandError {
    message: String,
    cancelled: bool,
}

impl From<EngineError> for CommandError {
    fn from(e: EngineError) -> Self {
        CommandError {
            cancelled: e.is_cancelled(),
            message: e.to_string(),
        }
    }
}

impl From<tauri::Error> for CommandError {
    fn from(e: tauri::Error) -> Self {
        CommandError {
            message: e.to_string(),
            cancelled: false,
        }
    }
}

type CmdResult<T> = Result<T, CommandError>;

#[derive(Clone, Serialize)]
struct JobEvent {
    job: String,
    #[serde(flatten)]
    event: Event,
}

/// Lets the webview load a project's media and renders through the asset protocol.
fn allow_project(app: &AppHandle, engine: &Engine, p: &Project) {
    let scope = app.asset_protocol_scope();
    for src in [&p.sources.audio, &p.sources.video].into_iter().flatten() {
        let _ = scope.allow_file(src);
    }
    let _ = scope.allow_directory(engine.root().join(&p.id), true);
}

/// Runs a long engine operation on a worker thread with cancellation and
/// throttled progress events tagged with `job`.
async fn run_job<T, F>(app: AppHandle, job: String, f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Engine, &Ctx) -> cover_engine::Result<T> + Send + 'static,
{
    let state = app.state::<AppState>();
    let engine = state.engine.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    state
        .jobs
        .lock()
        .unwrap()
        .insert(job.clone(), cancel.clone());

    let app2 = app.clone();
    let job2 = job.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let last: Mutex<Option<(Stage, f32)>> = Mutex::new(None);
        let emit = |event: Event| {
            if let Event::Progress { stage, fraction } = event {
                let mut l = last.lock().unwrap();
                if let Some((s, f)) = *l {
                    if s == stage && (fraction - f).abs() < 0.005 && fraction < 1.0 {
                        return;
                    }
                }
                *l = Some((stage, fraction));
            }
            let _ = app2.emit(
                "job",
                JobEvent {
                    job: job2.clone(),
                    event,
                },
            );
        };
        let ctx = Ctx {
            cancel: &cancel,
            emit: &emit,
        };
        f(&engine, &ctx)
    })
    .await;

    app.state::<AppState>().jobs.lock().unwrap().remove(&job);
    match result {
        Ok(r) => r.map_err(CommandError::from),
        Err(e) => Err(CommandError {
            message: format!("worker failed: {e}"),
            cancelled: false,
        }),
    }
}

async fn blocking<T, F>(state: State<'_, AppState>, f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Engine) -> cover_engine::Result<T> + Send + 'static,
{
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || f(&engine))
        .await
        .map_err(|e| CommandError {
            message: e.to_string(),
            cancelled: false,
        })?
        .map_err(CommandError::from)
}

#[tauri::command]
async fn system_info(state: State<'_, AppState>) -> CmdResult<SystemInfo> {
    blocking(state, |e| Ok(e.system_info())).await
}

#[tauri::command]
async fn list_projects(state: State<'_, AppState>) -> CmdResult<Vec<ProjectSummary>> {
    blocking(state, |e| e.list_projects()).await
}

#[tauri::command]
async fn probe_media(state: State<'_, AppState>, path: PathBuf) -> CmdResult<MediaInfo> {
    blocking(state, move |_| Ok(cover_engine::media::probe(&path)?)).await
}

#[tauri::command]
async fn create_project(
    app: AppHandle,
    state: State<'_, AppState>,
    name: Option<String>,
    audio: Option<PathBuf>,
    video: Option<PathBuf>,
) -> CmdResult<Project> {
    let engine = state.engine.clone();
    let p = blocking(state, move |e| {
        e.create_project(name, Sources { audio, video })
    })
    .await?;
    allow_project(&app, &engine, &p);
    Ok(p)
}

#[tauri::command]
async fn get_project(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<Project> {
    let engine = state.engine.clone();
    let p = blocking(state, move |e| e.load(&id)).await?;
    allow_project(&app, &engine, &p);
    Ok(p)
}

#[tauri::command]
async fn update_settings(
    state: State<'_, AppState>,
    id: String,
    settings: Settings,
) -> CmdResult<Project> {
    blocking(state, move |e| e.update_settings(&id, settings)).await
}

#[tauri::command]
async fn rename_project(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> CmdResult<Project> {
    blocking(state, move |e| e.rename(&id, &name)).await
}

#[tauri::command]
async fn delete_project(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    blocking(state, move |e| e.delete(&id)).await
}

#[tauri::command]
async fn video_frame(
    state: State<'_, AppState>,
    id: String,
    audio_sec: f64,
    max_width: u32,
) -> CmdResult<PathBuf> {
    blocking(state, move |e| e.video_frame(&id, audio_sec, max_width)).await
}

#[tauri::command]
async fn caption_preview(state: State<'_, AppState>, id: String) -> CmdResult<String> {
    blocking(state, move |e| Ok(cover_engine::caption(&e.load(&id)?))).await
}

#[tauri::command]
async fn default_output_dir(state: State<'_, AppState>, id: String) -> CmdResult<PathBuf> {
    blocking(state, move |e| Ok(e.default_output_dir(&e.load(&id)?))).await
}

#[tauri::command]
async fn analyze(app: AppHandle, id: String, job: String) -> CmdResult<Project> {
    run_job(app, job, move |e, ctx| e.analyze(&id, ctx)).await
}

#[tauri::command]
async fn render_preview(app: AppHandle, id: String, job: String) -> CmdResult<Project> {
    run_job(app, job, move |e, ctx| e.render_preview(&id, ctx)).await
}

#[tauri::command]
async fn export(
    app: AppHandle,
    id: String,
    request: ExportRequest,
    job: String,
) -> CmdResult<ExportRecord> {
    run_job(app, job, move |e, ctx| e.export(&id, &request, ctx)).await
}

#[tauri::command]
async fn download_model(app: AppHandle, model: String, job: String) -> CmdResult<PathBuf> {
    run_job(app, job, move |e, ctx| e.download_model(&model, ctx)).await
}

#[tauri::command]
async fn install_gpu_runtime(app: AppHandle, job: String) -> CmdResult<PathBuf> {
    run_job(app, job, move |e, ctx| e.install_gpu_runtime(ctx)).await
}

#[tauri::command]
fn cancel_job(state: State<'_, AppState>, job: String) {
    if let Some(flag) = state.jobs.lock().unwrap().get(&job) {
        flag.store(true, Ordering::Relaxed);
    }
}

/// Uses an ffmpeg shipped next to the app (Windows bundles) when present.
fn use_bundled_ffmpeg(resource_dir: &Path) {
    let exe = if cfg!(windows) { ".exe" } else { "" };
    for (var, name) in [
        ("COVER_STUDIO_FFMPEG", "ffmpeg"),
        ("COVER_STUDIO_FFPROBE", "ffprobe"),
    ] {
        let path = resource_dir.join("ffmpeg").join(format!("{name}{exe}"));
        if std::env::var_os(var).is_none() && path.is_file() {
            // SAFETY: called during setup, before any worker thread reads the environment.
            unsafe { std::env::set_var(var, path) };
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,ort=error".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            if let Ok(dir) = app.path().resource_dir() {
                use_bundled_ffmpeg(&dir);
            }
            let engine = Engine::new(Engine::default_root())?;
            app.manage(AppState {
                engine: Arc::new(engine),
                jobs: Mutex::new(HashMap::new()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            system_info,
            list_projects,
            probe_media,
            create_project,
            get_project,
            update_settings,
            rename_project,
            delete_project,
            video_frame,
            default_output_dir,
            caption_preview,
            analyze,
            render_preview,
            export,
            download_model,
            install_gpu_runtime,
            cancel_job,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Cover Studio");
}
