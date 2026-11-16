use crate::{MediaError, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// Resolved paths to the ffmpeg binaries.
#[derive(Debug, Clone)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

fn find(name: &'static str, env_var: &str) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(env_var).map(PathBuf::from) {
        if p.is_file() {
            return Some(p);
        }
    }
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    // Tauri sidecars are installed next to the application binary.
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        let p = dir.join(&exe);
        if p.is_file() {
            return Some(p);
        }
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(&exe))
            .find(|p| p.is_file())
    })
}

impl Tools {
    pub fn locate() -> Result<&'static Tools> {
        static TOOLS: OnceLock<Option<Tools>> = OnceLock::new();
        let t = TOOLS.get_or_init(|| {
            Some(Tools {
                ffmpeg: find("ffmpeg", "COVER_STUDIO_FFMPEG")?,
                ffprobe: find("ffprobe", "COVER_STUDIO_FFPROBE")?,
            })
        });
        t.as_ref().ok_or(MediaError::ToolMissing("ffmpeg/ffprobe"))
    }

    pub fn ffmpeg(&self) -> Command {
        let mut c = Command::new(&self.ffmpeg);
        c.args(["-hide_banner", "-nostdin", "-y"]);
        c
    }

    pub fn ffprobe(&self) -> Command {
        Command::new(&self.ffprobe)
    }

    pub fn version(&self) -> Option<String> {
        let out = Command::new(&self.ffmpeg).arg("-version").output().ok()?;
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .map(str::to_string)
    }
}

/// Runs a command and maps a non-zero exit to `ToolFailed` with stderr tail.
pub(crate) fn run(mut cmd: Command, tool: &'static str) -> Result<std::process::Output> {
    let out = cmd.stdin(Stdio::null()).output()?;
    if !out.status.success() {
        return Err(MediaError::ToolFailed {
            tool,
            message: stderr_tail(&out.stderr),
        });
    }
    Ok(out)
}

pub(crate) fn stderr_tail(stderr: &[u8]) -> String {
    let s = String::from_utf8_lossy(stderr);
    let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(6)..].join("\n")
}

/// True when NVENC can actually encode on this machine (not just compiled in).
pub fn gpu_encoder_available() -> bool {
    static NVENC: OnceLock<bool> = OnceLock::new();
    *NVENC.get_or_init(|| {
        let Ok(t) = Tools::locate() else {
            return false;
        };
        let mut cmd = t.ffmpeg();
        cmd.args([
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=256x256:d=0.1",
            "-c:v",
            "h264_nvenc",
            "-f",
            "null",
            "-",
        ]);
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        cmd.status().map(|s| s.success()).unwrap_or(false)
    })
}

/// True when the ffmpeg build has the zimg-based filters needed for HDR tonemapping.
pub fn has_zscale() -> bool {
    static Z: OnceLock<bool> = OnceLock::new();
    *Z.get_or_init(|| {
        let Ok(t) = Tools::locate() else {
            return false;
        };
        let Ok(out) = t.ffmpeg().args(["-filters"]).output() else {
            return false;
        };
        let s = String::from_utf8_lossy(&out.stdout);
        s.contains(" zscale ") && s.contains(" tonemap ")
    })
}
