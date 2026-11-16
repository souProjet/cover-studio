// In-browser stand-in for the Tauri backend so the UI can be developed with
// `pnpm ui:dev` in a regular browser. Never used inside the desktop app.

import type { JobEvent, Project, ProjectSummary, Settings, Stage, SystemInfo, Waveform } from "./types";

const listeners = new Set<(e: JobEvent) => void>();

function waveform(duration: number, seed: number, gaps: [number, number][]): Waveform {
  const n = 2400;
  const peaks = Array.from({ length: n }, (_, i) => {
    const t = (i / n) * duration;
    if (gaps.some(([a, b]) => t > a && t < b)) return 0.004 + 0.003 * Math.abs(Math.sin(i * 7.1));
    const phrase = 0.55 + 0.35 * Math.sin(t * 0.21 + seed) * Math.sin(t * 1.7);
    const grain = 0.25 * Math.abs(Math.sin(i * 12.9898 + seed) * 43758.5453 % 1);
    return Math.min(1, Math.max(0.02, phrase * 0.7 + grain));
  });
  return { duration_sec: duration, peaks };
}

const DURATION = 254.2;
const GAPS: [number, number][] = [[0, 16.5], [121.5, 129.5], [166.5, 168.2], [210, 211.8]];

const defaultSettings: Settings = {
  separation_model: "mdx-voc-ft",
  denoise_separation: false,
  device: "auto",
  preset: "balanced",
  instrument: "acoustic_guitar",
  tuning: {
    enabled: true,
    strength: 0.85,
    drift: 0.35,
    retune_speed: 0.35,
    vibrato: 0.2,
    preserve_formants: true,
    scale: { type: "auto" },
  },
  vocal: {
    blend: 0.8,
    presence_db: 1.5,
    air_db: 1,
    de_ess: 0.55,
    compression: 0.5,
    leveler: 0.55,
    breath: 0.4,
    cleanup: 0.45,
    harshness: 0.5,
    reverb: 0.18,
  },
  instrument_blend: 0.6,
  warmth_db: 0.8,
  room: 0.22,
  loudness: { lufs: -14, ceiling_dbtp: -1 },
  range: { take: null, start_sec: null, end_sec: null },
  sync_offset_sec: null,
  video: { aspect: "vertical", style: "social", quality: "high", fade_in_sec: 0.4, fade_out_sec: 1.2 },
  social: { title: "Mockingbird", artist: "Eminem", language: "fr", thumbnail_at_sec: null, extra_hashtags: [] },
};

function demoProject(): Project {
  return {
    id: "demo",
    name: "Mockingbird",
    created_at: new Date(Date.now() - 3600e3).toISOString(),
    updated_at: new Date().toISOString(),
    sources: { audio: "/demo/session.wav", video: "/demo/session.mp4" },
    media: {
      audio: { duration_sec: DURATION, video: null, audio: { codec: "pcm_s16le", sample_rate: 44100, channels: 1 } },
      video: {
        duration_sec: 263,
        video: { codec: "hevc", width: 2160, height: 3840, fps: 60, rotation: 90, pix_fmt: "yuv420p10le", hdr: "hlg" },
        audio: { codec: "aac", sample_rate: 48000, channels: 2 },
      },
    },
    analysis: {
      fingerprint: "demo",
      duration_sec: DURATION,
      sample_rate: 44100,
      sync: {
        offset_sec: 6.153,
        confidence: 1,
        correlation: 0.949,
        method: "clap_and_correlation",
        clap_video_sec: 6.687,
        clap_audio_sec: 0.534,
      },
      takes: [
        { index: 0, start_sec: 16.95, end_sec: 121.15, vocal_start_sec: 17.37, tuning_cents: 40, clipping: 0, score: 0.6 },
        { index: 1, start_sec: 129.7, end_sec: 166.25, vocal_start_sec: 130.09, tuning_cents: 38, clipping: 0, score: 0.3 },
        { index: 2, start_sec: 168.4, end_sec: 209.7, vocal_start_sec: 169.1, tuning_cents: 39, clipping: 0, score: 0.36 },
        { index: 3, start_sec: 211.9, end_sec: 253.7, vocal_start_sec: 212.4, tuning_cents: 38, clipping: 0, score: 0.4 },
      ],
      best_take: 0,
      key: { tonic: 8, mode: "minor", confidence: 0.42 },
      original_loudness: { integrated_lufs: -24.6, true_peak_dbtp: -1.9 },
      waveform: waveform(DURATION, 1, GAPS),
      vocal_waveform: waveform(DURATION, 4, GAPS),
      provider: "CUDA",
    },
    settings: structuredClone(defaultSettings),
    last_render: null,
    exports: [],
  };
}

