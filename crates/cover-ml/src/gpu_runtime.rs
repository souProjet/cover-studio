//! Optional CUDA 13 + cuDNN 9 runtime, installed per-user from NVIDIA's
//! official PyPI wheels so the GPU path works without a system CUDA install.

use crate::{MlError, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

struct Wheel {
    package: &'static str,
    url: &'static str,
    sha256: &'static str,
    size: u64,
}

#[cfg(target_os = "linux")]
const WHEELS: &[Wheel] = &[
    Wheel { package: "nvidia-cuda-runtime 13.5.37", url: "https://files.pythonhosted.org/packages/0b/7c/14b8f5518f8951e9dca57ea80f44f8ea8bcc8671812e6cc38009f3205e57/nvidia_cuda_runtime-13.5.37-py3-none-manylinux2014_x86_64.manylinux_2_17_x86_64.whl", sha256: "df2b79a8852f10d1fef8c848489c49eebb5754051bf870202a862831412b179d", size: 2_504_024 },
    Wheel { package: "nvidia-nvjitlink 13.5.37", url: "https://files.pythonhosted.org/packages/cc/1f/2321702b8d60b4b4dd18b957f61612dbac9cb1c07e940cf6582106b7c8b8/nvidia_nvjitlink-13.5.37-py3-none-manylinux2010_x86_64.manylinux_2_12_x86_64.whl", sha256: "280d7cb9649704145f074b8dbde500de04ce02d0872e066e20a83c17bcaa93eb", size: 46_642_491 },
    Wheel { package: "nvidia-cuda-nvrtc 13.5.37", url: "https://files.pythonhosted.org/packages/a9/54/db4e7f650746265f61c05cf1f8f556dd8c6739b58c823fbc9a7d6fda3247/nvidia_cuda_nvrtc-13.5.37-py3-none-manylinux2010_x86_64.manylinux_2_12_x86_64.whl", sha256: "86b4d68058807988b7d9af5b44917a0601e9c337f35ccd657bfb3c759ff8d8bc", size: 58_185_930 },
    Wheel { package: "nvidia-cublas 13.10.0.20", url: "https://files.pythonhosted.org/packages/b4/12/16628161ed8c109c8c73b38167df50cbc630a8a6e6548e2534d461526c3c/nvidia_cublas-13.10.0.20-py3-none-manylinux1_x86_64.manylinux_2_5_x86_64.whl", sha256: "dcc6d63884c53d07e267ce7e6feac17b2538cebaafec5db865736c3e28e4dc84", size: 444_289_903 },
    Wheel { package: "nvidia-curand 10.4.5.25", url: "https://files.pythonhosted.org/packages/12/63/636e5dc9afb2959bfc0042a357d5567358df829b66c96d9fc36d77dc0596/nvidia_curand-10.4.5.25-py3-none-manylinux_2_27_x86_64.whl", sha256: "22bb226a857e5888fe6b54ae465fc4371296bcff13a35f79e19691bcf454f529", size: 77_113_015 },
    Wheel { package: "nvidia-cufft 12.5.0.14", url: "https://files.pythonhosted.org/packages/c6/8f/f909edaadf9a7acf1b92f805a384b6984193bd595b45005f8e320d4170cf/nvidia_cufft-12.5.0.14-py3-none-manylinux2014_x86_64.manylinux_2_17_x86_64.whl", sha256: "b8be26837a5abad2b5b74f3e92dd29f622a42934b355be0961a48712cbba23ad", size: 165_072_555 },
    Wheel { package: "nvidia-cudnn-cu13 9.28.1.4", url: "https://files.pythonhosted.org/packages/ef/6f/94535c450d44a68ef0b43d7a5e54c0e5ecda2ca3db7daec5d54c8d94e0d8/nvidia_cudnn_cu13-9.28.1.4-py3-none-manylinux_2_27_x86_64.whl", sha256: "a54f205b1db1f2895d99f398754db1369816f0569dd459731f925635cf7b65ec", size: 516_817_937 },
];

#[cfg(target_os = "windows")]
const WHEELS: &[Wheel] = &[
    Wheel { package: "nvidia-cuda-runtime 13.5.37", url: "https://files.pythonhosted.org/packages/35/99/e07dd9565d2e0317c1a62ff39ae9fc0542c048f74bcfc777c8be97710347/nvidia_cuda_runtime-13.5.37-py3-none-win_amd64.whl", sha256: "f225d3fa5ea6bfbb58fc4979ef72375cbe661d9107f0d8a4ff22cc151525e4a9", size: 2_780_254 },
    Wheel { package: "nvidia-nvjitlink 13.5.37", url: "https://files.pythonhosted.org/packages/a7/53/c43398492e8374880d5e01c23c4dac33ba0c3bd03515d6ecc574254c0967/nvidia_nvjitlink-13.5.37-py3-none-win_amd64.whl", sha256: "4465d4e8f2212118220b580c82523c5654f93a62906bd51d29d79bdfc12376c1", size: 41_467_665 },
    Wheel { package: "nvidia-cuda-nvrtc 13.5.37", url: "https://files.pythonhosted.org/packages/9f/6c/241c5155199a6de55bad8862213ed64b487dc9526aae817e4af31afea1a4/nvidia_cuda_nvrtc-13.5.37-py3-none-win_amd64.whl", sha256: "e8e6c10b251385707ae3abd7b4ee118de6736fd7678d0f0f4eab4d759f679c21", size: 49_248_991 },
    Wheel { package: "nvidia-cublas 13.10.0.20", url: "https://files.pythonhosted.org/packages/12/39/e1b1b4adb8b171894d9a89035df0596d448cf73dc403802d24dbe30cc0be/nvidia_cublas-13.10.0.20-py3-none-win_amd64.whl", sha256: "c310b749eea855b88fafb906de08d6819b87216c6711640771764fea5dcd8546", size: 427_284_863 },
    Wheel { package: "nvidia-curand 10.4.5.25", url: "https://files.pythonhosted.org/packages/ee/b6/5b38490991b6875ae60c4f41e190ac91cadd1e6a1324b152999cc4a3f580/nvidia_curand-10.4.5.25-py3-none-win_amd64.whl", sha256: "f6a02a70f5b9715235df31a164a4bf3237a45be1600a5f435f5e4a0d8682eb22", size: 77_330_709 },
    Wheel { package: "nvidia-cufft 12.5.0.14", url: "https://files.pythonhosted.org/packages/94/79/8f2a7125f560b86571c0be3dbf10e33041187bc4a8cf66107541fe8e5cce/nvidia_cufft-12.5.0.14-py3-none-win_amd64.whl", sha256: "78e7585159ecf37d73af1ce43a5dea7fd3360e1af71cde08f070c07966a336bc", size: 164_476_208 },
    Wheel { package: "nvidia-cudnn-cu13 9.28.1.4", url: "https://files.pythonhosted.org/packages/48/9c/26e6bdd31c97c389aa565b49c916be0dec17dd61dad6cd33fee286d1b946/nvidia_cudnn_cu13-9.28.1.4-py3-none-win_amd64.whl", sha256: "73d874c6bd8a1c75ca040228f77c4e163efe9bab3b65bd85914e2b7334ac68ee", size: 407_065_099 },
];

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
const WHEELS: &[Wheel] = &[];

#[cfg(target_os = "windows")]
const NVJITLINK: &str = "nvJitLink_130_0.dll";
#[cfg(not(target_os = "windows"))]
const NVJITLINK: &str = "libnvJitLink.so.13";

#[derive(Debug, Clone, Serialize)]
pub struct GpuRuntimeStatus {
    pub supported: bool,
    pub installed: bool,
    pub dir: PathBuf,
    pub download_bytes: u64,
}

pub fn runtime_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("COVER_STUDIO_CUDA_DIR") {
        return PathBuf::from(p);
    }
    directories::ProjectDirs::from("", "", "cover-studio")
        .map(|d| d.data_dir().join("gpu-runtime"))
        .unwrap_or_else(|| std::env::temp_dir().join("cover-studio-gpu-runtime"))
}

