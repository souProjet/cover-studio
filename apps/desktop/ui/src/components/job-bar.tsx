"use client";

import { Check, X } from "lucide-react";
import { useI18n } from "@/lib/i18n";
import { useJobs } from "@/lib/jobs";
import type { Stage } from "@/lib/types";
import { Button, Progress } from "./ui";

const STAGES: Record<Stage, { fr: string; en: string }> = {
  prepare: { fr: "Préparation de l'audio", en: "Preparing audio" },
  download: { fr: "Téléchargement", en: "Downloading" },
  separate: { fr: "Séparation voix / instrument", en: "Separating voice and instrument" },
  pitch: { fr: "Suivi de la mélodie chantée", en: "Tracking the sung melody" },
  analyze: { fr: "Recherche des prises et de la tonalité", en: "Finding takes and key" },
  sync: { fr: "Synchronisation audio / vidéo", en: "Syncing audio and video" },
  render: { fr: "Traitement de la voix et de l'instrument", en: "Processing voice and instrument" },
  video: { fr: "Rendu de la vidéo", en: "Rendering the video" },
  social: { fr: "Création de la miniature", en: "Creating the thumbnail" },
  export: { fr: "Écriture des fichiers", en: "Writing files" },
};

/** Floating status for the running long operation. */
export function JobBar() {
  const { current, cancel, dismiss } = useJobs();
  const { t, lang } = useI18n();
  if (!current) return null;
  const stage = current.stage ? STAGES[current.stage][lang] : current.title;
  // Downloads carry the model or package name in their label.
  const detail = current.stage === "download" ? current.label.replace(/^Downloading\s*/, "") : "";
  return (
    <div
      role="status"
      aria-live="polite"
      className="panel fixed bottom-5 left-1/2 z-50 w-[min(560px,calc(100vw-2rem))] -translate-x-1/2 px-5 py-4 shadow-2xl shadow-black/40"
    >
      <div className="mb-2 flex items-center justify-between gap-4">
        <div className="min-w-0">
          <div className="truncate text-sm font-medium">
            {current.status === "failed" ? t("job.failed") : current.status === "cancelled" ? t("job.cancelled") : stage}
            {detail && <span className="text-muted"> · {detail}</span>}
          </div>
          {current.status === "failed" && <div className="mt-1 text-xs break-words text-danger">{current.error}</div>}
        </div>
        {current.status === "running" ? (
          <Button size="sm" variant="ghost" onClick={cancel}>
            {t("job.cancel")}
          </Button>
        ) : current.status === "done" ? (
          <Check className="h-5 w-5 text-ok" />
        ) : (
          <Button size="icon" variant="ghost" onClick={dismiss} aria-label={t("export.close")}>
            <X className="h-4 w-4" />
          </Button>
        )}
      </div>
      {current.status === "running" && <Progress value={current.fraction} />}
    </div>
  );
}
