"use client";

import { Pause, Play, RefreshCw } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { AbPlayer, NativePlayer, type PreviewPlayer } from "@/lib/ab-player";
import { api, fileBytes, fileUrl, inTauri } from "@/lib/api";
import { type TKey, useI18n } from "@/lib/i18n";
import type { Aspect, Project } from "@/lib/types";
import { cn, errorMessage, fmtTime } from "@/lib/utils";
import { Button } from "../ui";
import { WaveCanvas } from "./wave-canvas";

const ASPECT_RATIO: Record<Aspect, string | null> = {
  vertical: "9 / 16",
  landscape: "16 / 9",
  square: "1 / 1",
  source: null,
};

/** Video picture following the audio clock, framed like the export. */
function Picture({ project, absTime, playing }: { project: Project; absTime: number; playing: boolean }) {
  const video = useRef<HTMLVideoElement>(null);
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [frame, setFrame] = useState<string | null>(null);
  const offset = project.settings.sync_offset_sec ?? project.analysis?.sync?.offset_sec ?? 0;
  const info = project.media.video?.video;
  const ratio =
    ASPECT_RATIO[project.settings.video.aspect] ?? (info ? `${info.width} / ${info.height}` : "16 / 9");

  useEffect(() => {
    if (project.sources.video) fileUrl(project.sources.video).then((u) => (u ? setSrc(u) : setFailed(true)));
  }, [project.sources.video]);

  // Keep the video within a frame of the audio clock.
  useEffect(() => {
    const v = video.current;
    if (!v || failed || !src) return;
    const target = absTime + offset;
    if (Math.abs(v.currentTime - target) > (playing ? 0.12 : 0.02)) v.currentTime = Math.max(0, target);
    if (playing && v.paused) v.play().catch(() => {});
    if (!playing && !v.paused) v.pause();
  }, [absTime, offset, playing, failed, src]);

  // Fallback when the webview cannot decode the codec (HEVC 10-bit...): stills from ffmpeg.
  useEffect(() => {
    if (!failed || playing) return;
    const id = setTimeout(() => {
      api
        .videoFrame(project.id, absTime, 720)
        .then((p) => (p ? fileUrl(p) : null))
        .then(setFrame)
        .catch(() => {});
    }, 120);
    return () => clearTimeout(id);
  }, [failed, playing, absTime, project.id, offset]);

  return (
    <div className="flex h-full items-center justify-center">
      <div
        className="relative max-h-full max-w-full overflow-hidden rounded-lg bg-black shadow-xl shadow-black/40"
        style={{ aspectRatio: ratio, height: "100%" }}
      >
        {!failed && src ? (
          <video
            ref={video}
            src={src}
            muted
            playsInline
            preload="auto"
            onError={() => setFailed(true)}
            className="absolute inset-0 h-full w-full object-cover"
          />
        ) : frame ? (
          // eslint-disable-next-line @next/next/no-img-element
          <img src={frame} alt="" className="absolute inset-0 h-full w-full object-cover" />
        ) : (
          <div className="absolute inset-0 grid place-items-center text-xs text-faint">
            {fmtTime(absTime + offset)}
          </div>
        )}
      </div>
    </div>
  );
}

