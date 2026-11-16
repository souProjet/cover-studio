//! Audio DSP building blocks used by the Cover Studio engine.
//!
//! Everything here is plain Rust operating on `f32` sample buffers, with no
//! machine learning and no I/O, so it is easy to test and reuse.

pub mod analysis;
pub mod buffer;
pub mod dynamics;
pub mod filter;
pub mod loudness;
pub mod mel;
pub mod pitch_shift;
pub mod resample;
pub mod reverb;
pub mod stft;
pub mod util;

pub use buffer::AudioBuffer;

#[derive(Debug, thiserror::Error)]
pub enum DspError {
    #[error("loudness measurement failed: {0}")]
    Loudness(String),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}

pub type Result<T> = std::result::Result<T, DspError>;
