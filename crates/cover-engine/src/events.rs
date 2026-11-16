//! Progress reporting shared by the CLI and the desktop app.

use crate::error::{EngineError, Result};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Prepare,
    Download,
    Separate,
    Pitch,
    Analyze,
    Sync,
    Render,
    Video,
    Social,
    Export,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Stage { stage: Stage, label: String },
    Progress { stage: Stage, fraction: f32 },
    Log { message: String },
}

/// Cancellation flag plus event sink for one long-running operation.
pub struct Ctx<'a> {
    pub cancel: &'a AtomicBool,
    pub emit: &'a (dyn Fn(Event) + Sync),
}

impl Ctx<'_> {
    pub fn stage(&self, stage: Stage, label: impl Into<String>) {
        let label = label.into();
        tracing::info!(?stage, "{label}");
        (self.emit)(Event::Stage { stage, label });
    }

    pub fn progress(&self, stage: Stage, fraction: f32) {
        (self.emit)(Event::Progress {
            stage,
            fraction: fraction.clamp(0.0, 1.0),
        });
    }

    pub fn log(&self, message: impl Into<String>) {
        let message = message.into();
        tracing::info!("{message}");
        (self.emit)(Event::Log { message });
    }

    pub fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(EngineError::Cancelled)
        } else {
            Ok(())
        }
    }
}