fn marker() -> PathBuf {
    runtime_dir().join(".complete")
}

pub fn status() -> GpuRuntimeStatus {
    GpuRuntimeStatus {
        supported: !WHEELS.is_empty(),
        installed: marker().is_file(),
        dir: runtime_dir(),
        download_bytes: WHEELS.iter().map(|w| w.size).sum(),
    }
}

fn is_runtime_lib(name: &str) -> bool {
    let file = name.rsplit('/').next().unwrap_or(name);
    if cfg!(windows) {
        file.ends_with(".dll")
    } else {
        file.starts_with("lib") && file.contains(".so")
    }
}

fn extract(wheel: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(wheel)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| MlError::Download {
        model: "gpu-runtime".into(),
        message: e.to_string(),
    })?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| MlError::Download {
            model: "gpu-runtime".into(),
            message: e.to_string(),
        })?;
        let Ok(name) = entry.name().map(|n| n.to_string()) else {
            continue;
        };
        if entry.is_dir() || !is_runtime_lib(&name) {
            continue;
        }
        let file_name = name.rsplit('/').next().unwrap_or(&name);
        let mut out = std::fs::File::create(dest.join(file_name))?;
        std::io::copy(&mut entry, &mut out)?;
    }
    Ok(())
}

/// Downloads and unpacks the runtime. `progress(done_bytes, total_bytes, package)`.
pub fn install(cancel: &AtomicBool, progress: &mut dyn FnMut(u64, u64, &str)) -> Result<PathBuf> {
    let dir = runtime_dir();
    if marker().is_file() {
        return Ok(dir);
    }
    std::fs::create_dir_all(&dir)?;
    let total: u64 = WHEELS.iter().map(|w| w.size).sum();
    let mut done = 0u64;
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("cover-studio/", env!("CARGO_PKG_VERSION")))
        .timeout(None)
        .build()
        .map_err(|e| MlError::Download {
            model: "gpu-runtime".into(),
            message: e.to_string(),
        })?;
    for w in WHEELS {
        let err = |message: String| MlError::Download {
            model: w.package.to_string(),
            message,
        };
        let tmp = dir.join("download.whl.part");
        let mut resp = client
            .get(w.url)
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| err(e.to_string()))?;
        let mut file = std::fs::File::create(&tmp)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            if cancel.load(Ordering::Relaxed) {
                drop(file);
                let _ = std::fs::remove_file(&tmp);
                return Err(MlError::Cancelled);
            }
            let n = resp.read(&mut buf).map_err(|e| err(e.to_string()))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            hasher.update(&buf[..n]);
            done += n as u64;
            progress(done, total, w.package);
        }
        drop(file);
        if hex::encode(hasher.finalize()) != w.sha256 {
            let _ = std::fs::remove_file(&tmp);
            return Err(MlError::Checksum(w.package.to_string()));
        }
        extract(&tmp, &dir)?;
        std::fs::remove_file(&tmp)?;
    }
    std::fs::write(marker(), b"ok")?;
    Ok(dir)
}

/// Loads the per-user runtime into the process so the CUDA execution provider
/// resolves its dependencies from it. Safe to call many times.
pub fn preload() -> std::result::Result<(), String> {
    static RESULT: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    RESULT
        .get_or_init(|| {
            let dir = runtime_dir();
            if !marker().is_file() {
                return Err("GPU runtime not installed".into());
            }
            ort::util::preload_dylib(dir.join(NVJITLINK)).map_err(|e| e.to_string())?;
            ort::ep::cuda::preload_dylibs(Some(&dir), Some(&dir)).map_err(|e| e.to_string())
        })
        .clone()
}
