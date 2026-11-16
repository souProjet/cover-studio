// Mirrors the serde types of cover-engine. Keep in sync with crates/cover-engine.

export type HdrTransfer = "hlg" | "pq";

export interface MediaInfo {
  duration_sec: number;
  video: {
    codec: string;
    width: number;
    height: number;
    fps: number;
    rotation: number;
    pix_fmt: string;
    hdr: HdrTransfer | null;
  } | null;
  audio: { codec: string; sample_rate: number; channels: number } | null;
}

export interface Sources {
  audio: string | null;
  video: string | null;
}

export type SyncMethod = "correlation" | "clap_and_correlation" | "clap";

export interface SyncResult {
  offset_sec: number;
  confidence: number;
  correlation: number;
  method: SyncMethod;
  clap_video_sec: number | null;
  clap_audio_sec: number | null;
}

export interface Take {
  index: number;
  start_sec: number;
  end_sec: number;
  vocal_start_sec: number | null;
  tuning_cents: number | null;
  clipping: number;
  score: number;
}

export type Mode = "major" | "minor";

export interface Key {
  tonic: number;
  mode: Mode;
  confidence: number;
}

export interface Loudness {
  integrated_lufs: number;
  true_peak_dbtp: number;
}

export interface Waveform {
  duration_sec: number;
  peaks: number[];
}

export interface Analysis {
  fingerprint: string;
  duration_sec: number;
  sample_rate: number;
  sync: SyncResult | null;
  takes: Take[];
  best_take: number | null;
  key: Key;
  original_loudness: Loudness;
  waveform: Waveform;
  vocal_waveform: Waveform;
  provider: string;
}

export interface Note {
  start_sec: number;
  end_sec: number;
  sung_midi: number;
  target_midi: number;
  shift: number;
}

export interface RenderInfo {
  at: string;
  fingerprint: string;
  start_sec: number;
  end_sec: number;
  master: string;
  processed: string;
  original: string;
  stats: { loudness: Loudness; notes_corrected: number; mean_correction_cents: number };
  notes: Note[];
  waveform: Waveform;
}

export type ExportKind = "wav" | "audio" | "video" | "thumbnail" | "caption";

export interface ExportRecord {
  at: string;
  dir: string;
  files: { kind: ExportKind; path: string }[];
}

export type SoundPreset = "natural" | "balanced" | "polished";
export type InstrumentKind =
  | "acoustic_guitar"
  | "classical_guitar"
  | "electric_guitar"
  | "piano"
  | "ukulele"
  | "full_band"
  | "other";
export type ScaleMode = { type: "auto" } | { type: "chromatic" } | { type: "manual"; tonic: number; mode: Mode };
export type Device = "auto" | "cuda" | "cpu";
export type Aspect = "vertical" | "landscape" | "square" | "source";
export type VideoStyle = "natural" | "social" | "low_light";
export type VideoQuality = "draft" | "high" | "max";
export type Language = "fr" | "en";
export type AudioCodec = "aac" | "mp3" | "flac";

export interface VocalSettings {
  blend: number;
  presence_db: number;
  air_db: number;
  de_ess: number;
  compression: number;
  leveler: number;
  breath: number;
  cleanup: number;
  harshness: number;
  reverb: number;
}

export interface Settings {
  separation_model: string;
  denoise_separation: boolean;
  device: Device;
  preset: SoundPreset;
  instrument: InstrumentKind;
  tuning: {
    enabled: boolean;
    strength: number;
    drift: number;
    retune_speed: number;
    vibrato: number;
    preserve_formants: boolean;
    scale: ScaleMode;
  };
  vocal: VocalSettings;
  instrument_blend: number;
  warmth_db: number;
  room: number;
  loudness: { lufs: number; ceiling_dbtp: number };
  range: { take: number | null; start_sec: number | null; end_sec: number | null };
  sync_offset_sec: number | null;
  video: { aspect: Aspect; style: VideoStyle; quality: VideoQuality; fade_in_sec: number; fade_out_sec: number };
  social: {
    title: string;
    artist: string;
    language: Language;
    thumbnail_at_sec: number | null;
    extra_hashtags: string[];
  };
}

export interface Project {
  id: string;
  name: string;
  created_at: string;
  updated_at: string;
  sources: Sources;
  media: { audio: MediaInfo | null; video: MediaInfo | null };
  analysis: Analysis | null;
  settings: Settings;
  last_render: RenderInfo | null;
  exports: ExportRecord[];
}

export interface ProjectSummary {
  id: string;
  name: string;
  updated_at: string;
  has_video: boolean;
  analyzed: boolean;
  duration_sec: number | null;
  key: Key | null;
}

export interface ExportRequest {
  output_dir: string | null;
  wav: boolean;
  audio_formats: AudioCodec[];
  video: boolean;
  thumbnail: boolean;
  caption: boolean;
}

export interface ModelStatus {
  spec: {
    id: string;
    name: string;
    description: string;
    file: string;
    url: string;
    size_bytes: number;
    license: string;
    source: string;
  };
  downloaded: boolean;
  path: string;
}

export interface SystemInfo {
  version: string;
  ffmpeg: string | null;
  ffmpeg_error: string | null;
  nvenc: boolean;
  gpu_runtime: { supported: boolean; installed: boolean; dir: string; download_bytes: number };
  models: ModelStatus[];
  models_dir: string;
  projects_dir: string;
}

export type Stage =
  | "prepare"
  | "download"
  | "separate"
  | "pitch"
  | "analyze"
  | "sync"
  | "render"
  | "video"
  | "social"
  | "export";

export type JobEvent =
  | { job: string; type: "stage"; stage: Stage; label: string }
  | { job: string; type: "progress"; stage: Stage; fraction: number }
  | { job: string; type: "log"; message: string };

export interface CommandError {
  message: string;
  cancelled: boolean;
}
