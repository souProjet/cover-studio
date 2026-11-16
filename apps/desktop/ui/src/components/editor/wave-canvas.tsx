"use client";

import { useEffect, useRef } from "react";

export interface WaveLayer {
  peaks: number[];
  color: string;
  /** Vertical scale relative to the canvas half-height. */
  gain?: number;
}

/** Mirrored peak bars for one or more layers, drawn at device resolution. */
export function WaveCanvas({
  layers,
  view,
  duration,
  highlight,
  dim = "rgb(31 43 48 / 0.62)",
  className,
}: {
  layers: WaveLayer[];
  /** Visible time window in seconds; defaults to the whole duration. */
  view?: [number, number];
  duration: number;
  /** Region drawn at full strength; the rest is dimmed. */
  highlight?: [number, number] | null;
  dim?: string;
  className?: string;
}) {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const draw = () => {
      const dpr = window.devicePixelRatio || 1;
      const w = canvas.clientWidth;
      const h = canvas.clientHeight;
      if (!w || !h) return;
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
      const ctx = canvas.getContext("2d")!;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      const [v0, v1] = view ?? [0, duration];
      const bar = 2;
      const gap = 1;
      const cols = Math.floor(w / (bar + gap));
      const mid = h / 2;
      for (const layer of layers) {
        const n = layer.peaks.length;
        if (!n) continue;
        // Normalise to the loudest peak so quiet recordings stay readable.
        const max = Math.max(1e-4, ...layer.peaks);
        ctx.fillStyle = layer.color;
        for (let c = 0; c < cols; c++) {
          const t0 = v0 + ((v1 - v0) * c) / cols;
          const t1 = v0 + ((v1 - v0) * (c + 1)) / cols;
          const i0 = Math.floor((t0 / duration) * n);
          const i1 = Math.max(i0 + 1, Math.ceil((t1 / duration) * n));
          let p = 0;
          for (let i = Math.max(0, i0); i < Math.min(n, i1); i++) p = Math.max(p, layer.peaks[i]);
          const a = Math.max(1, Math.sqrt(p / max) * (mid - 2) * (layer.gain ?? 1));
          ctx.fillRect(c * (bar + gap), mid - a, bar, a * 2);
        }
      }
      if (highlight) {
        const x0 = ((highlight[0] - v0) / (v1 - v0)) * w;
        const x1 = ((highlight[1] - v0) / (v1 - v0)) * w;
        ctx.fillStyle = dim;
        if (x0 > 0) ctx.fillRect(0, 0, x0, h);
        if (x1 < w) ctx.fillRect(x1, 0, w - x1, h);
      }
    };
    draw();
    const ro = new ResizeObserver(draw);
    ro.observe(canvas);
    return () => ro.disconnect();
  }, [layers, view, duration, highlight, dim]);

  return <canvas ref={ref} className={className} />;
}
