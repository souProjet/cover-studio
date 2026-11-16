//! Cover Studio processing pipeline.
//!
//! [`Engine`] owns the projects folder and runs the long operations
//! (analysis, preview render, export). Every long operation takes a [`Ctx`]
//! for progress events and cancellation, and is synchronous: callers run it
//! on a worker thread.

pub mod analysis;
pub mod chain;
pub mod engine;
pub mod error;
pub mod events;
pub mod project;
pub mod render;
pub mod settings;
pub mod social;

pub use engine::{caption, scale_for, Engine, ExportRequest, ModelStatus, SystemInfo};
pub use error::{EngineError, Result};
pub use events::{Ctx, Event, Stage};
pub use project::{Project, ProjectSummary, Sources};
pub use settings::Settings;

pub use cover_media as media;
pub use cover_ml as ml;
