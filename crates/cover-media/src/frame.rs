use crate::encode::{frame_filter, Aspect};
use crate::probe::probe;
use crate::tools::{run, Tools};
use crate::{MediaError, Result};
use std::path::Path;

/// Grabs one frame at `t_sec`, reframed like the video export.
pub fn extract_frame(video: &Path, t_sec: f64, aspect: Aspect, out: &Path) -> Result<()> {
    let tools = Tools::locate()?;
    let v = probe(video)?.video.ok_or_else(|| {
        MediaError::Unsupported(format!("{} has no video stream", video.display()))
    })?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut cmd = tools.ffmpeg();
    cmd.args(["-loglevel", "error", "-ss"])
        .arg(format!("{:.3}", t_sec.max(0.0)))
        .arg("-i")
        .arg(video)
        .arg("-vf")
        .arg(format!(
            "{},format=rgb24",
            frame_filter(aspect, v.width, v.height, v.hdr)
        ))
        .args(["-frames:v", "1", "-q:v", "2"])
        .arg(out);
    run(cmd, "ffmpeg")?;
    Ok(())
}

/// Source-ratio frame scaled down for UI previews.
pub fn extract_preview(video: &Path, t_sec: f64, max_width: u32, out: &Path) -> Result<()> {
    let tools = Tools::locate()?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut cmd = tools.ffmpeg();
    cmd.args(["-loglevel", "error", "-ss"])
        .arg(format!("{:.3}", t_sec.max(0.0)))
        .arg("-i")
        .arg(video)
        .arg("-vf")
        .arg(format!("scale='min({max_width},iw)':-2"))
        .args(["-frames:v", "1", "-q:v", "4"])
        .arg(out);
    run(cmd, "ffmpeg")?;
    Ok(())
}