export function Preview({
  project,
  stale,
  onRender,
  onPlayhead,
}: {
  project: Project;
  stale: boolean;
  onRender: () => void;
  onPlayhead: (abs: number | null) => void;
}) {
  const { t } = useI18n();
  const r = project.last_render;
  const renderKey = r ? `${r.at}|${r.processed}` : null;
  const player = useRef<PreviewPlayer | null>(null);
  const [ready, setReady] = useState(false);
  const [ab, setAb] = useState<"studio" | "original">("studio");
  const [playing, setPlaying] = useState(false);
  const [time, setTime] = useState(0);
  const [playError, setPlayError] = useState<{ key: TKey; detail?: string } | null>(null);
  const wave = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setPlaying(false);
    setTime(0);
    setPlayError(null);
    setReady(false);
    if (!r) return;
    let live = true;
    let loaded: PreviewPlayer | null = null;
    const load = inTauri()
      ? NativePlayer.load(project.id)
      : Promise.all([fileBytes(r.processed, r.at), fileBytes(r.original, r.at)]).then(([a, b]) => AbPlayer.load(a, b));
    load
      .then((p) => {
        // The native player is shared: a stale load must not unload the current one.
        if (!live) return p instanceof NativePlayer ? undefined : p.dispose();
        loaded = player.current = p;
        setReady(true);
      })
      .catch((e) => {
        if (!live) return;
        const decode = e instanceof DOMException && e.name === "EncodingError";
        setPlayError({ key: decode ? "preview.unsupported" : "preview.playFailed", detail: errorMessage(e) });
      });
    return () => {
      live = false;
      loaded?.dispose();
      if (player.current === loaded) player.current = null;
    };
  }, [renderKey, project.id]);

  useEffect(() => {
    player.current?.setSide(ab === "studio" ? "a" : "b");
  }, [ab, ready]);

  function apply(s: { time: number; playing: boolean }) {
    setTime(s.time);
    setPlaying(s.playing);
  }

  useEffect(() => {
    onPlayhead(r ? r.start_sec + time : null);
  }, [time, r, onPlayhead]);

  useEffect(() => {
    if (!playing) return;
    const id = setInterval(() => {
      player.current
        ?.state()
        .then((s) => {
          setTime(s.time);
          if (!s.playing) setPlaying(false);
        })
        .catch(() => {});
    }, 50);
    return () => clearInterval(id);
  }, [playing]);

  function toggle() {
    const p = player.current;
    if (!p) {
      if (r && !playError) setPlayError({ key: "preview.notReady" });
      return;
    }
    (playing ? p.pause() : p.play())
      .then((s) => {
        setPlayError(null);
        apply(s);
      })
      .catch((e) => setPlayError({ key: "preview.playFailed", detail: errorMessage(e) }));
  }

  function seek(e: React.PointerEvent) {
    if (!r || !wave.current) return;
    const rect = wave.current.getBoundingClientRect();
    const tt = ((e.clientX - rect.left) / rect.width) * (r.end_sec - r.start_sec);
    setTime(tt);
    player.current
      ?.seek(tt)
      .then(apply)
      .catch(() => {});
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).closest("input,textarea,select")) return;
      if (e.code === "Space") {
        e.preventDefault();
        toggle();
      } else if (e.key.toLowerCase() === "b") setAb((v) => (v === "studio" ? "original" : "studio"));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const layers = useMemo(() => (r ? [{ peaks: r.waveform.peaks, color: ab === "studio" ? "#f2a541" : "#7fb7be" }] : []), [r, ab]);
  const len = r ? r.end_sec - r.start_sec : 0;
  const hasVideo = !!project.sources.video;

  return (
    <div className="grid h-full min-h-0 grid-rows-[1fr_auto] gap-4">
      <div className="min-h-0">
        {hasVideo ? (
          <Picture project={project} absTime={(r?.start_sec ?? 0) + time} playing={playing} />
        ) : (
          <div className="grid h-full place-items-center rounded-lg border border-dashed border-line text-sm text-faint">
            {t("video.none")}
          </div>
        )}
      </div>

      <div className="panel p-4">
        <div className="flex items-center gap-3">
          <Button
            variant="primary"
            size="icon"
            className="h-11 w-11 rounded-full"
            disabled={!r}
            onClick={toggle}
            aria-label={playing ? "Pause" : "Play"}
          >
            {playing ? <Pause className="h-5 w-5" /> : <Play className="ml-0.5 h-5 w-5" />}
          </Button>

          <div
            role="radiogroup"
            aria-label="A/B"
            className="flex rounded-full border border-line bg-console p-1 font-mono text-[11px] uppercase tracking-wider"
          >
            {(["original", "studio"] as const).map((k) => (
              <button
                key={k}
                type="button"
                role="radio"
                aria-checked={ab === k}
                disabled={!r}
                onClick={() => setAb(k)}
                className={cn(
                  "cursor-pointer rounded-full px-3 py-1 transition-colors",
                  ab === k ? (k === "studio" ? "bg-amber text-[#2a1a05]" : "bg-cyan text-[#0e2427]") : "text-muted",
                )}
              >
                {t(k === "studio" ? "preview.studio" : "preview.original")}
              </button>
            ))}
          </div>

          <span className="ml-auto font-mono text-xs text-muted tabular-nums">
            {fmtTime(time)} / {fmtTime(len)}
          </span>
          <Button size="sm" variant={stale || !r ? "primary" : "secondary"} onClick={onRender}>
            <RefreshCw className="h-3.5 w-3.5" />
            {t("preview.render")}
          </Button>
        </div>

        {r ? (
          <div ref={wave} onPointerDown={seek} className="relative mt-3 h-14 cursor-pointer">
            <WaveCanvas layers={layers} duration={len} className="absolute inset-0 h-full w-full" />
            <div className="pointer-events-none absolute inset-y-0 w-px bg-ink" style={{ left: `${(time / len) * 100}%` }} />
          </div>
        ) : (
          <p className="mt-3 text-sm text-muted">{t("preview.none")}</p>
        )}
        {playError && (
          <p role="alert" className="mt-2 text-sm text-danger">
            {t(playError.key)}
            {playError.detail && <span className="ml-1 font-mono text-xs opacity-80">({playError.detail})</span>}
          </p>
        )}
        <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs text-faint">
          {stale && r && <span className="text-amber">{t("preview.stale")}</span>}
          {r && (
            <>
              <span>{t("preview.levelMatched")}</span>
              {r.stats.notes_corrected > 0 && (
                <span>
                  {r.stats.notes_corrected} {t("preview.notes")}, {t("preview.mean")} {Math.round(r.stats.mean_correction_cents)}{" "}
                  {t("preview.cents")}
                </span>
              )}
              <span className="font-mono">{r.stats.loudness.integrated_lufs.toFixed(1)} LUFS</span>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