let project: Project | null = null;
let demoVideoProxy = false;
const getProject = () => (project ??= demoProject());

const system: SystemInfo = {
  version: "0.1.0",
  ffmpeg: "ffmpeg version 7.1",
  ffmpeg_error: null,
  nvenc: true,
  gpu_runtime: { supported: true, installed: false, dir: "~/.local/share/cover-studio/gpu-runtime", download_bytes: 1_310_000_000 },
  models: [
    {
      spec: {
        id: "mdx-voc-ft",
        name: "MDX-Net Voc FT",
        description: "Vocal / accompaniment separation. Clean vocals, good default for covers.",
        file: "UVR-MDX-NET-Voc_FT.onnx",
        url: "",
        size_bytes: 66_762_490,
        license: "Published by the Ultimate Vocal Remover project; see upstream for terms",
        source: "https://github.com/TRvlvr/model_repo",
      },
      downloaded: true,
      path: "~/.cache/cover-studio/models/UVR-MDX-NET-Voc_FT.onnx",
    },
    {
      spec: {
        id: "mdx-inst-hq3",
        name: "MDX-Net Inst HQ 3",
        description: "Instrument-first separation. Keeps more of the instrument for full-band covers.",
        file: "UVR-MDX-NET-Inst_HQ_3.onnx",
        url: "",
        size_bytes: 66_759_214,
        license: "Published by the Ultimate Vocal Remover project; see upstream for terms",
        source: "https://github.com/TRvlvr/model_repo",
      },
      downloaded: false,
      path: "~/.cache/cover-studio/models/UVR-MDX-NET-Inst_HQ_3.onnx",
    },
    {
      spec: {
        id: "rmvpe",
        name: "RMVPE",
        description: "Sung pitch tracking used for tuning and key detection.",
        file: "rmvpe.onnx",
        url: "",
        size_bytes: 361_688_443,
        license: "Published by the RVC project; see upstream for terms",
        source: "https://huggingface.co/lj1995/VoiceConversionWebUI",
      },
      downloaded: true,
      path: "~/.cache/cover-studio/models/rmvpe.onnx",
    },
  ],
  models_dir: "~/.cache/cover-studio/models",
  projects_dir: "~/.local/share/cover-studio/projects",
};

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function simulate(job: string, steps: [Stage, string][], ms = 900) {
  for (const [stage, label] of steps) {
    listeners.forEach((l) => l({ job, type: "stage", stage, label }));
    for (let i = 1; i <= 10; i++) {
      await sleep(ms / 10);
      listeners.forEach((l) => l({ job, type: "progress", stage, fraction: i / 10 }));
    }
  }
}

let toneUrl: string | null = null;

