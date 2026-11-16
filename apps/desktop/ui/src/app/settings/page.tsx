"use client";

import { Cpu, Download, FolderOpen, HardDrive } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { AppHeader } from "@/components/app-header";
import { Button, SectionTitle, Segmented } from "@/components/ui";
import { api, openExternal, revealFile } from "@/lib/api";
import { useI18n } from "@/lib/i18n";
import { useJobs } from "@/lib/jobs";
import type { SystemInfo } from "@/lib/types";
import { cn, errorMessage, fmtBytes } from "@/lib/utils";

function Row({ label, children, hint }: { label: string; children: React.ReactNode; hint?: React.ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-6 border-b border-line/60 py-4 last:border-0">
      <div className="min-w-0">
        <div className="text-sm">{label}</div>
        {hint && <div className="mt-1 max-w-md text-xs leading-relaxed text-faint">{hint}</div>}
      </div>
      <div className="flex shrink-0 items-center gap-2 text-sm">{children}</div>
    </div>
  );
}

function Led({ on }: { on: boolean }) {
  return (
    <span
      className={cn("inline-block h-2 w-2 rounded-full", on ? "bg-ok shadow-[0_0_8px_var(--color-ok)]" : "bg-line")}
    />
  );
}

export default function SettingsPage() {
  const { t, lang, setLang } = useI18n();
  const jobs = useJobs();
  const [info, setInfo] = useState<SystemInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api.systemInfo().then(setInfo).catch((e) => setError(errorMessage(e)));
  }, []);
  useEffect(refresh, [refresh]);

  const running = jobs.current?.status === "running";

  async function download(model: string) {
    await jobs.run(t("settings.download"), (job) => api.downloadModel(model, job)).catch(() => {});
    refresh();
  }

  async function installGpu() {
    await jobs.run(t("settings.gpu"), (job) => api.installGpuRuntime(job)).catch(() => {});
    refresh();
  }

  return (
    <div className="min-h-screen">
      <AppHeader />
      <main className="mx-auto max-w-3xl px-6 py-10">
        <h1 className="font-display text-4xl font-extrabold tracking-[-0.03em]">{t("settings.title")}</h1>
        {error && <p className="mt-4 text-sm text-danger">{error}</p>}

        <section className="panel mt-8 px-5 py-1">
          <SectionTitle className="pt-4">{t("settings.system")}</SectionTitle>
          <Row label={t("settings.ffmpeg")} hint={info?.ffmpeg_error ? t("settings.ffmpegMissing") : undefined}>
            <Led on={!!info?.ffmpeg} />
            <span className="max-w-64 truncate font-mono text-xs text-muted">
              {info?.ffmpeg?.replace(/^ffmpeg version /, "") ?? "—"}
            </span>
          </Row>
          <Row label={t("settings.nvenc")}>
            <Led on={!!info?.nvenc} />
            <span className="text-muted">{t(info?.nvenc ? "settings.available" : "settings.unavailable")}</span>
          </Row>
          <Row
            label={t("settings.gpu")}
            hint={info?.gpu_runtime.supported ? t("settings.gpuHint") : t("settings.gpuUnsupported")}
          >
            {info?.gpu_runtime.installed ? (
              <>
                <Led on />
                <span className="text-muted">{t("settings.gpuInstalled")}</span>
              </>
            ) : (
              info?.gpu_runtime.supported && (
                <Button size="sm" variant="primary" disabled={running} onClick={installGpu}>
                  <Cpu className="h-3.5 w-3.5" />
                  {t("settings.gpuInstall")} · {fmtBytes(info.gpu_runtime.download_bytes)}
                </Button>
              )
            )}
          </Row>
        </section>

        <section className="panel mt-6 px-5 py-1">
          <SectionTitle className="pt-4">{t("settings.models")}</SectionTitle>
          {info?.models.map((m) => (
            <Row
              key={m.spec.id}
              label={m.spec.name}
              hint={
                <>
                  {m.spec.description}
                  <br />
                  <button
                    type="button"
                    onClick={() => openExternal(m.spec.source)}
                    className="cursor-pointer text-left underline-offset-2 hover:underline"
                  >
                    {m.spec.license}
                  </button>
                </>
              }
            >
              <span className="font-mono text-xs text-faint">{fmtBytes(m.spec.size_bytes)}</span>
              {m.downloaded ? (
                <>
                  <Led on />
                  <span className="text-muted">{t("settings.downloaded")}</span>
                </>
              ) : (
                <Button size="sm" disabled={running} onClick={() => download(m.spec.id)}>
                  <Download className="h-3.5 w-3.5" />
                  {t("settings.download")}
                </Button>
              )}
            </Row>
          ))}
        </section>

        <section className="panel mt-6 px-5 py-1">
          <SectionTitle className="pt-4">{t("settings.folders")}</SectionTitle>
          {info &&
            [
              { label: t("settings.projects"), path: info.projects_dir },
              { label: t("settings.models"), path: info.models_dir },
            ].map((f) => (
              <Row key={f.label} label={f.label}>
                <span className="max-w-80 truncate font-mono text-xs text-muted">{f.path}</span>
                <Button size="icon" variant="ghost" aria-label={f.path} onClick={() => revealFile(f.path)}>
                  <FolderOpen className="h-4 w-4" />
                </Button>
              </Row>
            ))}
          <Row label={t("settings.language")}>
            <Segmented
              value={lang}
              onChange={setLang}
              className="w-48"
              options={[
                { value: "fr", label: "Français" },
                { value: "en", label: "English" },
              ]}
            />
          </Row>
          <Row label={t("settings.version")}>
            <HardDrive className="h-3.5 w-3.5 text-faint" />
            <span className="font-mono text-xs text-muted">{info?.version}</span>
          </Row>
        </section>
      </main>
    </div>
  );
}
