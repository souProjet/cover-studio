//! Known models, where they come from and where they are cached.

use crate::mdx::MdxParams;
use crate::{MlError, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelKind {
    /// MDX-Net separation; `primary` is the stem the network predicts.
    Mdx { params: MdxParams },
    /// RMVPE pitch estimator.
    Rmvpe,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size_bytes: u64,
    pub license: &'static str,
    pub source: &'static str,
    pub kind: ModelKind,
}

const UVR_RELEASES: &str = "https://github.com/TRvlvr/model_repo";

pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: "mdx-voc-ft",
        name: "MDX-Net Voc FT",
        description: "Vocal / accompaniment separation. Clean vocals, good default for covers.",
        file: "UVR-MDX-NET-Voc_FT.onnx",
        url: "https://github.com/TRvlvr/model_repo/releases/download/all_public_uvr_models/UVR-MDX-NET-Voc_FT.onnx",
        sha256: "534b2070fcc7df514b13ef660dc8cbb328679c2374d04354a5c42bb14ecce111",
        size_bytes: 66_762_490,
        license: "Published by the Ultimate Vocal Remover project; see upstream for terms",
        source: UVR_RELEASES,
        kind: ModelKind::Mdx {
            params: MdxParams {
                n_fft: 7680,
                hop: 1024,
                dim_f: 3072,
                dim_t_log2: 8,
                compensate: 1.021,
                primary_is_vocals: true,
            },
        },
    },
    ModelSpec {
        id: "mdx-inst-hq3",
        name: "MDX-Net Inst HQ 3",
        description: "Instrument-focused separation. Keeps more guitar/piano detail in the accompaniment.",
        file: "UVR-MDX-NET-Inst_HQ_3.onnx",
        url: "https://github.com/TRvlvr/model_repo/releases/download/all_public_uvr_models/UVR-MDX-NET-Inst_HQ_3.onnx",
        sha256: "317554b07fe1ea5279a77f2b1520a41ea4b93432560c4ffd08792c30fddf9adc",
        size_bytes: 66_759_214,
        license: "Published by the Ultimate Vocal Remover project; see upstream for terms",
        source: UVR_RELEASES,
        kind: ModelKind::Mdx {
            params: MdxParams {
                n_fft: 6144,
                hop: 1024,
                dim_f: 3072,
                dim_t_log2: 8,
                compensate: 1.022,
                primary_is_vocals: false,
            },
        },
    },
    ModelSpec {
        id: "rmvpe",
        name: "RMVPE",
        description: "Robust vocal pitch estimation, used for tuning and key detection.",
        file: "rmvpe.onnx",
        url: "https://huggingface.co/lj1995/VoiceConversionWebUI/resolve/main/rmvpe.onnx",
        sha256: "5370e71ac80af8b4b7c793d27efd51fd8bf962de3a7ede0766dac0befa3660fd",
        size_bytes: 361_688_443,
        license: "Published by the RVC project (MIT-licensed repository); see upstream for terms",
        source: "https://huggingface.co/lj1995/VoiceConversionWebUI",
        kind: ModelKind::Rmvpe,
    },
];

pub fn spec(id: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|m| m.id == id)
}

pub fn models_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("COVER_STUDIO_MODELS") {
        return PathBuf::from(p);
    }
    directories::ProjectDirs::from("", "", "cover-studio")
        .map(|d| d.cache_dir().join("models"))
        .unwrap_or_else(|| std::env::temp_dir().join("cover-studio-models"))
}

pub fn model_path(spec: &ModelSpec) -> PathBuf {
    models_dir().join(spec.file)
}

pub fn is_downloaded(spec: &ModelSpec) -> bool {
    model_path(spec)
        .metadata()
        .map(|m| m.len() == spec.size_bytes)
        .unwrap_or(false)
}

/// Downloads the model if needed and verifies its SHA-256.
pub fn ensure_model(
    spec: &ModelSpec,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<PathBuf> {
    let path = model_path(spec);
    if is_downloaded(spec) {
        return Ok(path);
    }
    std::fs::create_dir_all(models_dir())?;
    let part = path.with_extension("onnx.part");
    let err = |message: String| MlError::Download {
        model: spec.id.to_string(),
        message,
    };
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("cover-studio/", env!("CARGO_PKG_VERSION")))
        .timeout(None)
        .build()
        .map_err(|e| err(e.to_string()))?;
    let mut resp = client
        .get(spec.url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| err(e.to_string()))?;
    let total = resp.content_length().unwrap_or(spec.size_bytes);
    let mut file = std::fs::File::create(&part)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = std::fs::remove_file(&part);
            return Err(MlError::Cancelled);
        }
        let n = resp.read(&mut buf).map_err(|e| err(e.to_string()))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        done += n as u64;
        progress(done, total);
    }
    file.flush()?;
    drop(file);
    let digest = hex::encode(hasher.finalize());
    if digest != spec.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(MlError::Checksum(spec.id.to_string()));
    }
    std::fs::rename(&part, &path)?;
    Ok(path)
}