/** Short generated WAV so the A/B player has something to play. */
function tone(): string {
  if (toneUrl) return toneUrl;
  const sr = 22050;
  const n = sr * 8;
  const buf = new ArrayBuffer(44 + n * 2);
  const v = new DataView(buf);
  const w = (o: number, s: string) => [...s].forEach((c, i) => v.setUint8(o + i, c.charCodeAt(0)));
  w(0, "RIFF");
  v.setUint32(4, 36 + n * 2, true);
  w(8, "WAVEfmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true);
  v.setUint16(22, 1, true);
  v.setUint32(24, sr, true);
  v.setUint32(28, sr * 2, true);
  v.setUint16(32, 2, true);
  v.setUint16(34, 16, true);
  w(36, "data");
  v.setUint32(40, n * 2, true);
  for (let i = 0; i < n; i++) {
    const t = i / sr;
    const s = 0.2 * Math.sin(2 * Math.PI * 220 * t) * Math.exp(-((t % 1) * 3));
    v.setInt16(44 + i * 2, s * 32767, true);
  }
  toneUrl = URL.createObjectURL(new Blob([buf], { type: "audio/wav" }));
  return toneUrl;
}

export function mockFileUrl(path: string): string {
  if (path.endsWith(".wav")) return tone();
  return "";
}

export function mockListen(handler: (e: JobEvent) => void) {
  listeners.add(handler);
  return () => listeners.delete(handler);
}

export async function mockInvoke<T>(cmd: string, args: Record<string, unknown>): Promise<T> {
  const p = getProject();
  const job = String(args.job ?? "");
  switch (cmd) {
    case "system_info":
      return structuredClone(system) as T;
    case "list_projects": {
      const s: ProjectSummary = {
        id: p.id,
        name: p.name,
        updated_at: p.updated_at,
        has_video: true,
        analyzed: !!p.analysis,
        duration_sec: DURATION,
        key: { tonic: 8, mode: "minor", confidence: 0.42 },
      };
      return [s] as T;
    }
    case "create_project":
    case "get_project":
      return structuredClone(p) as T;
    case "update_settings":
      p.settings = structuredClone(args.settings as Settings);
      return structuredClone(p) as T;
    case "rename_project":
      p.name = String(args.name);
      return structuredClone(p) as T;
    case "caption_preview":
      return `${p.settings.social.title} (${p.settings.social.artist}) - reprise guitare-voix.\n\nEnregistré à la maison, en une prise.\n\n#cover #reprise` as T;
    case "default_output_dir":
      return "~/Musique/Cover Studio/mockingbird" as T;
    case "video_frame":
      return "" as T;
    case "video_proxy":
      return (demoVideoProxy ? "/demo/proxy.mp4" : null) as T;
    case "prepare_video_proxy":
      await simulate(job, [["video", "Preparing the video preview"]], 1200);
      demoVideoProxy = true;
      return "/demo/proxy.mp4" as T;
    case "analyze":
      await simulate(job, [
        ["separate", "Separating voice and instrument"],
        ["pitch", "Tracking the sung melody"],
        ["analyze", "Finding takes and key"],
        ["sync", "Synchronising audio and video"],
      ]);
      return structuredClone(p) as T;
    case "render_preview": {
      await simulate(job, [["render", "Processing voice and instrument"]], 1400);
      const take = p.analysis!.takes[p.settings.range.take ?? p.analysis!.best_take ?? 0];
      const start = p.settings.range.start_sec ?? take.start_sec;
      const end = p.settings.range.end_sec ?? take.end_sec;
      p.last_render = {
        at: new Date().toISOString(),
        fingerprint: "demo",
        start_sec: start,
        end_sec: end,
        master: "/demo/master.wav",
        processed: "/demo/preview.wav",
        original: "/demo/original.wav",
        stats: { loudness: { integrated_lufs: -14.2, true_peak_dbtp: -1.1 }, notes_corrected: 115, mean_correction_cents: 22 },
        notes: Array.from({ length: 60 }, (_, i) => {
          const t0 = start + ((end - start) * i) / 60;
          const target = 56 + [0, 3, 5, 7, 5, 3, 0, -2][i % 8];
          const sung = target + Math.sin(i * 2.3) * 0.35;
          return { start_sec: t0, end_sec: t0 + (end - start) / 75, sung_midi: sung, target_midi: target, shift: (target - sung) * 0.8 };
        }),
        waveform: waveform(end - start, 2, []),
      };
      return structuredClone(p) as T;
    }
    case "export":
      await simulate(job, [
        ["export", "Writing audio"],
        ["video", "Rendering the video"],
        ["social", "Creating the thumbnail"],
      ]);
      return {
        at: new Date().toISOString(),
        dir: "~/Musique/Cover Studio/mockingbird",
        files: [
          { kind: "wav", path: "~/Musique/Cover Studio/mockingbird/mockingbird.wav" },
          { kind: "video", path: "~/Musique/Cover Studio/mockingbird/mockingbird.mp4" },
          { kind: "thumbnail", path: "~/Musique/Cover Studio/mockingbird/mockingbird.jpg" },
          { kind: "caption", path: "~/Musique/Cover Studio/mockingbird/mockingbird.txt" },
        ],
      } as T;
    case "download_model":
    case "install_gpu_runtime":
      await simulate(job, [["download", "Downloading"]], 2500);
      return "" as T;
    case "delete_project":
      project = null;
      return undefined as T;
    case "cancel_job":
    case "probe_media":
      return undefined as T;
    default:
      throw { message: `mock: unknown command ${cmd}`, cancelled: false };
  }
}
