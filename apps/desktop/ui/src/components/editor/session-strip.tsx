"use client";

import { useMemo, useRef, useState } from "react";
import { useI18n } from "@/lib/i18n";
import type { Analysis } from "@/lib/types";
import { cn, fmtTime } from "@/lib/utils";
import { WaveCanvas } from "./wave-canvas";

type Drag = { kind: "new"; anchor: number } | { kind: "start" | "end" };

/**
 * The whole recording with its takes. Takes are tape labels above the
 * waveform; the selected range is lit, and can be dragged freely.
 */
export function SessionStrip({
  analysis,
  range,
  selectedTake,
  playhead,
  onSelectTake,
  onRange,
}: {
  analysis: Analysis;
  range: [number, number];
  selectedTake: number | null;
  playhead: number | null;
  onSelectTake: (index: number) => void;
  onRange: (r: [number, number]) => void;
}) {
  const { t } = useI18n();
  const box = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<Drag | null>(null);
  const [live, setLive] = useState<[number, number] | null>(null);
  const d = analysis.duration_sec;
  const shown = live ?? range;

  const layers = useMemo(
    () => [
      { peaks: analysis.waveform.peaks, color: "#5e7479" },
      { peaks: analysis.vocal_waveform.peaks, color: "#f2a541", gain: 0.85 },
    ],
    [analysis],
  );

  const timeAt = (clientX: number) => {
    const r = box.current!.getBoundingClientRect();
    return Math.min(d, Math.max(0, ((clientX - r.left) / r.width) * d));
  };
  const pct = (sec: number) => `${(sec / d) * 100}%`;

  function down(e: React.PointerEvent, kind: Drag["kind"]) {
    e.stopPropagation();
    (e.target as Element).setPointerCapture(e.pointerId);
    const at = timeAt(e.clientX);
    setDrag(kind === "new" ? { kind, anchor: at } : { kind });
    if (kind === "new") setLive([at, at]);
  }

  function move(e: React.PointerEvent) {
    if (!drag) return;
    const at = timeAt(e.clientX);
    if (drag.kind === "new") setLive([Math.min(drag.anchor, at), Math.max(drag.anchor, at)]);
    else if (drag.kind === "start") setLive([Math.min(at, shown[1] - 1), shown[1]]);
    else setLive([shown[0], Math.max(at, shown[0] + 1)]);
  }

  function up() {
    if (live && live[1] - live[0] >= 1) onRange(live);
    else if (drag?.kind === "new" && live) {
      // A click selects the take under the pointer.
      const take = analysis.takes.find((tk) => live[0] >= tk.start_sec && live[0] <= tk.end_sec);
      if (take) onSelectTake(take.index);
    }
    setDrag(null);
    setLive(null);
  }

  return (
    <div className="select-none">
      <div className="relative mb-1 h-8">
        {analysis.takes.map((tk) => {
          const best = tk.index === analysis.best_take;
          const active = tk.index === selectedTake;
          return (
            <button
              key={tk.index}
              type="button"
              onClick={() => onSelectTake(tk.index)}
              title={`${fmtTime(tk.start_sec)} – ${fmtTime(tk.end_sec)}`}
              className={cn(
                "tape absolute top-1 flex h-6 cursor-pointer items-center gap-1.5 overflow-hidden px-2 text-[11px] font-semibold whitespace-nowrap transition-[filter,transform]",
                active ? "-translate-y-0.5" : "opacity-70 hover:opacity-100",
                best && "ring-0",
              )}
              style={{ left: pct(tk.start_sec), maxWidth: `calc(${pct(tk.end_sec - tk.start_sec)} - 4px)` }}
            >
              <span>
                {t("editor.take")} {tk.index + 1}
              </span>
              {best && <span className="rounded-sm bg-[#3a3226] px-1 text-[9px] text-tape uppercase">{t("editor.best")}</span>}
            </button>
          );
        })}
      </div>

      <div
        ref={box}
        className="relative h-28 cursor-crosshair overflow-hidden rounded-lg border border-line bg-console"
        onPointerDown={(e) => down(e, "new")}
        onPointerMove={move}
        onPointerUp={up}
        onPointerCancel={up}
      >
        <WaveCanvas layers={layers} duration={d} highlight={shown} className="absolute inset-0 h-full w-full" />
        {analysis.takes.map((tk) => (
          <div
            key={tk.index}
            className="pointer-events-none absolute inset-y-0 border-l border-dashed border-tape/25"
            style={{ left: pct(tk.start_sec) }}
          />
        ))}
        <div
          className="pointer-events-none absolute inset-y-0 border-y-2 border-amber/70"
          style={{ left: pct(shown[0]), width: pct(shown[1] - shown[0]) }}
        />
        {(["start", "end"] as const).map((k) => (
          <div
            key={k}
            role="slider"
            aria-label={k}
            aria-valuenow={k === "start" ? shown[0] : shown[1]}
            tabIndex={-1}
            onPointerDown={(e) => down(e, k)}
            className="absolute inset-y-0 z-10 w-3 -translate-x-1/2 cursor-ew-resize"
            style={{ left: pct(k === "start" ? shown[0] : shown[1]) }}
          >
            <div className="mx-auto h-full w-[3px] rounded-full bg-amber" />
          </div>
        ))}
        {playhead !== null && (
          <div className="pointer-events-none absolute inset-y-0 w-px bg-ink" style={{ left: pct(playhead) }} />
        )}
      </div>
      <div className="mt-1.5 flex justify-between font-mono text-[11px] text-faint">
        <span>0:00</span>
        <span className="text-muted">
          {fmtTime(shown[0])} – {fmtTime(shown[1])} · {fmtTime(shown[1] - shown[0], 0)}
        </span>
        <span>{fmtTime(d, 0)}</span>
      </div>
    </div>
  );
}
