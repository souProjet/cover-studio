//! ONNX Runtime inference for Cover Studio.
//!
//! Spectral front-ends (STFT, mel) run in Rust via `cover-dsp`; the ONNX
//! graphs only contain the neural networks. This is what makes exported
//! separation and pitch models portable across runtimes.

pub mod gpu_runtime;
pub mod mdx;
pub mod registry;
pub mod rmvpe;
pub mod runtime;

pub use mdx::{MdxParams, MdxSeparator, Separation};
pub use registry::{ensure_model, model_path, models_dir, ModelKind, ModelSpec, MODELS};
pub use rmvpe::{PitchTrack, Rmvpe};
pub use runtime::{Device, Provider};

#[derive(Debug, thiserror::Error)]
pub enum MlError {
    #[error("onnxruntime: {0}")]
    Ort(#[from] ort::Error),
    #[error("model {0} is not downloaded")]
    ModelMissing(String),
    #[error("download of {model} failed: {message}")]
    Download { model: String, message: String },
    #[error("checksum mismatch for {0}")]
    Checksum(String),
    #[error("unexpected model output: {0}")]
    BadOutput(String),
    #[error("cancelled")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl From<ort::Error<ort::session::builder::SessionBuilder>> for MlError {
    fn from(e: ort::Error<ort::session::builder::SessionBuilder>) -> Self {
        MlError::Ort(e.into())
    }
}

pub type Result<T> = std::result::Result<T, MlError>;

/// Progress callback: (fraction 0..=1).
pub type Progress<'a> = &'a mut dyn FnMut(f32);
