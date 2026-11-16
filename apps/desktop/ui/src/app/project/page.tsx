"use client";

import { Sparkles, Upload } from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AppHeader } from "@/components/app-header";
import { Controls } from "@/components/editor/controls";
import { ExportDialog } from "@/components/editor/export-dialog";
import { Preview } from "@/components/editor/preview";
import { SessionStrip } from "@/components/editor/session-strip";
import { Button } from "@/components/ui";
import { api } from "@/lib/api";
import { useI18n } from "@/lib/i18n";
import { useJobs } from "@/lib/jobs";
import type { Project, Settings } from "@/lib/types";
import { errorMessage, keyName } from "@/lib/utils";

/** Settings that change the rendered audio (video, post and sync do not). */
function audioKey(s: Settings) {
  const { video: _v, social: _s, sync_offset_sec: _o, ...rest } = s;
  return JSON.stringify(rest);
}

/** Mirrors `Project::effective_range` in the engine. */
function effectiveRange(p: Project): [number, number] | null {
  const a = p.analysis;
  if (!a) return null;
  const r = p.settings.range;
  const take = a.takes.find((t) => t.index === (r.take ?? a.best_take));
  let s = take?.start_sec ?? 0;
  let e = take?.end_sec ?? a.duration_sec;
  if (r.start_sec !== null) s = r.start_sec;
  if (r.end_sec !== null) e = r.end_sec;
  s = Math.min(Math.max(0, s), a.duration_sec);
  e = Math.min(Math.max(s, e), a.duration_sec);
  return [s, e];
}

function Editor() {
  const params = useSearchParams();
  const id = params.get("id") ?? "";
  const autoAnalyze = params.get("analyze") === "1";
  const router = useRouter();
  const { t, lang } = useI18n();
  const jobs = useJobs();
  const [project, setProject] = useState<Project | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [renderedKey, setRenderedKey] = useState<string | null>(null);
  const [playhead, setPlayhead] = useState<number | null>(null);
  const [exporting, setExporting] = useState(false);
  const saveTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const pending = useRef<Settings | null>(null);
  const started = useRef(false);

  /** Writes settings that are still waiting for the debounce. */
  const flush = useCallback(async () => {
    clearTimeout(saveTimer.current);
    const s = pending.current;
    pending.current = null;
    if (s) await api.updateSettings(id, s);
  }, [id]);

  const load = useCallback(async () => {
    try {
      const p = await api.getProject(id);
      setProject(p);
      if (p.last_render) setRenderedKey(audioKey(p.settings) + JSON.stringify(effectiveRange(p)));
      return p;
    } catch (e) {
      setError(errorMessage(e));
      return null;
    }
  }, [id]);

  const renderPreview = useCallback(async () => {
    try {
      await flush();
      const p = await jobs.run(t("preview.render"), (job) => api.renderPreview(id, job));
      setProject(p);
      setRenderedKey(audioKey(p.settings) + JSON.stringify(effectiveRange(p)));
    } catch {
      // Shown in the job bar.
    }
  }, [id, jobs, t, flush]);

  const analyze = useCallback(async () => {
    try {
      await flush();
      const p = await jobs.run(t("editor.analyze"), (job) => api.analyze(id, job));
      setProject(p);
      await renderPreview();
    } catch {
      // Shown in the job bar.
    }
  }, [id, jobs, t, renderPreview, flush]);

  useEffect(() => {
    if (!id) return;
    load().then((p) => {
      if (p && !p.analysis && autoAnalyze && !started.current) {
        started.current = true;
        router.replace(`/project/?id=${id}`);
        analyze();
      }
    });
  }, [id, load, autoAnalyze, analyze, router]);

  const update = useCallback(
    (settings: Settings) => {
      setProject((p) => (p ? { ...p, settings } : p));
      pending.current = settings;
      clearTimeout(saveTimer.current);
      saveTimer.current = setTimeout(() => {
        flush().catch((e) => setError(errorMessage(e)));
      }, 350);
    },
    [flush],
  );

  const range = useMemo(() => (project ? effectiveRange(project) : null), [project]);
  const stale = !!project && renderedKey !== audioKey(project.settings) + JSON.stringify(range);
  const running = jobs.current?.status === "running";

  if (error && !project) {
    return (
      <div className="grid min-h-screen place-items-center text-center">
        <div>
          <p className="text-danger">{error}</p>
          <Link href="/" className="mt-4 inline-block text-muted underline">
            {t("editor.back")}
          </Link>
        </div>
      </div>
    );
  }
  if (!project) return <div className="min-h-screen" />;

  const a = project.analysis;
  const selectedTake = project.settings.range.start_sec === null && project.settings.range.end_sec === null
    ? (project.settings.range.take ?? a?.best_take ?? null)
    : null;

  return (
    <div className="flex h-screen flex-col overflow-hidden">
      <AppHeader>
        <span className="text-line">/</span>
        <h1 className="truncate font-display text-lg font-bold tracking-tight">{project.name}</h1>
        {a && (
          <div className="ml-2 hidden items-center gap-3 font-mono text-[11px] text-muted md:flex">
            <span className="rounded border border-line px-1.5 py-0.5">{keyName(a.key, lang)}</span>
            <span className="rounded border border-line px-1.5 py-0.5">{a.provider}</span>
          </div>
        )}
        <div className="ml-auto flex items-center gap-2">
          {a && (
            <Button size="sm" variant="ghost" disabled={running} onClick={analyze}>
              {t("editor.reanalyze")}
            </Button>
          )}
          <Button size="sm" variant="primary" disabled={!a || running} onClick={() => flush().then(() => setExporting(true))}>
            <Upload className="h-3.5 w-3.5" />
            {t("editor.export")}
          </Button>
        </div>
      </AppHeader>

      {!a ? (
        <div className="grid flex-1 place-items-center p-8">
          <div className="max-w-md text-center">
            <Sparkles className="mx-auto mb-4 h-8 w-8 text-amber" />
            <h2 className="font-display text-2xl font-bold">{t("editor.notAnalyzed")}</h2>
            <p className="mt-2 text-muted">{t("editor.notAnalyzedHint")}</p>
            <Button className="mt-6" variant="primary" size="lg" disabled={running} onClick={analyze}>
              {t("editor.analyze")}
            </Button>
          </div>
        </div>
      ) : (
        <div className="grid min-h-0 flex-1 grid-cols-[minmax(0,1fr)_400px] gap-0">
          <div className="flex min-h-0 flex-col gap-5 p-6">
            <section aria-label={t("editor.session")}>
              <SessionStrip
                analysis={a}
                range={range!}
                selectedTake={selectedTake}
                playhead={playhead}
                onSelectTake={(take) =>
                  update({ ...project.settings, range: { take, start_sec: null, end_sec: null } })
                }
                onRange={([s, e]) =>
                  update({ ...project.settings, range: { take: null, start_sec: s, end_sec: e } })
                }
              />
            </section>
            <section className="min-h-0 flex-1" aria-label={t("preview.title")}>
              <Preview project={project} stale={stale} onRender={renderPreview} onPlayhead={setPlayhead} />
            </section>
          </div>
          <aside className="min-h-0 border-l border-line bg-panel/40">
            <Controls project={project} update={update} playhead={playhead} />
          </aside>
        </div>
      )}

      {exporting && (
        <ExportDialog project={project} onClose={() => setExporting(false)} onDone={() => load()} />
      )}
    </div>
  );
}

export default function ProjectPage() {
  return (
    <Suspense fallback={<div className="min-h-screen" />}>
      <Editor />
    </Suspense>
  );
}
