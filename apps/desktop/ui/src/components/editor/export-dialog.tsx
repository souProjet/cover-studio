"use client";

import { FileAudio, FileText, FileVideo, FolderOpen, Image as ImageIcon, X } from "lucide-react";
import { useEffect, useState } from "react";
import { api, pickFolder, revealFile } from "@/lib/api";
import { useI18n } from "@/lib/i18n";
import { useJobs } from "@/lib/jobs";
import type { AudioCodec, ExportKind, ExportRecord, Project } from "@/lib/types";
import { basename, cn } from "@/lib/utils";
import { Button, SectionTitle } from "../ui";

type AudioFormat = "wav" | AudioCodec;
const FORMATS: { id: AudioFormat; label: string }[] = [
  { id: "wav", label: "WAV 24-bit" },
  { id: "aac", label: "M4A (AAC)" },
  { id: "mp3", label: "MP3" },
  { id: "flac", label: "FLAC" },
];

const ICONS: Record<ExportKind, typeof FileAudio> = {
  wav: FileAudio,
  audio: FileAudio,
  video: FileVideo,
  thumbnail: ImageIcon,
  caption: FileText,
};

function Check({ checked, onChange, children }: { checked: boolean; onChange: (v: boolean) => void; children: React.ReactNode }) {
  return (
    <label
      className={cn(
        "flex cursor-pointer items-center gap-2.5 rounded-lg border px-3 py-2 text-sm transition-colors",
        checked ? "border-amber/60 bg-amber/10" : "border-line hover:border-muted",
      )}
    >
      <input type="checkbox" className="accent-[#f2a541]" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      {children}
    </label>
  );
}

export function ExportDialog({
  project,
  onClose,
  onDone,
}: {
  project: Project;
  onClose: () => void;
  onDone: () => void;
}) {
  const { t } = useI18n();
  const jobs = useJobs();
  const hasVideo = !!project.sources.video;
  const [formats, setFormats] = useState<Set<AudioFormat>>(new Set(["wav", "aac"]));
  const [video, setVideo] = useState(hasVideo);
  const [thumbnail, setThumbnail] = useState(true);
  const [caption, setCaption] = useState(true);
  const [dir, setDir] = useState<string | null>(null);
  const [defaultDir, setDefaultDir] = useState("");
  const [result, setResult] = useState<ExportRecord | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api.defaultOutputDir(project.id).then(setDefaultDir).catch(() => {});
  }, [project.id]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && !busy && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, onClose]);

  const toggleFormat = (f: AudioFormat, on: boolean) =>
    setFormats((s) => {
      const n = new Set(s);
      if (on) n.add(f);
      else n.delete(f);
      return n;
    });

  async function go() {
    setBusy(true);
    try {
      const r = await jobs.run(t("export.title"), (job) =>
        api.export(
          project.id,
          {
            output_dir: dir,
            wav: formats.has("wav"),
            audio_formats: [...formats].filter((f): f is AudioCodec => f !== "wav"),
            video: video && hasVideo,
            thumbnail,
            caption,
          },
          job,
        ),
      );
      setResult(r);
      onDone();
    } catch {
      // The job bar shows the error.
    } finally {
      setBusy(false);
    }
  }

  const nothing = formats.size === 0 && !video && !thumbnail && !caption;

  return (
    <div className="fixed inset-0 z-40 grid place-items-center bg-black/55 p-6 backdrop-blur-sm" onMouseDown={(e) => e.target === e.currentTarget && !busy && onClose()}>
      <div role="dialog" aria-modal aria-labelledby="export-title" className="panel w-full max-w-lg p-6 shadow-2xl">
        <div className="mb-5 flex items-center justify-between">
          <h2 id="export-title" className="font-display text-2xl font-bold tracking-tight">
            {result ? t("export.done") : t("export.title")}
          </h2>
          <Button size="icon" variant="ghost" onClick={onClose} disabled={busy} aria-label={t("export.close")}>
            <X className="h-4 w-4" />
          </Button>
        </div>

        {result ? (
          <div className="grid gap-4">
            <ul className="grid gap-1.5">
              {result.files.map((f) => {
                const Icon = ICONS[f.kind];
                return (
                  <li key={f.path} className="flex items-center gap-3 rounded-lg bg-console px-3 py-2 text-sm">
                    <Icon className="h-4 w-4 shrink-0 text-amber" />
                    <span className="min-w-0 flex-1 truncate font-mono text-xs">{basename(f.path)}</span>
                  </li>
                );
              })}
            </ul>
            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={onClose}>
                {t("export.close")}
              </Button>
              <Button variant="primary" onClick={() => result.files[0] && revealFile(result.files[0].path)}>
                <FolderOpen className="h-4 w-4" />
                {t("export.reveal")}
              </Button>
            </div>
          </div>
        ) : (
          <div className="grid gap-5">
            <div className="grid gap-2">
              <SectionTitle>{t("export.audio")}</SectionTitle>
              <div className="grid grid-cols-2 gap-2">
                {FORMATS.map((f) => (
                  <Check key={f.id} checked={formats.has(f.id)} onChange={(v) => toggleFormat(f.id, v)}>
                    {f.label}
                  </Check>
                ))}
              </div>
            </div>
            <div className="grid gap-2">
              <SectionTitle>{t("tab.post")}</SectionTitle>
              <div className="grid grid-cols-3 gap-2">
                {hasVideo && (
                  <Check checked={video} onChange={setVideo}>
                    {t("export.video")}
                  </Check>
                )}
                <Check checked={thumbnail} onChange={setThumbnail}>
                  {t("export.thumbnail")}
                </Check>
                <Check checked={caption} onChange={setCaption}>
                  {t("export.caption")}
                </Check>
              </div>
            </div>
            <div className="grid gap-2">
              <SectionTitle>{t("export.folder")}</SectionTitle>
              <div className="flex items-center gap-2">
                <span className="min-w-0 flex-1 truncate rounded-lg border border-line bg-console px-3 py-2 font-mono text-xs text-muted">
                  {dir ?? defaultDir}
                </span>
                <Button size="sm" onClick={async () => setDir((await pickFolder()) ?? dir)}>
                  {t("export.change")}
                </Button>
              </div>
            </div>
            <div className="flex justify-end">
              <Button variant="primary" size="lg" disabled={busy || nothing} onClick={go}>
                {t("export.go")}
              </Button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
