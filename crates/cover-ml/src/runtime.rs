use crate::Result;
use ort::environment::Environment;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    /// CUDA when it initialises, CPU otherwise.
    #[default]
    Auto,
    Cuda,
    Cpu,
}

/// The execution provider a session actually ended up on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provider {
    Cuda,
    Cpu { reason: Option<String> },
}

impl Provider {
    pub fn label(&self) -> String {
        match self {
            Provider::Cuda => "CUDA".into(),
            Provider::Cpu { reason: None } => "CPU".into(),
            Provider::Cpu { reason: Some(r) } => format!("CPU (GPU unavailable: {r})"),
        }
    }
}

fn environment() -> Result<&'static Environment> {
    static ENV: OnceLock<Environment> = OnceLock::new();
    if let Some(e) = ENV.get() {
        return Ok(e);
    }
    let env = ort::init().with_name("cover-studio").build()?;
    Ok(ENV.get_or_init(|| env))
}

fn cpu_session(path: &Path) -> Result<Session> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    Ok(Session::builder(environment()?)?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .with_intra_threads(threads)?
        .commit_from_file(path)?)
}

#[cfg(feature = "cuda")]
fn cuda_session(path: &Path) -> Result<Session> {
    use ort::ep;
    // Falls back to a system CUDA install when the per-user runtime is absent.
    if let Err(e) = crate::gpu_runtime::preload() {
        tracing::debug!(%e, "per-user GPU runtime not preloaded");
    }
    Ok(Session::builder(environment()?)?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .with_execution_providers([ep::CUDA::default().build().error_on_failure()])?
        .commit_from_file(path)?)
}

#[cfg(not(feature = "cuda"))]
fn cuda_session(_path: &Path) -> Result<Session> {
    Err(crate::MlError::BadOutput(
        "this build was compiled without CUDA support".into(),
    ))
}

/// Opens a model on the requested device, falling back to CPU for `Auto`.
pub fn open_session(path: &Path, device: Device) -> Result<(Session, Provider)> {
    match device {
        Device::Cpu => Ok((cpu_session(path)?, Provider::Cpu { reason: None })),
        Device::Cuda => Ok((cuda_session(path)?, Provider::Cuda)),
        Device::Auto => match cuda_session(path) {
            Ok(s) => Ok((s, Provider::Cuda)),
            Err(e) => {
                let reason = e.to_string();
                tracing::warn!(%reason, "CUDA unavailable, using CPU");
                // ONNX Runtime prefixes errors with a source path; the cause is at the end.
                let flat = reason.split_whitespace().collect::<Vec<_>>().join(" ");
                let chars: Vec<char> = flat.chars().collect();
                let short: String = chars[chars.len().saturating_sub(200)..].iter().collect();
                Ok((
                    cpu_session(path)?,
                    Provider::Cpu {
                        reason: Some(short),
                    },
                ))
            }
        },
    }
}
