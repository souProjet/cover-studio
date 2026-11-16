"use client";

import { Film, Mic, Trash2, Video } from "lucide-react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { AppHeader } from "@/components/app-header";
import { Button, SectionTitle } from "@/components/ui";
import { api, mediaKind, onFileDrop, pickFile } from "@/lib/api";
import { useI18n } from "@/lib/i18n";
import type { ProjectSummary } from "@/lib/types";
import { basename, cn, errorMessage, fmtTime, keyName } from "@/lib/utils";

function stem(path: string) {
  return basename(path).replace(/\.[^.]+$/, "");
}

function InputChannel({
  n,
  icon: Icon,
  title,
  optional,
  path,
  onPick,
  onClear,
}: {
  n: number;
  icon: typeof Video;
  title: string;
  optional?: boolean;
  path: string | null;
  onPick: () => void;
  onClear: () => void;
}) {
  const { t } = useI18n();
  return (
    <div className={cn("flex items-center gap-4 rounded-xl border bg-console/60 px-4 py-3", path ? "border-amber/50" : "border-line")}>
      <span className="font-mono text-[11px] text-faint">CH{n}</span>
      <span
        className={cn(
          "h-2 w-2 shrink-0 rounded-full",
          path ? "bg-amber shadow-[0_0_10px_var(--color-amber)]" : "bg-line",
        )}
      />
      <Icon className="h-4 w-4 shrink-0 text-muted" />
      <div className="min-w-0 flex-1">
        <div className="text-sm">
          {title} {optional && <span className="text-faint">· {t("home.optional")}</span>}
        </div>
        {path && <div className="truncate font-mono text-xs text-muted">{basename(path)}</div>}
      </div>
      {path ? (
        <Button size="sm" variant="ghost" onClick={onClear}>
          {t("home.remove")}
        </Button>
      ) : (
        <Button size="sm" onClick={onPick}>
          {t("home.choose")}
        </Button>
      )}
    </div>
  );
}

export default function Home() {
  const { t, lang } = useI18n();
  const router = useRouter();
  const [projects, setProjects] = useState<ProjectSummary[] | null>(null);
  const [video, setVideo] = useState<string | null>(null);
  const [audio, setAudio] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [hover, setHover] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(() => {
    api.listProjects().then(setProjects).catch((e) => setError(errorMessage(e)));
  }, []);
  useEffect(refresh, [refresh]);

  const addPaths = useCallback((paths: string[]) => {
    for (const p of paths) {
      const kind = mediaKind(p);
      if (kind === "video") setVideo(p);
      else if (kind === "audio") setAudio(p);
    }
  }, []);

  useEffect(() => {
    let off: (() => void) | undefined;
    onFileDrop(addPaths, setHover).then((f) => (off = f));
    return () => off?.();
  }, [addPaths]);

  const suggested = audio ? stem(audio) : video ? stem(video) : "";

  async function create() {
    setBusy(true);
    setError(null);
    try {
      const p = await api.createProject(name.trim() || suggested || null, audio, video);
      router.push(`/project/?id=${p.id}&analyze=1`);
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  }

  async function remove(id: string) {
    if (!confirm(t("home.confirmDelete"))) return;
    await api.deleteProject(id).catch((e) => setError(errorMessage(e)));
    refresh();
  }

  return (
    <div className="min-h-screen">
      <AppHeader />
      <main className="mx-auto grid max-w-6xl gap-10 px-6 py-10 lg:grid-cols-[1.15fr_1fr]">
        <section>
          <h1 className="font-display text-4xl font-extrabold tracking-[-0.03em]">{t("home.title")}</h1>
          <p className="mt-2 text-muted">{t("app.tagline")}</p>

          <div
            className={cn(
              "panel mt-8 p-5 transition-[border-color,box-shadow]",
              hover && "border-amber shadow-[0_0_0_4px_rgb(242_165_65/0.15)]",
            )}
          >
            <div className="mb-5 flex items-start gap-4">
              <Film className="mt-0.5 h-5 w-5 shrink-0 text-amber" />
              <div>
                <div className="font-medium">{t("home.drop")}</div>
                <div className="mt-1 text-sm text-muted">{t("home.dropHint")}</div>
              </div>
            </div>
            <div className="grid gap-2">
              <InputChannel
                n={1}
                icon={Video}
                title={t("home.video")}
                path={video}
                onPick={async () => setVideo((await pickFile("video")) ?? video)}
                onClear={() => setVideo(null)}
              />
              <InputChannel
                n={2}
                icon={Mic}
                title={t("home.audio")}
                optional
                path={audio}
                onPick={async () => setAudio((await pickFile("audio")) ?? audio)}
                onClear={() => setAudio(null)}
              />
            </div>
            <div className="mt-5 flex items-end gap-3">
              <div className="grid flex-1 gap-1">
                <label htmlFor="name" className="text-[13px] font-medium text-muted">
                  {t("home.name")}
                </label>
                <input
                  id="name"
                  value={name}
                  placeholder={suggested}
                  onChange={(e) => setName(e.target.value)}
                  className="h-10 rounded-lg border border-line bg-console px-3 text-sm outline-none placeholder:text-faint focus:border-amber"
                />
              </div>
              <Button variant="primary" size="md" disabled={(!video && !audio) || busy} onClick={create}>
                {t("home.create")}
              </Button>
            </div>
            {error && <p className="mt-3 text-sm text-danger">{error}</p>}
          </div>
          <p className="mt-3 text-xs text-faint">{t("home.needModels")}</p>
        </section>

        <section>
          <SectionTitle className="mb-4 pt-3">{t("home.recent")}</SectionTitle>
          {projects && projects.length === 0 && <p className="text-sm text-muted">{t("home.empty")}</p>}
          <ul className="grid gap-2">
            {projects?.map((p) => (
              <li key={p.id} className="group flex items-center gap-1">
                <Link
                  href={`/project/?id=${p.id}`}
                  className="flex min-w-0 flex-1 items-center gap-4 rounded-xl border border-transparent px-4 py-3 transition-colors hover:border-line hover:bg-panel"
                >
                  <span
                    aria-hidden
                    className="tape flex h-7 min-w-14 items-center justify-center px-2 text-[11px] font-semibold"
                  >
                    {p.duration_sec ? fmtTime(p.duration_sec, 0) : "--:--"}
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="truncate font-medium">{p.name}</div>
                    <div className="mt-0.5 flex gap-3 text-xs text-muted">
                      {p.key && <span>{keyName(p.key, lang)}</span>}
                      {p.has_video && <span>{t("home.video").toLowerCase()}</span>}
                      <span className={p.analyzed ? "" : "text-amber"}>
                        {p.analyzed ? t("home.analyzed") : t("home.notAnalyzed")}
                      </span>
                    </div>
                  </div>
                  <span className="font-mono text-[11px] text-faint">
                    {new Date(p.updated_at).toLocaleDateString()}
                  </span>
                </Link>
                <button
                  type="button"
                  aria-label={t("home.delete")}
                  onClick={() => remove(p.id)}
                  className="cursor-pointer rounded-md p-2 text-faint opacity-0 transition-opacity group-hover:opacity-100 hover:text-danger focus:opacity-100"
                >
                  <Trash2 className="h-4 w-4" />
                </button>
              </li>
            ))}
          </ul>
        </section>
      </main>
    </div>
  );
}
