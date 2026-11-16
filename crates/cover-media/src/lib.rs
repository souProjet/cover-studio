//! Media I/O through the `ffmpeg` and `ffprobe` command line tools.
//!
//! Running ffmpeg as a subprocess keeps the build free of native FFmpeg
//! bindings and lets users bring any build (with NVENC, zimg, ...) they like.

pub mod decode;
pub mod encode;
pub mod frame;
pub mod probe;
pub mod tools;
pub mod wav;

pub use decode::decode_audio;
pub use encode::{
    encode_audio, render_proxy, render_video, Aspect, AudioCodec, VideoQuality, VideoRenderSpec,
    VideoStyle,
};
pub use frame::{extract_frame, extract_preview};
pub use probe::{probe, AudioStreamInfo, HdrTransfer, MediaInfo, VideoStreamInfo};
pub use tools::{gpu_encoder_available, Tools};

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("{0} not found: install FFmpeg or set COVER_STUDIO_FFMPEG / COVER_STUDIO_FFPROBE")]
    ToolMissing(&'static str),
    #[error("{tool} failed: {message}")]
    ToolFailed { tool: &'static str, message: String },
    #[error("unsupported media: {0}")]
    Unsupported(String),
    #[error("cancelled")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Wav(#[from] hound::Error),
}

pub type Result<T> = std::result::Result<T, MediaError>;
