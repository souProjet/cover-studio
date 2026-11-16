#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Media(#[from] cover_media::MediaError),
    #[error(transparent)]
    Ml(#[from] cover_ml::MlError),
    #[error(transparent)]
    Dsp(#[from] cover_dsp::DspError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Image(#[from] image::ImageError),
    #[error("project {0} not found")]
    ProjectNotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("cancelled")]
    Cancelled,
}

impl EngineError {
    pub fn is_cancelled(&self) -> bool {
        matches!(
            self,
            EngineError::Cancelled
                | EngineError::Media(cover_media::MediaError::Cancelled)
                | EngineError::Ml(cover_ml::MlError::Cancelled)
        )
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;
