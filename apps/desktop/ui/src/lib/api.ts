import type {
  ExportRecord,
  ExportRequest,
  JobEvent,
  MediaInfo,
  Project,
  ProjectSummary,
  Settings,
  SystemInfo,
} from "./types";

export const inTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri()) {
    const { mockInvoke } = await import("./mock");
    return mockInvoke<T>(cmd, args ?? {});
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

/** URL the webview can load for a local file (audio previews, frames). */
export async function fileUrl(path: string): Promise<string> {
  if (!inTauri()) {
    const { mockFileUrl } = await import("./mock");
    return mockFileUrl(path);
  }
  const { convertFileSrc } = await import("@tauri-apps/api/core");
  return convertFileSrc(path);
}

/** Raw bytes of a local file, bypassing `<audio>` (GStreamer cannot read `asset://` on Linux). */
export async function fileBytes(path: string, version: string): Promise<ArrayBuffer> {
  const url = await fileUrl(path);
  const sep = url.startsWith("blob:") || url.startsWith("data:") ? null : url.includes("?") ? "&" : "?";
  const res = await fetch(sep ? `${url}${sep}v=${encodeURIComponent(version)}` : url, { cache: "no-store" });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.arrayBuffer();
}

export function newJobId() {
  return Math.random().toString(36).slice(2, 10);
}

export const api = {
  systemInfo: () => invoke<SystemInfo>("system_info"),
  listProjects: () => invoke<ProjectSummary[]>("list_projects"),
  probeMedia: (path: string) => invoke<MediaInfo>("probe_media", { path }),
  createProject: (name: string | null, audio: string | null, video: string | null) =>
    invoke<Project>("create_project", { name, audio, video }),
  getProject: (id: string) => invoke<Project>("get_project", { id }),
  updateSettings: (id: string, settings: Settings) => invoke<Project>("update_settings", { id, settings }),
  renameProject: (id: string, name: string) => invoke<Project>("rename_project", { id, name }),
  deleteProject: (id: string) => invoke<void>("delete_project", { id }),
  videoFrame: (id: string, audioSec: number, maxWidth: number) =>
    invoke<string>("video_frame", { id, audioSec, maxWidth }),
  defaultOutputDir: (id: string) => invoke<string>("default_output_dir", { id }),
  captionPreview: (id: string) => invoke<string>("caption_preview", { id }),
  analyze: (id: string, job: string) => invoke<Project>("analyze", { id, job }),
  renderPreview: (id: string, job: string) => invoke<Project>("render_preview", { id, job }),
  export: (id: string, request: ExportRequest, job: string) =>
    invoke<ExportRecord>("export", { id, request, job }),
  downloadModel: (model: string, job: string) => invoke<string>("download_model", { model, job }),
  installGpuRuntime: (job: string) => invoke<string>("install_gpu_runtime", { job }),
  cancelJob: (job: string) => invoke<void>("cancel_job", { job }),
};

export async function onJobEvent(handler: (e: JobEvent) => void): Promise<() => void> {
  if (!inTauri()) {
    const { mockListen } = await import("./mock");
    return mockListen(handler);
  }
  const { listen } = await import("@tauri-apps/api/event");
  return listen<JobEvent>("job", (e) => handler(e.payload));
}

/** Files dropped on the window (Tauri gives real paths, browsers do not). */
export async function onFileDrop(handler: (paths: string[]) => void, hover: (over: boolean) => void) {
  if (!inTauri()) return () => {};
  const { getCurrentWebview } = await import("@tauri-apps/api/webview");
  return getCurrentWebview().onDragDropEvent((e) => {
    if (e.payload.type === "enter" || e.payload.type === "over") hover(true);
    else if (e.payload.type === "leave") hover(false);
    else if (e.payload.type === "drop") {
      hover(false);
      handler(e.payload.paths);
    }
  });
}

export const MEDIA_EXTENSIONS = {
  video: ["mp4", "mov", "m4v", "mkv", "webm", "avi", "3gp"],
  audio: ["wav", "flac", "mp3", "m4a", "aac", "ogg", "opus", "aiff", "aif", "wma"],
};

export function mediaKind(path: string): "video" | "audio" | null {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  if (MEDIA_EXTENSIONS.video.includes(ext)) return "video";
  if (MEDIA_EXTENSIONS.audio.includes(ext)) return "audio";
  return null;
}

export async function pickFile(kind: "video" | "audio"): Promise<string | null> {
  if (!inTauri()) return kind === "video" ? "/demo/session.mp4" : "/demo/session.wav";
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({
    multiple: false,
    filters: [{ name: kind === "video" ? "Video" : "Audio", extensions: MEDIA_EXTENSIONS[kind] }],
  });
  return typeof r === "string" ? r : null;
}

export async function pickFolder(): Promise<string | null> {
  if (!inTauri()) return "/demo/exports";
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ directory: true, multiple: false });
  return typeof r === "string" ? r : null;
}

export async function revealFile(path: string) {
  if (!inTauri()) return;
  const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
  await revealItemInDir(path);
}

export async function openExternal(url: string) {
  if (!inTauri()) return void window.open(url, "_blank", "noopener");
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  await openUrl(url);
}
